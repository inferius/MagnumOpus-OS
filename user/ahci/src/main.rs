//! ahci: ovladac radice SATA AHCI (PCI trida 01:06:01).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto poradi):
//! 1. omezeny Resource (BARy a konfigurace tohoto zarizeni, kvoty preruseni
//!    a DMA; s IOMMU vlastni domena DMA),
//! 2. stranka konfiguracniho prostoru PCI teto funkce.
//!
//! Zpravy od spravce na bootstrap kanalu: `pci <sbernice> <zarizeni>
//! <funkce>` a `bar <fyzicka adresa> <velikost>` (ABAR = BAR5, desitkove).
//! Ovladac pro kazdy disk SATA posle spravci `block` s kanalem pro klienta
//! (protokol `blockproto`) a nakonec `done`. Ochrana cizich disku jako
//! u ostatnich ovladacu (`blockproto::server::guard`).
//!
//! Preruseni pres MSI; radic bez MSI ovladac obsluhuje dotazovanim.

#![no_std]
#![no_main]

extern crate alloc;

mod hba;

use alloc::vec::Vec;

use blockproto::server::{self, Disks};
use blockproto::{SECTOR, feature};
use hba::{AUX_OFF, Ata, DATA_LEN, DATA_OFF, Hba, Port};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Channel, Error, Mapping, Result, Startup, Vmo, println};
use pci::Ecam;

libos::entry!(main);

const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_DSM: u8 = 0x06;
const ATA_FLUSH_CACHE: u8 = 0xE7;
const ATA_FLUSH_CACHE_EXT: u8 = 0xEA;
const ATA_IDENTIFY: u8 = 0xEC;
/// DATA SET MANAGEMENT: funkce TRIM.
const DSM_TRIM: u16 = 0x01;
/// Nejvic sektoru v jedne polozce TRIM (pole pocet ma 16 bitu).
const TRIM_ENTRY_MAX: u64 = 0xFFFF;

/// Disk na jednom portu.
struct Disk {
    port: Port,
    /// Kapacita v logickych sektorech (`sector_size`).
    sectors: u64,
    /// Logicky sektor (512 B, u disku 4Kn 4 KiB).
    sector_size: usize,
    /// Prikaz pro FLUSH (`None` = disk nema zapnutou mezipamet zapisu).
    flush: Option<u8>,
    /// Mezipamet zapisu je zapnuta, ale disk neumi FLUSH: bez zaruky.
    cache_without_flush: bool,
    /// Nejvic 512B bloku popisu useku v jednom TRIM (0 = TRIM neumi).
    trim_blocks: u16,
}

struct Ahci {
    hba: Hba,
    disks: Vec<Disk>,
    resource: Resource,
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(ecam_vmo)) = (handles.next(), handles.next()) else {
        println!("ahci: chybi handly ve startovni zprave");
        return 1;
    };
    let resource = Resource::from_handle(res);
    let ecam_vmo = Vmo::from_handle(ecam_vmo);
    let Some(address) = virtio::read_driver_config(&startup.bootstrap) else {
        println!("ahci: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let Some((bar, size)) = read_bar(&startup.bootstrap) else {
        println!("ahci {address}: chybi adresa registru (bar)");
        return 2;
    };
    let mut ahci = match Ecam::single(&ecam_vmo, address).and_then(|e| Ahci::open(&e, address, resource, bar, size)) {
        Ok(a) => a,
        Err(e) => {
            println!("ahci {address}: inicializace selhala: {e}");
            let _ = startup.bootstrap.write(b"done", Vec::new());
            return 3;
        }
    };
    let clients = match server::report_disks("ahci", &mut ahci, &startup.bootstrap) {
        Ok(c) => c,
        Err(e) => {
            println!("ahci {address}: hlaseni disku spravci selhalo: {e}");
            return 4;
        }
    };
    server::serve("ahci", &mut ahci, clients);
    let ports: Vec<Port> = ahci.disks.into_iter().map(|d| d.port).collect();
    ahci.hba.shutdown(&ports);
    0
}

/// Zprava `bar <fyzicka adresa> <velikost>`.
fn read_bar(bootstrap: &Channel) -> Option<(u64, u64)> {
    let msg = bootstrap.read(None).ok()?;
    let text = core::str::from_utf8(&msg.data).ok()?;
    let mut w = text.split_whitespace();
    if w.next()? != "bar" {
        return None;
    }
    Some((w.next()?.parse().ok()?, w.next()?.parse().ok()?))
}

/// Slovo `i` vysledku IDENTIFY DEVICE.
fn word(m: &Mapping, i: u64) -> u16 {
    m.read::<u16>(AUX_OFF + i * 2)
}

/// Retezec z IDENTIFY (slova s prohozenymi bajty).
fn ident_string(m: &Mapping, first: u64, words: u64, out: &mut [u8]) -> usize {
    let mut n = 0;
    for i in 0..words {
        let w = word(m, first + i);
        for b in [(w >> 8) as u8, w as u8] {
            if n < out.len() {
                out[n] = if b.is_ascii_graphic() || b == b' ' { b } else { b'?' };
                n += 1;
            }
        }
    }
    n
}

impl Ahci {
    fn open(ecam: &Ecam, a: pci::Address, resource: Resource, bar: u64, size: u64) -> Result<Self> {
        let mut hba = Hba::open(&resource, bar, size)?;
        ecam.enable(a);
        hba.reset()?;
        let cap = hba.reg(hba::CAP);
        let vs = hba.reg(hba::VS);
        println!(
            "ahci {a}: AHCI {}.{}, {} portu, NCQ {}",
            vs >> 16,
            (vs >> 8) & 0xFF,
            (cap & 0x1F) + 1,
            if cap & (1 << 30) != 0 { "ano" } else { "ne" }
        );
        // Preruseni: MSI (radice AHCI MSI-X obvykle nemaji), jinak dotazovani.
        match resource.interrupt() {
            Ok(irq) => match pci::msi::enable(ecam, a, irq.msi()) {
                Ok(()) => hba.irq = Some(irq),
                Err(e) => println!("ahci {a}: MSI nelze zapnout ({e}), obsluha dotazovanim"),
            },
            Err(e) => println!("ahci {a}: preruseni nelze vytvorit ({e}), obsluha dotazovanim"),
        }
        let pi = hba.reg(hba::PI);
        let mut prepared = Vec::new();
        for p in (0..32).filter(|p| pi & (1 << p) != 0) {
            match hba.prepare_port(&resource, p) {
                Ok(port) => prepared.push(port),
                Err(e) => println!("ahci {a}: port {p}: inicializace selhala: {e}"),
            }
        }
        let up = hba.wait_links(&prepared);
        let mut disks = Vec::new();
        for (port, up) in prepared.into_iter().zip(up) {
            if !up {
                hba.release_port(port);
                continue;
            }
            let p = port.index;
            match hba.start_port(port) {
                Ok(Some(port)) => disks.extend(identify(&hba, port)),
                Ok(None) => {}
                Err(e) => println!("ahci {a}: port {p}: inicializace selhala: {e}"),
            }
        }
        if hba.irq.is_some() {
            hba.enable_interrupts();
        }
        Ok(Self {
            hba,
            disks,
            resource,
        })
    }

    /// Cteni nebo zapis `bytes` bajtu (cele logicke sektory) pres datovy
    /// buffer portu.
    fn rw(&mut self, d: usize, command: u8, lba: u64, bytes: usize, write: bool) -> Result<()> {
        let disk = &mut self.disks[d];
        let sectors = bytes / disk.sector_size;
        self.hba.command(
            &mut disk.port,
            Ata {
                command,
                lba,
                count: sectors as u16,
                write,
                bytes: bytes as u32,
                ..Default::default()
            },
        )
    }
}

/// IDENTIFY DEVICE a vlastnosti disku; `None` = disk nepouzitelny.
fn identify(hba: &Hba, mut port: Port) -> Option<Disk> {
    let p = port.index;
    let aux = port.phys + AUX_OFF;
    let r = hba.command(
        &mut port,
        Ata {
            command: ATA_IDENTIFY,
            bytes: 512,
            data_phys: Some(aux),
            ..Default::default()
        },
    );
    if let Err(e) = r {
        println!("ahci: port {p}: IDENTIFY selhal: {e}");
        return None;
    }
    let m = &port.mem;
    let mut model = [0u8; 40];
    let n = ident_string(m, 27, 20, &mut model);
    let model = core::str::from_utf8(&model[..n]).unwrap_or("?").trim();
    let w83 = word(m, 83);
    if w83 & (1 << 10) == 0 {
        println!("ahci: port {p}: disk '{model}' neumi LBA48 - vynechan");
        return None;
    }
    // Slovo 106: platne (bit 14 = 1, bit 15 = 0) a bit 12 = logicky sektor
    // vetsi nez 512 B (velikost ve slovech 117-118). Pocty sektoru v prikazech
    // i kapacita jsou pak v techto sektorech.
    let w106 = word(m, 106);
    let sector_size = if w106 & 0xC000 == 0x4000 && w106 & (1 << 12) != 0 {
        2 * (word(m, 117) as usize | (word(m, 118) as usize) << 16)
    } else {
        SECTOR
    };
    if !sector_size.is_power_of_two() || !(SECTOR..=DATA_LEN as usize).contains(&sector_size) {
        println!("ahci: port {p}: disk '{model}' ma logicky sektor {sector_size} B - nepodporovano, vynechan");
        return None;
    }
    let sectors = (0..4).fold(0u64, |acc, i| acc | (word(m, 100 + i) as u64) << (16 * i));
    let cache_supported = word(m, 82) & (1 << 5) != 0;
    let cache_enabled = cache_supported && word(m, 85) & (1 << 5) != 0;
    let flush = if !cache_enabled {
        None
    } else if w83 & (1 << 13) != 0 {
        Some(ATA_FLUSH_CACHE_EXT)
    } else if w83 & (1 << 12) != 0 {
        Some(ATA_FLUSH_CACHE)
    } else {
        None
    };
    let trim = word(m, 169) & 1 != 0;
    // Slovo 105: nejvic 512B bloku popisu v jednom DSM (0 = neuvedeno).
    let trim_blocks = if trim {
        (word(m, 105).max(1)).min((DATA_LEN / 512) as u16)
    } else {
        0
    };
    println!(
        "ahci: port {p}: '{model}', {} MiB, sektor {sector_size} B, mezipamet zapisu {}, TRIM {}",
        sectors * sector_size as u64 / (1024 * 1024),
        if cache_enabled { "ano" } else { "ne" },
        if trim { "ano" } else { "ne" }
    );
    Some(Disk {
        port,
        sectors,
        sector_size,
        flush,
        cache_without_flush: cache_enabled && flush.is_none(),
        trim_blocks,
    })
}

impl Disks for Ahci {
    fn count(&self) -> usize {
        self.disks.len()
    }

    fn sectors(&self, d: usize) -> u64 {
        self.disks[d].sectors
    }

    fn sector_size(&self, d: usize) -> usize {
        self.disks[d].sector_size
    }

    fn features(&self, d: usize) -> u64 {
        let disk = &self.disks[d];
        let mut f = if disk.flush.is_some() {
            feature::FLUSH
        } else if disk.cache_without_flush {
            // Mezipamet bez moznosti ulozeni: zadna zaruka trvanlivosti.
            0
        } else {
            feature::WRITE_THROUGH | feature::FLUSH
        };
        if disk.trim_blocks > 0 {
            f |= feature::DISCARD;
        }
        f
    }

    fn read(&mut self, d: usize, lba: u64, out: &mut [u8]) -> Result<()> {
        self.rw(d, ATA_READ_DMA_EXT, lba, out.len(), false)?;
        self.disks[d].port.mem.read_bytes(DATA_OFF, out);
        Ok(())
    }

    fn write(&mut self, d: usize, lba: u64, data: &[u8]) -> Result<()> {
        self.disks[d].port.mem.write_bytes(DATA_OFF, data);
        self.rw(d, ATA_WRITE_DMA_EXT, lba, data.len(), true)
    }

    fn flush(&mut self, d: usize) -> Result<()> {
        let disk = &mut self.disks[d];
        match disk.flush {
            Some(command) => self.hba.command(
                &mut disk.port,
                Ata {
                    command,
                    ..Default::default()
                },
            ),
            // Mezipamet je vypnuta: zapis je trvaly po dokonceni.
            None if !disk.cache_without_flush => Ok(()),
            None => Err(Error::NotSupported),
        }
    }

    fn discard(&mut self, d: usize, lba: u64, count: u64) -> Result<()> {
        let disk = &mut self.disks[d];
        if disk.trim_blocks == 0 {
            return Err(Error::NotSupported);
        }
        let per_cmd = disk.trim_blocks as u64 * 64;
        let mut done = 0;
        while done < count {
            // Popis useku: 8 B (LBA 48 bitu, pocet 16 bitu), 64 v bloku 512 B.
            let mut n_entries = 0u64;
            while n_entries < per_cmd && done < count {
                let n = (count - done).min(TRIM_ENTRY_MAX);
                let entry = ((lba + done) & 0xFFFF_FFFF_FFFF) | n << 48;
                disk.port.mem.write::<u64>(DATA_OFF + n_entries * 8, entry);
                done += n;
                n_entries += 1;
            }
            let blocks = n_entries.div_ceil(64);
            // Zbytek posledniho bloku nulovy (polozka s poctem 0 se ignoruje).
            for e in n_entries..blocks * 64 {
                disk.port.mem.write::<u64>(DATA_OFF + e * 8, 0);
            }
            self.hba.command(
                &mut disk.port,
                Ata {
                    command: ATA_DSM,
                    feature: DSM_TRIM,
                    count: blocks as u16,
                    write: true,
                    bytes: (blocks * 512) as u32,
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }

    /// Necha disk precist sektor 0 do bezne stranky ovladace, ktera neni
    /// DMA pamet (viz `nvme`). Zapis jde jen do vlastni pameti ovladace.
    fn test_dma_protection(&mut self, d: usize) -> Result<u64> {
        const PATTERN: u8 = 0xAA;
        let ss = self.disks[d].sector_size;
        // Jeden sektor do jedne stranky (jedna polozka PRDT).
        if ss > 4096 {
            return Err(Error::NotSupported);
        }
        let page = Vmo::create(4096)?;
        let map = Mapping::new(&page, 0, 4096, MAP_READ | MAP_WRITE)?;
        map.write_bytes(0, &alloc::vec![PATTERN; ss]);
        let phys = self.resource.phys_addr(&page, 0)?;
        let disk = &mut self.disks[d];
        let _ = self.hba.command(
            &mut disk.port,
            Ata {
                command: ATA_READ_DMA_EXT,
                count: 1,
                bytes: ss as u32,
                data_phys: Some(phys),
                ..Default::default()
            },
        );
        let mut back = alloc::vec![0u8; ss];
        map.read_bytes(0, &mut back);
        Ok(back.iter().all(|&b| b == PATTERN) as u64)
    }
}
