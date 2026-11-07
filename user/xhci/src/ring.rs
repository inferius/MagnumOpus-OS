//! DMA pamet, TRB a kruhove fronty (prikazy, prenosy, udalosti).

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Mapping, Result, Vmo};

/// Souvisly kus DMA pameti s fyzickou adresou (pro zarizeni).
pub struct Dma {
    _vmo: Vmo,
    pub map: Mapping,
    pub phys: u64,
    pub len: u64,
}

impl Dma {
    pub fn new(res: &Resource, len: u64) -> Result<Self> {
        let len = len.next_multiple_of(4096);
        let vmo = res.dma_memory(len)?;
        let phys = res.phys_addr(&vmo, 0)?;
        let map = Mapping::new(&vmo, 0, len, MAP_READ | MAP_WRITE)?;
        Ok(Self {
            _vmo: vmo,
            map,
            phys,
            len,
        })
    }
}

/// Typy TRB (bity 15:10 dwordu control).
pub mod trb {
    pub const NORMAL: u32 = 1;
    pub const SETUP: u32 = 2;
    pub const DATA: u32 = 3;
    pub const STATUS: u32 = 4;
    pub const LINK: u32 = 6;
    pub const ENABLE_SLOT: u32 = 9;
    pub const DISABLE_SLOT: u32 = 10;
    pub const ADDRESS_DEVICE: u32 = 11;
    pub const CONFIGURE_EP: u32 = 12;
    pub const EVALUATE_CTX: u32 = 13;
    pub const RESET_EP: u32 = 14;
    pub const SET_TR_DEQUEUE: u32 = 16;
    pub const TRANSFER_EVENT: u32 = 32;
    pub const CMD_COMPLETION: u32 = 33;
    pub const PORT_STATUS: u32 = 34;

    pub const CYCLE: u32 = 1 << 0;
    /// Link: prepnout cyklus.
    pub const TOGGLE: u32 = 1 << 1;
    /// Preruseni pri kratkem paketu.
    pub const ISP: u32 = 1 << 2;
    pub const IOC: u32 = 1 << 5;
    /// Data primo v TRB (setup paket).
    pub const IDT: u32 = 1 << 6;
    /// Smer IN (data a status stage).
    pub const DIR_IN: u32 = 1 << 16;
}

/// Jeden TRB (16 B).
#[derive(Debug, Clone, Copy, Default)]
pub struct Trb {
    pub param: u64,
    pub status: u32,
    pub control: u32,
}

impl Trb {
    pub fn kind(&self) -> u32 {
        (self.control >> 10) & 0x3F
    }

    /// Kod dokonceni (udalosti).
    pub fn code(&self) -> u32 {
        self.status >> 24
    }

    /// Slot (udalosti a prikazy).
    pub fn slot(&self) -> u8 {
        (self.control >> 24) as u8
    }

    /// DCI endpointu (udalost prenosu).
    pub fn endpoint(&self) -> u8 {
        ((self.control >> 16) & 0x1F) as u8
    }
}

/// Pocet TRB ve fronte (jedna stranka; posledni je Link).
pub const RING_TRBS: u64 = 256;

/// Fronta prikazu nebo prenosu (producent = ovladac).
pub struct Ring {
    pub dma: Dma,
    enqueue: u64,
    cycle: bool,
}

impl Ring {
    pub fn new(res: &Resource) -> Result<Self> {
        let dma = Dma::new(res, RING_TRBS * 16)?;
        let mut r = Self {
            dma,
            enqueue: 0,
            cycle: true,
        };
        // Link z posledniho TRB zpet na zacatek, s prepnutim cyklu.
        let last = (RING_TRBS - 1) * 16;
        r.dma.map.write::<u64>(last, r.dma.phys);
        r.dma.map.write::<u32>(last + 8, 0);
        r.dma.map.write::<u32>(last + 12, trb::LINK << 10 | trb::TOGGLE);
        r.enqueue = 0;
        Ok(r)
    }

    /// Fyzicka adresa a cyklus pro registr / kontext (dequeue pointer).
    pub fn start(&self) -> u64 {
        self.dma.phys | self.cycle as u64
    }

    /// Aktualni pozice zapisu s cyklem (pro Set TR Dequeue).
    pub fn current(&self) -> u64 {
        (self.dma.phys + self.enqueue * 16) | self.cycle as u64
    }

    /// Vlozi TRB (bit cyklu doplni) a vrati jeho fyzickou adresu.
    pub fn push(&mut self, t: Trb) -> u64 {
        let off = self.enqueue * 16;
        let phys = self.dma.phys + off;
        self.dma.map.write::<u64>(off, t.param);
        self.dma.map.write::<u32>(off + 8, t.status);
        // Bit cyklu az po ostatnich datech: radic TRB uvidi jako platny az
        // s nim.
        libos::barrier::dma_wmb();
        let control = (t.control & !trb::CYCLE) | self.cycle as u32;
        self.dma.map.write::<u32>(off + 12, control);
        self.enqueue += 1;
        if self.enqueue == RING_TRBS - 1 {
            // Predat Link radici a pokracovat od zacatku s opacnym cyklem.
            let link = (RING_TRBS - 1) * 16;
            let c = self.dma.map.read::<u32>(link + 12);
            libos::barrier::dma_wmb();
            self.dma
                .map
                .write::<u32>(link + 12, (c & !trb::CYCLE) | self.cycle as u32);
            self.enqueue = 0;
            self.cycle = !self.cycle;
        }
        libos::barrier::dma_wmb();
        phys
    }
}

/// Fronta udalosti (producent = radic) s jednim segmentem a tabulkou ERST.
pub struct EventRing {
    pub dma: Dma,
    /// Tabulka segmentu (1 polozka).
    pub erst: Dma,
    dequeue: u64,
    cycle: bool,
}

impl EventRing {
    pub fn new(res: &Resource) -> Result<Self> {
        let dma = Dma::new(res, RING_TRBS * 16)?;
        let erst = Dma::new(res, 64)?;
        erst.map.write::<u64>(0, dma.phys);
        erst.map.write::<u32>(8, RING_TRBS as u32);
        erst.map.write::<u32>(12, 0);
        Ok(Self {
            dma,
            erst,
            dequeue: 0,
            cycle: true,
        })
    }

    /// Dalsi udalost, pokud ji radic zapsal.
    pub fn pop(&mut self) -> Option<Trb> {
        let off = self.dequeue * 16;
        let control = self.dma.map.read::<u32>(off + 12);
        if (control & trb::CYCLE != 0) != self.cycle {
            return None;
        }
        // Obsah TRB az po bitu cyklu.
        libos::barrier::dma_rmb();
        let t = Trb {
            param: self.dma.map.read(off),
            status: self.dma.map.read(off + 8),
            control,
        };
        self.dequeue += 1;
        if self.dequeue == RING_TRBS {
            self.dequeue = 0;
            self.cycle = !self.cycle;
        }
        Some(t)
    }

    /// Hodnota pro ERDP (s bitem EHB = potvrzeni zpracovani).
    pub fn erdp(&self) -> u64 {
        (self.dma.phys + self.dequeue * 16) | crate::regs::ERDP_EHB
    }
}
