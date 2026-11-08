//! HDMI a DisplayPort: nastaveni digitalnich cest, ELD monitoru
//! a audio infoframe (`hdaspec::hdmi`).
//!
//! Digitalni vystup se zapne, jen kdyz je monitor pripojeny a ELD platne;
//! pak jeho prevodnik dostane stejny stream jako analogove vystupy
//! (stereo, format zarizeni). Vicekanalovy zvuk a vyber kanalu (channel
//! mapping), DP MST a overeni formatu proti SAD monitoru zatim nejsou.

use alloc::vec::Vec;

use hdaspec::graph::Path;
use hdaspec::hdmi::{self, Eld};
use hdaspec::verb;
use libos::{Result, println};

use crate::codec::Codec;
use crate::ctrl::Ctrl;

/// Nejvetsi ELD, ktere cteme (specifikace: nejvys 256 bajtu).
const MAX_ELD: u32 = 256;

pub struct Hdmi {
    pub path: Path,
    pub present: bool,
    pub eld: Option<Eld>,
}

impl Hdmi {
    /// Hraje (monitor pripojen a umi PCM)?
    pub fn active(&self) -> bool {
        self.present && self.eld.as_ref().is_some_and(|e| e.lpcm_channels > 0)
    }
}

impl Codec {
    /// Pripravi digitalni cesty (napajeni, vybery, pin, prevodnik).
    pub fn setup_hdmi(&mut self, c: &mut Ctrl) -> Result<()> {
        let a = self.addr;
        for h in &self.hdmi {
            for s in &h.path.steps {
                if let Some(w) = self.graph.get(s.nid) {
                    if w.caps.power_control {
                        c.verb12(a, s.nid, verb::SET_POWER_STATE, verb::POWER_D0)?;
                    }
                    if let Some(idx) = s.input
                        && w.conns.len() > 1
                    {
                        c.verb12(a, s.nid, verb::SET_CONN_SELECT, idx)?;
                    }
                }
            }
            c.verb12(a, h.path.pin, verb::SET_PIN_CONTROL, verb::PIN_OUT)?;
            // Digitalni prevodnik zapnout (DigEn), dva kanaly, sloty 0 a 1.
            c.verb12(a, h.path.dac, verb::SET_DIGI_CONVERT_1, 1)?;
            c.verb12(a, h.path.dac, hdmi::verb::SET_CVT_CHAN_COUNT, 1)?;
            for ch in 0..2u8 {
                let _ = c.verb12(a, h.path.pin, hdmi::verb::SET_CHAN_SLOT, ch << 4 | ch);
            }
        }
        for i in 0..self.hdmi.len() {
            self.sense_hdmi(c, i);
        }
        Ok(())
    }

    /// Precte pripojeni monitoru a ELD; pri zmene zapise infoframe. Vrati
    /// `true`, kdyz se zmenilo, zda vystup hraje.
    pub fn sense_hdmi(&mut self, c: &mut Ctrl, i: usize) -> bool {
        let a = self.addr;
        let pin = self.hdmi[i].path.pin;
        let before = self.hdmi[i].active();
        if self.graph.get(pin).and_then(|w| w.pin_caps).is_some_and(|p| p.trigger) {
            let _ = c.verb12(a, pin, verb::EXEC_PIN_SENSE, 0);
        }
        let sense = c.verb12(a, pin, verb::GET_PIN_SENSE, 0).unwrap_or(0);
        let present = sense & hdmi::SENSE_PRESENT != 0;
        let eld = if present && sense & hdmi::SENSE_ELDV != 0 { read_eld(c, a, pin) } else { None };
        let changed = present != self.hdmi[i].present || eld != self.hdmi[i].eld;
        self.hdmi[i].present = present;
        self.hdmi[i].eld = eld;
        if changed {
            match &self.hdmi[i].eld {
                Some(e) => {
                    println!("hda: monitor '{}' na pinu {pin:#x} ({}, PCM {} kanalu)", e.monitor, if e.dp { "DP" } else { "HDMI" }, e.lpcm_channels);
                    let frame = hdmi::audio_infoframe(2, e.dp);
                    let _ = c.verb12(a, pin, hdmi::verb::SET_DIP_INDEX, 0);
                    for b in frame {
                        let _ = c.verb12(a, pin, hdmi::verb::SET_DIP_DATA, b);
                    }
                    let _ = c.verb12(a, pin, hdmi::verb::SET_DIP_XMIT, hdmi::verb::XMIT_BEST);
                }
                None if present => println!("hda: monitor na pinu {pin:#x} bez platneho ELD"),
                None => {}
            }
        }
        before != self.hdmi[i].active()
    }
}

fn read_eld(c: &mut Ctrl, a: u8, pin: u16) -> Option<Eld> {
    let size = c.verb12(a, pin, hdmi::verb::GET_DIP_SIZE, hdmi::verb::DIPSIZE_ELD).ok()? & 0xFF;
    let size = if size == 0 { MAX_ELD } else { size.min(MAX_ELD) };
    let mut b = Vec::with_capacity(size as usize);
    for i in 0..size {
        let v = c.verb12(a, pin, hdmi::verb::GET_ELDD, i as u8).ok()?;
        if v & 1 << 31 == 0 {
            break; // za koncem platnych dat
        }
        b.push(v as u8);
    }
    hdmi::parse_eld(&b)
}
