//! usb-storage: ovladac disku USB (trida mass storage, SCSI pres
//! Bulk-Only Transport) - flash disky, externi disky, ctecky karet.
//!
//! Spousti ho spravce zarizeni pro rozhrani 08/06/50. Startovni zprava
//! (kind USER, v tomto poradi): kanal rozhrani (`usbproto::msg`).
//! Bootstrap kanal vede ke spravci zarizeni: pro kazdy pouzitelny LUN
//! posle `block` s kanalem pro klienta (protokol `blockproto`), nakonec
//! `done` (`blockproto::server::report_disks`). Ochrana cizich disku jako
//! u ostatnich ovladacu (`diskguard`): cizi flash disk jde jen cist.
//!
//! Start: Get Max LUN, pro kazdy LUN INQUIRY (jen disky s primym
//! pristupem), TEST UNIT READY s opakovanim (zarizeni se po pripojeni
//! pripravuje), READ CAPACITY (10, pripadne 16) a MODE SENSE (ochrana
//! proti zapisu). Cteni a zapis pres READ/WRITE (10), nad 2 TiB (16).
//! Flush = SYNCHRONIZE CACHE; zarizeni, ktere ho nezna, nema mezipamet
//! (zapis je pak hotovy uz po CSW).
//!
//! UAS (USB Attached SCSI) zatim ne: zarizeni UAS ma skoro vzdy
//! i zakladni nastaveni BOT, ktere se pouzije.

#![no_std]
#![no_main]

extern crate alloc;

mod bot;

use alloc::vec::Vec;
use core::time::Duration;

use blockproto::feature;
use blockproto::server::{self, Disks};
use bot::{Bot, Status};
use libos::{Channel, Error, Result, Startup, println, time};
use usbclient::{UsbClient, is_stall};
use usbproto::desc::{Configuration, Transfer};
use usbproto::storage::{self, scsi};

libos::entry!(main);

/// Jak dlouho cekat, nez je jednotka pripravena (roztoceni disku, flash
/// po pripojeni).
const READY_TIMEOUT: Duration = Duration::from_secs(15);
/// Kolikrat zopakovat cteni nebo zapis po chybe prenosu.
const IO_RETRIES: u32 = 3;
/// Nejvic LUN (ctecky karet maji 4-8).
const MAX_LUNS: u8 = 16;

/// Jeden pouzitelny LUN.
struct Lun {
    lun: u8,
    /// Vyrobce a vyrobek z INQUIRY (pro instalator).
    model: alloc::string::String,
    sectors: u64,
    sector_size: usize,
    read_only: bool,
    /// Bit RMB z INQUIRY: vymenne medium (flash disk, ctecka karet).
    removable: bool,
    /// SYNCHRONIZE CACHE funguje (po ILLEGAL REQUEST uz se nezkousi).
    flush: bool,
}

struct Storage {
    bot: Bot,
    luns: Vec<Lun>,
    /// Nejvic bajtu jednoho prikazu READ (spojena cteni, `max_read`).
    max_read: usize,
}

/// Nejvic bajtu jednoho cteni podle rychlosti: USB 3 1 MiB, starsi 120 KiB
/// (240 sektoru; nektere flash disky USB 2 vetsi prikaz nezvladnou, stejny
/// limit pouziva Linux).
fn max_read_for(speed: usbproto::Speed) -> usize {
    match speed {
        usbproto::Speed::Super => server::BATCH_MAX,
        _ => 240 * 512,
    }
}

fn main(startup: Startup) -> i64 {
    let Some(usb) = startup.handles.into_iter().next() else {
        println!("usb-storage: chybi kanal rozhrani ve startovni zprave");
        return 1;
    };
    let mut usb = UsbClient::new(Channel::from_handle(usb));
    let info = match usb.info() {
        Ok(i) => i,
        Err(e) => {
            println!("usb-storage: INFO selhalo: {e}");
            let _ = startup.bootstrap.write(b"done", Vec::new());
            return 2;
        }
    };
    let iface = Configuration::parse(&info.config)
        .and_then(|c| c.interfaces.into_iter().find(|i| i.number == info.interface));
    let eps = iface.as_ref().and_then(|i| {
        let bulk = |is_in: bool| {
            i.endpoints
                .iter()
                .find(|e| e.transfer == Transfer::Bulk && e.is_in() == is_in)
                .map(|e| e.address)
        };
        Some((bulk(true)?, bulk(false)?))
    });
    let Some((ep_in, ep_out)) = eps else {
        println!("usb-storage: rozhrani {} nema bulk IN a OUT", info.interface);
        let _ = startup.bootstrap.write(b"done", Vec::new());
        return 2;
    };
    println!(
        "usb-storage: {:04x}:{:04x} ({}, port {:?}), rozhrani {}",
        info.device.vendor,
        info.device.product,
        info.speed.name(),
        info.path,
        info.interface
    );
    let mut dev = Storage {
        bot: Bot::new(usb, info.interface, ep_in, ep_out),
        luns: Vec::new(),
        max_read: max_read_for(info.speed),
    };
    dev.probe();
    let clients = match server::report_disks("usb-storage", &mut dev, &startup.bootstrap) {
        Ok(c) => c,
        Err(e) => {
            println!("usb-storage: hlaseni disku spravci selhalo: {e}");
            return 4;
        }
    };
    server::serve("usb-storage", &mut dev, clients);
    // Konec: zapsat mezipamet (vytazeni flash disku po vypnuti systemu).
    for i in 0..dev.luns.len() {
        let _ = dev.flush(i);
    }
    0
}

impl Storage {
    /// Najde LUN s pouzitelnym diskem.
    fn probe(&mut self) {
        let max_lun = match self.bot.usb.control(storage::get_max_lun(self.bot.interface), &[]) {
            Ok(d) => d.first().copied().unwrap_or(0).min(MAX_LUNS - 1),
            // STALL = jediny LUN (BOT 3.2).
            Err(e) if is_stall(&e) => 0,
            Err(e) => {
                println!("usb-storage: Get Max LUN selhalo ({e}), beru jeden LUN");
                0
            }
        };
        for lun in 0..=max_lun {
            match self.probe_lun(lun) {
                Ok(Some(l)) => {
                    println!(
                        "usb-storage: LUN {lun}: {} MiB, sektor {} B{}",
                        l.sectors * l.sector_size as u64 / (1024 * 1024),
                        l.sector_size,
                        if l.read_only { ", chraneno proti zapisu" } else { "" }
                    );
                    self.luns.push(l);
                }
                Ok(None) => {}
                Err(e) => println!("usb-storage: LUN {lun}: nepouzitelny ({e})"),
            }
        }
    }

    fn probe_lun(&mut self, lun: u8) -> Result<Option<Lun>> {
        let mut out = Vec::new();
        match self.bot.command(lun, &scsi::inquiry(), scsi::INQUIRY_LEN as usize, &mut out, &[])? {
            Status::Passed(_) => {}
            Status::Failed => return Err(Error::IoError),
        }
        let inq = scsi::parse_inquiry(&out).ok_or(Error::Corrupted)?;
        println!(
            "usb-storage: LUN {lun}: '{}' '{}' rev '{}', typ {}{}",
            inq.vendor,
            inq.product,
            inq.revision,
            inq.device_type,
            if inq.removable { ", vymenitelne medium" } else { "" }
        );
        // Jen disky s primym pristupem (ne CD-ROM, ne zarizeni mimo).
        if inq.device_type != 0 {
            return Ok(None);
        }
        if !self.wait_ready(lun)? {
            println!("usb-storage: LUN {lun}: medium chybi");
            return Ok(None);
        }
        let (last, bs) = self.capacity(lun)?;
        let sector_size = bs as usize;
        if !sector_size.is_power_of_two() || !(512..=server::IO_MAX).contains(&sector_size) {
            println!("usb-storage: LUN {lun}: nepodporovany sektor {bs} B");
            return Ok(None);
        }
        // Ochrana proti zapisu (prepinac na karte SD, flash disku); kdyz
        // MODE SENSE nejde, beru disk jako zapisovatelny (zapis pak selze
        // s DATA PROTECT).
        let mut mode = Vec::new();
        let read_only = matches!(
            self.bot.command(lun, &scsi::mode_sense_6(), 4, &mut mode, &[]),
            Ok(Status::Passed(n)) if n >= 3 && scsi::write_protected(&mode)
        );
        Ok(Some(Lun {
            lun,
            model: alloc::format!("{} {}", inq.vendor, inq.product),
            sectors: last + 1,
            sector_size,
            read_only,
            removable: inq.removable,
            flush: true,
        }))
    }

    /// TEST UNIT READY s opakovanim; `false` = medium chybi.
    fn wait_ready(&mut self, lun: u8) -> Result<bool> {
        let deadline = time::deadline_after(READY_TIMEOUT);
        let mut out = Vec::new();
        loop {
            if self.bot.command(lun, &scsi::test_unit_ready(), 0, &mut out, &[])? == Status::Passed(0) {
                return Ok(true);
            }
            let sense = self.bot.sense(lun);
            if sense.is_some_and(|s| s.no_medium()) {
                return Ok(false);
            }
            if time::now() > deadline || sense.is_some_and(|s| !s.retryable()) {
                println!("usb-storage: LUN {lun}: jednotka neni pripravena ({sense:?})");
                return Err(Error::TimedOut);
            }
            time::sleep(Duration::from_millis(100));
        }
    }

    /// (posledni LBA, velikost sektoru).
    fn capacity(&mut self, lun: u8) -> Result<(u64, u32)> {
        let mut out = Vec::new();
        for _ in 0..3 {
            match self.bot.command(lun, &scsi::read_capacity_10(), 8, &mut out, &[])? {
                Status::Passed(_) => {
                    let (last, bs) = scsi::parse_capacity_10(&out).ok_or(Error::Corrupted)?;
                    if last != u32::MAX as u64 {
                        return Ok((last, bs));
                    }
                    // Disk nad 2 TiB.
                    if let Ok(Status::Passed(_)) = self.bot.command(lun, &scsi::read_capacity_16(), 32, &mut out, &[]) {
                        return scsi::parse_capacity_16(&out).ok_or(Error::Corrupted);
                    }
                    return Err(Error::NotSupported);
                }
                // UNIT ATTENTION po pripojeni: precist duvod a zkusit znovu.
                Status::Failed => {
                    let _ = self.bot.sense(lun);
                }
            }
        }
        Err(Error::IoError)
    }

    /// Cteni nebo zapis s opakovanim po chybe prenosu a UNIT ATTENTION.
    fn rw(&mut self, d: usize, lba: u64, read: Option<&mut [u8]>, data: &[u8]) -> Result<()> {
        let (lun, ss) = (self.luns[d].lun, self.luns[d].sector_size);
        let write = read.is_none();
        let len = if write { data.len() } else { read.as_ref().map_or(0, |b| b.len()) };
        let count = (len / ss) as u32;
        let (cdb, n) = scsi::rw(write, lba, count);
        let mut out = Vec::new();
        let mut last = Error::IoError;
        for _ in 0..IO_RETRIES {
            let r = if write {
                self.bot.command(lun, &cdb[..n], 0, &mut out, data)
            } else {
                self.bot.command(lun, &cdb[..n], len, &mut out, &[])
            };
            match r {
                Ok(Status::Passed(moved)) if moved == len => {
                    if let Some(buf) = read {
                        buf.copy_from_slice(&out);
                    }
                    return Ok(());
                }
                Ok(Status::Passed(_)) => last = Error::IoError,
                Ok(Status::Failed) => {
                    let s = self.bot.sense(lun);
                    match s {
                        Some(s) if s.key == scsi::key::DATA_PROTECT => return Err(Error::ReadOnly),
                        Some(s) if s.retryable() => last = Error::IoError,
                        Some(s) if s.key == scsi::key::ILLEGAL_REQUEST => return Err(Error::OutOfRange),
                        _ => return Err(Error::IoError),
                    }
                }
                Err(Error::PeerClosed) => return Err(Error::PeerClosed),
                Err(e) => last = e,
            }
        }
        println!(
            "usb-storage: LUN {lun}: {} LBA {lba} ({count} s.) selhalo: {last}",
            if write { "zapis" } else { "cteni" }
        );
        Err(last)
    }
}

impl Disks for Storage {
    /// Vytazeni flash disku: kanal USB zarizeni se zavre.
    fn gone(&self) -> Option<&libos::Handle> {
        Some(self.bot.usb.channel.handle())
    }

    fn model(&self, d: usize) -> alloc::string::String {
        self.luns[d].model.clone()
    }

    fn count(&self) -> usize {
        self.luns.len()
    }

    fn sectors(&self, d: usize) -> u64 {
        self.luns[d].sectors
    }

    fn sector_size(&self, d: usize) -> usize {
        self.luns[d].sector_size
    }

    fn removable(&self, d: usize) -> bool {
        self.luns[d].removable
    }

    fn features(&self, d: usize) -> u64 {
        let l = &self.luns[d];
        if l.read_only {
            feature::READ_ONLY
        } else if l.flush {
            feature::FLUSH
        } else {
            // Bez mezipameti: zapis je trvaly po CSW.
            feature::WRITE_THROUGH
        }
    }

    fn read(&mut self, d: usize, lba: u64, out: &mut [u8]) -> Result<()> {
        self.rw(d, lba, Some(out), &[])
    }

    /// Prikaz BOT stoji tri prenosy (CBW, data, CSW): navazujici cteni
    /// klienta server spoji do jednoho prikazu (`max_read_for`).
    fn max_read(&self, d: usize) -> usize {
        let ss = self.luns[d].sector_size;
        (self.max_read / ss * ss).max(server::IO_MAX)
    }

    fn write(&mut self, d: usize, lba: u64, data: &[u8]) -> Result<()> {
        if self.luns[d].read_only {
            return Err(Error::ReadOnly);
        }
        self.rw(d, lba, None, data)
    }

    fn flush(&mut self, d: usize) -> Result<()> {
        if !self.luns[d].flush {
            return Ok(());
        }
        let lun = self.luns[d].lun;
        let mut out = Vec::new();
        match self.bot.command(lun, &scsi::synchronize_cache(), 0, &mut out, &[])? {
            Status::Passed(_) => Ok(()),
            Status::Failed => {
                let s = self.bot.sense(lun);
                if s.is_some_and(|s| s.key == scsi::key::ILLEGAL_REQUEST) {
                    // Prikaz nezna: zarizeni nema mezipamet zapisu (vetsina
                    // flash disku), zapis je trvaly uz po CSW.
                    self.luns[d].flush = false;
                    Ok(())
                } else {
                    Err(Error::IoError)
                }
            }
        }
    }

    fn discard(&mut self, _d: usize, _lba: u64, _count: u64) -> Result<()> {
        // UNMAP flash disky USB skoro neumi; TRIM je jen napoveda.
        Err(Error::NotSupported)
    }

    fn test_dma_protection(&mut self, _d: usize) -> Result<u64> {
        // DMA dela radic xHCI, ne tento ovladac.
        Err(Error::NotSupported)
    }
}
