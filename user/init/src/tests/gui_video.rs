//! Video do kompozitoru bez kopie (krok 6a-4): okno s bufferem NV12 a I420
//! (`BUFFER_CREATE_DESC`, popis `bufdesc`) a ploty ziskani a uvolneni
//! (`SYNC_POINTS`). Testovaci klient je primo init (surovy protokol
//! `guiproto::window`).
//!
//! - commit s nesplnenym plotem ziskani se nezobrazi; po signalu ano
//!   a pixel ma presne barvu podle vzorcu `libs/yuv` (BT.709),
//! - plot uvolneni bufferu YUV se splni hned po prevodu,
//! - plot ziskani skonci chybou: commit se pouzije, klient dostane
//!   `ERROR` s `FENCE_FAILED`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use bufdesc::{BufDesc, format};
use guiproto::control::{ConnectInfo, WindowInfo};
use guiproto::window::{ClientMsg, LayerKind, LayerProps, Mode, ServerMsg, op, prop};
use guiproto::Rect;
use libos::fence::Fence;
use libos::{Channel, Startup, Vmo, time};

use super::gui::{State, pixel, settle, windows, with};
use super::{TestResult, fail};
use crate::{check, check_eq};

const APP: &str = "test.video";
const W: u32 = 128;
const H: u32 = 96;

fn send(ch: &Channel, m: ClientMsg, handles: Vec<libos::Handle>) -> Result<(), String> {
    ch.write(&m.encode(), handles).map_err(|e| fail("zprava kompozitoru", e))
}

/// Dalsi zprava kompozitoru, ktera splni `want` (ostatni se zahodi).
fn expect(ch: &Channel, what: &str, want: impl Fn(&ServerMsg) -> bool) -> Result<ServerMsg, String> {
    let end = time::deadline_after(Duration::from_secs(5));
    loop {
        let m = ch.read(Some(end)).map_err(|e| fail(what, e))?;
        let m = ServerMsg::decode(&m.data).map_err(|e| fail(what, e))?;
        if want(&m) {
            return Ok(m);
        }
    }
}

/// Buffer `fourcc` vyplneny barvou (r, g, b); vrati VMO a ocekavany pixel.
fn frame(fourcc: u32, rgb: (u8, u8, u8)) -> Result<(BufDesc, Vmo, u32), String> {
    let desc = BufDesc::linear(fourcc, W, H, 16).map_err(|e| alloc::format!("popis: {e:?}"))?;
    let size = desc.min_size().map_err(|e| alloc::format!("velikost: {e:?}"))?;
    let (y, u, v) = yuv::rgb_to_yuv(rgb.0, rgb.1, rgb.2);
    let mut data = vec![0u8; size as usize];
    let p = &desc.planes;
    let rows = |i: usize, n: u32| p[i].offset as usize..p[i].offset as usize + p[i].pitch as usize * n as usize;
    data[rows(0, H)].fill(y);
    match fourcc {
        format::NV12 => {
            for pair in data[rows(1, H / 2)].chunks_mut(2) {
                pair[0] = u;
                pair[1] = v;
            }
        }
        _ => {
            data[rows(1, H / 2)].fill(u);
            data[rows(2, H / 2)].fill(v);
        }
    }
    let vmo = Vmo::create(size).map_err(|e| fail("VMO bufferu", e))?;
    vmo.write(0, &data).map_err(|e| fail("zapis bufferu", e))?;
    Ok((desc, vmo, yuv::pixel(y, u, v)))
}

fn window(s: &State) -> Result<Option<WindowInfo>, String> {
    Ok(windows(&s.inspect)?.into_iter().find(|w| w.app == APP))
}

/// Pixel uprostred okna (s cekanim na prekresleni).
fn center(s: &State, w: &WindowInfo) -> Result<u32, String> {
    settle(&s.inspect)?;
    pixel(&s.inspect, w.rect.x + W as i32 / 2, w.rect.y + H as i32 / 2)
}

fn attach(ch: &Channel, buffer: u32) -> Result<(), String> {
    let props = LayerProps { mask: prop::BUFFER | prop::RECT, rect: Rect::new(0, 0, W, H), buffer, ..Default::default() };
    send(ch, ClientMsg::LayerSet { layer: 1, props }, Vec::new())
}

fn commit(ch: &Channel, txn: u32) -> Result<(), String> {
    send(ch, ClientMsg::Commit { txn, flags: 0, window: 1, serial: 0, token: 0 }, Vec::new())
}

pub fn nv12_with_fences(_: &Startup) -> TestResult {
    with(|s| {
        let info = ConnectInfo { id: APP.into(), name: "Video".into(), perms: Vec::new(), actions: Vec::new(), system: false };
        let ch = s.gui.channel(&info.encode()).map_err(|e| fail("okno pro test", e))?;
        send(&ch, ClientMsg::Hello { version: guiproto::VERSION }, Vec::new())?;
        expect(&ch, "WELCOME", |m| matches!(m, ServerMsg::Welcome { .. }))?;
        send(&ch, ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w: W, h: H, title: "Video".into() }, Vec::new())?;
        send(&ch, ClientMsg::LayerCreate { layer: 1, window: 1, parent: 0, kind: LayerKind::Buffer }, Vec::new())?;

        // 1. NV12 s plotem ziskani: commit ceka na producenta.
        let (desc, vmo, want) = frame(format::NV12, (200, 60, 30))?;
        let h = vmo.handle().duplicate(None).map_err(|e| fail("duplikat VMO", e))?;
        send(&ch, ClientMsg::BufferCreateDesc { buffer: 1, desc }, vec![h])?;
        attach(&ch, 1)?;
        let acquire = Fence::create(0).map_err(|e| fail("plot ziskani", e))?;
        let release = Fence::create(0).map_err(|e| fail("plot uvolneni", e))?;
        let handles = vec![
            acquire.for_waiter().map_err(|e| fail("plot pro cekani", e))?.into_handle(),
            release.handle().duplicate(None).map_err(|e| fail("plot uvolneni", e))?,
        ];
        send(&ch, ClientMsg::SyncPoints { window: 1, acquire: Some(1), release: Some(1) }, handles)?;
        commit(&ch, 1)?;
        time::sleep(Duration::from_millis(300));
        let shown = window(s)?.map(|w| center(s, &w)).transpose()?;
        check!(shown != Some(want), "snimek se zobrazil pred splnenim plotu ziskani");
        check!(release.query().map(|i| i.value).unwrap_or(0) == 0, "plot uvolneni pred pouzitim commitu");
        acquire.signal(1).map_err(|e| fail("signal ziskani", e))?;
        let end = time::deadline_after(Duration::from_secs(5));
        let w = loop {
            if let Some(w) = window(s)? {
                break w;
            }
            check!(time::now() < end, "okno se po plotu ziskani neobjevilo");
            time::sleep(Duration::from_millis(20));
        };
        check_eq!(center(s, &w)?, want, "pixel NV12 podle BT.709");
        release.wait(1, Some(time::deadline_after(Duration::from_secs(5)))).map_err(|e| fail("plot uvolneni NV12", e))?;

        // 2. I420 s plotem ziskani, ktery skonci chybou: commit se pouzije,
        // klient dostane ERROR.
        let (desc, vmo2, want2) = frame(format::YUV420, (30, 90, 220))?;
        let h = vmo2.handle().duplicate(None).map_err(|e| fail("duplikat VMO", e))?;
        send(&ch, ClientMsg::BufferCreateDesc { buffer: 2, desc }, vec![h])?;
        attach(&ch, 2)?;
        let handles = vec![
            acquire.for_waiter().map_err(|e| fail("plot pro cekani", e))?.into_handle(),
            release.handle().duplicate(None).map_err(|e| fail("plot uvolneni", e))?,
        ];
        send(&ch, ClientMsg::SyncPoints { window: 1, acquire: Some(2), release: Some(2) }, handles)?;
        commit(&ch, 2)?;
        acquire.fail(2).map_err(|e| fail("chyba ziskani", e))?;
        let e = expect(&ch, "ERROR po chybe plotu", |m| matches!(m, ServerMsg::Error { op: o, .. } if *o == op::COMMIT))?;
        check_eq!(e, ServerMsg::Error { op: op::COMMIT, code: libos::Error::FenceFailed }, "chyba plotu ziskani");
        check_eq!(center(s, &w)?, want2, "pixel I420 (commit pouzit i po chybe plotu)");
        release.wait(2, Some(time::deadline_after(Duration::from_secs(5)))).map_err(|e| fail("plot uvolneni I420", e))?;

        // 3. Spatny popis: VMO mensi nez popis.
        let (desc, _, _) = frame(format::NV12, (0, 0, 0))?;
        let small = Vmo::create(4096).map_err(|e| fail("male VMO", e))?;
        send(&ch, ClientMsg::BufferCreateDesc { buffer: 3, desc }, vec![small.into_handle()])?;
        let e = expect(&ch, "ERROR maleho VMO", |m| matches!(m, ServerMsg::Error { op: o, .. } if *o == op::BUFFER_CREATE_DESC))?;
        check!(matches!(e, ServerMsg::Error { .. }), "male VMO odmitnuto");
        libos::println!("init: test gui: video NV12 {want:#010x} a I420 {want2:#010x} s ploty");
        drop(ch);
        Ok(())
    })
}
