//! Nahravani: nastaveni cesty ADC -> mikrofon / linkovy vstup
//! (`hdaspec::input`) a prirazeni streamu prevodniku A/D.

use hdaspec::caps::WidgetKind;
use hdaspec::input::InKind;
use hdaspec::verb::{self, AmpSet};
use libos::Result;

use crate::codec::Codec;
use crate::ctrl::Ctrl;

impl Codec {
    /// Ma kodek cestu pro nahravani?
    pub fn has_input(&self) -> bool {
        self.input.is_some()
    }

    /// Nastavi cestu pro nahravani: napajeni, vybery, vstupni zesilovace na
    /// 0 dB, pin jako vstup (u mikrofonu napajeni 80 %, kdyz ho pin umi).
    pub fn setup_input(&self, c: &mut Ctrl) -> Result<()> {
        let Some(p) = self.input.as_ref() else { return Ok(()) };
        let a = self.addr;
        for s in &p.steps {
            let Some(w) = self.graph.get(s.nid) else { continue };
            if w.caps.power_control {
                c.verb12(a, s.nid, verb::SET_POWER_STATE, verb::POWER_D0)?;
            }
            let Some(idx) = s.input else { continue };
            if w.caps.kind != WidgetKind::Mixer && w.conns.len() > 1 {
                c.verb12(a, s.nid, verb::SET_CONN_SELECT, idx)?;
            }
            if w.caps.in_amp {
                if w.caps.kind == WidgetKind::Mixer {
                    for i in 0..w.conns.len().min(16) as u8 {
                        let on = i == idx;
                        let gain = if on { w.amp_in.unity() } else { 0 };
                        c.verb4(a, s.nid, verb::SET_AMP_GAIN_MUTE, AmpSet::input(i, gain, !on).encode())?;
                    }
                } else {
                    c.verb4(a, s.nid, verb::SET_AMP_GAIN_MUTE, AmpSet::input(idx, w.amp_in.unity(), false).encode())?;
                }
            }
        }
        let pin = self.graph.get(p.pin);
        if let Some(w) = pin
            && w.caps.in_amp
        {
            c.verb4(a, p.pin, verb::SET_AMP_GAIN_MUTE, AmpSet::input(0, w.amp_in.unity(), false).encode())?;
        }
        let mic = matches!(p.kind, InKind::MicJack | InKind::MicInternal);
        let ctl = verb::PIN_IN | if mic && p.vref80 { verb::PIN_VREF80 } else { 0 };
        c.verb12(a, p.pin, verb::SET_PIN_CONTROL, ctl)?;
        Ok(())
    }

    /// Format a stream prevodniku A/D (`tag` 0 = odpojit).
    pub fn set_input_stream(&self, c: &mut Ctrl, tag: u8, fmt: u16) -> Result<()> {
        let Some(p) = self.input.as_ref() else { return Ok(()) };
        if tag != 0 {
            c.verb4(self.addr, p.adc, verb::SET_CONVERTER_FORMAT, fmt)?;
        }
        c.verb12(self.addr, p.adc, verb::SET_STREAM_CHANNEL, tag << 4).map(|_| ())
    }
}
