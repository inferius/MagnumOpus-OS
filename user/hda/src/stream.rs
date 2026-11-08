//! Stream DMA (vystupni i vstupni, registry jsou stejne): buffer
//! z period, seznam bufferu (BDL), start, stop a pozice.
//!
//! Buffer je kruh `periods` period; kazda perioda je jedna polozka BDL
//! s priznakem IOC, takze radic po kazde periode vyvola preruseni.
//! Pozici (LPIB, bajty v kruhu) ovladac prevadi na monotonni pocet
//! snimku a zapisuje do stranky stavu pro `audiod`.

use core::time::Duration;

use audioproto::device::{OpenStream, StatusPage, flags};
use hdaspec::regs::{self, sd};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Error, Event, Mapping, Result, Vmo, time};

use crate::ctrl::Ctrl;

pub struct DmaStream {
    /// Index deskriptoru (bit v INTCTL).
    pub index: u32,
    off: u64,
    pub fmt: u16,
    pub req: OpenStream,
    pub buf: Vmo,
    _bdl: Vmo,
    pub status_vmo: Vmo,
    _status_map: Mapping,
    pub status: StatusPage,
    pub event: Event,
    cbl: u32,
    frame_bytes: u32,
    last_lpib: u32,
    pos_bytes: u64,
    /// Cas posledniho cteni LPIB (odhad celych otocek, viz `update`).
    last_read: u64,
    pub running: bool,
    /// Kdy naposled skoncila perioda (rezim bez preruseni).
    last_period: u64,
}

/// Zarovnani periody: adresa i delka polozky BDL na 128 bajtu.
fn period_bytes(frames: u32, frame_bytes: u32) -> u32 {
    let mut b = (frames * frame_bytes).next_multiple_of(regs::BDL_ALIGN as u32);
    while b % frame_bytes != 0 {
        b += regs::BDL_ALIGN as u32;
    }
    b
}

fn wait_bit(c: &Ctrl, off: u64, mask: u8, set: bool) -> bool {
    let end = time::deadline_after(Duration::from_millis(50));
    loop {
        if (c.r8(off) & mask != 0) == set {
            return true;
        }
        if time::now() > end {
            return false;
        }
        core::hint::spin_loop();
    }
}

impl DmaStream {
    /// Pripravi stream `index` s cislem `tag` a formatem `fmt` (kodovani
    /// HDA). Zarizeni ho jeste nespousti.
    pub fn open(c: &Ctrl, res: &Resource, index: u32, tag: u8, fmt: u16, want: OpenStream) -> Result<Self> {
        let fb = want.format.frame_bytes();
        let pb = period_bytes(want.period, fb);
        let periods = want.periods;
        let cbl = pb * periods;
        let page = libos::vm::page_size();
        let buf = res.dma_memory((cbl as u64).next_multiple_of(page))?;
        let buf_phys = res.phys_addr(&buf, 0)?;
        let bdl = res.dma_memory(page)?;
        let bdl_phys = res.phys_addr(&bdl, 0)?;
        if !c.gcap.addr64 && (buf_phys | bdl_phys) >> 32 != 0 {
            return Err(Error::NotSupported);
        }
        {
            // Ticho na zacatku (pamet DMA nemusi byt vynulovana).
            let m = Mapping::new(&buf, 0, cbl as u64, MAP_READ | MAP_WRITE)?;
            m.write_bytes(0, &alloc::vec![0u8; cbl as usize]);
            let bm = Mapping::new(&bdl, 0, page, MAP_READ | MAP_WRITE)?;
            for i in 0..periods {
                let e = regs::bdl_entry(buf_phys + (i * pb) as u64, pb, true);
                bm.write_bytes(i as u64 * regs::BDL_ENTRY as u64, &e);
            }
            libos::barrier::mmio_wmb();
        }
        let status_vmo = Vmo::create(audioproto::device::STATUS_SIZE as u64)?;
        let status_map = Mapping::new(&status_vmo, 0, audioproto::device::STATUS_SIZE as u64, MAP_READ | MAP_WRITE)?;
        // SAFETY: mapovani ma STATUS_SIZE bajtu a zije v `DmaStream`.
        let status = unsafe { StatusPage::new(status_map.addr() as *mut u8) };
        status.init(cbl / fb);
        let off = regs::sd_offset(index);
        // Zastavit a resetovat deskriptor.
        c.w8(off + sd::CTL, 0);
        wait_bit(c, off + sd::CTL, sd::CTL_RUN, false);
        c.w8(off + sd::CTL, sd::CTL_SRST);
        wait_bit(c, off + sd::CTL, sd::CTL_SRST, true);
        c.w8(off + sd::CTL, 0);
        if !wait_bit(c, off + sd::CTL, sd::CTL_SRST, false) {
            return Err(Error::TimedOut);
        }
        c.w8(off + sd::STS, sd::STS_BCIS | sd::STS_FIFOE | sd::STS_DESE);
        c.w32(off + sd::CBL, cbl);
        c.w16(off + sd::LVI, (periods - 1) as u16);
        c.w16(off + sd::FMT, fmt);
        c.w32(off + sd::BDPL, bdl_phys as u32);
        c.w32(off + sd::BDPU, (bdl_phys >> 32) as u32);
        c.w8(off + sd::CTL2, tag << 4);
        let req = OpenStream { format: want.format, period: pb / fb, periods };
        Ok(Self {
            index,
            off,
            fmt,
            req,
            buf,
            _bdl: bdl,
            status_vmo,
            _status_map: status_map,
            status,
            event: Event::create()?,
            cbl,
            frame_bytes: fb,
            last_lpib: 0,
            pos_bytes: 0,
            last_read: 0,
            running: false,
            last_period: 0,
        })
    }

    pub fn start(&mut self, c: &Ctrl, polled: bool) {
        if self.running {
            return;
        }
        let ints = c.r32(regs::INTCTL);
        c.w32(regs::INTCTL, ints | regs::INTCTL_GIE | 1 << self.index);
        self.last_lpib = c.r32(self.off + sd::LPIB) % self.cbl.max(1);
        let ctl = sd::CTL_RUN | sd::CTL_IOCE | sd::CTL_FEIE | sd::CTL_DEIE;
        c.w8(self.off + sd::CTL, ctl);
        self.running = true;
        self.last_period = time::now();
        self.last_read = self.last_period;
        self.status.set_flags(flags::RUNNING | if polled { flags::POLLED } else { 0 }, flags::ERROR);
    }

    pub fn stop(&mut self, c: &Ctrl) {
        let ctl = c.r8(self.off + sd::CTL);
        c.w8(self.off + sd::CTL, ctl & !(sd::CTL_RUN | sd::CTL_IOCE));
        wait_bit(c, self.off + sd::CTL, sd::CTL_RUN, false);
        let ints = c.r32(regs::INTCTL);
        c.w32(regs::INTCTL, ints & !(1 << self.index));
        self.update(c);
        self.running = false;
        self.status.set_flags(0, flags::RUNNING);
    }

    /// Precte pozici DMA a zapise ji do stranky stavu. Vrati `true`, kdyz
    /// od posledni periody uplynula aspon jedna cela perioda.
    pub fn update(&mut self, c: &Ctrl) -> bool {
        if !self.running {
            return false;
        }
        let lpib = c.r32(self.off + sd::LPIB);
        if lpib >= self.cbl {
            return false; // nesmysl behem resetu
        }
        let mut delta = ((lpib + self.cbl - self.last_lpib) % self.cbl) as u64;
        self.last_lpib = lpib;
        // LPIB je jen pozice v kruhu: kdyz ovladac precte pozdeji nez za
        // celou otocku (zatizeny system), otocky se ztrati. Podle uplynuleho
        // casu se doplni cele otocky, ktere zarizeni muselo prehrat.
        let now = time::now();
        let elapsed = now.saturating_sub(self.last_read);
        self.last_read = now;
        let rate_bytes = self.req.format.rate as u64 * self.frame_bytes as u64;
        let expected = (elapsed as u128 * rate_bytes as u128 / 1_000_000_000) as u64;
        let cbl = self.cbl as u64;
        if expected > delta + cbl / 2 {
            delta += (expected - delta + cbl / 2) / cbl * cbl;
        }
        let before = self.pos_bytes / (self.req.period * self.frame_bytes) as u64;
        self.pos_bytes += delta;
        let after = self.pos_bytes / (self.req.period * self.frame_bytes) as u64;
        self.status.set_position(self.pos_bytes / self.frame_bytes as u64, time::now());
        after != before
    }

    /// Obsluha preruseni: potvrdi stav streamu, aktualizuje pozici
    /// a pri konci periody probudi `audiod`. Chyba DMA stream zastavi.
    pub fn interrupt(&mut self, c: &Ctrl) {
        let sts = c.r8(self.off + sd::STS);
        c.w8(self.off + sd::STS, sts & (sd::STS_BCIS | sd::STS_FIFOE | sd::STS_DESE));
        if sts & (sd::STS_FIFOE | sd::STS_DESE) != 0 {
            self.status.add_error();
            libos::println!("hda: chyba DMA streamu {} (stav {sts:#x}), zastavuji", self.index);
            self.stop(c);
            self.status.set_flags(flags::ERROR, 0);
            let _ = self.event.signal();
            return;
        }
        if sts & sd::STS_BCIS != 0 {
            self.status.add_irq();
        }
        if self.update(c) || sts & sd::STS_BCIS != 0 {
            let _ = self.event.signal();
        }
    }

    /// Rezim bez preruseni: volat aspon dvakrat za periodu.
    pub fn poll(&mut self, c: &Ctrl) {
        if self.update(c) {
            self.last_period = time::now();
            let _ = self.event.signal();
        }
    }

    /// Delka periody v ns.
    pub fn period_ns(&self) -> u64 {
        self.req.period as u64 * 1_000_000_000 / self.req.format.rate as u64
    }

    /// Uvolneni: zastavit DMA (buffer muze dal drzet audiod, ale zarizeni
    /// do nej uz necte).
    pub fn close(&mut self, c: &Ctrl) {
        self.stop(c);
        c.w32(self.off + sd::BDPL, 0);
        c.w32(self.off + sd::BDPU, 0);
    }
}
