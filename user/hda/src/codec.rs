//! Vycet kodeku a jejich widgetu, nastaveni cest k vystupum a detekce
//! zasunuti konektoru (cteni i nevyzadane odpovedi). Nahravani je
//! v `codec_in.rs`, HDMI/DP v `codec_hdmi.rs`.
//!
//! Rozhodovani (ktere piny, ktera cesta, opravy desek) je v `hdaspec`;
//! tady se jen ctou parametry a posilaji verby.

use alloc::string::String;
use alloc::vec::Vec;

use audioproto::device::{Jack, Output, OutputKind};
use hdaspec::caps::{self, AmpCaps, PcmCaps, PinCaps, PinConfig, WidgetCaps, WidgetKind};
use hdaspec::fixup::{self, Fixup};
use hdaspec::graph::{self, Graph, OutKind, Path, Plan, Widget};
use hdaspec::input::{self, InPath};
use hdaspec::verb::{self, AmpSet, param};
use libos::{Result, println};

use crate::codec_hdmi::Hdmi;
use crate::ctrl::Ctrl;

/// Znacka nevyzadanych odpovedi: analogove konektory 1.., HDMI od 32.
const TAG_HDMI: u8 = 32;

/// Nejvic widgetu jedne funkcni skupiny (ochrana pred nesmyslnou odpovedi).
const MAX_WIDGETS: u32 = 128;

pub struct Codec {
    pub addr: u8,
    pub vendor: u32,
    pub afg: u16,
    pub graph: Graph,
    pub plan: Plan,
    /// Formaty prevodniku prvni analogove cesty.
    pub pcm: PcmCaps,
    /// ID subsystemu (vyrobce desky) a pouzita oprava.
    pub subsystem: u32,
    pub fixup: Option<&'static Fixup>,
    /// Cesta pro nahravani.
    pub input: Option<InPath>,
    /// Digitalni vystupy (HDMI/DP).
    pub hdmi: Vec<Hdmi>,
    /// Posledni zname stavy konektoru (pin, stav).
    jacks: Vec<(u16, Jack)>,
    speakers_muted: bool,
}

/// Pocka, az funkcni skupina skutecne prejde do D0 (GET_POWER_STATE,
/// bity 7:4 = aktualni stav). Z D3 to muze trvat desitky ms; prikazy
/// behem prechodu kodek nemusi zpracovat.
fn wait_d0(c: &mut Ctrl, addr: u8, afg: u16) {
    let end = libos::time::deadline_after(core::time::Duration::from_millis(100));
    while libos::time::now() < end {
        match c.verb12(addr, afg, verb::GET_POWER_STATE, 0) {
            Ok(v) if v >> 4 & 0xF == 0 => return,
            _ => libos::time::sleep(core::time::Duration::from_millis(1)),
        }
    }
}

fn out_kind(k: OutKind) -> OutputKind {
    match k {
        OutKind::LineOut => OutputKind::LineOut,
        OutKind::Speaker => OutputKind::Speaker,
        OutKind::Headphone => OutputKind::Headphone,
        OutKind::Digital => OutputKind::Digital,
    }
}

impl Codec {
    /// Precte kodek na adrese `addr`. `None` = nema audio funkcni skupinu
    /// (modem).
    pub fn probe(c: &mut Ctrl, addr: u8) -> Result<Option<Self>> {
        let vendor = c.param(addr, 0, param::VENDOR_ID)?;
        let nodes = c.param(addr, 0, param::NODE_COUNT)?;
        let (start, count) = ((nodes >> 16 & 0xFF) as u16, (nodes & 0xFF) as u16);
        let mut afg = None;
        for fg in start..start.saturating_add(count).min(256) {
            if c.param(addr, fg, param::FUNCTION_TYPE)? & 0xFF == verb::FG_AUDIO as u32 {
                afg = Some(fg);
                break;
            }
        }
        let Some(afg) = afg else { return Ok(None) };
        c.verb12(addr, afg, verb::SET_POWER_STATE, verb::POWER_D0)?;
        wait_d0(c, addr, afg);
        let subsystem = c.verb12(addr, afg, fixup::verb::GET_SUBSYSTEM_ID, 0).unwrap_or(0);
        let afg_pcm = c.param(addr, afg, param::PCM)?;
        let def_in = AmpCaps::decode(c.param(addr, afg, param::AMP_IN_CAPS)?);
        let def_out = AmpCaps::decode(c.param(addr, afg, param::AMP_OUT_CAPS)?);
        let nodes = c.param(addr, afg, param::NODE_COUNT)?;
        let (ws, wc) = ((nodes >> 16 & 0xFF) as u16, (nodes & 0xFF).min(MAX_WIDGETS) as u16);
        let mut widgets = Vec::new();
        let mut dac_pcm: Vec<(u16, u32)> = Vec::new();
        for nid in ws..ws.saturating_add(wc).min(256) {
            let w = Self::widget(c, addr, nid, def_in, def_out)?;
            if w.caps.kind == WidgetKind::Output && w.caps.format_override {
                dac_pcm.push((nid, c.param(addr, nid, param::PCM)?));
            }
            widgets.push(w);
        }
        let mut graph = Graph { widgets };
        let fix = fixup::find(fixup::FIXUPS, vendor, subsystem);
        if let Some(f) = fix {
            println!("hda: kodek {addr} ({vendor:08x}, deska {subsystem:08x}): oprava {}", f.name);
            fixup::apply_pins(&mut graph, f);
        }
        let mut plan = graph::plan_outputs(&graph);
        if plan.analog.is_empty() && graph.widgets.iter().any(|w| w.pin_caps.is_some_and(|p| p.output && !p.hdmi)) {
            println!("hda: kodek {addr}: konfigurace pinu neuvadi vystup, zkousim vsechny piny jako linkovy vystup");
            let fb = graph::plan_fallback(&graph);
            plan.analog = fb.analog;
        }
        let pcm_raw = plan
            .analog
            .first()
            .and_then(|p| dac_pcm.iter().find(|(n, _)| *n == p.dac).map(|(_, v)| *v))
            .unwrap_or(afg_pcm);
        let input = input::plan_input(&graph);
        let hdmi = plan.digital.iter().filter(|p| p.hdmi).map(|p| Hdmi { path: p.clone(), present: false, eld: None }).collect();
        Ok(Some(Self {
            addr,
            vendor,
            afg,
            graph,
            plan,
            pcm: PcmCaps { raw: pcm_raw },
            subsystem,
            fixup: fix,
            input,
            hdmi,
            jacks: Vec::new(),
            speakers_muted: false,
        }))
    }

    fn widget(c: &mut Ctrl, addr: u8, nid: u16, def_in: AmpCaps, def_out: AmpCaps) -> Result<Widget> {
        let caps = WidgetCaps::decode(c.param(addr, nid, param::WIDGET_CAPS)?);
        let conns = if caps.conn_list {
            let (len, long) = caps::conn_list_len(c.param(addr, nid, param::CONN_LIST_LEN)?);
            caps::conn_list(len, long, |i| c.verb12(addr, nid, verb::GET_CONN_LIST, i).ok())
        } else {
            Vec::new()
        };
        let (pin_caps, pin_config) = if caps.kind == WidgetKind::Pin {
            (
                Some(PinCaps::decode(c.param(addr, nid, param::PIN_CAPS)?)),
                Some(PinConfig::decode(c.verb12(addr, nid, verb::GET_CONFIG_DEFAULT, 0)?)),
            )
        } else {
            (None, None)
        };
        let amp = |c: &mut Ctrl, present: bool, p: u8, def: AmpCaps| -> Result<AmpCaps> {
            Ok(match (present, caps.amp_override) {
                (false, _) => AmpCaps::default(),
                (true, true) => AmpCaps::decode(c.param(addr, nid, p)?),
                (true, false) => def,
            })
        };
        let amp_in = amp(c, caps.in_amp, param::AMP_IN_CAPS, def_in)?;
        let amp_out = amp(c, caps.out_amp, param::AMP_OUT_CAPS, def_out)?;
        Ok(Widget { nid, caps, conns, pin_caps, pin_config, amp_in, amp_out })
    }

    /// Ma kodek analogovy vystup?
    pub fn has_output(&self) -> bool {
        !self.plan.analog.is_empty()
    }

    /// Hraje nektery vystup (analogovy, nebo HDMI s monitorem)?
    pub fn has_active_output(&self) -> bool {
        self.has_output() || self.hdmi.iter().any(|h| h.active())
    }

    /// Popis do logu.
    pub fn describe(&self) -> String {
        let mut s = alloc::format!("kodek {} ({:08x}, deska {:08x}, AFG {:#x}):", self.addr, self.vendor, self.subsystem, self.afg);
        for p in &self.plan.analog {
            s.push_str(&alloc::format!(" {}@{:#x}->dac {:#x}", out_kind(p.kind).name(), p.pin, p.dac));
        }
        for p in &self.plan.digital {
            s.push_str(&alloc::format!(" {}@{:#x}->{:#x}", if p.hdmi { "hdmi" } else { "spdif" }, p.pin, p.dac));
        }
        if let Some(i) = &self.input {
            s.push_str(&alloc::format!(" vstup {:?}@{:#x}<-adc {:#x}", i.kind, i.pin, i.adc));
        }
        if self.plan.analog.is_empty() && self.plan.digital.is_empty() {
            s.push_str(" zadny vystup");
        }
        s
    }

    /// Nastavi cesty ke vsem vystupum (napajeni, vybery, zesilovace na
    /// 0 dB, piny, EAPD), HDMI/DP, vstup, opravy desky a nevyzadane
    /// odpovedi konektoru.
    pub fn setup_outputs(&mut self, c: &mut Ctrl) -> Result<()> {
        let paths = self.plan.analog.clone();
        for p in &paths {
            self.setup_path(c, p)?;
        }
        if fixup::generic_eapd(self.vendor) {
            // Realtek: EAPD na vsech pinech, ktere ho maji (externi zesilovace).
            for w in self.graph.widgets.iter().filter(|w| w.pin_caps.is_some_and(|p| p.eapd && p.output)) {
                let _ = c.verb12(self.addr, w.nid, verb::SET_EAPD_BTL, verb::EAPD);
            }
        }
        if let Some(f) = self.fixup {
            if let Some(g) = f.gpio {
                c.verb12(self.addr, self.afg, verb::SET_GPIO_MASK, g.mask)?;
                c.verb12(self.addr, self.afg, verb::SET_GPIO_DIR, g.dir)?;
                c.verb12(self.addr, self.afg, verb::SET_GPIO_DATA, g.data)?;
            }
            for &(nid, idx, val) in f.coefs {
                c.verb4(self.addr, nid, fixup::verb::SET_COEF_INDEX, idx)?;
                c.verb4(self.addr, nid, fixup::verb::SET_PROC_COEF, val)?;
            }
        }
        self.setup_hdmi(c)?;
        self.setup_input(c)?;
        self.enable_unsol(c);
        self.poll_jacks(c, true);
        Ok(())
    }

    /// Nevyzadane odpovedi pro konektory s detekci (znacka = index + 1)
    /// a piny HDMI.
    fn enable_unsol(&self, c: &mut Ctrl) {
        for (i, p) in self.plan.analog.iter().enumerate().filter(|(_, p)| p.jack_detect) {
            let _ = c.verb12(self.addr, p.pin, verb::SET_UNSOL, verb::UNSOL_ENABLE | (i as u8 + 1));
        }
        for (i, h) in self.hdmi.iter().enumerate() {
            let _ = c.verb12(self.addr, h.path.pin, verb::SET_UNSOL, verb::UNSOL_ENABLE | (TAG_HDMI + i as u8));
        }
    }

    fn setup_path(&self, c: &mut Ctrl, p: &Path) -> Result<()> {
        let a = self.addr;
        for s in &p.steps {
            let Some(w) = self.graph.get(s.nid) else { continue };
            if w.caps.power_control {
                c.verb12(a, s.nid, verb::SET_POWER_STATE, verb::POWER_D0)?;
            }
            if let Some(idx) = s.input {
                if w.caps.kind != WidgetKind::Mixer && w.conns.len() > 1 {
                    c.verb12(a, s.nid, verb::SET_CONN_SELECT, idx)?;
                }
                if w.caps.in_amp && w.caps.kind == WidgetKind::Selector {
                    c.verb4(a, s.nid, verb::SET_AMP_GAIN_MUTE, AmpSet::input(idx, w.amp_in.unity(), false).encode())?;
                }
            }
            if w.caps.out_amp {
                c.verb4(a, s.nid, verb::SET_AMP_GAIN_MUTE, AmpSet::output(w.amp_out.unity(), false).encode())?;
            }
        }
        // Mixery: jen vstup z cesty, ostatni (smycky z mikrofonu, CD) ztlumit.
        for (nid, idx, n) in graph::path_mixers(&self.graph, p) {
            let Some(w) = self.graph.get(nid) else { continue };
            for i in 0..n.min(16) as u8 {
                let on = i == idx;
                let gain = if on { w.amp_in.unity() } else { 0 };
                c.verb4(a, nid, verb::SET_AMP_GAIN_MUTE, AmpSet::input(i, gain, !on).encode())?;
            }
        }
        if p.eapd {
            c.verb12(a, p.pin, verb::SET_EAPD_BTL, verb::EAPD)?;
        }
        self.set_pin(c, p, true)
    }

    fn set_pin(&self, c: &mut Ctrl, p: &Path, on: bool) -> Result<()> {
        let mut ctl = 0;
        if on {
            ctl = verb::PIN_OUT;
            if p.hp_drive {
                ctl |= verb::PIN_HP;
            }
        }
        c.verb12(self.addr, p.pin, verb::SET_PIN_CONTROL, ctl).map(|_| ())
    }

    /// Nastavi format a stream vsem DAC analogovych cest a hrajicich HDMI
    /// (`tag` 0 = odpojit). Neaktivni HDMI se odpoji.
    pub fn set_stream(&self, c: &mut Ctrl, tag: u8, fmt: u16) -> Result<()> {
        let mut done: Vec<u16> = Vec::new();
        let hdmi = self.hdmi.iter().map(|h| (h.path.dac, h.active()));
        let analog = self.plan.analog.iter().map(|p| (p.dac, true));
        for (dac, on) in analog.chain(hdmi) {
            if done.contains(&dac) {
                continue;
            }
            done.push(dac);
            let t = if on { tag } else { 0 };
            if t != 0 {
                c.verb4(self.addr, dac, verb::SET_CONVERTER_FORMAT, fmt)?;
            }
            c.verb12(self.addr, dac, verb::SET_STREAM_CHANNEL, t << 4)?;
        }
        Ok(())
    }

    /// Nevyzadana odpoved patri tomuto kodeku: precte konektory. Vrati
    /// `true`, kdyz se zmenilo, ktere digitalni vystupy hraji.
    pub fn unsolicited(&mut self, c: &mut Ctrl, tag: u8) -> bool {
        if tag >= TAG_HDMI {
            let i = (tag - TAG_HDMI) as usize;
            return i < self.hdmi.len() && self.sense_hdmi(c, i);
        }
        self.poll_jacks(c, false);
        false
    }

    fn sense(&self, c: &mut Ctrl, p: &Path) -> Jack {
        if !p.jack_detect {
            return Jack::Unknown;
        }
        let trigger = self.graph.get(p.pin).and_then(|w| w.pin_caps).is_some_and(|pc| pc.trigger);
        if trigger {
            let _ = c.verb12(self.addr, p.pin, verb::EXEC_PIN_SENSE, 0);
        }
        match c.verb12(self.addr, p.pin, verb::GET_PIN_SENSE, 0) {
            Ok(v) if v & 1 << 31 != 0 => Jack::Plugged,
            Ok(_) => Jack::Unplugged,
            Err(_) => Jack::Unknown,
        }
    }

    /// Precte konektory a podle sluchatek ztlumi/zapne reproduktory.
    /// Vrati `true`, kdyz se neco zmenilo.
    pub fn poll_jacks(&mut self, c: &mut Ctrl, force: bool) -> bool {
        let paths = self.plan.analog.clone();
        let mut jacks = Vec::with_capacity(paths.len());
        for p in &paths {
            jacks.push((p.pin, self.sense(c, p)));
        }
        let mut changed = jacks != self.jacks;
        self.jacks = jacks;
        for i in 0..self.hdmi.len() {
            changed |= self.sense_hdmi(c, i);
        }
        let muted = graph::speakers_muted(&self.plan, |pin| self.jacks.iter().any(|&(n, j)| n == pin && j == Jack::Plugged));
        if force || muted != self.speakers_muted {
            self.speakers_muted = muted;
            for p in paths.iter().filter(|p| p.kind == OutKind::Speaker) {
                let _ = self.set_pin(c, p, !muted);
            }
            if !force {
                println!("hda: sluchatka {}, reproduktory {}", if muted { "zasunuta" } else { "vytazena" }, if muted { "ztlumeny" } else { "zapnute" });
            }
        }
        changed
    }

    /// Vystupy pro `INFO`.
    pub fn outputs(&self) -> Vec<Output> {
        let mut v = Vec::new();
        for p in &self.plan.analog {
            let jack = self.jacks.iter().find(|j| j.0 == p.pin).map_or(Jack::Unknown, |j| j.1);
            let muted = p.kind == OutKind::Speaker && self.speakers_muted;
            v.push(Output { kind: out_kind(p.kind), jack, active: !muted });
        }
        for h in &self.hdmi {
            let jack = if h.present { Jack::Plugged } else { Jack::Unplugged };
            v.push(Output { kind: OutputKind::Digital, jack, active: h.active() });
        }
        v
    }

    /// Ma nektery vystup detekci zasunuti (ma smysl se ptat)?
    pub fn has_jack_detect(&self) -> bool {
        self.plan.analog.iter().any(|p| p.jack_detect) || !self.hdmi.is_empty()
    }
}
