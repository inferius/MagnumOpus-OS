//! Radic: inicializace, prikazy, prenosy a zpracovani udalosti.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::hw::{Interrupt, Resource};
use libos::{Error, Result, Signals, println, time};
use pci::msix::MsixTable;
use pci::{Address, Ecam};
use usbproto::desc::Setup;
use usbproto::msg::completion;

use crate::device::Device;
use crate::regs::*;
use crate::ring::{Dma, EventRing, Ring, Trb, trb};

/// Jak dlouho cekat na dokonceni prikazu nebo synchronniho prenosu.
pub const SYNC_TIMEOUT: Duration = Duration::from_secs(2);

/// Kdo ceka na prenos: klient (index kanalu a id pozadavku) nebo radic sam
/// (enumerace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Client(usize, u32),
    Internal,
}

/// Rozpracovany prenos (na jeden endpoint nejvys jeden).
pub struct Pending {
    pub slot: u8,
    pub dci: u8,
    /// TRB s IOC, jehoz udalost prenos konci.
    pub last_trb: u64,
    pub owner: Owner,
    pub requested: u32,
    /// Prenesene bajty, kdyz prisel kratky paket (udalost na datovem TRB).
    pub short: Option<u32>,
    pub is_in: bool,
}

/// Dokonceny prenos.
#[derive(Debug, Clone)]
pub struct Done {
    pub owner: Owner,
    pub slot: u8,
    pub dci: u8,
    pub code: u32,
    pub actual: u32,
    pub data: Vec<u8>,
}

pub struct Hc {
    pub regs: Regs,
    pub res: Resource,
    pub irq: Interrupt,
    _msix: MsixTable,
    /// Velikost jednoho kontextu (32 nebo 64 B).
    pub ctx_size: u64,
    pub max_slots: u8,
    pub ports: u8,
    /// Porty USB 3 (z capability Supported Protocol), bit N = port N.
    pub usb3_ports: u64,
    dcbaa: Dma,
    _scratch: Vec<Dma>,
    cmd: Ring,
    events: EventRing,
    pub devices: Vec<Option<Device>>,
    pub pending: Vec<Pending>,
    /// Hotove prenosy klientu (doruci je hlavni smycka).
    pub done: Vec<Done>,
    /// Vysledek prikazu: (adresa TRB prikazu, udalost dokonceni).
    cmd_result: Option<(u64, Trb)>,
    /// Endpointy zastavene chybou prenosu (obnovi je `recover_halted`;
    /// prikaz nejde poslat primo ze zpracovani udalosti).
    halted: Vec<(u8, u8)>,
}

impl Hc {
    pub fn open(ecam: &Ecam, a: Address, res: Resource, bar_phys: u64, bar_len: u64) -> Result<Self> {
        ecam.enable(a);
        let vmo = res.device_memory(bar_phys, bar_len)?;
        let map = libos::Mapping::new(
            &vmo,
            0,
            bar_len,
            libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE,
        )?;
        let regs = Regs::new(map);
        let hcs1 = regs.cap32(HCSPARAMS1);
        let hcs2 = regs.cap32(HCSPARAMS2);
        let hcc1 = regs.cap32(HCCPARAMS1);
        let max_slots = (hcs1 & 0xFF).min(64) as u8;
        let ports = (hcs1 >> 24) as u8;
        let ctx_size = if hcc1 & (1 << 2) != 0 { 64 } else { 32 };
        let scratch = ((hcs2 >> 27) & 0x1F) | (((hcs2 >> 21) & 0x1F) << 5);

        let irq = res.interrupt()?;
        let msix = MsixTable::open(ecam, a, &res)?;
        msix.set(0, irq.msi())?;
        msix.enable(ecam);

        let mut hc = Self {
            dcbaa: Dma::new(&res, 2048)?,
            cmd: Ring::new(&res)?,
            events: EventRing::new(&res)?,
            regs,
            irq,
            _msix: msix,
            ctx_size,
            max_slots,
            ports,
            usb3_ports: 0,
            _scratch: Vec::new(),
            devices: (0..=max_slots).map(|_| None).collect(),
            pending: Vec::new(),
            done: Vec::new(),
            cmd_result: None,
            halted: Vec::new(),
            res,
        };
        hc.take_ownership();
        hc.reset()?;
        hc.find_protocols();
        if scratch > 0 {
            hc.setup_scratchpad(scratch)?;
        }
        hc.start()?;
        Ok(hc)
    }

    /// Rozsirene capability: (offset, id).
    fn xcaps(&self) -> Vec<(u64, u8)> {
        let mut v = Vec::new();
        let mut off = ((self.regs.cap32(HCCPARAMS1) >> 16) as u64) << 2;
        let mut guard = 0;
        while off != 0 && guard < 64 {
            let w = self.regs.read32(off);
            v.push((off, w as u8));
            let next = ((w >> 8) & 0xFF) as u64;
            off = if next == 0 { 0 } else { off + (next << 2) };
            guard += 1;
        }
        v
    }

    /// Prevzeti radice od firmwaru (USB Legacy Support): firmware prestane
    /// emulovat PS/2 a pustit radic.
    fn take_ownership(&self) {
        for (off, id) in self.xcaps() {
            if id != XCAP_LEGACY {
                continue;
            }
            let v = self.regs.read32(off);
            if v & LEGACY_BIOS_OWNED == 0 {
                return;
            }
            self.regs.write32(off, v | LEGACY_OS_OWNED);
            let deadline = time::deadline_after(Duration::from_secs(1));
            while self.regs.read32(off) & LEGACY_BIOS_OWNED != 0 {
                if time::now() > deadline {
                    println!("xhci: firmware radic nepustil, pokracuji");
                    break;
                }
                libos::time::sleep(Duration::from_millis(1));
            }
            // Vypnout SMI od radice (Legacy Support Control).
            self.regs.write32(off + 4, 0);
        }
    }

    /// Ktere porty jsou USB 3 (Supported Protocol, revize 3).
    fn find_protocols(&mut self) {
        for (off, id) in self.xcaps() {
            if id != XCAP_PROTOCOL {
                continue;
            }
            let major = (self.regs.read32(off) >> 24) as u8;
            let w2 = self.regs.read32(off + 8);
            let (first, count) = ((w2 & 0xFF) as u64, ((w2 >> 8) & 0xFF) as u64);
            if major >= 3 {
                for p in first..first + count {
                    if p < 64 {
                        self.usb3_ports |= 1 << p;
                    }
                }
            }
        }
    }

    fn wait_status(&self, mask: u32, set: bool, what: &str) -> Result<()> {
        let deadline = time::deadline_after(Duration::from_secs(1));
        while (self.regs.op32(USBSTS) & mask != 0) != set {
            if time::now() > deadline {
                println!("xhci: radic nedokoncil {what}");
                return Err(Error::TimedOut);
            }
            libos::time::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    fn reset(&self) -> Result<()> {
        let cmd = self.regs.op32(USBCMD);
        self.regs.set_op32(USBCMD, cmd & !CMD_RUN);
        self.wait_status(STS_HALTED, true, "zastaveni")?;
        self.regs.set_op32(USBCMD, CMD_RESET);
        let deadline = time::deadline_after(Duration::from_secs(1));
        while self.regs.op32(USBCMD) & CMD_RESET != 0 {
            if time::now() > deadline {
                return Err(Error::TimedOut);
            }
            libos::time::sleep(Duration::from_millis(1));
        }
        self.wait_status(STS_NOT_READY, false, "reset")
    }

    fn setup_scratchpad(&mut self, count: u32) -> Result<()> {
        let array = Dma::new(&self.res, count as u64 * 8)?;
        for i in 0..count as u64 {
            let page = Dma::new(&self.res, 4096)?;
            array.map.write::<u64>(i * 8, page.phys);
            self._scratch.push(page);
        }
        self.dcbaa.map.write::<u64>(0, array.phys);
        self._scratch.push(array);
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        self.regs.set_op32(CONFIG, self.max_slots as u32);
        self.regs.set_op64(DCBAAP, self.dcbaa.phys);
        self.regs.set_op64(CRCR, self.cmd.start());
        self.regs.set_ir32(ERSTSZ, 1);
        self.regs.set_ir64(ERDP, self.events.erdp());
        self.regs.set_ir64(ERSTBA, self.events.erst.phys);
        self.regs.set_ir32(IMOD, 4000); // 1 ms (jednotky 250 ns)
        self.regs.set_ir32(IMAN, IMAN_IE | IMAN_IP);
        self.regs.set_op32(USBCMD, CMD_RUN | CMD_INTE);
        self.wait_status(STS_HALTED, false, "start")
    }

    /// Zpracuje vsechny udalosti, ktere radic zapsal.
    pub fn poll_events(&mut self) {
        let mut any = false;
        while let Some(ev) = self.events.pop() {
            any = true;
            self.event(ev);
        }
        if any {
            self.regs.set_ir64(ERDP, self.events.erdp());
        }
        // Potvrdit preruseni interrupteru (MSI-X) a stav.
        self.regs.set_ir32(IMAN, IMAN_IE | IMAN_IP);
        let sts = self.regs.op32(USBSTS);
        if sts & STS_HOST_ERROR != 0 {
            println!("xhci: chyba radice (Host System Error)");
        }
        self.regs.set_op32(USBSTS, sts & (STS_EINT | STS_PCD));
    }

    fn event(&mut self, ev: Trb) {
        match ev.kind() {
            trb::CMD_COMPLETION => self.cmd_result = Some((ev.param, ev)),
            // Zmeny portu cte root hub primo z PORTSC (`port_changes`).
            trb::PORT_STATUS => {}
            trb::TRANSFER_EVENT => self.transfer_event(ev),
            _ => {}
        }
    }

    fn transfer_event(&mut self, ev: Trb) {
        let (slot, dci) = (ev.slot(), ev.endpoint());
        let Some(i) = self
            .pending
            .iter()
            .position(|p| p.slot == slot && p.dci == dci)
        else {
            return;
        };
        let residual = ev.status & 0xFF_FFFF;
        let code = ev.code();
        let p = &mut self.pending[i];
        if ev.param != p.last_trb {
            // Kratky paket na datovem TRB (ISP): TD pokracuje do status stage.
            if code == completion::SHORT_PACKET {
                p.short = Some(p.requested.saturating_sub(residual));
                return;
            }
            // Chyba na datovem TRB ukonci prenos.
        }
        let p = self.pending.swap_remove(i);
        let actual = match (p.short, ev.param == p.last_trb && p.dci != 1) {
            (Some(n), _) => n,
            // Normal TRB (interrupt): zbytek je v teto udalosti.
            (None, true) => p.requested.saturating_sub(residual),
            (None, false) => p.requested,
        };
        let ok = code == completion::SUCCESS || code == completion::SHORT_PACKET;
        let actual = if ok { actual.min(p.requested) } else { 0 };
        if !ok {
            self.halted.push((slot, dci));
        }
        let data = if ok && p.is_in && actual > 0 {
            self.read_buffer(slot, dci, actual)
        } else {
            Vec::new()
        };
        self.done.push(Done {
            owner: p.owner,
            slot,
            dci,
            code: if ok { completion::SUCCESS } else { code },
            actual,
            data,
        });
    }

    fn read_buffer(&self, slot: u8, dci: u8, len: u32) -> Vec<u8> {
        let mut v = vec![0u8; len as usize];
        if let Some(Some(d)) = self.devices.get(slot as usize)
            && let Some(buf) = d.buffer(dci)
        {
            buf.map.read_bytes(0, &mut v);
        }
        v
    }

    /// Pocka na preruseni (nebo termin) a zpracuje udalosti.
    pub fn wait_events(&mut self, deadline: u64) -> Result<()> {
        match self.irq.wait(Some(deadline)) {
            Ok(()) | Err(Error::TimedOut) => {}
            Err(e) => return Err(e),
        }
        self.poll_events();
        Ok(())
    }

    /// Provede prikaz a pocka na dokonceni; vraci udalost dokonceni.
    pub fn command(&mut self, t: Trb) -> Result<Trb> {
        let phys = self.cmd.push(t);
        self.regs.ring_doorbell(0, 0);
        let deadline = time::deadline_after(SYNC_TIMEOUT);
        loop {
            self.poll_events();
            if let Some((p, ev)) = self.cmd_result
                && p == phys
            {
                self.cmd_result = None;
                return match ev.code() {
                    completion::SUCCESS => Ok(ev),
                    code => {
                        println!("xhci: prikaz typu {} selhal (kod {code})", t.control >> 10 & 0x3F);
                        Err(Error::IoError)
                    }
                };
            }
            if time::now() > deadline {
                return Err(Error::TimedOut);
            }
            self.wait_events(time::deadline_after(Duration::from_millis(10)))?;
        }
    }

    /// Vlozi kontrolni prenos na endpoint 0 zarizeni `slot`; data OUT
    /// zapise do bufferu az po kontrole, ze endpoint nema rozpracovany
    /// prenos (jinak by prepsala jeho data).
    pub fn submit_control(&mut self, slot: u8, setup: Setup, out: &[u8], owner: Owner) -> Result<()> {
        if self.pending.iter().any(|p| p.slot == slot && p.dci == 1) {
            return Err(Error::AlreadyExists);
        }
        let dev = self.devices[slot as usize].as_mut().ok_or(Error::NotFound)?;
        if out.len() as u64 > dev.ep0_buf.len {
            return Err(Error::InvalidArgs);
        }
        if !out.is_empty() {
            dev.ep0_buf.map.write_bytes(0, out);
        }
        let len = setup.length as u32;
        let is_in = setup.is_in();
        let trt = match (len, is_in) {
            (0, _) => 0,
            (_, false) => 2,
            (_, true) => 3,
        };
        let buf_phys = dev.ep0_buf.phys;
        let ring = &mut dev.ep0;
        ring.push(Trb {
            param: u64::from_le_bytes(setup.to_bytes()),
            status: 8,
            control: trb::SETUP << 10 | trb::IDT | trt << 16,
        });
        if len > 0 {
            ring.push(Trb {
                param: buf_phys,
                status: len,
                control: trb::DATA << 10 | trb::ISP | if is_in { trb::DIR_IN } else { 0 },
            });
        }
        // Status stage ma opacny smer nez data (bez dat IN).
        let status_in = len == 0 || !is_in;
        let last = ring.push(Trb {
            param: 0,
            status: 0,
            control: trb::STATUS << 10 | trb::IOC | if status_in { trb::DIR_IN } else { 0 },
        });
        self.pending.push(Pending {
            slot,
            dci: 1,
            last_trb: last,
            owner,
            requested: len,
            short: None,
            is_in,
        });
        self.regs.ring_doorbell(slot, 1);
        Ok(())
    }

    /// Vlozi prenos Normal na endpoint `dci` (interrupt IN/OUT).
    pub fn submit_normal(&mut self, slot: u8, dci: u8, len: u32, owner: Owner) -> Result<()> {
        if self.pending.iter().any(|p| p.slot == slot && p.dci == dci) {
            return Err(Error::AlreadyExists);
        }
        let dev = self.devices[slot as usize].as_mut().ok_or(Error::NotFound)?;
        let ep = dev.endpoint_mut(dci).ok_or(Error::NotFound)?;
        if len as u64 > ep.buf.len {
            return Err(Error::InvalidArgs);
        }
        let last = ep.ring.push(Trb {
            param: ep.buf.phys,
            status: len,
            control: trb::NORMAL << 10 | trb::IOC | trb::ISP,
        });
        self.pending.push(Pending {
            slot,
            dci,
            last_trb: last,
            owner,
            requested: len,
            short: None,
            is_in: dci & 1 == 1,
        });
        self.regs.ring_doorbell(slot, dci as u32);
        Ok(())
    }

    /// Synchronni kontrolni prenos (enumerace): vraci prectena data.
    pub fn control_sync(&mut self, slot: u8, setup: Setup, out: &[u8]) -> Result<Vec<u8>> {
        self.submit_control(slot, setup, out, Owner::Internal)?;
        let deadline = time::deadline_after(SYNC_TIMEOUT);
        loop {
            if let Some(i) = self
                .done
                .iter()
                .position(|d| d.owner == Owner::Internal && d.slot == slot && d.dci == 1)
            {
                let d = self.done.swap_remove(i);
                self.recover_halted();
                if d.code != completion::SUCCESS {
                    return Err(Error::IoError);
                }
                return Ok(d.data);
            }
            if time::now() > deadline {
                self.pending.retain(|p| !(p.slot == slot && p.dci == 1));
                return Err(Error::TimedOut);
            }
            self.wait_events(time::deadline_after(Duration::from_millis(10)))?;
        }
    }

    /// Endpoint po chybe prenosu (STALL, chyba transakce, babble ...) je
    /// zastaveny: reset a posun fronty za nedokonceny prenos. Zarizeni,
    /// ktere uz neexistuje (odpojene), nic nepotrebuje.
    pub fn recover(&mut self, slot: u8, dci: u8) {
        if !matches!(self.devices.get(slot as usize), Some(Some(_))) {
            return;
        }
        let reset = Trb {
            param: 0,
            status: 0,
            control: trb::RESET_EP << 10 | (dci as u32) << 16 | (slot as u32) << 24,
        };
        let _ = self.command(reset);
        let Some(Some(dev)) = self.devices.get(slot as usize) else {
            return;
        };
        let deq = match dci {
            1 => dev.ep0.current(),
            d => match dev.endpoint(d) {
                Some(e) => e.ring.current(),
                None => return,
            },
        };
        let _ = self.command(Trb {
            param: deq,
            status: 0,
            control: trb::SET_TR_DEQUEUE << 10 | (dci as u32) << 16 | (slot as u32) << 24,
        });
    }

    /// Obnovi endpointy zastavene chybou prenosu.
    pub fn recover_halted(&mut self) {
        while let Some((slot, dci)) = self.halted.pop() {
            self.recover(slot, dci);
        }
    }

    /// Zapise adresu vystupniho kontextu zarizeni do DCBAA.
    pub fn set_dcbaa(&self, slot: u8, phys: u64) {
        self.dcbaa.map.write::<u64>(slot as u64 * 8, phys);
    }

    /// Zrusi rozpracovane prenosy zarizeni (odpojeno): klienti dostanou
    /// chybu.
    pub fn cancel_device(&mut self, slot: u8) {
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].slot == slot {
                let p = self.pending.swap_remove(i);
                self.done.push(Done {
                    owner: p.owner,
                    slot,
                    dci: p.dci,
                    code: completion::TRANSACTION,
                    actual: 0,
                    data: Vec::new(),
                });
            } else {
                i += 1;
            }
        }
    }

    /// Kanal preruseni (pro cekani v hlavni smycce).
    pub fn irq_signals(&self) -> (&libos::Handle, Signals) {
        (self.irq.handle(), Signals::SIGNALED)
    }
}
