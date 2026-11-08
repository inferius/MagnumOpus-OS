//! nvme: ovladac disku NVMe (PCI trida 01:08:02).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto poradi):
//! 1. omezeny Resource (BARy a konfigurace tohoto zarizeni, kvoty preruseni
//!    a DMA; s IOMMU vlastni domena DMA),
//! 2. stranka konfiguracniho prostoru PCI teto funkce.
//!
//! Zpravy od spravce na bootstrap kanalu: `pci <sbernice> <zarizeni>
//! <funkce>` a `bar <fyzicka adresa> <velikost>` (BAR0, desitkove).
//! Ovladac pro kazdy jmenny prostor (disk) posle spravci `block` s kanalem
//! pro klienta (protokol `blockproto`) a nakonec `done`. Pred obsluhou
//! klienta rozebere disk (`blockproto::server::guard`): cizi disk je jen
//! pro cteni, u disku s nasim oddilem GPT klient vidi jen tento oddil.

#![no_std]
#![no_main]

extern crate alloc;

mod ctrl;

use alloc::vec::Vec;

use blockproto::server::{self, Disks};
use blockproto::feature;
use ctrl::{AUX_OFF, Command, Controller, DATA_OFF, IO_DSM, IO_FLUSH, IO_READ, IO_WRITE, Namespace};
use libos::hw::Resource;
use libos::{Error, Result, Startup, Vmo, println};
use pci::Ecam;

libos::entry!(main);

/// Nejvic jmennych prostoru, ktere ovladac nabidne.
const MAX_NAMESPACES: usize = 16;
/// Nejvic sektoru v jednom useku DSM a useku v jednom prikazu.
const DSM_RANGE_SECTORS: u64 = 1 << 24;
const DSM_RANGES: u64 = 256;

struct Nvme {
    c: Controller,
    namespaces: Vec<Namespace>,
    write_cache: bool,
    dsm: bool,
    /// Nejvic bajtu v jednom prikazu (MDTS).
    max_bytes: u64,
    resource: Resource,
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(ecam_vmo)) = (handles.next(), handles.next()) else {
        println!("nvme: chybi handly ve startovni zprave");
        return 1;
    };
    let resource = Resource::from_handle(res);
    let ecam_vmo = Vmo::from_handle(ecam_vmo);
    let Some(address) = virtio::read_driver_config(&startup.bootstrap) else {
        println!("nvme: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let Some((bar, size)) = pci::read_bar_message(&startup.bootstrap, "bar") else {
        println!("nvme {address}: chybi adresa registru (bar)");
        return 2;
    };
    let opened = Ecam::single(&ecam_vmo, address)
        .and_then(|e| Controller::open(&e, address, &resource, bar, size))
        .and_then(|c| Nvme::new(c, resource));
    let mut nvme = match opened {
        Ok(n) => n,
        Err(e) => {
            println!("nvme {address}: inicializace selhala: {e}");
            let _ = startup.bootstrap.write(b"done", Vec::new());
            return 3;
        }
    };
    for ns in &nvme.namespaces {
        println!(
            "nvme {address}: jmenny prostor {}: {} MiB, sektor {} B, mezipamet zapisu {}, TRIM {}{}",
            ns.nsid,
            ns.sectors * ns.sector_size as u64 / (1024 * 1024),
            ns.sector_size,
            if nvme.write_cache { "ano" } else { "ne" },
            if nvme.dsm { "ano" } else { "ne" },
            if ns.write_protected { ", chranen proti zapisu" } else { "" },
        );
    }
    let clients = match server::report_disks("nvme", &mut nvme, &startup.bootstrap) {
        Ok(c) => c,
        Err(e) => {
            println!("nvme {address}: hlaseni disku spravci selhalo: {e}");
            nvme.c.shutdown();
            return 4;
        }
    };
    server::serve("nvme", &mut nvme, clients);
    nvme.c.shutdown();
    0
}

impl Nvme {
    fn new(mut c: Controller, resource: Resource) -> Result<Self> {
        let id = c.identify_controller()?;
        // Sektor musi projit jednim prenosem (MDTS) i bufferem ovladace.
        let namespaces = c.namespaces(&id, MAX_NAMESPACES)?;
        Ok(Self {
            c,
            namespaces,
            write_cache: id.write_cache,
            dsm: id.dsm,
            max_bytes: id.max_bytes,
            resource,
        })
    }

    /// Cteni nebo zapis `count` sektoru pres datovy buffer (jeden prikaz).
    fn rw(&mut self, d: usize, opcode: u8, lba: u64, count: u32) -> Result<()> {
        let nsid = self.namespaces[d].nsid;
        let ss = self.namespaces[d].sector_size as u64;
        let (prp1, prp2) = self.c.data_prps(count as u64 * ss);
        self.c
            .io_command(Command {
                opcode,
                nsid,
                prp1,
                prp2,
                cdw10: lba as u32,
                cdw11: (lba >> 32) as u32,
                cdw12: count - 1,
            })
            .map(|_| ())
    }
}

impl Nvme {
    /// Velikost sektoru a nejvetsi prenos jednoho prikazu (cele sektory).
    fn step(&self, d: usize) -> (usize, usize) {
        let ss = self.namespaces[d].sector_size;
        (ss, (self.max_bytes as usize / ss).max(1) * ss)
    }
}

impl Disks for Nvme {
    fn count(&self) -> usize {
        self.namespaces.len()
    }

    fn sectors(&self, d: usize) -> u64 {
        self.namespaces[d].sectors
    }

    fn sector_size(&self, d: usize) -> usize {
        self.namespaces[d].sector_size
    }


    fn features(&self, d: usize) -> u64 {
        // Bez volatilni mezipameti je dokonceny zapis trvaly.
        let mut f = if self.write_cache {
            feature::FLUSH
        } else {
            feature::WRITE_THROUGH | feature::FLUSH
        };
        if self.dsm {
            f |= feature::DISCARD;
        }
        if self.namespaces[d].write_protected {
            f |= feature::READ_ONLY;
        }
        f
    }

    fn read(&mut self, d: usize, lba: u64, out: &mut [u8]) -> Result<()> {
        let (ss, step) = self.step(d);
        for (i, chunk) in out.chunks_mut(step).enumerate() {
            let at = lba + (i * step / ss) as u64;
            self.rw(d, IO_READ, at, (chunk.len() / ss) as u32)?;
            self.c.mem.read_bytes(DATA_OFF, chunk);
        }
        Ok(())
    }

    fn write(&mut self, d: usize, lba: u64, data: &[u8]) -> Result<()> {
        let (ss, step) = self.step(d);
        for (i, chunk) in data.chunks(step).enumerate() {
            let at = lba + (i * step / ss) as u64;
            self.c.mem.write_bytes(DATA_OFF, chunk);
            self.rw(d, IO_WRITE, at, (chunk.len() / ss) as u32)?;
        }
        Ok(())
    }

    fn flush(&mut self, d: usize) -> Result<()> {
        if !self.write_cache {
            // Bez mezipameti neni co ukladat (zapis je trvaly po dokonceni).
            return Ok(());
        }
        let nsid = self.namespaces[d].nsid;
        self.c
            .io_command(Command {
                opcode: IO_FLUSH,
                nsid,
                ..Default::default()
            })
            .map(|_| ())
    }

    fn discard(&mut self, d: usize, lba: u64, count: u64) -> Result<()> {
        if !self.dsm {
            return Err(Error::NotSupported);
        }
        let nsid = self.namespaces[d].nsid;
        let mut done = 0;
        while done < count {
            // Az 256 useku po 16 B (atributy, delka, zacatek) v jedne strance.
            let mut ranges = 0u64;
            while ranges < DSM_RANGES && done < count {
                let n = (count - done).min(DSM_RANGE_SECTORS);
                let e = AUX_OFF + ranges * 16;
                self.c.mem.write::<u32>(e, 0);
                self.c.mem.write::<u32>(e + 4, n as u32);
                self.c.mem.write::<u64>(e + 8, lba + done);
                done += n;
                ranges += 1;
            }
            self.c.io_command(Command {
                opcode: IO_DSM,
                nsid,
                prp1: self.c.phys + AUX_OFF,
                cdw10: ranges as u32 - 1,
                // Atribut Deallocate.
                cdw11: 1 << 2,
                ..Default::default()
            })?;
        }
        Ok(())
    }

    /// Necha radic precist sektor 0 do bezne stranky ovladace, ktera neni
    /// DMA pamet. 1 = stranka nezmenena (IOMMU zapis zablokovala), 0 =
    /// prepsana. Zapis jde jen do vlastni pameti ovladace.
    fn test_dma_protection(&mut self, d: usize) -> Result<u64> {
        // Jeden sektor do jedne stranky (PRP1); vetsi sektor test neumi.
        let probe = server::DmaProbe::new(&self.resource, self.namespaces[d].sector_size)?;
        let phys = probe.phys;
        let nsid = self.namespaces[d].nsid;
        // Vysledek prikazu je vedlejsi (radic chybu prenosu hlasi nebo ne).
        let _ = self.c.io_command(Command {
            opcode: IO_READ,
            nsid,
            prp1: phys,
            ..Default::default()
        });
        Ok(probe.untouched())
    }
}
