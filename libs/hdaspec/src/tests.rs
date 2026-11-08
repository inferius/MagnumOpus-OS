//! Testy dekodovani a hledani cest na hostiteli (grafy podle QEMU
//! hda-duplex, kodeku typu Realtek ALC a graficke karty s HDMI).

use alloc::vec;
use alloc::vec::Vec;

use crate::caps::{self, AmpCaps, Connectivity, DefaultDevice, PcmCaps, PinCaps, PinConfig, WidgetCaps, WidgetKind};
use crate::graph::{self, Graph, OutKind, Widget};
use crate::regs::{self, Gcap};
use crate::verb::{self, AmpSet};
use crate::format;

/// Schopnosti widgetu daneho typu (stereo, zesilovace podle prani).
fn wcaps(kind: u32, in_amp: bool, out_amp: bool, digital: bool) -> WidgetCaps {
    let mut v = kind << 20 | 1 | 1 << 8;
    if in_amp {
        v |= 1 << 1;
    }
    if out_amp {
        v |= 1 << 2;
    }
    if digital {
        v |= 1 << 9;
    }
    WidgetCaps::decode(v)
}

fn dac(nid: u16, digital: bool) -> Widget {
    Widget {
        nid,
        caps: wcaps(0, false, true, digital),
        conns: vec![],
        pin_caps: None,
        pin_config: None,
        amp_in: AmpCaps::default(),
        amp_out: AmpCaps::decode(0x8000_3F27),
    }
}

fn mixer(nid: u16, conns: &[u16]) -> Widget {
    Widget {
        nid,
        caps: wcaps(2, true, false, false),
        conns: conns.to_vec(),
        pin_caps: None,
        pin_config: None,
        amp_in: AmpCaps::decode(0x8000_0000),
        amp_out: AmpCaps::default(),
    }
}

fn selector(nid: u16, conns: &[u16]) -> Widget {
    Widget { caps: wcaps(3, false, false, false), ..mixer(nid, conns) }
}

/// Pin: `device` = vychozi zarizeni (bity 23:20), `conn` = pripojeni
/// (0 jack, 1 nic, 2 pevne), asociace a poradi.
fn pin(nid: u16, conns: &[u16], device: u32, conn: u32, assoc: u32, seq: u32, pcaps: u32) -> Widget {
    let cfg = conn << 30 | 0x01 << 24 | device << 20 | assoc << 4 | seq;
    Widget {
        nid,
        caps: wcaps(4, false, true, false),
        conns: conns.to_vec(),
        pin_caps: Some(PinCaps::decode(pcaps)),
        pin_config: Some(PinConfig::decode(cfg)),
        amp_in: AmpCaps::default(),
        amp_out: AmpCaps::decode(0x8000_0000),
    }
}

const OUT: u32 = 1 << 4;
const PRESENCE: u32 = 1 << 2;
const HP_DRIVE: u32 = 1 << 3;
const EAPD: u32 = 1 << 16;
const HDMI: u32 = 1 << 7;

#[test]
fn verbs_and_amp_encoding() {
    assert_eq!(verb::cmd12(0, 1, verb::GET_PARAMETER, verb::param::VENDOR_ID), 0x001F_0000);
    assert_eq!(verb::cmd12(2, 0x14, verb::SET_PIN_CONTROL, verb::PIN_OUT), 0x2147_0740);
    assert_eq!(verb::cmd4(0, 2, verb::SET_CONVERTER_FORMAT, 0x11), 0x0022_0011);
    assert_eq!(AmpSet::output(0x27, false).encode(), 0xB027);
    assert_eq!(AmpSet::input(2, 0, true).encode(), 0x7280);
}

#[test]
fn register_helpers() {
    // QEMU intel-hda: 4 vystupni a 4 vstupni streamy, 64bitove adresy.
    let g = Gcap::decode(0x4401);
    assert_eq!((g.output_streams, g.input_streams, g.bidi_streams, g.addr64), (4, 4, 0, true));
    assert_eq!(g.first_output(), 4);
    assert_eq!(regs::sd_offset(4), 0x100);
    assert_eq!(regs::ring_size(0x42), (2, 256));
    assert_eq!(regs::ring_size(0x20), (1, 16));
    assert_eq!(regs::ring_size(0x00), (0, 2));
    let e = regs::bdl_entry(0x1234_5678_9ABC_DE80, 1024, true);
    assert_eq!(&e[..8], &0x1234_5678_9ABC_DE80u64.to_le_bytes());
    assert_eq!(&e[8..12], &1024u32.to_le_bytes());
    assert_eq!(e[12], 1);
}

#[test]
fn stream_format() {
    assert_eq!(format::encode(48_000, 16, 2), Some(0x0011));
    assert_eq!(format::encode(44_100, 16, 2), Some(0x4011));
    assert_eq!(format::encode(96_000, 24, 2), Some(0x0831));
    assert_eq!(format::encode(192_000, 32, 8), Some(0x1847));
    assert_eq!(format::encode(8_000, 16, 1), Some(0x0510));
    assert_eq!(format::encode(22_050, 8, 1), Some(0x4100));
    assert_eq!(format::encode(12_345, 16, 2), None);
    assert_eq!(format::encode(48_000, 12, 2), None);
    for (r, b, c) in [(48_000, 16, 2), (44_100, 24, 6), (16_000, 32, 1), (176_400, 20, 2)] {
        assert_eq!(format::decode(format::encode(r, b, c).unwrap()), (r, b, c));
    }
}

#[test]
fn capability_decoding() {
    // Pin (typ 4): stereo, vystupni zesilovac, seznam spojeni, napajeni.
    let w = WidgetCaps::decode(0x0040_050D);
    assert_eq!(w.kind, WidgetKind::Pin);
    assert!(w.stereo && w.out_amp && w.amp_override && w.conn_list && w.power_control && !w.digital);
    assert_eq!(w.channels, 2);
    let p = PinCaps::decode(OUT | PRESENCE | HP_DRIVE | EAPD);
    assert!(p.output && p.presence_detect && p.headphone_drive && p.eapd && !p.hdmi);
    let a = AmpCaps::decode(0x8005_3F27);
    assert_eq!((a.offset, a.steps, a.step_size, a.mute), (0x27, 0x3F, 5, true));
    assert_eq!(a.unity(), 0x27);
    // Typicka konfigurace predniho konektoru sluchatek Realtek.
    let c = PinConfig::decode(0x0221_401F);
    assert_eq!(c.connectivity, Connectivity::Jack);
    assert_eq!(c.location, 0x02);
    assert_eq!(c.device, DefaultDevice::HeadphoneOut);
    assert_eq!((c.association, c.sequence), (1, 0xF));
    assert!(c.jack_detect_allowed());
    let none = PinConfig::decode(0x4000_0000);
    assert!(!none.connected());
    let pcm = PcmCaps { raw: 0x000E_0060 };
    assert!(pcm.rate(44_100) && pcm.rate(48_000) && !pcm.rate(96_000));
    assert!(pcm.bits(16) && pcm.bits(20) && pcm.bits(24) && !pcm.bits(32));
}

#[test]
fn connection_lists() {
    // Kratky tvar: 0x02, 0x03, rozsah do 0x06, 0x0B.
    let resp = [u32::from_le_bytes([0x02, 0x03, 0x86, 0x0B])];
    let l = caps::conn_list(4, false, |i| resp.get(i as usize / 4).copied());
    assert_eq!(l, vec![2, 3, 4, 5, 6, 0x0B]);
    // Dlouhy tvar, licha delka.
    let resp = [0x0123_0045u32, 0x0000_0067];
    let l = caps::conn_list(3, true, |i| resp.get(i as usize / 2).copied());
    assert_eq!(l, vec![0x45, 0x123, 0x67]);
    // Nesmyslny rozsah (sestupny) se vynecha, kodek bez odpovedi = konec.
    let l = caps::conn_list(2, false, |_| Some(0x0000_8510));
    assert_eq!(l, vec![0x10]);
    assert!(caps::conn_list(4, false, |_| None).is_empty());
    assert_eq!(caps::conn_list_len(0x85), (5, true));
}

/// QEMU hda-duplex: DAC 2 -> pin 3 (linkovy vystup), pin 5 vstup -> ADC 4.
fn qemu_duplex() -> Graph {
    Graph {
        widgets: vec![
            dac(2, false),
            pin(3, &[2], 0x0, 0, 1, 0, OUT),
            Widget { caps: wcaps(1, true, false, false), conns: vec![5], ..mixer(4, &[]) },
            pin(5, &[], 0x8, 0, 2, 0, 1 << 5),
        ],
    }
}

#[test]
fn qemu_duplex_path() {
    let g = qemu_duplex();
    let plan = graph::plan_outputs(&g);
    assert_eq!(plan.analog.len(), 1);
    let p = &plan.analog[0];
    assert_eq!((p.pin, p.dac, p.kind), (3, 2, OutKind::LineOut));
    assert_eq!(p.steps.len(), 2);
    assert_eq!(p.steps[0].input, Some(0));
    assert!(!p.jack_detect);
    assert!(plan.digital.is_empty());
}

/// Typ Realtek ALC: dva DAC, mixery, reproduktor, sluchatka, linkovy vystup.
fn realtek() -> Graph {
    Graph {
        widgets: vec![
            dac(0x02, false),
            dac(0x03, false),
            mixer(0x0B, &[0x18, 0x19]),
            mixer(0x0C, &[0x02, 0x0B]),
            mixer(0x0D, &[0x03, 0x0B]),
            pin(0x14, &[0x0C, 0x0D], 0x1, 2, 1, 0, OUT | EAPD),
            pin(0x15, &[0x0C, 0x0D], 0x2, 0, 1, 0xF, OUT | PRESENCE | HP_DRIVE | EAPD),
            pin(0x1B, &[0x0C, 0x0D], 0x0, 0, 1, 1, OUT | PRESENCE),
            // Nezapojeny pin a vstup mikrofonu.
            pin(0x1A, &[0x0C], 0x0, 1, 0, 0, OUT),
            pin(0x18, &[], 0xA, 0, 2, 0, 1 << 5),
        ],
    }
}

#[test]
fn realtek_paths_share_stream() {
    let g = realtek();
    assert_eq!(g.output_pins(), vec![(0x14, OutKind::Speaker), (0x1B, OutKind::LineOut), (0x15, OutKind::Headphone)]);
    let plan = graph::plan_outputs(&g);
    let pins: Vec<u16> = plan.analog.iter().map(|p| p.pin).collect();
    assert_eq!(pins, vec![0x14, 0x1B, 0x15]);
    // Prvni dva dostanou vlastni DAC, treti sdili.
    assert_eq!(plan.analog[0].dac, 0x02);
    assert_eq!(plan.analog[1].dac, 0x03);
    assert!([0x02, 0x03].contains(&plan.analog[2].dac));
    let hp = &plan.analog[2];
    assert!(hp.jack_detect && hp.hp_drive && hp.eapd);
    // Mixer na ceste reproduktoru: vstup 0 (DAC), ostatni se ztlumi.
    assert_eq!(graph::path_mixers(&g, &plan.analog[0]), vec![(0x0C, 0, 2)]);
    // Auto-mute: zasunuta sluchatka ztlumi reproduktor.
    assert!(graph::speakers_muted(&plan, |p| p == 0x15));
    assert!(!graph::speakers_muted(&plan, |_| false));
}

#[test]
fn selector_choice_is_not_changed_by_later_path() {
    // Selektor 0x0F vybira mezi DAC 2 a 3; oba piny jdou pres nej.
    let g = Graph {
        widgets: vec![
            dac(2, false),
            dac(3, false),
            selector(0x0F, &[2, 3]),
            pin(0x10, &[0x0F], 0x0, 0, 1, 0, OUT),
            pin(0x11, &[0x0F], 0x2, 0, 1, 1, OUT),
        ],
    };
    let plan = graph::plan_outputs(&g);
    assert_eq!(plan.analog.len(), 2);
    assert_eq!(plan.analog[0].dac, 2);
    // Druhy pin by chtel volny DAC 3, ale selektor uz je nastaveny na 2.
    assert_eq!(plan.analog[1].dac, 2);
    assert_eq!(plan.analog[1].steps[1].input, Some(0));
}

#[test]
fn hdmi_is_prepared_separately_and_depth_is_limited() {
    let mut w = vec![dac(2, true), pin(3, &[2], 0x5, 0, 1, 0, OUT | HDMI)];
    // Analogovy pin za retezcem 6 selektoru: hloubka nestaci.
    w.push(dac(0x20, false));
    let mut prev = 0x20;
    for n in 0x21..0x27 {
        w.push(selector(n, &[prev]));
        prev = n;
    }
    w.push(pin(0x30, &[prev], 0x0, 0, 2, 0, OUT));
    let g = Graph { widgets: w };
    let plan = graph::plan_outputs(&g);
    assert_eq!(plan.digital.len(), 1);
    assert!(plan.digital[0].hdmi);
    assert_eq!(plan.digital[0].kind, OutKind::Digital);
    assert!(plan.analog.is_empty());
    // Analogovy pin nesmi skoncit u digitalniho prevodniku.
    let g2 = Graph { widgets: vec![dac(2, true), pin(3, &[2], 0x0, 0, 1, 0, OUT)] };
    assert!(graph::plan_outputs(&g2).analog.is_empty());
}

#[test]
fn cycles_do_not_hang() {
    let g = Graph {
        widgets: vec![mixer(0x0C, &[0x0D]), mixer(0x0D, &[0x0C]), pin(0x14, &[0x0C], 0x0, 0, 1, 0, OUT)],
    };
    assert!(graph::plan_outputs(&g).analog.is_empty());
}

#[test]
fn fallback_when_bios_hides_outputs() {
    // Vsechny piny "nezapojene": bezny plan nic, zalozni linkovy vystup.
    let g = Graph { widgets: vec![dac(2, false), pin(3, &[2], 0x0, 1, 0, 0, OUT)] };
    assert!(graph::plan_outputs(&g).analog.is_empty());
    let plan = graph::plan_fallback(&g);
    assert_eq!(plan.analog.len(), 1);
    assert_eq!(plan.analog[0].kind, OutKind::LineOut);
}
