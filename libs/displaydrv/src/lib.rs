//! Kostra nativniho ovladace displeje (bochs-display, intel-display):
//! protokol `guiproto::display` nad [`Backend`] vyrobce. Bez zavislosti
//! na jadre OS, testuje se na hostiteli (`cargo test -p displaydrv`).
//!
//! - [`Server`]: obsluha pozadavku kompozitoru, dvojity buffer (page
//!   flip ve vsync) s poskozenymi obdelniky, udalosti `HOTPLUG`/`RESET`.
//! - [`snapshot`]: zaznam registru firmwaru (GOP) pro bezpecny navrat
//!   po chybe nebo padu ovladace.
//!
//! **Dvojity buffer**: kompozitor kresli do sve VMO (stin). Pri `PRESENT`
//! se poskozene obdelniky zkopiruji do zadniho bufferu a ten se ve vsync
//! prepne na obrazovku. Zadni buffer je o jeden snimek pozadu, proto se do
//! nej kopiruje i poskozeni predchoziho snimku (`prev`). Po `ATTACH` nebo
//! zmene rezimu se oba buffery kopiruji cele. `PRESENTED` prijde az po
//! dokonceni prepnuti (cas vsync od backendu).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod snapshot;
#[cfg(feature = "os")]
pub mod runtime;

#[cfg(test)]
mod tests;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use fbcopy::{Format, Surface};
use guiproto::display::{Output, Reply, Request, caps, op};
use guiproto::modes::Connector;
use guiproto::{Error, Rect};

pub type Result<T> = core::result::Result<T, Error>;

/// Stin kompozitoru (jeho VMO namapovana jen pro cteni).
pub trait Shadow {
    /// Plocha stinu (B8G8R8A8). Volajici zarucuje platnost po celou dobu
    /// zivota objektu.
    fn surface(&self) -> Surface;
}

/// Buffer obrazovky (pamet zarizeni) a jeho format.
#[derive(Debug, Clone, Copy)]
pub struct Scanout {
    pub surface: Surface,
    pub format: Format,
}

/// Vyrobce displeje: co umi hardware.
pub trait Backend {
    /// Schopnosti (`caps::MODESET`, `HOTPLUG`, `VSYNC`, `MULTI`).
    fn caps(&self) -> u32;
    /// Konektory s monitory a rezimy (aktualni rezim s `CURRENT`).
    fn connectors(&mut self) -> Vec<Connector>;
    /// Rozmery vystupu, pokud je zapnuty.
    fn size(&self, output: u32) -> Option<(u32, u32)>;
    /// Nastavi rezim; vraci skutecne `(w, h, obnova mHz)`. Pri chybe musi
    /// vystup zustat v puvodnim stavu (backend vraci predchozi nastaveni).
    fn set_mode(&mut self, output: u32, w: u32, h: u32, refresh_mhz: u32) -> Result<(u32, u32, u32)>;
    /// Pocet bufferu vystupu (1 = kopie primo do zobrazovaneho, 2 = flip).
    fn buffers(&self, output: u32) -> usize;
    /// Buffer `index` vystupu.
    fn buffer(&mut self, output: u32, index: usize) -> Option<Scanout>;
    /// Zobrazit buffer `index` od pristiho vsync.
    fn flip(&mut self, output: u32, index: usize, now: u64) -> Result<()>;
    /// Cas vsync, ve kterem se posledni `flip` projevil (`None` = jeste ne).
    fn flip_done(&mut self, output: u32, now: u64) -> Option<u64>;
    /// Konektory, ktere se zmenily od posledniho volani (hotplug).
    fn poll_hotplug(&mut self, _now: u64) -> Vec<u32> {
        Vec::new()
    }
}

/// Stav jednoho vystupu.
#[derive(Default)]
struct Out {
    shadow: Option<Box<dyn Shadow>>,
    /// Zobrazeny buffer.
    front: usize,
    /// Poskozeni posledniho snimku (zadni buffer ho jeste nema).
    prev: Vec<Rect>,
    /// Kolik bufferu jeste potrebuje celou kopii.
    full: usize,
    /// Snimek ceka na vsync.
    inflight: Option<u64>,
    /// Dalsi snimek prisel driv, nez dobehl predchozi.
    queued: Option<(u64, Vec<Rect>)>,
}

pub struct Server<B: Backend> {
    pub backend: B,
    outs: Vec<Out>,
    /// Udalosti k odeslani (RESET po startu nove instance).
    events: Vec<Reply>,
}

impl<B: Backend> Server<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            outs: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Nova instance po padu predchozi: kompozitor musi znovu pripojit snimek.
    pub fn announce_reset(&mut self) {
        self.events.push(Reply::Reset);
    }

    fn out(&mut self, output: u32) -> &mut Out {
        let i = output as usize;
        if self.outs.len() <= i {
            self.outs.resize_with(i + 1, Out::default);
        }
        &mut self.outs[i]
    }

    /// Obslouzi pozadavek; `shadow` = VMO z `ATTACH`. Vraci odpovedi (u
    /// `PRESENT` prijde `PRESENTED` az z [`Server::poll`]).
    pub fn handle(&mut self, req: Request, shadow: Option<Box<dyn Shadow>>, now: u64) -> Vec<Reply> {
        let code = req_op(&req);
        let err = |code_e: Error| vec![Reply::Error { op: code, code: code_e }];
        match req {
            Request::Info => {
                let cs = self.backend.connectors();
                let outputs = cs
                    .iter()
                    .map(|c| {
                        let s = self.backend.size(c.output);
                        Output {
                            w: s.map_or(0, |s| s.0),
                            h: s.map_or(0, |s| s.1),
                            enabled: s.is_some(),
                        }
                    })
                    .collect();
                vec![Reply::Modes {
                    caps: self.backend.caps() & !caps::VIRGL,
                    outputs,
                }]
            }
            Request::Sync { token } => vec![Reply::Synced { token }],
            Request::Connectors => vec![Reply::Connectors {
                connectors: self.backend.connectors(),
            }],
            Request::SetMode { output, w, h, refresh_mhz } => {
                // Rozpracovany snimek dokoncit (kompozitor ho jinak ceka).
                let mut out = self.finish_inflight(output, now);
                match self.backend.set_mode(output, w, h, refresh_mhz) {
                    Ok((w, h, hz)) => {
                        let o = self.out(output);
                        o.shadow = None;
                        o.queued = None;
                        o.prev.clear();
                        o.front = 0;
                        out.push(Reply::ModeSet { output, w, h, refresh_mhz: hz });
                    }
                    Err(e) => out.extend(err(e)),
                }
                out
            }
            Request::Attach { output, w, h, stride } => {
                let Some(shadow) = shadow else { return err(Error::InvalidArgs) };
                match self.backend.size(output) {
                    None => return err(Error::NotFound),
                    Some(s) if s != (w, h) => return err(Error::NotSupported),
                    _ => {}
                }
                let s = shadow.surface();
                if s.width != w || s.height != h || s.stride != stride || !s.is_valid() {
                    return err(Error::InvalidArgs);
                }
                let mut out = self.finish_inflight(output, now);
                let n = self.backend.buffers(output).max(1);
                let o = self.out(output);
                o.shadow = Some(shadow);
                o.full = n;
                o.prev.clear();
                o.queued = None;
                // Prvni snimek hned (bez `PRESENTED`, kompozitor ho necte).
                match self.present(output, &[Rect::new(0, 0, w, h)], now) {
                    Ok(()) => {
                        self.out(output).inflight = None;
                        out.push(Reply::Attached { output });
                        out
                    }
                    Err(e) => {
                        self.out(output).shadow = None;
                        out.extend(err(e));
                        out
                    }
                }
            }
            Request::Present { seq, output, rects } => {
                let rects = if rects.is_empty() { vec![Rect::new(0, 0, u32::MAX, u32::MAX)] } else { rects };
                if self.out(output).shadow.is_none() {
                    return err(Error::BadState);
                }
                if self.out(output).inflight.is_some() {
                    // Slit s cekajicim (kompozitor ma normalne jen jeden snimek na ceste).
                    let o = self.out(output);
                    match o.queued.as_mut() {
                        Some((s, r)) => {
                            *s = seq;
                            r.extend(rects);
                        }
                        None => o.queued = Some((seq, rects)),
                    }
                    return Vec::new();
                }
                match self.present(output, &rects, now) {
                    Ok(()) => {
                        self.out(output).inflight = Some(seq);
                        Vec::new()
                    }
                    Err(e) => err(e),
                }
            }
            #[allow(unreachable_patterns)]
            _ => err(Error::NotSupported),
        }
    }

    /// Kopie do zadniho bufferu a flip.
    fn present(&mut self, output: u32, rects: &[Rect], now: u64) -> Result<()> {
        let (w, h) = self.backend.size(output).ok_or(Error::NotFound)?;
        let n = self.backend.buffers(output).max(1);
        let screen = Rect::new(0, 0, w, h);
        let o = &mut self.outs[output as usize];
        let back = if n >= 2 { 1 - o.front.min(1) } else { 0 };
        let mut damage = guiproto::display::normalize_rects(rects, screen);
        let copy: Vec<Rect> = if o.full > 0 {
            o.full -= 1;
            vec![screen]
        } else if n >= 2 {
            let mut all = damage.clone();
            all.extend(o.prev.iter().copied());
            guiproto::display::normalize_rects(&all, screen)
        } else {
            damage.clone()
        };
        let src = o.shadow.as_ref().ok_or(Error::BadState)?.surface();
        let dst = self.backend.buffer(output, back).ok_or(Error::BadState)?;
        for r in &copy {
            // SAFETY: stin je platny po dobu zivota `Shadow` (overeno pri
            // ATTACH), buffer obrazovky po dobu rezimu (backend); obdelnik
            // se orizne na obe plochy.
            unsafe { fbcopy::copy_rect(&src, &dst.surface, fbcopy::Rect::new(r.x, r.y, r.w, r.h), dst.format) };
        }
        // Buffery jsou mapovane write-combining: vyprazdnit zapisy driv,
        // nez flip ukaze buffer zarizeni.
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.backend.flip(output, back, now)?;
        let o = &mut self.outs[output as usize];
        o.front = back;
        o.prev = core::mem::take(&mut damage);
        Ok(())
    }

    /// Dokonci snimek na ceste (pred zmenou rezimu): `PRESENTED` hned.
    fn finish_inflight(&mut self, output: u32, now: u64) -> Vec<Reply> {
        let o = self.out(output);
        o.queued = None;
        match o.inflight.take() {
            Some(seq) => vec![Reply::Presented { seq, time: now }],
            None => Vec::new(),
        }
    }

    /// Dokoncene flipy (`PRESENTED`), cekajici snimky, hotplug a udalosti.
    pub fn poll(&mut self, now: u64) -> Vec<Reply> {
        let mut out = core::mem::take(&mut self.events);
        for i in 0..self.outs.len() {
            let output = i as u32;
            if self.outs[i].inflight.is_none() {
                continue;
            }
            let Some(t) = self.backend.flip_done(output, now) else { continue };
            let seq = self.outs[i].inflight.take().unwrap_or(0);
            out.push(Reply::Presented { seq, time: t });
            if let Some((qseq, rects)) = self.outs[i].queued.take() {
                match self.present(output, &rects, now) {
                    Ok(()) => self.outs[i].inflight = Some(qseq),
                    Err(e) => out.push(Reply::Error { op: op::PRESENT, code: e }),
                }
            }
        }
        for c in self.backend.poll_hotplug(now) {
            out.push(Reply::Hotplug { output: c });
        }
        out
    }

    /// Ceka nejaky vystup na vsync (ovladac pak budi casteji)?
    pub fn busy(&self) -> bool {
        self.outs.iter().any(|o| o.inflight.is_some()) || !self.events.is_empty()
    }
}

fn req_op(r: &Request) -> u32 {
    match r {
        Request::Info => op::INFO,
        Request::Attach { .. } => op::ATTACH,
        Request::Present { .. } => op::PRESENT,
        Request::Sync { .. } => op::SYNC,
        Request::Connectors => op::CONNECTORS,
        Request::SetMode { .. } => op::SET_MODE,
        // Bez GPU: chyba musi nest kod pozadavku, jinak ji klient
        // (kompozitor u `backend gpu`) nesparuje a ceka do vyprseni.
        Request::OpenRender => op::OPEN_RENDER,
    }
}

/// Simulovane svisle zatemneni pro zarizeni bez preruseni vsync
/// (bochs-display): perioda podle obnovy, faze od nastaveni rezimu.
#[derive(Debug, Clone, Copy)]
pub struct VblankClock {
    pub start: u64,
    pub period_ns: u64,
}

impl VblankClock {
    pub fn new(start: u64, refresh_mhz: u32) -> Self {
        let r = refresh_mhz.max(1000) as u64;
        Self {
            start,
            period_ns: 1_000_000_000_000 / r,
        }
    }

    /// Prvni vsync po case `t` (vcetne).
    pub fn next(&self, t: u64) -> u64 {
        if t <= self.start {
            return self.start;
        }
        let k = (t - self.start).div_ceil(self.period_ns);
        self.start + k * self.period_ns
    }
}

/// Konfigurace od spravce zarizeni nebo initu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub restore: bool,
}

/// `pci <b> <d> <f> [restore]`.
pub fn parse_config(text: &str) -> Option<Config> {
    let mut w = text.split_whitespace();
    if w.next()? != "pci" {
        return None;
    }
    let mut n = || w.next()?.parse::<u8>().ok();
    let (bus, device, function) = (n()?, n()?, n()?);
    if device > 31 || function > 7 {
        return None;
    }
    let restore = match w.next() {
        None => false,
        Some("restore") => true,
        Some(_) => return None,
    };
    Some(Config { bus, device, function, restore })
}
