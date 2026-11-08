//! Testy nad simulovanymi registry ([`crate::sim::Sim`]): stav od
//! firmwaru, EDID pres AUX a GMBUS, prevzeti s page flipem, scaler,
//! zaznam a obnova, hotplug, navazani linky Gen9.
//!
//! Scenare odpovidaji tomu, co GOP typicky nastavi (eDP na transcoderu
//! EDP, HDMI na pipe B); hodnoty registru jsou sestavene podle formatu
//! z `intel_display_regs.h`, ne zaznamenane ze skutecneho stroje.

mod data;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use display::dp::auxch::{Msg, cmd};
use display::dp::train::{Drive, Pattern};
use display::{Error as DErr, dp};
use displaydrv::{Backend, Server, Shadow};
use fbcopy::Surface;
use guiproto::display::{Reply, Request, caps};
use guiproto::modes::kind;

use crate::backend::{Buffers, Intel, probe};
use crate::platform::{Detect, Family, Pch, detect, supported_ids};
use crate::regs::{self, Trans, ddi_func};
use crate::sim::{FakeClock, Sim};
use crate::{dpaux, gmbus, gtt, phy, state};
use data::{AOC_2243W, LGD_LP133WH2};

fn platform(id: u16) -> crate::platform::Platform {
    match detect(0x8086, id) {
        Detect::Supported(p) => p,
        d => panic!("{id:#x}: {d:?}"),
    }
}

/// Transcoder s casovanim `w` x `h` (porchy jako CVT-RB).
fn program_trans(s: &mut Sim, t: Trans, w: u32, h: u32) {
    let b = t.offset();
    let (ht, hs, he) = (w + 160, w + 48, w + 80);
    let (vt, vs, ve) = (h + 31, h + 3, h + 8);
    s.set(b + regs::HTOTAL, (ht - 1) << 16 | (w - 1));
    s.set(b + regs::HBLANK, (ht - 1) << 16 | (w - 1));
    s.set(b + regs::HSYNC, (he - 1) << 16 | (hs - 1));
    s.set(b + regs::VTOTAL, (vt - 1) << 16 | (h - 1));
    s.set(b + regs::VBLANK, (vt - 1) << 16 | (h - 1));
    s.set(b + regs::VSYNC, (ve - 1) << 16 | (vs - 1));
    s.set(t.pipe_offset() + regs::TRANSCONF, regs::TRANSCONF_ENABLE | regs::TRANSCONF_STATE);
}

fn program_plane(s: &mut Sim, pipe: u32, w: u32, h: u32, surf: u32) {
    s.set(Trans::ALL[pipe as usize].offset() + regs::PIPESRC, (w - 1) << 16 | (h - 1));
    s.set(regs::plane(pipe, 0, regs::PLANE_CTL), regs::PLANE_CTL_ENABLE | regs::PLANE_CTL_FORMAT_XRGB8888);
    s.set(regs::plane(pipe, 0, regs::PLANE_STRIDE), w * 4 / 64);
    s.set(regs::plane(pipe, 0, regs::PLANE_SIZE), (h - 1) << 16 | (w - 1));
    s.set(regs::plane(pipe, 0, regs::PLANE_SURF), surf);
    s.set(regs::plane(pipe, 0, regs::PLANE_SURFLIVE), surf);
}

/// Prijemce DP na AUX: DPCD v pameti, EDID pres I2C, trenink uspeje od
/// napeti 1.
fn dp_sink(edid: Vec<u8>) -> crate::sim::AuxSink {
    let mut dpcd = vec![0u8; 0x300];
    dpcd[0] = 0x12;
    dpcd[1] = 0x0A;
    dpcd[2] = 0x82 | dp::bits::TPS3_SUPPORTED;
    let mut off = 0usize;
    Box::new(move |tx: &[u8]| {
        let (m, data) = Msg::decode(tx).unwrap();
        if m.is_native() {
            let a = m.address as usize;
            if m.is_read() {
                let mut r = vec![0u8];
                r.extend_from_slice(&dpcd[a..a + m.len]);
                return r;
            }
            dpcd[a..a + data.len()].copy_from_slice(data);
            // Stav lane podle buzeni a vzoru.
            let v = dpcd[0x103] & 3;
            let pat = dpcd[0x102] & 0xF;
            let cr = v >= 1;
            let eq = cr && pat >= 2;
            let lane = cr as u8 | (eq as u8) << 1 | (eq as u8) << 2;
            dpcd[0x202] = lane | lane << 4;
            dpcd[0x204] = eq as u8;
            dpcd[0x206] = 0x11;
            return vec![0];
        }
        match (m.address, m.cmd & cmd::I2C_READ != 0) {
            (0x50, false) => off = data[0] as usize,
            (0x50, true) if m.len > 0 => {
                let mut r = vec![0u8];
                r.extend_from_slice(&edid[off..off + m.len]);
                off += m.len;
                return r;
            }
            _ => {}
        }
        vec![0]
    })
}

/// KBL s vestavenym panelem (transcoder EDP, port A, DP 2 lane).
fn kbl_edp() -> Sim {
    let mut s = Sim::new();
    program_trans(&mut s, Trans::Edp, 1366, 768);
    s.set(Trans::Edp.offset() + regs::TRANS_DDI_FUNC_CTL, ddi_func::ENABLE | (ddi_func::MODE_DP_SST << ddi_func::MODE_SHIFT) | (1 << ddi_func::WIDTH_SHIFT));
    program_plane(&mut s, 0, 1366, 768, 0x0010_0000);
    s.set(regs::GEN8_DE_PORT_ISR, regs::DE_PORT_HOTPLUG_A);
    s.aux.insert(0, dp_sink(Vec::from(&LGD_LP133WH2[..])));
    s
}

/// KBL s HDMI monitorem na pipe B, port B (GMBUS pin 5).
fn kbl_hdmi() -> Sim {
    let mut s = Sim::new();
    program_trans(&mut s, Trans::B, 1920, 1080);
    s.set(Trans::B.offset() + regs::TRANS_DDI_FUNC_CTL, ddi_func::ENABLE | (1 << ddi_func::PORT_SHIFT) | ddi_func::PHSYNC | ddi_func::PVSYNC);
    program_plane(&mut s, 1, 1920, 1080, 0x0020_0000);
    s.set(regs::SDEISR, regs::SDE_PORTB_HOTPLUG_CPT);
    s.ddc.insert(5, Vec::from(&AOC_2243W[..]));
    s
}

#[test]
fn platforms() {
    assert_eq!(platform(0x5916).family, Family::Gen9);
    assert_eq!(platform(0x5916).pch, Pch::Spt);
    assert_eq!(platform(0x3E92).pch, Pch::Cnp);
    assert_eq!(platform(0x8A52).family, Family::Gen11);
    assert!(platform(0x9A49).tgl_ddi_select() && !platform(0x9A49).has_edp_transcoder());
    assert_eq!(platform(0x46A6).family, Family::Gen13);
    assert!(platform(0x56A0).discrete);
    assert_eq!(platform(0x7D55).family, Family::Gen14);
    assert!(!platform(0x7D55).has_legacy_aux());
    assert_eq!(platform(0xE20B).family, Family::Gen20);
    assert!(matches!(detect(0x8086, 0xB080), Detect::Unsupported(_)));
    assert_eq!(detect(0x8086, 0x0166), Detect::Unknown);
    assert_eq!(detect(0x1234, 0x5916), Detect::Unknown);
    // Zadne ID ve dvou platformach.
    let ids: Vec<u16> = supported_ids().map(|(i, _)| i).collect();
    for (i, a) in ids.iter().enumerate() {
        assert!(!ids[i + 1..].contains(a), "{a:#x}");
    }
}

#[test]
fn ddi_decode() {
    let kbl = platform(0x5916);
    let d = state::decode_ddi(&kbl, Trans::A, ddi_func::ENABLE | (2 << ddi_func::PORT_SHIFT) | (2 << ddi_func::MODE_SHIFT) | (3 << ddi_func::WIDTH_SHIFT) | (1 << ddi_func::BPC_SHIFT)).unwrap();
    assert_eq!((d.port, d.output, d.lanes, d.bpc), (2, state::Output::Dp, 4, 10));
    // TGL: port + 1 v bitech 30:27 (TC1 = port 3 -> 4).
    let tgl = platform(0x9A49);
    let d = state::decode_ddi(&tgl, Trans::B, ddi_func::ENABLE | (4 << ddi_func::TGL_PORT_SHIFT)).unwrap();
    assert_eq!((d.port, d.output), (3, state::Output::Hdmi));
    assert!(state::decode_ddi(&tgl, Trans::B, ddi_func::ENABLE).is_none());
    assert!(state::decode_ddi(&kbl, Trans::A, 0).is_none());
}

#[test]
fn reads_firmware_state() {
    let p = platform(0x5916);
    let mut s = kbl_edp();
    let a = state::read(&mut s, &p);
    assert_eq!(a.len(), 1);
    let a = &a[0];
    assert_eq!((a.trans, a.pipe, a.src), (Trans::Edp, 0, (1366, 768)));
    assert_eq!((a.timing.hactive, a.timing.hfront, a.timing.hsync, a.timing.hback), (1366, 48, 32, 80));
    assert_eq!((a.timing.vactive, a.timing.vfront, a.timing.vsync, a.timing.vback), (768, 3, 5, 23));
    assert_eq!(a.ddi.unwrap().output, state::Output::Edp);
    assert!(a.plane.enabled() && a.plane.is_linear_xrgb());
    assert_eq!(a.plane.stride_bytes(), 1366 * 4 / 64 * 64);
    // Zadne zapisy pri cteni stavu.
    assert!(s.writes.is_empty());
}

#[test]
fn edid_over_aux_and_gmbus() {
    let mut clk = FakeClock::default();
    let mut s = kbl_edp();
    let mut aux = dpaux::IntelAux { mmio: &mut s, clock: &mut clk, ch: 0 };
    assert_eq!(dp::auxch::read_edid(&mut aux).unwrap(), LGD_LP133WH2);
    // Kanal bez prijemce: timeout.
    let mut aux = dpaux::IntelAux { mmio: &mut s, clock: &mut clk, ch: 1 };
    let mut b = [0u8; 1];
    assert_eq!(dp::auxch::dpcd_read(&mut aux, 0, &mut b), Err(DErr::Io));
    // GMBUS: EDID na pinu 5, jiny pin NAK.
    let mut s = kbl_hdmi();
    let e = display::edid::read(|i, b| gmbus::read_edid_block(&mut s, &mut clk, 5, i, b)).unwrap();
    assert_eq!(e, AOC_2243W);
    let mut b = [0u8; 128];
    assert_eq!(gmbus::read_edid_block(&mut s, &mut clk, 4, 0, &mut b), Err(DErr::Nack));
    assert_eq!(gmbus::pin_for_port(Pch::Spt, 1), Some(5));
    assert_eq!(gmbus::pin_for_port(Pch::Icp, 3), Some(9));
}

#[test]
fn probe_heads() {
    let mut clk = FakeClock::default();
    let p = platform(0x5916);
    let mut s = kbl_edp();
    let h = probe(&mut s, &mut clk, &p).unwrap();
    assert_eq!((h.kind, h.native), (kind::EDP, (1366, 768)));
    assert_eq!(h.monitor, "LGD 0217");
    assert_eq!(h.refresh_mhz, 59_978);
    assert!(h.scaled.is_empty());
    let mut s = kbl_hdmi();
    let h = probe(&mut s, &mut clk, &p).unwrap();
    assert_eq!((h.kind, h.monitor.as_str(), h.active.pipe), (kind::HDMI, "2243W", 1));
    assert!(h.scaled.contains(&(1280, 720)) && h.scaled.contains(&(1024, 768)));
    assert!(h.scaled.iter().all(|&(w, hh)| w <= 1920 && hh <= 1080));
    // Dlazdicovy framebuffer firmwaru neumime prevzit.
    let mut s = kbl_hdmi();
    s.set(regs::plane(1, 0, regs::PLANE_CTL), regs::PLANE_CTL_ENABLE | regs::PLANE_CTL_FORMAT_XRGB8888 | (1 << 10));
    assert_eq!(probe(&mut s, &mut clk, &p).err(), Some(guiproto::Error::NotSupported));
    // Nic zapnuto.
    assert_eq!(probe(&mut Sim::new(), &mut clk, &p).err(), Some(guiproto::Error::NotFound));
}

struct VecShadow(Vec<u32>, u32, u32);

impl Shadow for VecShadow {
    fn surface(&self) -> Surface {
        Surface { base: self.0.as_ptr() as *mut u8, width: self.1, height: self.2, stride: self.1 * 4 }
    }
}

/// Server nad backendem Intel s buffery ve Vec (krok = nativni krok).
fn server(mut s: Sim, safe: bool, mem: &mut [Vec<u8>; 2]) -> Server<Intel<Sim, FakeClock>> {
    let p = platform(0x5916);
    let mut clk = FakeClock::default();
    let head = probe(&mut s, &mut clk, &p).unwrap();
    let surf = head.active.plane.ggtt();
    let bufs = Buffers { ggtt: [surf, 0xF000_0000], cpu: [mem[0].as_mut_ptr(), mem[1].as_mut_ptr()], count: 2 };
    let mut b = Intel::new(s, clk, p, head, bufs, safe);
    b.mark_saved();
    b.mmio.writes.clear();
    Server::new(b)
}

#[test]
fn takeover_flips_at_vblank() {
    let mut mem = [vec![0u8; 1920 * 1080 * 4], vec![0u8; 1920 * 1080 * 4]];
    let mut srv = server(kbl_hdmi(), false, &mut mem);
    let surf_reg = regs::plane(1, 0, regs::PLANE_SURF);
    let mut shadow = VecShadow(vec![0x00AB_CDEF; 1920 * 1080], 1920, 1080);
    shadow.0[0] = 0x0012_3456;
    let r = srv.handle(Request::Attach { output: 0, w: 1920, h: 1080, stride: 1920 * 4 }, Some(Box::new(shadow)), 0);
    assert_eq!(r, vec![Reply::Attached { output: 0 }]);
    // Prvni snimek do vlastniho bufferu (1), flip = zapis PLANE_SURF.
    assert_eq!(srv.backend.mmio.writes_in(surf_reg, surf_reg + 1), vec![(surf_reg, 0xF000_0000)]);
    assert_eq!(&mem[1][..4], &[0x56, 0x34, 0x12, 0x00]);
    // PRESENT: buffer 0 (framebuffer firmwaru), PRESENTED az po vsync.
    let r = srv.handle(Request::Present { seq: 1, output: 0, rects: vec![] }, None, 1_000);
    assert!(r.is_empty());
    assert!(srv.poll(2_000).is_empty());
    srv.backend.mmio.vblank();
    assert!(matches!(srv.poll(3_000)[..], [Reply::Presented { seq: 1, .. }]));
    let writes: Vec<u32> = srv.backend.mmio.writes_in(surf_reg, surf_reg + 1).iter().map(|w| w.1).collect();
    assert_eq!(writes, [0xF000_0000, 0x0020_0000]);
    // Schopnosti a konektor.
    match &srv.handle(Request::Info, None, 4_000)[0] {
        Reply::Modes { caps: c, .. } => assert!(c & caps::MODESET != 0 && c & caps::VSYNC != 0),
        r => panic!("{r:?}"),
    }
}

#[test]
fn scaled_mode_and_restore() {
    let mut mem = [vec![0u8; 1920 * 1080 * 4], vec![0u8; 1920 * 1080 * 4]];
    let mut srv = server(kbl_hdmi(), false, &mut mem);
    let snap = srv.backend.snapshot();
    assert_eq!(snap.regs.last().unwrap().0, regs::plane(1, 0, regs::PLANE_SURF));
    let r = srv.handle(Request::SetMode { output: 0, w: 1024, h: 768, refresh_mhz: 0 }, None, 0);
    assert!(matches!(r[..], [Reply::ModeSet { w: 1024, h: 768, .. }]), "{r:?}");
    let m = &mut srv.backend.mmio;
    let ps = |r| regs::scaler(1, 0, r);
    // 4:3 na 16:9: okno 1440x1080 na stredu.
    assert_eq!(m.get(ps(regs::PS_CTRL)) & regs::PS_SCALER_EN, regs::PS_SCALER_EN);
    assert_eq!(m.get(ps(regs::PS_WIN_POS)), 240 << 16);
    assert_eq!(m.get(ps(regs::PS_WIN_SZ)), 1440 << 16 | 1080);
    assert_eq!(m.get(Trans::B.offset() + regs::PIPESRC), 1023 << 16 | 767);
    assert_eq!(m.get(regs::plane(1, 0, regs::PLANE_SIZE)), 767 << 16 | 1023);
    // Casovani transcoderu se nemenilo.
    assert!(m.writes.iter().all(|&(r, _)| !(0x61000..0x61018).contains(&r)));
    // Nepodporovany rezim (vetsi nez nativni).
    assert!(matches!(srv.handle(Request::SetMode { output: 0, w: 2560, h: 1440, refresh_mhz: 0 }, None, 0)[0], Reply::Error { .. }));
    // Navrat na nativni vypne scaler.
    srv.handle(Request::SetMode { output: 0, w: 1920, h: 1080, refresh_mhz: 0 }, None, 0);
    assert_eq!(srv.backend.mmio.get(ps(regs::PS_CTRL)), 0);
    // Obnova zaznamu: registry jako od firmwaru, adresa povrchu posledni.
    srv.backend.mmio.writes.clear();
    srv.backend.restore(&snap);
    for &(r, v) in &snap.regs {
        assert_eq!(srv.backend.mmio.get(r), v, "{r:#x}");
    }
    assert_eq!(srv.backend.mmio.writes.last().unwrap().0, regs::plane(1, 0, regs::PLANE_SURF));
}

#[test]
fn window_keeps_aspect() {
    assert_eq!(Intel::<Sim, FakeClock>::window((1920, 1080), 1280, 720), (0, 0, 1920, 1080));
    assert_eq!(Intel::<Sim, FakeClock>::window((1920, 1080), 1024, 768), (240, 0, 1440, 1080));
    assert_eq!(Intel::<Sim, FakeClock>::window((1920, 1200), 1920, 1080), (0, 60, 1920, 1080));
}

#[test]
fn safe_mode_only_firmware() {
    let mut mem = [vec![0u8; 1920 * 1080 * 4], vec![0u8; 1920 * 1080 * 4]];
    let mut srv = server(kbl_hdmi(), true, &mut mem);
    assert_eq!(srv.backend.caps() & caps::MODESET, 0);
    assert_eq!(srv.backend.buffers(0), 1);
    let c = srv.backend.connectors();
    assert_eq!(c[0].modes.len(), 1);
    assert!(matches!(srv.handle(Request::SetMode { output: 0, w: 1024, h: 768, refresh_mhz: 0 }, None, 0)[0], Reply::Error { .. }));
    // Kopie primo do framebufferu firmwaru (buffer 0).
    let shadow = VecShadow(vec![0x0001_0203; 1920 * 1080], 1920, 1080);
    srv.handle(Request::Attach { output: 0, w: 1920, h: 1080, stride: 1920 * 4 }, Some(Box::new(shadow)), 0);
    assert_eq!(&mem[0][..4], &[0x03, 0x02, 0x01, 0x00]);
}

#[test]
fn hotplug_poll() {
    let mut mem = [vec![0u8; 16], vec![0u8; 16]];
    let mut srv = server(kbl_hdmi(), false, &mut mem);
    assert!(srv.poll(0).is_empty());
    srv.backend.mmio.set(regs::SDEISR, 0);
    assert_eq!(srv.poll(1), vec![Reply::Hotplug { output: 0 }]);
    assert!(!srv.backend.connectors()[0].connected);
    assert!(srv.poll(2).is_empty());
}

#[test]
fn ggtt_entries() {
    let mut s = Sim::new();
    gtt::map(&mut s, 0x10_0000, 0x8000_0000, 3 * 4096, false);
    assert_eq!(gtt::contiguous(&mut s, 0x10_0000, 3 * 4096), Some((0x8000_0000, false)));
    assert!(gtt::contiguous(&mut s, 0x10_0000, 4 * 4096).is_none());
    assert!(gtt::is_free(&mut s, 0x20_0000, 8192));
    assert!(!gtt::is_free(&mut s, 0x10_1000, 4096));
    // Polozka: adresa | platna; LMEM bit 1.
    assert_eq!(s.get(regs::GGTT_OFFSET + 0x100 * 8), 0x8000_0001);
    gtt::map(&mut s, 0, 0x1000, 4096, true);
    assert_eq!(gtt::read(&mut s, 0).unwrap().local, true);
}

#[test]
fn gen9_link_training() {
    let mut s = kbl_edp();
    let mut clk = FakeClock::default();
    let mut link = phy::Gen9Link { mmio: &mut s, clock: &mut clk, port: 0, aux_ch: 0 };
    let mut base = [0u8; 16];
    dp::auxch::dpcd_read(&mut link, 0, &mut base).unwrap();
    let caps = dp::Caps::parse(&base, None, 0).unwrap();
    let cfg = dp::LinkConfig { rate: dp::HBR, lanes: 2, enhanced_frame: true, edp_rate_index: None };
    let t = dp::train::train(&mut link, &caps, &cfg).unwrap();
    assert_eq!(t.drive, Drive { vswing: 1, preemph: 0 });
    assert_eq!(t.pattern, Pattern::Tps3);
    // Vzory v DP_TP_CTL: TPS1, TPS3, normalni provoz.
    let tp: Vec<u32> = s.writes_in(regs::dp_tp_ctl(0), regs::dp_tp_ctl(0) + 1).iter().map(|w| (w.1 & regs::DP_TP_CTL_TRAIN_MASK) >> 8).collect();
    assert_eq!(tp.first(), Some(&regs::DP_TP_PAT1));
    assert!(tp.contains(&regs::DP_TP_PAT3));
    assert_eq!(tp.last(), Some(&regs::DP_TP_NORMAL));
    // Buzeni (1, 0) = polozka 4 tabulky.
    assert_eq!((s.get(regs::ddi_buf_ctl(0)) & regs::DDI_BUF_EMP_MASK) >> 24, 4);
    assert_eq!(phy::level(Drive { vswing: 3, preemph: 0 }), 9);
}
