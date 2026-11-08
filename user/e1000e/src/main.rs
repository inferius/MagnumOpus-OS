//! e1000e: ovladac sitove karty Intel e1000e: 82574L/LA, 82583V a I217/I218/I219 (PCH).
//!
//! Logika cipu je v `nic::e1000e`, startovni zprava a smycka vuci netd
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
    Ok(Box::new(nic::e1000e::E1000e::open(hw, dev.device, seed)?))
}

fn main(startup: Startup) -> i64 {
    nic::driver::run("e1000e", startup, open)
}
