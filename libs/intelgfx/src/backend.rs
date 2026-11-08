//! Backend kostry ovladace (`displaydrv::Backend`) pro Intel.
//!
//! **Bezpecne prevzeti**: dokud kompozitor nepripoji snimek, ovladac na
//! registry obrazu nesaha. Prevezme jen vystup, ktery zapnul firmware,
//! a jen kdyz ma linearni XRGB8888 rovinu (jinak chyba -> init pouzije
//! framebuffer firmwaru). Casovani, PLL ani linku nemeni:
//!
//! - **page flip**: buffer 0 = framebuffer firmwaru, buffer 1 = vlastni
//!   (ovladac ho namapuje do GGTT); prepnuti = zapis `PLANE_SURF`, ktery
//!   hardware prevezme ve vsync; dokonceni = `PLANE_SURFLIVE`.
//! - **zmena rozliseni**: mensi rezimy z EDID se zobrazi pres scaler pipe
//!   (velikost zdroje `PIPESRC` + okno scaleru se zachovanim pomeru stran),
//!   casovani monitoru zustava nativni. Jina obnovovaci frekvence nebo
//!   vetsi rozliseni nez od firmwaru by vyzadovaly nastaveni PLL a novou
//!   linku - to tato verze nedela.
//!
//! Zaznam firmwaru ([`Intel::snapshot`]) obsahuje vse, co ovladac meni;
//! [`Intel::restore`] ho zapise zpet (adresa povrchu posledni, tim se
//! zmena projevi celym snimkem).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use display::dp::auxch;
use display::edid::Edid;
use display::modes::{Limits, modes};
use displaydrv::snapshot::{Snapshot, driver};
use displaydrv::{Backend, Scanout};
use fbcopy::{Format, Surface};
use guiproto::Error;
use guiproto::display::caps;
use guiproto::modes::{Connector, Mode, kind, mode_flags};

use crate::dpaux::IntelAux;
use crate::platform::Platform;
use crate::regs::{self, Trans};
use crate::state::{self, Active, Output};
use crate::{Clock, Mmio, gmbus, hotplug};

/// Buffery obrazu (pripravi ovladac v OS: framebuffer firmwaru a vlastni).
#[derive(Debug, Clone, Copy)]
pub struct Buffers {
    /// Adresa v GGTT.
    pub ggtt: [u32; 2],
    /// Mapovani pro CPU.
    pub cpu: [*mut u8; 2],
    /// 1 = jen framebuffer firmwaru (bez flipu), 2 = flip.
    pub count: usize,
}

/// Vystup, ktery zapnul firmware.
#[derive(Debug, Clone)]
pub struct Head {
    pub active: Active,
    pub kind: u8,
    pub monitor: String,
    pub size_mm: (u32, u32),
    /// Nativni rozmery (aktivni oblast casovani).
    pub native: (u32, u32),
    pub refresh_mhz: u32,
    /// Mensi rezimy z EDID (pres scaler).
    pub scaled: Vec<(u32, u32)>,
    pub edid: Option<Edid>,
}

/// Najde vystup od firmwaru a precte EDID monitoru.
pub fn probe<M: Mmio + ?Sized, C: Clock + ?Sized>(m: &mut M, c: &mut C, p: &Platform) -> Result<Head, Error> {
    let list = state::read(m, p);
    let a = list.into_iter().find(|a| a.plane.enabled()).ok_or(Error::NotFound)?;
    if !a.plane.is_linear_xrgb() {
        return Err(Error::NotSupported);
    }
    let native = (a.timing.hactive as u32, a.timing.vactive as u32);
    let ddi = a.ddi;
    let kind = match ddi.map(|d| d.output) {
        Some(Output::Hdmi) => kind::HDMI,
        Some(Output::Dvi) => kind::DVI,
        Some(Output::Dp) => kind::DISPLAYPORT,
        Some(Output::Edp) => kind::EDP,
        None => kind::FIRMWARE,
    };
    // EDID: DP pres AUX (kanal = port), HDMI/DVI pres GMBUS.
    let raw = match ddi {
        Some(d) if matches!(d.output, Output::Dp | Output::Edp) && p.has_legacy_aux() => {
            let mut aux = IntelAux { mmio: &mut *m, clock: &mut *c, ch: d.port };
            auxch::read_edid(&mut aux).ok()
        }
        Some(d) if matches!(d.output, Output::Hdmi | Output::Dvi) => {
            gmbus::pin_for_port(p.pch, d.port).and_then(|pin| display::edid::read(|i, b| gmbus::read_edid_block(&mut *m, &mut *c, pin, i, b)).ok())
        }
        _ => None,
    };
    let edid = raw.and_then(|r| Edid::parse(&r).ok());
    let mut refresh_mhz = measure_refresh(m, c, a.pipe);
    let mut scaled = Vec::new();
    let mut monitor = String::new();
    let mut size_mm = (0, 0);
    if let Some(e) = &edid {
        monitor = e.display_name();
        let s = e.size_mm();
        size_mm = (s.0 as u32, s.1 as u32);
        for md in modes(e, &Limits::new(u32::MAX, native.0 as u16, native.1 as u16)) {
            let t = md.timing;
            let wh = (t.hactive as u32, t.vactive as u32);
            if wh != native && !scaled.contains(&wh) {
                scaled.push(wh);
            }
            if wh == native && refresh_mhz == 0 {
                refresh_mhz = t.refresh_mhz();
            }
        }
    }
    if refresh_mhz == 0 {
        refresh_mhz = 60_000;
    }
    Ok(Head {
        active: a,
        kind,
        monitor,
        size_mm,
        native,
        refresh_mhz,
        scaled,
        edid,
    })
}

/// Obnova podle citace snimku za 100 ms (0 = citac stoji).
pub fn measure_refresh<M: Mmio + ?Sized, C: Clock + ?Sized>(m: &mut M, c: &mut C, pipe: regs::Pipe) -> u32 {
    let reg = Trans::ALL[pipe as usize].pipe_offset() + regs::FRMCOUNT;
    let t0 = c.now();
    let f0 = m.read32(reg);
    c.delay_us(100_000);
    let f1 = m.read32(reg);
    let dt = c.now().saturating_sub(t0);
    let frames = f1.wrapping_sub(f0) as u64;
    if frames == 0 || dt == 0 {
        return 0;
    }
    (frames * 1_000_000_000_000 / dt) as u32
}

pub struct Intel<M: Mmio, C: Clock> {
    pub mmio: M,
    pub clock: C,
    pub platform: Platform,
    pub head: Head,
    pub buffers: Buffers,
    /// Aktualni velikost zdroje.
    cur: (u32, u32),
    /// Po padu: jen rezim firmwaru.
    safe: bool,
    /// Ceka se na prevzeti adresy povrchu: adresa, cas, citac snimku.
    flip: Option<(u32, u64, u32)>,
    connected: bool,
    /// Zaznam firmwaru ulozen (driv nesmi zadny zapis).
    saved: bool,
}

/// Nejdel cekani na prevzeti flipu (pak se povazuje za hotovy).
const FLIP_TIMEOUT_NS: u64 = 100_000_000;

impl<M: Mmio, C: Clock> Intel<M, C> {
    pub fn new(mut mmio: M, clock: C, platform: Platform, head: Head, buffers: Buffers, safe: bool) -> Self {
        let port = head.active.ddi.map(|d| d.port);
        let connected = port.is_none_or(|p| hotplug::connected(&mut mmio, &platform, p));
        let cur = head.active.src;
        Intel {
            mmio,
            clock,
            platform,
            head,
            buffers,
            cur,
            safe,
            flip: None,
            connected,
            saved: false,
        }
    }

    fn pipe(&self) -> regs::Pipe {
        self.head.active.pipe
    }

    fn frmcount(&self) -> u32 {
        Trans::ALL[self.pipe() as usize].pipe_offset() + regs::FRMCOUNT
    }

    fn pipesrc(&self) -> u32 {
        Trans::ALL[self.pipe() as usize].offset() + regs::PIPESRC
    }

    /// Registry, ktere ovladac meni, v poradi obnovy (adresa povrchu posledni).
    pub fn snapshot(&mut self) -> Snapshot {
        let pipe = self.pipe();
        let mut list = vec![
            regs::scaler(pipe, 0, regs::PS_WIN_POS),
            regs::scaler(pipe, 0, regs::PS_WIN_SZ),
            regs::scaler(pipe, 0, regs::PS_CTRL),
            self.pipesrc(),
        ];
        for r in [regs::PLANE_CTL, regs::PLANE_STRIDE, regs::PLANE_POS, regs::PLANE_SIZE, regs::PLANE_OFFSET, regs::PLANE_SURF] {
            list.push(regs::plane(pipe, 0, r));
        }
        let regs_v = list.into_iter().map(|r| (r, self.mmio.read32(r))).collect();
        Snapshot {
            driver: driver::INTEL,
            w: self.head.native.0,
            h: self.head.native.1,
            stride: self.head.active.plane.stride_bytes(),
            fb_offset: self.head.active.plane.ggtt() as u64,
            regs: regs_v,
        }
    }

    pub fn mark_saved(&mut self) {
        self.saved = true;
    }

    /// Zapise zaznam firmwaru zpet (obnova po padu, cisty konec).
    pub fn restore(&mut self, s: &Snapshot) {
        for &(r, v) in &s.regs {
            self.mmio.write32(r, v);
        }
        self.cur = (s.w, s.h);
        self.flip = None;
    }

    /// Okno scaleru: rozmery `w` x `h` na nativni plose se zachovanim
    /// pomeru stran, na stred.
    pub fn window(native: (u32, u32), w: u32, h: u32) -> (u32, u32, u32, u32) {
        let (nw, nh) = native;
        let (ww, wh) = if (w as u64) * (nh as u64) >= (h as u64) * (nw as u64) { (nw, (h as u64 * nw as u64 / w as u64) as u32) } else { ((w as u64 * nh as u64 / h as u64) as u32, nh) };
        ((nw - ww) / 2, (nh - wh) / 2, ww, wh)
    }

    fn mode_list(&self) -> Vec<(u32, u32)> {
        let mut v = vec![self.head.native];
        if !self.safe {
            v.extend(self.head.scaled.iter().copied());
        }
        v
    }
}

impl<M: Mmio, C: Clock> Backend for Intel<M, C> {
    fn caps(&self) -> u32 {
        if self.safe { caps::VSYNC } else { caps::MODESET | caps::VSYNC | caps::HOTPLUG }
    }

    fn connectors(&mut self) -> Vec<Connector> {
        let r = self.head.refresh_mhz;
        let modes = self
            .mode_list()
            .into_iter()
            .enumerate()
            .map(|(i, (w, h))| {
                let mut f = if i == 0 { mode_flags::PREFERRED } else { 0 };
                if (w, h) == self.cur {
                    f |= mode_flags::CURRENT;
                }
                Mode { w, h, refresh_mhz: r, clock_khz: self.head.active.timing.clock_khz, flags: f }
            })
            .collect();
        vec![Connector {
            output: 0,
            kind: self.head.kind,
            connected: self.connected,
            monitor: self.head.monitor.clone(),
            size_mm: self.head.size_mm,
            modes,
        }]
    }

    fn size(&self, output: u32) -> Option<(u32, u32)> {
        (output == 0).then_some(self.cur)
    }

    fn set_mode(&mut self, output: u32, w: u32, h: u32, _refresh_mhz: u32) -> Result<(u32, u32, u32), Error> {
        if output != 0 {
            return Err(Error::NotFound);
        }
        if !self.mode_list().contains(&(w, h)) {
            return Err(Error::NotSupported);
        }
        if !self.saved {
            return Err(Error::BadState);
        }
        let pipe = self.pipe();
        let (nw, nh) = self.head.native;
        let ps = |r| regs::scaler(pipe, 0, r);
        if (w, h) == (nw, nh) {
            self.mmio.write32(ps(regs::PS_CTRL), 0);
        } else {
            let (x, y, ww, wh) = Self::window((nw, nh), w, h);
            self.mmio.write32(ps(regs::PS_CTRL), regs::PS_SCALER_EN | regs::PS_FILTER_MEDIUM);
            self.mmio.write32(ps(regs::PS_WIN_POS), x << 16 | y);
            // Zapis velikosti okna scaler aktivuje (dvojity buffer ve vsync).
            self.mmio.write32(ps(regs::PS_WIN_SZ), ww << 16 | wh);
        }
        let src = self.pipesrc();
        self.mmio.write32(src, (w - 1) << 16 | (h - 1));
        self.mmio.write32(regs::plane(pipe, 0, regs::PLANE_POS), 0);
        self.mmio.write32(regs::plane(pipe, 0, regs::PLANE_OFFSET), 0);
        self.mmio.write32(regs::plane(pipe, 0, regs::PLANE_SIZE), (h - 1) << 16 | (w - 1));
        // Adresa povrchu (beze zmeny) zmeny roviny ozbroji.
        let surf = self.mmio.read32(regs::plane(pipe, 0, regs::PLANE_SURF));
        self.mmio.write32(regs::plane(pipe, 0, regs::PLANE_SURF), surf);
        self.cur = (w, h);
        Ok((w, h, self.head.refresh_mhz))
    }

    fn buffers(&self, _output: u32) -> usize {
        if self.safe { 1 } else { self.buffers.count.clamp(1, 2) }
    }

    fn buffer(&mut self, output: u32, index: usize) -> Option<Scanout> {
        if output != 0 || index >= self.buffers(0) {
            return None;
        }
        Some(Scanout {
            surface: Surface {
                base: self.buffers.cpu[index],
                width: self.cur.0,
                height: self.cur.1,
                stride: self.head.active.plane.stride_bytes(),
            },
            format: Format::Bgr,
        })
    }

    fn flip(&mut self, output: u32, index: usize, now: u64) -> Result<(), Error> {
        if output != 0 || index >= self.buffers(0) {
            return Err(Error::InvalidArgs);
        }
        if !self.saved {
            return Err(Error::BadState);
        }
        let addr = self.buffers.ggtt[index];
        let reg = regs::plane(self.pipe(), 0, regs::PLANE_SURF);
        let old = self.mmio.read32(reg);
        let frm = self.mmio.read32(self.frmcount());
        self.mmio.write32(reg, (old & !regs::PLANE_SURF_ADDR_MASK) | addr);
        self.flip = Some((addr, now, frm));
        Ok(())
    }

    fn flip_done(&mut self, _output: u32, now: u64) -> Option<u64> {
        let (addr, start, frm) = self.flip?;
        let live = self.mmio.read32(regs::plane(self.pipe(), 0, regs::PLANE_SURFLIVE)) & regs::PLANE_SURF_ADDR_MASK;
        // Prevzato ve vsync: adresa sedi a probehl aspon jeden vsync (adresa
        // muze byt stejna jako pred flipem, kdyz se vraci na buffer 0).
        let vsync = self.mmio.read32(self.frmcount()) != frm;
        if (live == addr && vsync) || now.saturating_sub(start) > FLIP_TIMEOUT_NS {
            self.flip = None;
            return Some(now);
        }
        None
    }

    fn poll_hotplug(&mut self, _now: u64) -> Vec<u32> {
        let Some(port) = self.head.active.ddi.map(|d| d.port) else { return Vec::new() };
        let c = hotplug::connected(&mut self.mmio, &self.platform, port);
        if c != self.connected {
            self.connected = c;
            return vec![0];
        }
        Vec::new()
    }
}
