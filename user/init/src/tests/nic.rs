//! Ovladace sitovych karet s MMIO (`nic`): druha karta v QEMU je Intel
//! 82574L (`-device e1000e`) ve vlastni siti slirp. netd ji nepouziva
//! (bere prvni kartu, virtio-net), test mluvi s ovladacem primo
//! protokolem `netproto::link`: INFO, stav linky a ramce ARP na branu
//! slirp (10.0.2.2), ktera odpovida bez spojeni ven.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Startup, time};
use netproto::link;

use super::{Test, TestResult, devices, fail};
use crate::check;

pub const TESTS: &[Test] = &[
    ("e1000e: INFO, MAC z NVM a linka 1 Gb/s", info_and_link),
    ("e1000e: ARP na branu slirp", arp_reply),
    ("e1000e: 640 ramcu tam i zpet (kruhy dokola)", many_frames),
];

/// Prvni zprava ovladace (precte se jednou, i pri `--repeat`).
static INFO: spin::Once<Result<link::Info, String>> = spin::Once::new();
/// Stav linky ze zprav `LINK` (pocatecni z INFO).
static LINK: spin::Mutex<Option<link::Status>> = spin::Mutex::new(None);

const ETH_ARP: [u8; 2] = [0x08, 0x06];
const OUR_IP: [u8; 4] = [10, 0, 2, 15];
const GATEWAY: [u8; 4] = [10, 0, 2, 2];

/// Kanal k ovladaci druhe karty.
fn channel() -> Result<&'static Channel, String> {
    let d = devices().ok_or("spravce zarizeni nebezi")?;
    d.nets.first().ok_or_else(|| String::from("druha sitova karta (e1000e) nenalezena"))
}

fn info() -> Result<link::Info, String> {
    let ch = channel()?;
    INFO.call_once(|| {
        let m = ch
            .read(Some(time::deadline_after(Duration::from_secs(10))))
            .map_err(|e| fail("INFO od ovladace", e))?;
        let i = link::Info::decode(&m.data).map_err(|e| fail("INFO", e))?;
        *LINK.lock() = Some(link::Status {
            flags: i.flags,
            speed_mbps: i.speed_mbps,
        });
        Ok(i)
    })
    .clone()
}

/// Precte zpravy do `deadline`: `LINK` aktualizuje stav, ramce ARP
/// odpovedi od brany se pocitaji. Vraci pocet odpovedi; konci driv, kdyz
/// jich je `want`.
fn collect(ch: &Channel, mac: [u8; 6], want: usize, deadline: u64) -> Result<usize, String> {
    let mut replies = 0;
    while replies < want {
        let m = match ch.read(Some(deadline)) {
            Ok(m) => m,
            Err(Error::TimedOut) => break,
            Err(e) => return Err(fail("cteni od ovladace", e)),
        };
        match m.data.first() {
            Some(&link::LINK) => {
                if let Ok(s) = link::Status::decode(&m.data) {
                    *LINK.lock() = Some(s);
                }
            }
            Some(&link::FRAME) if is_arp_reply(&m.data[1..], mac) => replies += 1,
            _ => {}
        }
    }
    Ok(replies)
}

fn is_arp_reply(f: &[u8], mac: [u8; 6]) -> bool {
    f.len() >= 42
        && f[..6] == mac
        && f[12..14] == ETH_ARP
        && f[20..22] == [0, 2]
        && f[28..32] == GATEWAY
        && f[32..38] == mac
}

fn arp_request(mac: [u8; 6]) -> Vec<u8> {
    let mut f = Vec::with_capacity(1 + 42);
    f.push(link::FRAME);
    f.extend_from_slice(&[0xFF; 6]);
    f.extend_from_slice(&mac);
    f.extend_from_slice(&ETH_ARP);
    // Ethernet, IPv4, delky 6 a 4, dotaz.
    f.extend_from_slice(&[0, 1, 0x08, 0x00, 6, 4, 0, 1]);
    f.extend_from_slice(&mac);
    f.extend_from_slice(&OUR_IP);
    f.extend_from_slice(&[0; 6]);
    f.extend_from_slice(&GATEWAY);
    f
}

/// Pocka na pripojenou linku (autonegociace emulovane karty ~0,5 s).
fn wait_link_up(ch: &Channel, mac: [u8; 6]) -> Result<link::Status, String> {
    let deadline = time::deadline_after(Duration::from_secs(10));
    loop {
        if let Some(s) = *LINK.lock()
            && s.flags & link::FLAG_UP != 0
        {
            return Ok(s);
        }
        if time::now() > deadline {
            return Err(format!("linka se nepripojila do 10 s ({:?})", *LINK.lock()));
        }
        collect(ch, mac, 1, time::deadline_after(Duration::from_millis(200)))?;
    }
}

fn info_and_link(_: &Startup) -> TestResult {
    let i = info()?;
    // QEMU dava kartam MAC 52:54:00:...; mistni (02:...) = ovladac ji
    // z karty neprecetl.
    check!(i.mac[..3] == [0x52, 0x54, 0x00], format!("MAC {:02x?} neni z NVM karty", i.mac));
    check!(i.mtu == 1500, format!("MTU {}", i.mtu));
    let s = wait_link_up(channel()?, i.mac)?;
    check!(s.speed_mbps == 1000, format!("rychlost {} Mb/s", s.speed_mbps));
    check!(s.flags & link::FLAG_FULL_DUPLEX != 0, "linka bez plneho duplexu");
    Ok(())
}

fn arp_reply(_: &Startup) -> TestResult {
    let i = info()?;
    let ch = channel()?;
    wait_link_up(ch, i.mac)?;
    let req = arp_request(i.mac);
    for _ in 0..5 {
        ch.write(&req, Vec::new()).map_err(|e| fail("odeslani ARP", e))?;
        if collect(ch, i.mac, 1, time::deadline_after(Duration::from_secs(2)))? > 0 {
            return Ok(());
        }
    }
    Err("brana slirp na ARP neodpovedela (5 pokusu)".into())
}

fn many_frames(_: &Startup) -> TestResult {
    let i = info()?;
    let ch = channel()?;
    wait_link_up(ch, i.mac)?;
    // Odpovedi z drivejsich testu zahodit.
    collect(ch, i.mac, usize::MAX, time::deadline_after(Duration::from_millis(100)))?;
    let req = arp_request(i.mac);
    const ROUNDS: usize = 20;
    const BATCH: usize = 32;
    let mut total = 0;
    for round in 0..ROUNDS {
        let mut got = 0;
        // Ztraceny ramec (plna fronta kanalu) poslat znovu, nejvys 3x.
        for _ in 0..3 {
            for _ in got..BATCH {
                match ch.write(&req, Vec::new()) {
                    Ok(()) | Err(Error::ShouldWait) => {}
                    Err(e) => return Err(fail("odeslani ARP", e)),
                }
            }
            got += collect(ch, i.mac, BATCH - got, time::deadline_after(Duration::from_secs(2)))?;
            if got >= BATCH {
                break;
            }
        }
        check!(got >= BATCH, format!("kolo {round}: {got}/{BATCH} odpovedi"));
        total += BATCH;
    }
    // 640 > 2 x 256 (delka kruhu ovladace): oba kruhy dvakrat dokola.
    check!(total == ROUNDS * BATCH, "pocet ramcu");
    Ok(())
}
