//! Buffery s popisem `bufdesc` (video bez kopie do aplikace) a explicitni
//! synchronizace commitu (ploty ziskani a uvolneni).
//!
//! - `BUFFER_CREATE_DESC`: popis se overi (`BufDesc::validate`) a VMO
//!   klienta se namapuje jen pro cteni presne v potrebne velikosti (mensi
//!   VMO = chyba mapovani v jadre). BGRA (`ARGB8888`, `XRGB8888`, `LINEAR`,
//!   prvni rovina na offsetu 0, krok delitelny 4) jde do kompozitoru primo
//!   jako dosud. NV12 a I420 dostanou stinovy buffer BGRA (vlastni VMO
//!   kompozitoru, dve mapovani: zapis pro prevod, cteni pro `comp`); prevod
//!   (`libs/yuv`, SIMD, BT.709 omezeny rozsah) probehne pri commitu, ktery
//!   buffer pripojil nebo poskodil. Zdroj YUV se pak uz necte.
//! - `SYNC_POINTS` plati pro pristi `COMMIT` okna: s plotem ziskani se
//!   commit (a vsechny dalsi zpravy klienta za nim) podrzi, dokud plot
//!   nedosahne hodnoty, nejdele [`ACQUIRE_TIMEOUT`]. Chyba plotu nebo
//!   vyprseni: commit se pouzije stejne (nejhorsi vysledek je nedokresleny
//!   snimek vlastniho okna), klient dostane `ERROR` s kodem plotu.
//! - Plot uvolneni se posune, az kompozitor buffery pripojene commitem
//!   prestane cist: YUV hned po prevodu, BGRA po `RELEASED`.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::time::Duration;

use bufdesc::{BufDesc, format, modifier};
use comp::Pixels;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::fence::Fence;
use libos::{Error, Handle, Mapping, Result, Vmo};

use crate::peers::VmoPixels;

/// Jak dlouho drzet commit kvuli plotu ziskani. Stejne jako termin tridy
/// render (`renderproto::HANG_TIMEOUT`, 10 s): producentem je obvykle GPU
/// nebo dekoder a pomalejsi zarizeni by se nevyplatilo prehlasit za
/// zaseknute driv nez ovladac. Ostatni okna mezitim bezi dal.
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

/// Buffer YUV klienta a jeho stin BGRA.
pub struct YuvBuffer {
    desc: BufDesc,
    src: Mapping,
    _src_vmo: Vmo,
    shadow: Mapping,
}

impl YuvBuffer {
    /// Prevede aktualni obsah zdroje do stinu.
    pub fn convert(&self) {
        let (w, h) = (self.desc.width as usize, self.desc.height as usize);
        // SAFETY: zdroj je mapovani `min_size` bajtu (jen cteni), stin
        // `w * h` pixelu (zapis); obe ziji s `self`. Aplikace muze zdroj
        // menit behem prevodu: nejhorsi vysledek je roztrzeny snimek jejiho
        // okna (prevod cte jen v mezich, ktere overil `validate`).
        let (src, out) = unsafe {
            (
                core::slice::from_raw_parts(self.src.addr() as *const u8, self.src.len() as usize),
                core::slice::from_raw_parts_mut(self.shadow.addr() as *mut u32, w * h),
            )
        };
        let p = &self.desc.planes;
        let plane = |i: usize| &src[p[i].offset as usize..];
        let r = match self.desc.fourcc {
            format::NV12 => yuv::nv12_to_bgra(plane(0), p[0].pitch as usize, plane(1), p[1].pitch as usize, w, h, out, w),
            _ => yuv::i420_to_bgra(
                plane(0),
                p[0].pitch as usize,
                plane(1),
                p[1].pitch as usize,
                plane(2),
                p[2].pitch as usize,
                w,
                h,
                out,
                w,
            ),
        };
        // `validate` zarucuje meze; chyba by byla chyba kompozitoru.
        debug_assert!(r.is_ok());
    }
}

/// Vysledek `BUFFER_CREATE_DESC`: co dostane kompozitor (`comp`) jako
/// `BUFFER_CREATE` a pripadny buffer YUV k prevodu.
pub struct Created {
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub px: Box<dyn Pixels>,
    pub yuv: Option<YuvBuffer>,
}

/// Overi popis a namapuje VMO klienta.
pub fn create(desc: BufDesc, vmo: Vmo) -> Result<Created> {
    let need = desc.min_size().map_err(|_| Error::NotSupported)?;
    desc.validate(need).map_err(|_| Error::InvalidArgs)?;
    if desc.modifier != modifier::LINEAR {
        return Err(Error::NotSupported);
    }
    let (w, h) = (desc.width, desc.height);
    match desc.fourcc {
        format::ARGB8888 | format::XRGB8888 => {
            let p = desc.planes[0];
            if p.offset != 0 || p.pitch % 4 != 0 {
                return Err(Error::NotSupported);
            }
            let stride = p.pitch / 4;
            if !comp::scene::buffer_ok(w, h, stride) {
                return Err(Error::InvalidArgs);
            }
            let px = VmoPixels::map(vmo, stride as u64 * h as u64)?;
            Ok(Created { w, h, stride, px: Box::new(px), yuv: None })
        }
        format::NV12 | format::YUV420 => {
            if !comp::scene::buffer_ok(w, h, w) {
                return Err(Error::InvalidArgs);
            }
            let src = Mapping::new(&vmo, 0, need, MAP_READ)?;
            let shadow_vmo = Vmo::create(w as u64 * h as u64 * 4)?;
            let shadow = Mapping::new(&shadow_vmo, 0, w as u64 * h as u64 * 4, MAP_READ | MAP_WRITE)?;
            let px = VmoPixels::map(shadow_vmo, w as u64 * h as u64)?;
            Ok(Created { w, h, stride: w, px: Box::new(px), yuv: Some(YuvBuffer { desc, src, _src_vmo: vmo, shadow }) })
        }
        _ => Err(Error::NotSupported),
    }
}

/// Bod uvolneni: plot a hodnota, buffery BGRA, ktere jeste cte kompozitor.
struct Release {
    fence: Fence,
    value: u64,
    pending: Vec<u32>,
}

/// Commit podrzeny kvuli plotu ziskani.
pub struct Held {
    pub data: Vec<u8>,
    pub handles: Vec<Handle>,
    fence: Fence,
    value: u64,
    deadline: u64,
}

/// Jak dopadlo cekani na plot ziskani.
pub enum Acquire {
    Wait,
    Ready,
    /// Plot skoncil chybou, byl opusten, nebo vyprsel termin.
    Failed(Error),
}

/// Stav synchronizace jednoho klienta.
#[derive(Default)]
pub struct ClientSync {
    /// Buffery YUV klienta.
    pub yuv: BTreeMap<u32, YuvBuffer>,
    /// `SYNC_POINTS` pro pristi commit (okno, ziskani, uvolneni).
    staged: Option<(u32, Option<(Fence, u64)>, Option<(Fence, u64)>)>,
    /// Buffery pripojene (`LAYER_SET` s bufferem) nebo poskozene od
    /// posledniho commitu.
    touched: Vec<u32>,
    releases: Vec<Release>,
    pub held: Option<Held>,
}

impl ClientSync {
    /// `SYNC_POINTS`: handly ziskani a uvolneni podle priznaku.
    pub fn stage(&mut self, window: u32, acquire: Option<u64>, release: Option<u64>, handles: Vec<Handle>) -> Result<()> {
        let expected = acquire.is_some() as usize + release.is_some() as usize;
        if handles.len() != expected {
            return Err(Error::InvalidArgs);
        }
        let mut it = handles.into_iter().map(Fence::from_handle);
        let a = acquire.map(|v| (it.next().unwrap(), v));
        let r = release.map(|v| (it.next().unwrap(), v));
        self.staged = Some((window, a, r));
        Ok(())
    }

    pub fn touch(&mut self, buffer: u32) {
        if !self.touched.contains(&buffer) {
            self.touched.push(buffer);
        }
    }

    /// Plot ziskani pro commit okna `window`, nebo `None` (commit hned).
    pub fn acquire_for(&mut self, window: u32) -> Option<(Fence, u64)> {
        match &mut self.staged {
            Some((w, a, _)) if *w == window => a.take(),
            _ => None,
        }
    }

    /// Podrzi commit do splneni plotu (nebo terminu).
    pub fn hold(&mut self, data: Vec<u8>, handles: Vec<Handle>, fence: Fence, value: u64, now: u64) {
        let deadline = now.saturating_add(ACQUIRE_TIMEOUT.as_nanos() as u64);
        self.held = Some(Held { data, handles, fence, value, deadline });
    }

    /// Stav podrzeneho commitu.
    pub fn poll(&self, now: u64) -> Acquire {
        let Some(h) = &self.held else { return Acquire::Ready };
        match h.fence.query() {
            Ok(i) if i.value >= h.value => Acquire::Ready,
            Ok(i) if i.status != 0 => Acquire::Failed(Error::from_code(i.status as i64).unwrap_or(Error::FenceFailed)),
            Ok(_) if now >= h.deadline => Acquire::Failed(Error::TimedOut),
            Ok(_) => Acquire::Wait,
            Err(e) => Acquire::Failed(e),
        }
    }

    /// Termin podrzeneho commitu.
    pub fn deadline(&self) -> Option<u64> {
        self.held.as_ref().map(|h| h.deadline)
    }

    /// Commit okna `window` jde do kompozitoru: prevest dotcene buffery YUV
    /// a zalozit bod uvolneni (buffery BGRA cekaji na `RELEASED`).
    pub fn commit(&mut self, window: u32) {
        let touched = core::mem::take(&mut self.touched);
        for b in &touched {
            if let Some(y) = self.yuv.get(b) {
                y.convert();
            }
        }
        let release = match self.staged.take() {
            Some((w, _, r)) if w == window => r,
            other => {
                // Body jineho okna plati dal (commit tohoto okna je nespotreboval).
                self.staged = other;
                None
            }
        };
        if let Some((fence, value)) = release {
            let pending: Vec<u32> = touched.into_iter().filter(|b| !self.yuv.contains_key(b)).collect();
            if pending.is_empty() {
                let _ = fence.signal(value);
            } else {
                self.releases.push(Release { fence, value, pending });
            }
        }
    }

    /// Kompozitor pustil buffer (`RELEASED`): posunout splnene body.
    pub fn released(&mut self, buffer: u32) {
        self.releases.retain_mut(|r| {
            r.pending.retain(|&b| b != buffer);
            if r.pending.is_empty() {
                let _ = r.fence.signal(r.value);
                false
            } else {
                true
            }
        });
    }

    /// Buffer zrusen: uz se necte.
    pub fn destroyed(&mut self, buffer: u32) {
        self.yuv.remove(&buffer);
        self.released(buffer);
    }
}
