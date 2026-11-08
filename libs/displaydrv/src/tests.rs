//! Testy kostry ovladace nad simulovanym backendem (pamet bufferu ve Vec).

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use fbcopy::{Format, Surface};
use guiproto::display::{Reply, Request, caps};
use guiproto::modes::{Connector, Mode, kind, mode_flags};
use guiproto::{Error, Rect};

use crate::snapshot::{Snapshot, driver};
use crate::{Backend, Scanout, Server, Shadow, VblankClock};

struct Fake {
    w: u32,
    h: u32,
    bufs: [Vec<u32>; 2],
    shown: usize,
    flips: u32,
    flip_at: Option<u64>,
    clock: VblankClock,
    fail_mode: bool,
    hotplug: Vec<u32>,
}

impl Fake {
    fn new(w: u32, h: u32) -> Self {
        Fake {
            w,
            h,
            bufs: [vec![0; (w * h) as usize], vec![0; (w * h) as usize]],
            shown: 0,
            flips: 0,
            flip_at: None,
            clock: VblankClock::new(0, 60_000),
            fail_mode: false,
            hotplug: Vec::new(),
        }
    }
}

impl Backend for Fake {
    fn caps(&self) -> u32 {
        caps::MODESET | caps::VSYNC | caps::HOTPLUG
    }

    fn connectors(&mut self) -> Vec<Connector> {
        let m = |w, h, f| Mode { w, h, refresh_mhz: 60_000, clock_khz: 0, flags: f };
        let cur = |w, h| if (w, h) == (self.w, self.h) { mode_flags::CURRENT } else { 0 };
        vec![Connector {
            output: 0,
            kind: kind::VIRTUAL,
            connected: true,
            monitor: "test".into(),
            size_mm: (0, 0),
            modes: vec![m(64, 32, mode_flags::PREFERRED | cur(64, 32)), m(32, 16, cur(32, 16))],
        }]
    }

    fn size(&self, output: u32) -> Option<(u32, u32)> {
        (output == 0).then_some((self.w, self.h))
    }

    fn set_mode(&mut self, _o: u32, w: u32, h: u32, _r: u32) -> crate::Result<(u32, u32, u32)> {
        if self.fail_mode || !matches!((w, h), (64, 32) | (32, 16)) {
            return Err(Error::NotSupported);
        }
        self.w = w;
        self.h = h;
        self.bufs = [vec![0; (w * h) as usize], vec![0; (w * h) as usize]];
        Ok((w, h, 60_000))
    }

    fn buffers(&self, _o: u32) -> usize {
        2
    }

    fn buffer(&mut self, _o: u32, i: usize) -> Option<Scanout> {
        Some(Scanout {
            surface: Surface { base: self.bufs[i].as_mut_ptr() as *mut u8, width: self.w, height: self.h, stride: self.w * 4 },
            format: Format::Bgr,
        })
    }

    fn flip(&mut self, _o: u32, i: usize, now: u64) -> crate::Result<()> {
        self.shown = i;
        self.flips += 1;
        self.flip_at = Some(self.clock.next(now + 1));
        Ok(())
    }

    fn flip_done(&mut self, _o: u32, now: u64) -> Option<u64> {
        match self.flip_at {
            Some(t) if now >= t => {
                self.flip_at = None;
                Some(t)
            }
            _ => None,
        }
    }

    fn poll_hotplug(&mut self, _now: u64) -> Vec<u32> {
        core::mem::take(&mut self.hotplug)
    }
}

/// Stin ve Vec (sdileny ukazatel: test do nej kresli i po predani).
struct VecShadow {
    px: *mut u32,
    w: u32,
    h: u32,
}

impl Shadow for VecShadow {
    fn surface(&self) -> Surface {
        Surface { base: self.px as *mut u8, width: self.w, height: self.h, stride: self.w * 4 }
    }
}

fn attach(s: &mut Server<Fake>, px: &mut Vec<u32>, w: u32, h: u32) -> Vec<Reply> {
    let sh = Box::new(VecShadow { px: px.as_mut_ptr(), w, h });
    s.handle(Request::Attach { output: 0, w, h, stride: w * 4 }, Some(sh), 0)
}

/// Co je prave na obrazovce (bez alfa bajtu).
fn screen(s: &Server<Fake>) -> Vec<u32> {
    s.backend.bufs[s.backend.shown].iter().map(|p| p & 0x00FF_FFFF).collect()
}

fn fill(px: &mut [u32], w: u32, r: Rect, c: u32) {
    for y in r.y as u32..r.y as u32 + r.h {
        for x in r.x as u32..r.x as u32 + r.w {
            px[(y * w + x) as usize] = c;
        }
    }
}

#[test]
fn info_and_connectors() {
    let mut s = Server::new(Fake::new(64, 32));
    match &s.handle(Request::Info, None, 0)[0] {
        Reply::Modes { caps: c, outputs } => {
            assert!(c & caps::VSYNC != 0);
            assert_eq!((outputs[0].w, outputs[0].h, outputs[0].enabled), (64, 32, true));
        }
        r => panic!("{r:?}"),
    }
    match &s.handle(Request::Connectors, None, 0)[0] {
        Reply::Connectors { connectors } => assert_eq!(connectors[0].current().unwrap().w, 64),
        r => panic!("{r:?}"),
    }
    assert_eq!(s.handle(Request::Sync { token: 7 }, None, 0), vec![Reply::Synced { token: 7 }]);
}

#[test]
fn double_buffer_keeps_screen_equal_to_shadow() {
    let (w, h) = (64u32, 32u32);
    let mut s = Server::new(Fake::new(w, h));
    let mut px = vec![0x11u32; (w * h) as usize];
    assert_eq!(attach(&mut s, &mut px, w, h), vec![Reply::Attached { output: 0 }]);
    let expect = |px: &Vec<u32>| px.iter().map(|p| p & 0x00FF_FFFF).collect::<Vec<u32>>();
    assert_eq!(screen(&s), expect(&px));
    let mut now = 0;
    // Nekolik snimku s ruznym poskozenim: obrazovka vzdy odpovida stinu.
    let damages = [Rect::new(0, 0, 10, 10), Rect::new(30, 5, 8, 20), Rect::new(50, 20, 14, 12), Rect::new(5, 5, 3, 3)];
    for (i, d) in damages.iter().enumerate() {
        fill(&mut px, w, *d, 0x100 * (i as u32 + 2));
        let r = s.handle(Request::Present { seq: i as u64 + 1, output: 0, rects: vec![*d] }, None, now);
        assert!(r.is_empty());
        // Pred vsync PRESENTED neprijde.
        assert!(s.poll(now).iter().all(|r| !matches!(r, Reply::Presented { .. })));
        now += 17_000_000;
        let p = s.poll(now);
        assert!(matches!(p[0], Reply::Presented { seq, time } if seq == i as u64 + 1 && time <= now), "{p:?}");
        assert_eq!(screen(&s), expect(&px), "snimek {i}");
    }
    assert!(s.backend.flips >= 5);
}

#[test]
fn present_while_inflight_is_merged() {
    let (w, h) = (64u32, 32u32);
    let mut s = Server::new(Fake::new(w, h));
    let mut px = vec![0u32; (w * h) as usize];
    attach(&mut s, &mut px, w, h);
    fill(&mut px, w, Rect::new(0, 0, 4, 4), 0xAA);
    s.handle(Request::Present { seq: 1, output: 0, rects: vec![Rect::new(0, 0, 4, 4)] }, None, 0);
    fill(&mut px, w, Rect::new(60, 28, 4, 4), 0xBB);
    assert!(s.handle(Request::Present { seq: 2, output: 0, rects: vec![Rect::new(60, 28, 4, 4)] }, None, 1).is_empty());
    assert!(s.busy());
    let p = s.poll(20_000_000);
    assert!(matches!(p[0], Reply::Presented { seq: 1, .. }));
    let p = s.poll(40_000_000);
    assert!(matches!(p[0], Reply::Presented { seq: 2, .. }));
    assert_eq!(screen(&s)[(31 * w + 63) as usize], 0xBB);
    assert!(!s.busy());
}

#[test]
fn set_mode_and_reattach() {
    let mut s = Server::new(Fake::new(64, 32));
    let mut px = vec![0u32; 64 * 32];
    attach(&mut s, &mut px, 64, 32);
    s.handle(Request::Present { seq: 1, output: 0, rects: vec![] }, None, 0);
    // Zmena rezimu dokonci snimek na ceste a zrusi pripojeni.
    let r = s.handle(Request::SetMode { output: 0, w: 32, h: 16, refresh_mhz: 0 }, None, 5);
    assert_eq!(r, vec![Reply::Presented { seq: 1, time: 5 }, Reply::ModeSet { output: 0, w: 32, h: 16, refresh_mhz: 60_000 }]);
    let r = s.handle(Request::Present { seq: 2, output: 0, rects: vec![] }, None, 6);
    assert!(matches!(r[0], Reply::Error { code: Error::BadState, .. }));
    // ATTACH stare velikosti neprojde, nove ano.
    let mut old = vec![0u32; 64 * 32];
    assert!(matches!(attach(&mut s, &mut old, 64, 32)[0], Reply::Error { code: Error::NotSupported, .. }));
    let mut px = vec![0x42u32; 32 * 16];
    assert_eq!(attach(&mut s, &mut px, 32, 16), vec![Reply::Attached { output: 0 }]);
    assert!(screen(&s).iter().all(|&p| p == 0x42));
    // Neuspesna zmena rezimu: chyba, pripojeni zustava.
    s.backend.fail_mode = true;
    let r = s.handle(Request::SetMode { output: 0, w: 64, h: 32, refresh_mhz: 0 }, None, 7);
    assert!(matches!(r[0], Reply::Error { code: Error::NotSupported, .. }));
    assert!(s.handle(Request::Present { seq: 3, output: 0, rects: vec![] }, None, 8).is_empty());
}

#[test]
fn attach_validates() {
    let mut s = Server::new(Fake::new(64, 32));
    let r = s.handle(Request::Attach { output: 0, w: 64, h: 32, stride: 256 }, None, 0);
    assert!(matches!(r[0], Reply::Error { code: Error::InvalidArgs, .. }));
    let mut px = vec![0u32; 64 * 32];
    let sh = Box::new(VecShadow { px: px.as_mut_ptr(), w: 64, h: 32 });
    let r = s.handle(Request::Attach { output: 3, w: 64, h: 32, stride: 256 }, Some(sh), 0);
    assert!(matches!(r[0], Reply::Error { code: Error::NotFound, .. }));
}

#[test]
fn hotplug_and_reset_events() {
    let mut s = Server::new(Fake::new(64, 32));
    s.announce_reset();
    s.backend.hotplug.push(0);
    assert_eq!(s.poll(0), vec![Reply::Reset, Reply::Hotplug { output: 0 }]);
    assert!(s.poll(1).is_empty());
}

#[test]
fn vblank_clock() {
    let c = VblankClock::new(1000, 50_000);
    assert_eq!(c.period_ns, 20_000_000);
    assert_eq!(c.next(0), 1000);
    assert_eq!(c.next(1001), 20_001_000);
    assert_eq!(c.next(20_001_000), 20_001_000);
}

#[test]
fn snapshot_roundtrip() {
    let snap = Snapshot { driver: driver::INTEL, w: 1920, h: 1080, stride: 7680, fb_offset: 0x1000_0000, regs: vec![(0x70180, 0x8400_0000), (0x7019C, 0)] };
    let mut b = vec![0u8; 4096];
    assert!(snap.write(&mut b));
    assert_eq!(Snapshot::read(&b, driver::INTEL), Some(snap.clone()));
    // Jiny ovladac nebo zneplatneny zaznam se nepouzije.
    assert_eq!(Snapshot::read(&b, driver::BOCHS), None);
    Snapshot::clear(&mut b);
    assert_eq!(Snapshot::read(&b, driver::INTEL), None);
    // Prilis maly objekt pameti.
    assert!(!snap.write(&mut [0u8; 16]));
}
