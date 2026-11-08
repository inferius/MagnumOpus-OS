//! Graf widgetu audio funkcni skupiny a hledani cest k vystupum.
//!
//! Zvuk tece od prevodniku D/A (DAC) pres mixery a selektory k pinu.
//! Seznam spojeni widgetu rika, odkud bere vstup, takze cestu hledame od
//! pinu "proti proudu" k DAC (do hloubky [`MAX_DEPTH`]).
//!
//! Postup ([`plan_outputs`]):
//! 1. vystupni piny podle vychozi konfigurace (linkovy vystup,
//!    sluchatka, reproduktor; HDMI/DP zvlast) serazene podle asociace
//!    a poradi, jak je zapojil vyrobce desky,
//! 2. pro kazdy pin nejkratsi cesta k volnemu DAC, kdyz neni, ke DAC uz
//!    pouzitemu. Vsechny DAC dostanou stejny stream, takze vsechny vystupy
//!    hraji totez (sluchatka i reproduktor). Selektor muze mit jen jeden
//!    vyber: cesta, ktera by zmenila vyber jine cesty, se odmitne,
//! 3. reproduktory se ztlumi, kdyz jsou zasunuta sluchatka (auto-mute,
//!    rozhoduje ovladac podle detekce zasunuti, [`speakers_muted`]).

use alloc::vec;
use alloc::vec::Vec;

use crate::caps::{AmpCaps, DefaultDevice, PinCaps, PinConfig, WidgetCaps, WidgetKind};

/// Nejvic uzlu cesty (pin, mixery/selektory, DAC).
pub const MAX_DEPTH: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widget {
    pub nid: u16,
    pub caps: WidgetCaps,
    /// Vstupy widgetu (NID v poradi seznamu spojeni).
    pub conns: Vec<u16>,
    pub pin_caps: Option<PinCaps>,
    pub pin_config: Option<PinConfig>,
    pub amp_in: AmpCaps,
    pub amp_out: AmpCaps,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    pub widgets: Vec<Widget>,
}

/// Druh vystupu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OutKind {
    LineOut,
    Headphone,
    Speaker,
    /// HDMI, DisplayPort nebo S/PDIF.
    Digital,
}

/// Uzel cesty a index vstupu, kterym se jde dal k DAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub nid: u16,
    /// Index v seznamu spojeni tohoto uzlu (u DAC `None`).
    pub input: Option<u8>,
}

/// Cesta od pinu k DAC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub pin: u16,
    pub kind: OutKind,
    pub dac: u16,
    /// Od pinu k DAC (posledni krok je DAC).
    pub steps: Vec<Step>,
    /// Pin ma detekci zasunuti (a konfigurace ji nezakazuje).
    pub jack_detect: bool,
    pub eapd: bool,
    pub hp_drive: bool,
    /// HDMI nebo DisplayPort (ne S/PDIF).
    pub hdmi: bool,
}

/// Vysledek planovani: analogove cesty (aktivuji se) a digitalni
/// (zatim jen pripravene).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub analog: Vec<Path>,
    pub digital: Vec<Path>,
}

impl Graph {
    pub fn get(&self, nid: u16) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.nid == nid)
    }

    /// Vystupni piny: (nid, druh), serazene podle asociace a poradi.
    pub fn output_pins(&self) -> Vec<(u16, OutKind)> {
        let mut pins: Vec<(u8, u8, u16, OutKind)> = Vec::new();
        for w in &self.widgets {
            let (Some(pc), Some(cfg)) = (w.pin_caps, w.pin_config) else { continue };
            if w.caps.kind != WidgetKind::Pin || !pc.output || !cfg.connected() {
                continue;
            }
            let kind = match cfg.device {
                _ if pc.hdmi || pc.dp => OutKind::Digital,
                DefaultDevice::LineOut => OutKind::LineOut,
                DefaultDevice::Speaker => OutKind::Speaker,
                DefaultDevice::HeadphoneOut => OutKind::Headphone,
                DefaultDevice::SpdifOut | DefaultDevice::DigitalOtherOut => OutKind::Digital,
                _ => continue,
            };
            // Asociace 0 = nepouzito: az na konec (15 je "vsechno ostatni").
            let assoc = if cfg.association == 0 { 16 } else { cfg.association };
            pins.push((assoc, cfg.sequence, w.nid, kind));
        }
        pins.sort();
        pins.into_iter().map(|(_, _, nid, kind)| (nid, kind)).collect()
    }

    /// Nejkratsi cesta od `pin` k DAC (digitalnimu, kdyz `digital`).
    /// `fixed` = vybery selektoru jinych cest (uzel, index); `avoid` = DAC,
    /// kterym se ma cesta vyhnout (prvni pokus: jen volne DAC).
    pub fn find_path(&self, pin: u16, digital: bool, fixed: &[(u16, u8)], avoid: &[u16]) -> Option<Vec<Step>> {
        // BFS: fronta (uzel, predchudce v poli `seen`, index vstupu u predchudce).
        let mut seen: Vec<(u16, Option<usize>, u8)> = vec![(pin, None, 0)];
        let mut depth: Vec<usize> = vec![1];
        let mut head = 0;
        while head < seen.len() {
            let (nid, _, _) = seen[head];
            let d = depth[head];
            let w = self.get(nid)?;
            let is_end = w.caps.kind == WidgetKind::Output && head != 0;
            if is_end {
                if w.caps.digital == digital && !avoid.contains(&nid) {
                    return Some(self.unwind(&seen, head));
                }
                head += 1;
                continue;
            }
            if d < MAX_DEPTH && (head == 0 || matches!(w.caps.kind, WidgetKind::Mixer | WidgetKind::Selector)) {
                let selects = w.caps.kind != WidgetKind::Mixer && w.conns.len() > 1;
                for (i, &src) in w.conns.iter().enumerate() {
                    if selects && fixed.iter().any(|&(n, idx)| n == nid && idx as usize != i) {
                        continue;
                    }
                    if seen.iter().any(|s| s.0 == src) {
                        continue;
                    }
                    let Some(sw) = self.get(src) else { continue };
                    if !matches!(sw.caps.kind, WidgetKind::Output | WidgetKind::Mixer | WidgetKind::Selector) {
                        continue;
                    }
                    seen.push((src, Some(head), i as u8));
                    depth.push(d + 1);
                }
            }
            head += 1;
        }
        None
    }

    fn unwind(&self, seen: &[(u16, Option<usize>, u8)], end: usize) -> Vec<Step> {
        // Od DAC k pinu, pak otocit. Index vstupu patri predchudci.
        let mut rev = vec![Step { nid: seen[end].0, input: None }];
        let mut cur = end;
        while let Some(prev) = seen[cur].1 {
            rev.push(Step { nid: seen[prev].0, input: Some(seen[cur].2) });
            cur = prev;
        }
        rev.reverse();
        rev
    }

    fn make_path(&self, pin: u16, kind: OutKind, steps: Vec<Step>) -> Option<Path> {
        let w = self.get(pin)?;
        let pc = w.pin_caps?;
        let cfg = w.pin_config?;
        Some(Path {
            pin,
            kind,
            dac: steps.last()?.nid,
            steps,
            jack_detect: pc.presence_detect && cfg.jack_detect_allowed(),
            eapd: pc.eapd,
            hp_drive: pc.headphone_drive && kind == OutKind::Headphone,
            hdmi: pc.hdmi || pc.dp,
        })
    }
}

/// Vybery, ktere cesta nastavi (uzly s vice vstupy krome mixeru).
fn selections(g: &Graph, steps: &[Step]) -> Vec<(u16, u8)> {
    steps
        .iter()
        .filter_map(|s| {
            let w = g.get(s.nid)?;
            let idx = s.input?;
            (w.caps.kind != WidgetKind::Mixer && w.conns.len() > 1).then_some((s.nid, idx))
        })
        .collect()
}

/// Naplanuje cesty ke vsem vystupum (viz popis modulu).
pub fn plan_outputs(g: &Graph) -> Plan {
    let mut plan = Plan::default();
    let mut fixed: Vec<(u16, u8)> = Vec::new();
    let mut used: Vec<u16> = Vec::new();
    let mut used_digital: Vec<u16> = Vec::new();
    for (pin, kind) in g.output_pins() {
        let digital = kind == OutKind::Digital;
        let taken = if digital { &used_digital } else { &used };
        let steps = g.find_path(pin, digital, &fixed, taken).or_else(|| {
            // Volny DAC neni: sdilet (digitalni vystupy maji kazdy svuj).
            if digital { None } else { g.find_path(pin, false, &fixed, &[]) }
        });
        let Some(steps) = steps else { continue };
        let Some(path) = g.make_path(pin, kind, steps) else { continue };
        fixed.extend(selections(g, &path.steps));
        if digital {
            used_digital.push(path.dac);
            plan.digital.push(path);
        } else {
            if !used.contains(&path.dac) {
                used.push(path.dac);
            }
            plan.analog.push(path);
        }
    }
    plan
}

/// Zalozni plan, kdyz vychozi konfigurace pinu zadny analogovy vystup
/// neuvadi (spatne vyplneny BIOS, nektere virtualni stroje): kazdy pin
/// s vystupem (krome HDMI/DP) se bere jako linkovy vystup.
pub fn plan_fallback(g: &Graph) -> Plan {
    let mut g = g.clone();
    for w in &mut g.widgets {
        if let Some(pc) = w.pin_caps
            && pc.output
            && !pc.hdmi
            && !pc.dp
        {
            // Konektor, zadni panel, linkovy vystup, zelena, asociace 1.
            w.pin_config = Some(PinConfig::decode(0x0101_4010));
        }
    }
    plan_outputs(&g)
}

/// Ztlumit reproduktory? Ano, kdyz nektera sluchatka s detekci hlasi
/// zasunuti (`plugged(pin)`).
pub fn speakers_muted(plan: &Plan, mut plugged: impl FnMut(u16) -> bool) -> bool {
    plan.analog.iter().any(|p| p.kind == OutKind::Headphone && p.jack_detect && plugged(p.pin))
}

/// Mixery na ceste: (uzel, index vstupu na ceste, pocet vstupu) - ostatni
/// vstupy se ztlumi (smycky z mikrofonu, CD apod.).
pub fn path_mixers(g: &Graph, path: &Path) -> Vec<(u16, u8, usize)> {
    path.steps
        .iter()
        .filter_map(|s| {
            let w = g.get(s.nid)?;
            (w.caps.kind == WidgetKind::Mixer && w.caps.in_amp).then_some((s.nid, s.input?, w.conns.len()))
        })
        .collect()
}
