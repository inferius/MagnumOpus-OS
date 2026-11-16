//! Zarizeni pro nahravani.
//!
//! Nahrava se ze zvoleneho zarizeni (`SELECT_INPUT`, [`Service::input_pref`]),
//! jinak z aktivniho, kdyz ma vstup. Kdyz ne (napr. vystup jde pres kartu
//! bez mikrofonu), vezme se prvni jine zarizeni se vstupem a otevre se
//! zvlast ([`Service::rec`]); jeho vystup se nespousti.

use libos::{Channel, println};

use audioproto::{Direction, Error, Result};

use crate::device::Device;
use crate::service::Service;

impl Service {
    /// Nahrava se z aktivniho zarizeni? (Ma vstup a uzivatel nezvolil jine.)
    fn active_has_input(&self) -> bool {
        self.device.as_ref().is_some_and(|d| d.info.input) && self.input_pref.is_none_or(|j| Some(j) == self.active)
    }

    /// Zarizeni, ze ktereho se nahrava nebo bude nahravat (`OVERVIEW`).
    pub fn input_index(&self) -> Option<usize> {
        if self.active_has_input() {
            return self.active;
        }
        if let Some((j, _)) = &self.rec {
            return Some(*j);
        }
        match self.input_pref {
            Some(j) if self.slots.get(j).is_some_and(|s| s.chan.is_some() && s.info.input) => Some(j),
            _ => self.slots.iter().position(|s| s.chan.is_some() && s.info.input),
        }
    }

    /// Zvoli zarizeni pro nahravani (255 = automaticky). Za behu nahravani
    /// se zmena projevi u dalsiho otevreni vstupu (`BUSY` by uzivatele
    /// zbytecne zdrzel; bezici zaznam se neprerusi).
    pub fn select_input(&mut self, index: u8) -> Result<()> {
        let pref = if index == u8::MAX { None } else { Some(index as usize) };
        if let Some(j) = pref {
            let ok = Some(j) == self.active && self.device.as_ref().is_some_and(|d| d.info.input) || self.slots.get(j).is_some_and(|s| s.info.input && (s.chan.is_some() || self.rec.as_ref().is_some_and(|r| r.0 == j)));
            if !ok {
                return Err(Error::InvalidArgs);
            }
        }
        if self.input_pref == pref {
            return Ok(());
        }
        self.input_pref = pref;
        let capturing = self.streams.iter().any(|s| s.dir == Direction::Input && s.active());
        if !capturing && self.rec.as_ref().is_some_and(|r| Some(r.0) != pref) {
            self.release_rec();
        }
        println!("audiod: nahravani ze zarizeni {}", pref.map_or(alloc::string::String::from("automaticky"), |j| alloc::format!("{j}")));
        Ok(())
    }

    /// Zarizeni pro nahravani (otevre samostatne, kdyz je treba).
    pub fn input_dev(&mut self) -> Result<&mut Device> {
        if self.active_has_input() {
            return self.device.as_mut().ok_or(Error::NotFound);
        }
        if self.rec.is_none() {
            let pick = |s: &crate::service::Slot| s.chan.is_some() && s.info.input;
            let j = match self.input_pref {
                Some(j) if self.slots.get(j).is_some_and(pick) => j,
                _ => self.slots.iter().position(pick).ok_or(Error::NotSupported)?,
            };
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
