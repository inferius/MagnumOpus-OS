//! r8169: ovladac sitove karty Realtek RTL8111/8168.
//!
//! Logika cipu je v `nic::r8169`, startovni zprava a smycka vuci netd
//! v `nic::driver` (spolecne pro vsechny ovladace karet s MMIO).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;

use libos::Startup;
use nic::driver::{Device, Mmio};
use nic::{HwError, Nic};

libos::entry!(main);

fn open(hw: Mmio, dev: &Device, seed: u64) -> Result<Box<dyn Nic>, HwError> {
    Ok(Box::new(nic::r8169::R8169::open(hw, dev.device, seed)?))
}

fn main(startup: Startup) -> i64 {
    nic::driver::run("r8169", startup, open)
}
