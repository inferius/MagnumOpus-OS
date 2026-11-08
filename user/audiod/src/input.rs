//! Zarizeni pro nahravani.
//!
//! Nahrava se z aktivniho zarizeni, kdyz ma vstup. Kdyz ne (napr. vystup
//! jde pres kartu bez mikrofonu), vezme se prvni jine zarizeni se vstupem
//! a otevre se zvlast ([`Service::rec`]); jeho vystup se nespousti.

use libos::{Channel, println};

use audioproto::{Direction, Error, Result};

use crate::device::Device;
use crate::service::Service;

impl Service {
    /// Ma aktivni zarizeni vlastni vstup?
    fn active_has_input(&self) -> bool {
        self.device.as_ref().is_some_and(|d| d.info.input)
    }

    /// Zarizeni pro nahravani (otevre samostatne, kdyz je treba).
    pub fn input_dev(&mut self) -> Result<&mut Device> {
        if self.active_has_input() {
            return self.device.as_mut().ok_or(Error::NotFound);
        }
        if self.rec.is_none() {
            let j = self
                .slots
                .iter()
                .position(|s| s.chan.is_some() && s.info.input)
                .ok_or(Error::NotSupported)?;
            let chan = self.slots[j].chan.take().ok_or(Error::NotFound)?;
            let info = self.slots[j].info.clone();
            match Device::open(Channel::from_handle(chan.handle().duplicate(None)?), info) {
                Ok(d) => {
                    println!("audiod: nahravani ze zarizeni {j} ({})", d.info.name);
                    self.rec = Some((j, d));
                }
                Err(e) => {
                    self.slots[j].chan = Some(chan);
                    return Err(e);
                }
            }
        }
        self.rec.as_mut().map(|r| &mut r.1).ok_or(Error::NotFound)
    }

    /// Zarizeni, ze ktereho se prave nahrava (bez otevirani).
    pub fn input_dev_mut(&mut self) -> Option<&mut Device> {
        if self.active_has_input() {
            self.device.as_mut()
        } else {
            self.rec.as_mut().map(|r| &mut r.1)
        }
    }

    /// Je k dispozici nejaky vstup?
    pub fn any_input(&self) -> bool {
        self.active_has_input() || self.rec.is_some() || self.slots.iter().any(|s| s.chan.is_some() && s.info.input)
    }

    /// Zavre samostatne nahravaci zarizeni a vrati jeho kanal do seznamu.
    pub fn release_rec(&mut self) {
        if let Some((j, mut d)) = self.rec.take() {
            d.close();
            if let Ok(h) = d.channel().handle().duplicate(None) {
                self.slots[j].chan = Some(Channel::from_handle(h));
            }
        }
    }

    /// Ovladac samostatneho nahravaciho zarizeni skoncil.
    pub fn rec_lost(&mut self) {
        if let Some((j, d)) = self.rec.take() {
            println!("audiod: ovladac nahravaciho zarizeni {} skoncil", d.info.name);
            self.slots[j].chan = None;
        }
        for s in self.streams.iter_mut().filter(|s| s.dir == Direction::Input) {
            s.stop();
        }
    }
}
