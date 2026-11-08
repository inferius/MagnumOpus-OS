//! fbdisplay: ovladac displeje nad framebufferem z firmwaru (UEFI GOP).
//!
//! Nahrada virtio-gpu pro stroje bez nej (VMware, Hyper-V gen2,
//! VirtualBox, skutecne PC): kompozitor mluvi stejnym protokolem
//! `guiproto::display`, jen misto prikazu zarizeni ovladac zapisuje primo
//! do pameti obrazovky. Spousti ho init, kdyz spravce zarizeni nenahlasi
//! zadny displej.
//!
//! Startovni zprava (kind USER, v tomto poradi):
//! 1. fyzicky objekt pameti framebufferu (od jadra pres init, bootstrap
//!    `FRAMEBUFFER`),
//! 2. kanal ke klientovi (kompozitor, protokol `guiproto::display`);
//!    jeho zavreni = konec ovladace.
//!
//! Geometrii zjisti ovladac sam (`FB_CONTROL` `INFO`) a obrazovku prevezme
//! vyslovne (`CLAIM`: jadro prestane kreslit log). Pri konci procesu
//! (i padu) ji jadro vrati samo, pokud ji mezitim neprevzal nekdo jiny
//! (zachranna konzole `user/console`, kterou spousti init).
//!
//! Omezeni: jediny vystup s pevnym rozlisenim z firmwaru (`ATTACH` jine
//! velikosti = `NOT_SUPPORTED`), bez akcelerace a bez vsync - `PRESENTED`
//! prijde hned po kopii. Framebuffer je mapovany write-combining (jadro),
//! do nej se jen zapisuje.
//!
//! RIZIKO: kompozitor muze VMO menit i behem kopie - roztrhne to jen jeden
//! snimek (jako u virtio-gpu).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};

use fbcopy::{Format, Surface};
use guiproto::Rect;
use guiproto::display::{Output, Reply, Request};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Error, Mapping, Result, Signals, Startup, Vmo, println};

libos::entry!(main);

/// Geometrie framebufferu (z `FB_CONTROL` `INFO`).
#[derive(Debug, Clone, Copy)]
struct Geometry {
    width: u32,
    height: u32,
    /// Krok radku v pixelech.
    stride: u32,
    format: Format,
}

impl Geometry {
    fn query(vmo: &Vmo) -> Result<Self> {
        let i = libos::sys::fb_info(vmo.handle().raw())?;
        let format = Format::from_abi(i.format).ok_or(Error::NotSupported)?;
        let dim = 1..=guiproto::MAX_DIM;
        if !dim.contains(&i.width) || !dim.contains(&i.height) || i.stride < i.width {
            return Err(Error::NotSupported);
        }
        Ok(Self {
            width: i.width,
            height: i.height,
            stride: i.stride,
            format,
        })
    }

    fn bytes(&self) -> u64 {
        self.stride as u64 * self.height as u64 * 4
    }
}

/// Zdroj kompozitoru (VMO jen pro cteni).
struct Source {
    map: Mapping,
    _vmo: Vmo,
    stride: u32,
}

struct Display {
    geo: Geometry,
    fb: Mapping,
    _fb_vmo: Vmo,
    src: Option<Source>,
}

impl Display {
    fn open(vmo: Vmo) -> Result<Self> {
        let geo = Geometry::query(&vmo)?;
        // Jadro mapuje framebuffer write-combining (vlastnost objektu).
        let fb = Mapping::new(&vmo, 0, geo.bytes(), MAP_READ | MAP_WRITE)?;
        // Az je vse pripravene: od ted do framebufferu kreslime jen my.
        libos::sys::fb_claim(vmo.handle().raw(), true)?;
        Ok(Self {
            geo,
            fb,
            _fb_vmo: vmo,
            src: None,
        })
    }

    fn surface_fb(&self) -> Surface {
        Surface {
            base: self.fb.addr() as *mut u8,
            width: self.geo.width,
            height: self.geo.height,
            stride: self.geo.stride * 4,
        }
    }

    fn attach(&mut self, output: u32, w: u32, h: u32, stride: u32, vmo: Vmo) -> Result<()> {
        if output != 0 {
            return Err(Error::NotFound);
        }
        // Rozliseni urcuje firmware: jine nez jeho nejde.
        if w != self.geo.width || h != self.geo.height {
            return Err(Error::NotSupported);
        }
        if stride % 4 != 0 || (stride as u64) < w as u64 * 4 || stride as u64 * h as u64 > 256 << 20 {
            return Err(Error::InvalidArgs);
        }
        // Mapovani overi, ze je VMO dost velka.
        let map = Mapping::new(&vmo, 0, stride as u64 * h as u64, MAP_READ)?;
        self.src = Some(Source {
            map,
            _vmo: vmo,
            stride,
        });
        self.present(0, &[Rect::new(0, 0, w, h)])
    }

    fn present(&mut self, output: u32, rects: &[Rect]) -> Result<()> {
        if output != 0 {
            return Err(Error::NotFound);
        }
        let src = self.src.as_ref().ok_or(Error::BadState)?;
        let s = Surface {
            base: src.map.addr() as *mut u8,
            width: self.geo.width,
            height: self.geo.height,
            stride: src.stride,
        };
        let d = self.surface_fb();
        let screen = Rect::new(0, 0, self.geo.width, self.geo.height);
        for r in guiproto::display::normalize_rects(rects, screen) {
            let r = fbcopy::Rect::new(r.x, r.y, r.w, r.h);
            // SAFETY: zdroj je mapovany cely (`stride * h`, overeno v
            // `attach`), framebuffer take (`open`); obe mapovani ziji se
            // `self`. Do framebufferu pise jen tento proces (jadro po
            // predani uz ne).
            unsafe { fbcopy::copy_rect(&s, &d, r, self.geo.format) };
        }
        // Vyprazdnit buffery write-combining, at je snimek videt driv nez
        // odpoved `PRESENTED`.
        fence(Ordering::SeqCst);
        Ok(())
    }
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(fb), Some(client)) = (handles.next(), handles.next()) else {
        println!("fbdisplay: chybi handly ve startovni zprave");
        return 1;
    };
    let fb = Vmo::from_handle(fb);
    let client = Channel::from_handle(client);
    let mut display = match Display::open(fb) {
        Ok(d) => d,
        Err(e) => {
            println!("fbdisplay: framebuffer nelze prevzit: {e}");
            return 3;
        }
    };
    let geo = display.geo;
    println!(
        "fbdisplay: framebuffer {}x{} (krok {}, {})",
        geo.width,
        geo.height,
        geo.stride,
        geo.format.name()
    );
    loop {
        // Ovladac konci, az klient (init, pak kompozitor) zavre kanal.
        let items = [(client.handle(), Signals::READABLE | Signals::PEER_CLOSED)];
        let Ok((_, obs)) = libos::wait_many(&items, None) else {
            break;
        };
        if obs[0].intersects(Signals::READABLE) {
            while let Ok(m) = client.try_read() {
                let reply = handle(&mut display, &m.data, m.handles);
                if client.write(&reply.encode(), Vec::new()).is_err() {
                    break;
                }
            }
        } else if obs[0].intersects(Signals::PEER_CLOSED) {
            break;
        }
    }
    // Obrazovku vrati jadro samo pri konci procesu, a jen kdyz ji porad
    // drzime. Vyslovny `RELEASE` tu ne: jadro neoveruje, kdo obrazovku
    // drzi, a po padu kompozitoru ji uz mohla prevzit zachranna konzole
    // (smazali bychom ji obraz).
    drop(display);
    0
}

fn handle(d: &mut Display, data: &[u8], handles: Vec<libos::Handle>) -> Reply {
    let op = guiproto::window::ClientMsg::op_of(data);
    let err = |code: Error| Reply::Error { op, code };
    let req = match Request::decode(data) {
        Ok(r) => r,
        Err(e) => return err(e),
    };
    match req {
        Request::Info => Reply::Modes {
            caps: 0,
            outputs: vec![Output {
                w: d.geo.width,
                h: d.geo.height,
                enabled: true,
            }],
        },
        Request::Attach { output, w, h, stride } => {
            let Some(vmo) = handles.into_iter().next().map(Vmo::from_handle) else {
                return err(Error::InvalidArgs);
            };
            match d.attach(output, w, h, stride, vmo) {
                Ok(()) => Reply::Attached { output },
                Err(e) => err(e),
            }
        }
        // Znacka pro novou instanci kompozitoru po jeho padu (zahodi
        // odpovedi stare instance), stejne jako ovladac virtio-gpu.
        Request::Sync { token } => Reply::Synced { token },
        // Framebuffer firmwaru nema 3D (render uzel ma jen virtio-gpu).
        Request::OpenRender => err(Error::NotSupported),
        Request::Present { seq, output, rects } => {
            let rects: Vec<Rect> = if rects.is_empty() { vec![Rect::new(0, 0, u32::MAX, u32::MAX)] } else { rects };
            match d.present(output, &rects) {
                Ok(()) => Reply::Presented {
                    seq,
                    time: libos::time::now(),
                },
                Err(e) => err(e),
            }
        }
    }
}
