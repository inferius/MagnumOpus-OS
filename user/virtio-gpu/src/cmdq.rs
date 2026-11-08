//! Fronta prikazu virtio-gpu s vic soubeznymi pozadavky.
//!
//! Kazdy pozadavek ma vlastni slot: kus DMA pameti pro prikaz a odpoved
//! a dvojici deskriptoru (hlava = `2 * slot`). Synchronni prikaz
//! (`command`) ceka jen na svuj slot; asynchronni (`submit_async`, u
//! `SUBMIT_3D` s plotem) se jen odesle a jeho dokonceni (`Completion`)
//! vyzvedne hlavni smycka ovladace, az zarizeni slot vrati (preruseni je
//! ve `wait_many` smycky). Pomaly renderer jednoho klienta tak neblokuje
//! displej ani jine klienty.
//!
//! Zarizeni, ktere vrati neznamy slot nebo neodpovi na synchronni prikaz
//! vcas, je vadne (`broken`): ovladac ho pak resetuje a skonci.

use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Error, Mapping, Result, Vmo};
use virtio::Buffer;

/// Velikost slotu: prikaz (do `RESP_OFF`, i proud `SUBMIT_3D` 60 KiB
/// + hlavicka) a odpoved (za `RESP_OFF`, i obsah capsetu).
const SLOT_SIZE: u64 = 72 * 1024;
const RESP_OFF: u64 = 64 * 1024;
/// Nejvic slotu (32 x 72 KiB = 2,25 MiB souvisle DMA pameti).
const MAX_SLOTS: usize = 32;
/// Sloty, ktere asynchronni pozadavky nikdy nedostanou: synchronni prikaz
/// (displej, bloby) tak ma vzdy volny slot, i kdyz plot klienta visi.
const RESERVED: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Free,
    /// Ceka na nej `command`.
    Sync,
    /// Asynchronni pozadavek klienta `tag`.
    Async { tag: u64 },
}

/// Dokonceny asynchronni pozadavek: klient a hlavicka odpovedi.
#[derive(Debug, Clone, Copy)]
pub struct Completion {
    pub tag: u64,
    pub resp_type: u32,
}

pub struct CmdQueue {
    pub device: virtio::Device,
    queue: virtio::Virtqueue,
    pub irq: Interrupt,
    _vmo: Vmo,
    map: Mapping,
    phys: u64,
    slots: Vec<Slot>,
    /// Dokoncene asynchronni pozadavky, ktere jeste nikdo nevyzvedl.
    done: Vec<Completion>,
    /// Slot, ktery prave vratilo zarizeni a ceka na nej `command`.
    finished_sync: Vec<usize>,
    pub broken: bool,
}

impl CmdQueue {
    pub fn new(device: virtio::Device, queue: virtio::Virtqueue, irq: Interrupt, resource: &Resource) -> Result<Self> {
        let n = (queue.size() as usize / 2).clamp(1, MAX_SLOTS);
        let size = SLOT_SIZE * n as u64;
        let vmo = resource.dma_memory(size)?;
        let phys = resource.phys_addr(&vmo, 0)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        Ok(Self {
            device,
            queue,
            irq,
            _vmo: vmo,
            map,
            phys,
            slots: alloc::vec![Slot::Free; n],
            done: Vec::new(),
            finished_sync: Vec::new(),
            broken: false,
        })
    }

    fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| *s == Slot::Free)
    }

    /// Zapise prikaz do slotu a preda ho zarizeni.
    fn post(&mut self, i: usize, c: &[u8], resp_len: usize) -> Result<()> {
        let base = i as u64 * SLOT_SIZE;
        self.map.write_bytes(base, c);
        self.map.write_bytes(base + RESP_OFF, &[0u8; vgpu::HEADER_SIZE]);
        let phys = self.phys + base;
        let chain = [
            Buffer { phys, len: c.len() as u32, device_writes: false },
            Buffer { phys: phys + RESP_OFF, len: resp_len as u32, device_writes: true },
        ];
        self.queue.submit_at((2 * i) as u16, &chain)?;
        self.device.notify(&self.queue);
        Ok(())
    }

    fn check_sizes(c: &[u8], resp_len: usize) -> Result<()> {
        if c.len() > RESP_OFF as usize || resp_len > (SLOT_SIZE - RESP_OFF) as usize || resp_len < vgpu::HEADER_SIZE {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }

    /// Vyzvedne vsechny slotu, ktere zarizeni vratilo. Neznamy slot nebo
    /// slot, ktery nikdo neodeslal = vadne zarizeni.
    pub fn poll(&mut self) -> Result<()> {
        if self.broken {
            return Err(Error::IoError);
        }
        loop {
            let id = match self.queue.take_posted() {
                Ok(Some((id, _))) => id as usize,
                Ok(None) => return Ok(()),
                Err(e) => {
                    self.broken = true;
                    return Err(e);
                }
            };
            let i = id / 2;
            if id % 2 != 0 || i >= self.slots.len() {
                self.broken = true;
                return Err(Error::Corrupted);
            }
            match self.slots[i] {
                Slot::Free => {
                    self.broken = true;
                    return Err(Error::Corrupted);
                }
                Slot::Sync => self.finished_sync.push(i),
                Slot::Async { tag } => {
                    let resp_type: u32 = self.map.read(i as u64 * SLOT_SIZE + RESP_OFF);
                    self.done.push(Completion { tag, resp_type });
                    self.slots[i] = Slot::Free;
                }
            }
        }
    }

    /// Posle prikaz a pocka na jeho odpoved (`resp_len` bajtu). Mezitim
    /// dokoncene asynchronni pozadavky se ulozi (`take_completions`).
    /// Vyprseni nebo chyba fronty = zarizeni je vadne.
    pub fn command(&mut self, c: &[u8], resp_len: usize, timeout: Duration) -> Result<Vec<u8>> {
        if self.broken {
            return Err(Error::IoError);
        }
        Self::check_sizes(c, resp_len)?;
        // Volny slot je vzdy (`RESERVED`, synchronni prikaz je jen jeden).
        let i = self.free_slot().ok_or(Error::NoResources)?;
        self.slots[i] = Slot::Sync;
        if let Err(e) = self.post(i, c, resp_len) {
            self.slots[i] = Slot::Free;
            return Err(e);
        }
        self.wait_until(timeout, |q| q.finished_sync.contains(&i))?;
        self.finished_sync.retain(|&s| s != i);
        self.slots[i] = Slot::Free;
        let mut resp = alloc::vec![0u8; resp_len];
        self.map.read_bytes(i as u64 * SLOT_SIZE + RESP_OFF, &mut resp);
        Ok(resp)
    }

    /// Ceka (preruseni), dokud neplati `done`; vyprseni = vadne zarizeni.
    fn wait_until(&mut self, timeout: Duration, done: impl Fn(&Self) -> bool) -> Result<()> {
        let deadline = libos::time::deadline_after(timeout);
        loop {
            self.poll()?;
            if done(self) {
                return Ok(());
            }
            if let Err(e) = self.irq.wait(Some(deadline)) {
                self.broken = true;
                return Err(e);
            }
        }
    }

    /// Odesle prikaz bez cekani; dokonceni prijde jako `Completion` s `tag`.
    /// Bez volneho slotu `SHOULD_WAIT`.
    pub fn submit_async(&mut self, c: &[u8], tag: u64) -> Result<()> {
        if self.broken {
            return Err(Error::IoError);
        }
        Self::check_sizes(c, vgpu::HEADER_SIZE)?;
        if self.free_slots() <= RESERVED {
            return Err(Error::ShouldWait);
        }
        let i = self.free_slot().ok_or(Error::ShouldWait)?;
        self.slots[i] = Slot::Async { tag };
        if let Err(e) = self.post(i, c, vgpu::HEADER_SIZE) {
            self.slots[i] = Slot::Free;
            return Err(e);
        }
        Ok(())
    }

    /// Dokoncene asynchronni pozadavky (od posledniho volani).
    pub fn take_completions(&mut self) -> Vec<Completion> {
        core::mem::take(&mut self.done)
    }

    /// Volnych slotu (pro limit asynchronnich pozadavku).
    pub fn free_slots(&self) -> usize {
        self.slots.iter().filter(|s| **s == Slot::Free).count()
    }
}
