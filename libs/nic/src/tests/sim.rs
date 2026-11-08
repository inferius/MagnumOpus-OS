//! Simulace karty pro testy: registry (model cipu) a DMA pamet, do ktere
//! model saha jako skutecne zarizeni (cte deskriptory a buffery podle
//! adres, ktere do nich zapsal ovladac).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::vec;
use std::vec::Vec;

use crate::Hw;

/// DMA pamet s adresou pro zarizeni `base`. Pristup mimo ni = chyba
/// ovladace (spatna adresa v deskriptoru nebo registru).
pub struct Mem {
    pub data: RefCell<Vec<u8>>,
    pub base: u64,
}

impl Mem {
    fn index(&self, addr: u64, len: usize) -> usize {
        let data = self.data.borrow();
        assert!(
            addr >= self.base && addr + len as u64 <= self.base + data.len() as u64,
            "zarizeni sahlo mimo DMA pamet: {addr:#x}+{len}"
        );
        (addr - self.base) as usize
    }

    pub fn read(&self, addr: u64, buf: &mut [u8]) {
        let i = self.index(addr, buf.len());
        buf.copy_from_slice(&self.data.borrow()[i..i + buf.len()]);
    }

    pub fn write(&self, addr: u64, src: &[u8]) {
        let i = self.index(addr, src.len());
        self.data.borrow_mut()[i..i + src.len()].copy_from_slice(src);
    }

    pub fn read32(&self, addr: u64) -> u32 {
        let mut b = [0u8; 4];
        self.read(addr, &mut b);
        u32::from_le_bytes(b)
    }

    pub fn write32(&self, addr: u64, v: u32) {
        self.write(addr, &v.to_le_bytes());
    }

    pub fn read64(&self, addr: u64) -> u64 {
        let mut b = [0u8; 8];
        self.read(addr, &mut b);
        u64::from_le_bytes(b)
    }

    pub fn bytes(&self, addr: u64, len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len];
        self.read(addr, &mut v);
        v
    }
}

/// Model registru jednoho cipu.
pub trait Model {
    /// `width` = 1, 2 nebo 4 bajty.
    fn read(&mut self, mem: &Mem, reg: u32, width: u8) -> u32;
    fn write(&mut self, mem: &Mem, reg: u32, width: u8, v: u32);
    /// Simulovany cas pokrocil (cekani ovladace).
    fn tick(&mut self, _us: u64) {}
}

/// Ovladani simulace v testech (spolecne pro vsechny cipy).
pub trait Control {
    /// Prijeti ramce ze site (bez FCS). `false` = karta ho zahodila (plny
    /// kruh, prijem vypnuty).
    fn deliver(&mut self, mem: &Mem, frame: &[u8]) -> bool;
    /// Ramce odeslane do site od posledniho volani.
    fn take_sent(&mut self) -> Vec<Vec<u8>>;
    fn set_link(&mut self, up: bool);
    /// Kolikrat karta poslala preruseni.
    fn interrupts(&self) -> u32;
    /// Karta zmizi ze sbernice (cteni vraci same jednicky).
    fn unplug(&mut self);
}

pub struct Sim<M> {
    pub mem: Mem,
    pub model: RefCell<M>,
    pub slept_us: Cell<u64>,
}

pub type Handle<M> = Rc<Sim<M>>;

pub fn new<M>(model: M, base: u64) -> Handle<M> {
    Rc::new(Sim {
        mem: Mem {
            data: RefCell::new(vec![0u8; crate::Layout::SIZE]),
            base,
        },
        model: RefCell::new(model),
        slept_us: Cell::new(0),
    })
}

impl<M: Control> Sim<M> {
    pub fn deliver(&self, frame: &[u8]) -> bool {
        self.model.borrow_mut().deliver(&self.mem, frame)
    }
    pub fn take_sent(&self) -> Vec<Vec<u8>> {
        self.model.borrow_mut().take_sent()
    }
    pub fn set_link(&self, up: bool) {
        self.model.borrow_mut().set_link(up)
    }
    pub fn interrupts(&self) -> u32 {
        self.model.borrow().interrupts()
    }
    pub fn unplug(&self) {
        self.model.borrow_mut().unplug()
    }
}

impl<M: Model> Hw for Handle<M> {
    fn read8(&self, reg: u32) -> u8 {
        self.model.borrow_mut().read(&self.mem, reg, 1) as u8
    }
    fn read16(&self, reg: u32) -> u16 {
        self.model.borrow_mut().read(&self.mem, reg, 2) as u16
    }
    fn read32(&self, reg: u32) -> u32 {
        self.model.borrow_mut().read(&self.mem, reg, 4)
    }
    fn write8(&self, reg: u32, v: u8) {
        self.model.borrow_mut().write(&self.mem, reg, 1, v.into())
    }
    fn write16(&self, reg: u32, v: u16) {
        self.model.borrow_mut().write(&self.mem, reg, 2, v.into())
    }
    fn write32(&self, reg: u32, v: u32) {
        self.model.borrow_mut().write(&self.mem, reg, 4, v)
    }
    fn dma_addr(&self) -> u64 {
        self.mem.base
    }
    fn dma_read(&self, off: usize, buf: &mut [u8]) {
        self.mem.read(self.mem.base + off as u64, buf)
    }
    fn dma_write(&self, off: usize, data: &[u8]) {
        self.mem.write(self.mem.base + off as u64, data)
    }
    fn dma_read32(&self, off: usize) -> u32 {
        self.mem.read32(self.mem.base + off as u64)
    }
    fn dma_write32(&self, off: usize, v: u32) {
        self.mem.write32(self.mem.base + off as u64, v)
    }
    fn dma_wmb(&self) {}
    fn dma_rmb(&self) {}
    fn mmio_wmb(&self) {}
    fn sleep_us(&self, us: u64) {
        self.slept_us.set(self.slept_us.get() + us);
        self.model.borrow_mut().tick(us);
    }
}

/// Registry jako mapa 32bitovych slov (cipy Intel: jen 32bitove pristupy).
#[derive(Default)]
pub struct Regs32(pub std::collections::BTreeMap<u32, u32>);

impl Regs32 {
    pub fn get(&self, reg: u32) -> u32 {
        self.0.get(&reg).copied().unwrap_or(0)
    }
    pub fn set(&mut self, reg: u32, v: u32) {
        self.0.insert(reg, v);
    }
}

/// Testovaci ramec: cilova MAC, zdrojova, EtherType 0x88B5 (lokalni
/// experimenty), `n` jako obsah.
pub fn frame(n: u32, len: usize) -> Vec<u8> {
    let mut f = vec![0u8; len];
    f[..6].copy_from_slice(&[0x02, 0, 0, 0, 0, 1]);
    f[6..12].copy_from_slice(&[0x02, 0, 0, 0, 0, 2]);
    f[12..14].copy_from_slice(&[0x88, 0xB5]);
    for (i, b) in f[14..].iter_mut().enumerate() {
        *b = (n as usize).wrapping_mul(31).wrapping_add(i) as u8;
    }
    if len >= 18 {
        f[14..18].copy_from_slice(&n.to_le_bytes());
    }
    f
}
