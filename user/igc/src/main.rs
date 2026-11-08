//! igc: ovladac sitove karty Intel igc: I225, I226 (2,5 Gb/s), I220-V, I221-V.
//!
//! Logika cipu je v `nic::igc`, startovni zprava a smycka vuci netd
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
    Ok(Box::new(nic::igc::Igc::open(hw, dev.device, seed)?))
}

fn main(startup: Startup) -> i64 {
    nic::driver::run("igc", startup, open)
}
