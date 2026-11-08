//! Cesta pro nahravani: od prevodniku A/D (ADC) "proti proudu" ke
//! vstupnimu pinu (mikrofon, linkovy vstup).
//!
//! Poradi pinu: mikrofon v konektoru, vestaveny mikrofon, linkovy vstup,
//! aux. Pro kazdy pin se zkusi vsechny analogove ADC; vyhraje prvni
//! nalezena nejkratsi cesta (nejvys [`crate::graph::MAX_DEPTH`] uzlu).

use alloc::vec;
use alloc::vec::Vec;

use crate::caps::{Connectivity, DefaultDevice, WidgetKind};
use crate::graph::{Graph, MAX_DEPTH, Step};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InKind {
    MicJack,
    MicInternal,
    LineIn,
    Aux,
}

/// Cesta ADC -> pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InPath {
    pub adc: u16,
    pub pin: u16,
    pub kind: InKind,
    /// Od ADC k pinu; kazdy uzel krome pinu ma index vstupu, kterym se
    /// jde dal k pinu.
    pub steps: Vec<Step>,
    /// Pin umi napajeni mikrofonu 80 % (VREF).
    pub vref80: bool,
}

/// Vstupni piny podle priority.
pub fn input_pins(g: &Graph) -> Vec<(u16, InKind)> {
    let mut v: Vec<(InKind, u8, u16)> = Vec::new();
    for w in &g.widgets {
        let (Some(pc), Some(cfg)) = (w.pin_caps, w.pin_config) else { continue };
        if w.caps.kind != WidgetKind::Pin || !pc.input || !cfg.connected() || pc.hdmi || pc.dp {
            continue;
        }
        let kind = match cfg.device {
            DefaultDevice::MicIn if cfg.connectivity == Connectivity::Jack => InKind::MicJack,
            DefaultDevice::MicIn => InKind::MicInternal,
            DefaultDevice::LineIn => InKind::LineIn,
            DefaultDevice::Aux => InKind::Aux,
            _ => continue,
        };
        v.push((kind, cfg.sequence, w.nid));
    }
    v.sort();
    v.into_iter().map(|(k, _, n)| (n, k)).collect()
}

/// Nejkratsi cesta z `from` (ADC) k `pin` pres mixery a selektory.
pub fn route(g: &Graph, from: u16, pin: u16) -> Option<Vec<Step>> {
    let mut seen: Vec<(u16, Option<usize>, u8)> = vec![(from, None, 0)];
    let mut depth = vec![1usize];
    let mut head = 0;
    while head < seen.len() {
        let nid = seen[head].0;
        if nid == pin && head != 0 {
            let mut rev = vec![Step { nid, input: None }];
            let mut cur = head;
            while let Some(prev) = seen[cur].1 {
                rev.push(Step { nid: seen[prev].0, input: Some(seen[cur].2) });
                cur = prev;
            }
            rev.reverse();
            return Some(rev);
        }
        let w = g.get(nid)?;
        let pass = head == 0 || matches!(w.caps.kind, WidgetKind::Mixer | WidgetKind::Selector);
        if pass && depth[head] < MAX_DEPTH {
            for (i, &src) in w.conns.iter().enumerate() {
                if seen.iter().any(|s| s.0 == src) {
                    continue;
                }
                let Some(sw) = g.get(src) else { continue };
                let ok = src == pin || matches!(sw.caps.kind, WidgetKind::Mixer | WidgetKind::Selector);
                if ok {
                    seen.push((src, Some(head), i as u8));
                    depth.push(depth[head] + 1);
                }
            }
        }
        head += 1;
    }
    None
}

/// Naplanuje vstup: nejlepsi pin, ke kteremu vede cesta z nektereho ADC.
pub fn plan_input(g: &Graph) -> Option<InPath> {
    let adcs: Vec<u16> = g
        .widgets
        .iter()
        .filter(|w| w.caps.kind == WidgetKind::Input && !w.caps.digital)
        .map(|w| w.nid)
        .collect();
    for (pin, kind) in input_pins(g) {
        for &adc in &adcs {
            if let Some(steps) = route(g, adc, pin) {
                let vref80 = g.get(pin).and_then(|w| w.pin_caps).is_some_and(|p| p.vref80);
                return Some(InPath { adc, pin, kind, steps, vref80 });
            }
        }
    }
    None
}
