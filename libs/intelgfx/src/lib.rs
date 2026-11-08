//! Obrazova cast grafiky Intel (Gen9 Skylake az Xe2) bez akcelerace:
//! rozpoznani generace, cteni stavu od firmwaru (GOP), EDID pres GMBUS
//! a AUX, hotplug, prevzeti obrazu s page flipem ve vsync, zmena
//! rozliseni pres scaler pipe a zaznam/obnova stavu firmwaru.
//!
//! Bez zavislosti na jadre OS: pristup k registrum jde pres [`Mmio`],
//! takze vse bezi i na hostiteli nad simulovanymi registry
//! ([`sim::Sim`], `cargo test -p intelgfx`). Ovladac v OS je
//! `user/intel-display`.
//!
//! - [`platform`]: PCI ID -> generace.
//! - [`regs`]: offsety a bity registru.
//! - [`state`]: stav pipe/transcoderu/roviny/DDI od firmwaru.
//! - [`gtt`]: polozky GGTT (kde lezi framebuffer).
//! - [`dpaux`]: kanal AUX DisplayPortu (DPCD, EDID).
//! - [`gmbus`]: I2C pro DDC (EDID u HDMI/DVI).
//! - [`hotplug`]: stav pripojeni portu.
//! - [`backend`]: [`displaydrv::Backend`] pro kostru ovladace.
//! - [`phy`]: vzory treninku DP a buzeni (Gen9) pro `display::dp::train`.
//!
//! **Puvod**: registry a postupy podle linuxoveho i915 (soubory pod MIT,
//! seznam v THIRD-PARTY.md) a verejnych Intel PRM; kod napsany znovu.
//! **Neovereno na hardwaru** - viz `docs/cs/displej.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod backend;
pub mod dpaux;
pub mod gmbus;
pub mod gtt;
pub mod hotplug;
pub mod phy;
pub mod platform;
pub mod regs;
pub mod sim;
pub mod state;

#[cfg(test)]
mod tests;

/// Pristup k registrum MMIO (BAR0). Offsety v bajtech.
pub trait Mmio {
    fn read32(&mut self, reg: u32) -> u32;
    fn write32(&mut self, reg: u32, v: u32);

    fn read64(&mut self, reg: u32) -> u64 {
        self.read32(reg) as u64 | (self.read32(reg + 4) as u64) << 32
    }

    fn write64(&mut self, reg: u32, v: u64) {
        self.write32(reg, v as u32);
        self.write32(reg + 4, (v >> 32) as u32);
    }

    fn rmw(&mut self, reg: u32, clear: u32, set: u32) {
        let v = self.read32(reg);
        self.write32(reg, (v & !clear) | set);
    }
}

/// Cas a cekani (OS nebo simulace).
pub trait Clock {
    /// Monotonni cas v ns.
    fn now(&self) -> u64;
    fn delay_us(&mut self, us: u32);
}

/// Ceka, az `cond(cteni registru)` plati, nejvys `timeout_us`.
pub fn wait_for<M: Mmio + ?Sized, C: Clock + ?Sized>(m: &mut M, c: &mut C, reg: u32, timeout_us: u32, cond: impl Fn(u32) -> bool) -> Option<u32> {
    let mut waited = 0;
    loop {
        let v = m.read32(reg);
        if cond(v) {
            return Some(v);
        }
        if waited >= timeout_us {
            return None;
        }
        c.delay_us(10);
        waited += 10;
    }
}
