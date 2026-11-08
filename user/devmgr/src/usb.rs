//! USB: spusteni ovladace radice xHCI a tridnich ovladacu podle zprav
//! `usb-interface` od nej.
//!
//! Radic hlasi kazde rozhrani (a svuj virtualni root hub) zpravou
//! `usb-interface <trida> <podtrida> <protokol> <vid> <pid>` (sestnactkove)
//! s kanalem rozhrani. Spravce podle tridy spusti `usb-hub` (09),
//! `usb-hid` (03) nebo `usb-storage` (08/06/50, disk pres Bulk-Only
//! Transport); tridni ovladace nedostanou zadny Resource, jen kanal
//! rozhrani (a `usb-hid` kanal ke sluzbe vstupu, jehoz druhy konec jde
//! initu zpravou `input`). `usb-storage` hlasi disky jako ovladace radicu
//! disku (`block` + kanal, nakonec `done`); spravce je preda initu.

use core::time::Duration;

use alloc::vec;
use alloc::vec::Vec;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Handle, Message, Process, Result, println};

/// Trida PCI radice xHCI: serial bus / USB / xHCI.
pub const XHCI_CLASS: (u8, u8, u8) = (0x0C, 0x03, 0x30);

/// Bezici radic.
pub struct Controller {
    pub _process: Process,
    pub channel: Channel,
    pub ready: bool,
    /// Radic skoncil (kanal zavreny): uz na nej necekat.
    pub closed: bool,
}

/// Zpracuje zpravu od radice. Vraci `true` pro `usb-ready`.
pub fn handle(
    fs: &Initfs,
    msg: Message,
    parent: &Channel,
    procs: &mut Vec<Process>,
) -> bool {
    let text = core::str::from_utf8(&msg.data).unwrap_or("");
    let mut w = text.split_whitespace();
    match w.next() {
        Some("usb-ready") => true,
        Some("usb-interface") => {
            let mut num = || w.next().and_then(|s| u16::from_str_radix(s, 16).ok());
            let (Some(class), Some(sub), Some(proto), Some(vid), Some(pid)) =
                (num(), num(), num(), num(), num())
            else {
                println!("devmgr: neplatna zprava od xhci: {text}");
                return false;
            };
            let Some(ch) = msg.handles.into_iter().next() else {
                return false;
            };
            match start_class(fs, (class as u8, sub as u8, proto as u8), ch, parent) {
                Ok(Some(p)) => procs.push(p),
                Ok(None) => println!(
                    "devmgr: usb {vid:04x}:{pid:04x} rozhrani {class:02x}/{sub:02x}/{proto:02x}: ovladac neni"
                ),
                Err(e) => println!("devmgr: tridni ovladac USB nelze spustit: {e}"),
            }
            false
        }
        _ => {
            println!("devmgr: neznama zprava od xhci: {text}");
            false
        }
    }
}

/// Jak dlouho cekat, nez ovladac disku USB nahlasi sve disky (TEST UNIT
/// READY ceka az 15 s na kazdy LUN).
const STORAGE_REPORT_TIMEOUT: Duration = Duration::from_secs(40);
/// Behem cekani posilat initu `busy` (init ceka na zpravu omezene dlouho).
const BUSY_INTERVAL: Duration = Duration::from_secs(5);

/// Spusti tridni ovladac pro rozhrani (trida, podtrida, protokol);
/// `Ok(None)` = trida bez ovladace (kanal se zavre).
fn start_class(fs: &Initfs, (class, sub, proto): (u8, u8, u8), iface: Handle, parent: &Channel) -> Result<Option<Process>> {
    if usbproto::storage::is_bot_disk(class, sub, proto) {
        return start_storage(fs, iface, parent).map(Some);
    }
    match class {
        0x09 => {
            let image = fs.find("usb-hub").ok_or(libos::Error::NotFound)?;
            let (p, _control) = libos::spawn("usb-hub", image, vec![StartHandle::user(iface)])?;
            Ok(Some(p))
        }
        0x03 => {
            let image = fs.find("usb-hid").ok_or(libos::Error::NotFound)?;
            let (client, driver) = Channel::create()?;
            let handles = vec![
                StartHandle::user(iface),
                StartHandle::user(driver.into_handle()),
            ];
            let (p, _control) = libos::spawn("usb-hid", image, handles)?;
            parent.write(b"input", vec![client.into_handle()])?;
            Ok(Some(p))
        }
        _ => Ok(None),
    }
}

/// Spusti `usb-storage` a initu preda kanaly k diskum, ktere nahlasi.
/// Ceka synchronne: disk pripojeny pri startu musi byt nahlasen pred
/// `done` (init na nem hleda souborovy system - zive USB).
fn start_storage(fs: &Initfs, iface: Handle, parent: &Channel) -> Result<Process> {
    let image = fs.find("usb-storage").ok_or(libos::Error::NotFound)?;
    let (p, control) = libos::spawn("usb-storage", image, vec![StartHandle::user(iface)])?;
    let deadline = libos::time::deadline_after(STORAGE_REPORT_TIMEOUT);
    loop {
        let slice = libos::time::deadline_after(BUSY_INTERVAL).min(deadline);
        let msg = match control.read(Some(slice)) {
            Ok(m) => m,
            Err(libos::Error::TimedOut) if slice < deadline => {
                parent.write(b"busy", Vec::new())?;
                continue;
            }
            Err(e) => {
                println!("devmgr: usb-storage nenahlasil disky: {e}");
                break;
            }
        };
        match &msg.data[..] {
            b"block" => parent.write(b"block", msg.handles)?,
            b"done" => break,
            other => println!("devmgr: usb-storage: neznama zprava {:?}", other),
        }
    }
    Ok(p)
}
