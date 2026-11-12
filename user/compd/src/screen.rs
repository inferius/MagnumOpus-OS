//! Displej kompozitoru: synchronizace s ovladacem, vyber vystupu
//! (konektory a rezimy, `guiproto::modes`), snimek ve VMO, zmena rezimu
//! za behu (`mode`) a reakce na pripojeni monitoru (`HOTPLUG`).
//!
//! Vymena snimku pri zmene velikosti: nova VMO, `Comp::resize`, prvni
//! snimek slozit, `ATTACH` (ovladac ho hned ukaze), az pak zahodit starou.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use comp::Comp;
use gfx::text::Family;
use guiproto::display::{Reply, Request, caps, op};
use guiproto::modes::{self, Connector};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Error, Mapping, Result, Rights, Vmo, println};

use crate::{Service, token};

/// Jak dlouho cekat na odpoved ovladace (nastaveni rezimu a navazani
/// linky DP trva stovky ms).
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Obraz obrazovky (VMO sdilena s ovladacem displeje).
pub struct Screen {
    pub display: Channel,
    /// Vystup ovladace, na ktery kreslime.
    pub output: u32,
    pub caps: u32,
    pub w: u32,
    pub h: u32,
    pub _vmo: Vmo,
    pub map: Mapping,
    pub seq: u64,
}

impl Screen {
    pub fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: mapovani `w * h` pixelu zije se `Screen`; ovladac ho jen
        // cte (soubezne cteni muze dat roztrzeny snimek, ne chybu pameti).
        unsafe { core::slice::from_raw_parts_mut(self.map.addr() as *mut u32, (self.w * self.h) as usize) }
    }
}

/// Co prislo behem cekani na odpoved (zpracuje se potom).
#[derive(Default)]
pub struct Pending {
    pub hotplug: bool,
    /// Ovladac zacal znovu: snimek pripojit znovu.
    pub reset: bool,
    pub presented: Option<u64>,
    /// Obnovovaci frekvence vybraneho rezimu v mHz (0 = nevi): takt
    /// kompozitoru.
    pub refresh_mhz: u32,
}

/// Dalsi odpoved ovladace; udalosti a potvrzeni snimku si poznamena.
pub fn await_reply(ch: &Channel, deadline: u64, p: &mut Pending) -> Result<Reply> {
    loop {
        let r = Reply::decode(&ch.read(Some(deadline))?.data)?;
        match r {
            Reply::Hotplug { .. } => p.hotplug = true,
            Reply::Reset => p.reset = true,
            Reply::Presented { time, .. } => p.presented = Some(time),
            Reply::Error { op: op::PRESENT, .. } => p.presented = Some(libos::time::now()),
            other => return Ok(other),
        }
    }
}

/// Konektory od ovladace; ovladac bez `CONNECTORS` (starsi) -> `None`.
pub fn connectors(ch: &Channel, deadline: u64, p: &mut Pending) -> Result<Option<Vec<Connector>>> {
    ch.write(&Request::Connectors.encode(), Vec::new())?;
    match await_reply(ch, deadline, p)? {
        Reply::Connectors { connectors } => Ok(Some(connectors)),
        Reply::Error { op: op::CONNECTORS, code: Error::NotSupported } => Ok(None),
        Reply::Error { code, .. } => Err(code),
        _ => Err(Error::Corrupted),
    }
}

/// Nastavi rezim vystupu; vraci skutecne rozmery.
pub fn set_mode(ch: &Channel, output: u32, w: u32, h: u32, refresh_mhz: u32, deadline: u64, p: &mut Pending) -> Result<(u32, u32, u32)> {
    ch.write(&Request::SetMode { output, w, h, refresh_mhz }.encode(), Vec::new())?;
    match await_reply(ch, deadline, p)? {
        Reply::ModeSet { w, h, refresh_mhz, .. } => Ok((w, h, refresh_mhz)),
        Reply::Error { code, .. } => Err(code),
        _ => Err(Error::Corrupted),
    }
}

/// Vystup a rozmery pro kompozitor: hlavni pripojeny konektor s jeho
/// aktualnim rezimem; bez nastaveneho rezimu (monitor prave pripojen)
/// se nastavi preferovany.
pub fn choose(ch: &Channel, outputs: &[guiproto::display::Output], deadline: u64, p: &mut Pending) -> Result<(u32, u32, u32)> {
    let Some(cs) = connectors(ch, deadline, p)? else {
        let o = outputs.first().ok_or(Error::NotFound)?;
        return Ok((0, o.w, o.h));
    };
    let c = modes::primary(&cs).ok_or(Error::NotFound)?;
    if let Some(m) = c.current() {
        p.refresh_mhz = m.refresh_mhz;
        return Ok((c.output, m.w, m.h));
    }
    let m = c.preferred().or(c.modes.first()).ok_or(Error::NotFound)?;
    let (w, h, hz) = set_mode(ch, c.output, m.w, m.h, m.refresh_mhz, deadline, p)?;
    p.refresh_mhz = hz;
    Ok((c.output, w, h))
}

fn deadline() -> u64 {
    libos::time::deadline_after(REPLY_TIMEOUT)
}

impl Service {
    /// `display`: kanal ovladace grafiky. Synchronizace, vyber vystupu,
    /// snimek. Kompozitor uz bezi-li (novy ovladac po padu stareho, napr.
    /// navrat na framebuffer firmwaru), okna zustanou.
    pub fn attach_display(&mut self, handles: Vec<libos::Handle>) -> Result<(String, Vec<libos::Handle>)> {
        if self.screen.is_some() {
            return Err(Error::InvalidArgs);
        }
        let ch = Channel::from_handle(handles.into_iter().next().ok_or(Error::InvalidArgs)?);
        let end = deadline();
        // Konec kanalu mohla drzet predchozi instance (drzak koncu
        // supervisoru): nejdriv zahodit jeji odpovedi.
        let sync = token() as u64;
        ch.write(&Request::Sync { token: sync }.encode(), Vec::new())?;
        let mut skipped = 0;
        loop {
            match Reply::decode(&ch.read(Some(end))?.data) {
                Ok(r) if r.ends_sync(sync) => break,
                _ => skipped += 1,
            }
        }
        if skipped > 0 {
            println!("compd: displej: zahozeno {skipped} odpovedi predchozi instance");
        }
        let mut p = Pending::default();
        ch.write(&Request::Info.encode(), Vec::new())?;
        let Reply::Modes { outputs, caps } = await_reply(&ch, end, &mut p)? else {
            return Err(Error::Corrupted);
        };
        let (output, w, h) = choose(&ch, &outputs, end, &mut p)?;
        self.install(ch, caps, output, w, h, end)?;
        self.set_refresh(p.refresh_mhz);
        let virgl = if caps & caps::VIRGL != 0 { " (umi virgl)" } else { "" };
        let modeset = if caps & caps::MODESET != 0 { ", zmena rezimu" } else { "" };
        println!("compd: displej {w}x{h} na vystupu {output}{virgl}{modeset}");
        self.pending_hotplug |= p.hotplug;
        self.pending_reset |= p.reset;
        Ok((format!("{w} {h}"), Vec::new()))
    }

    /// Vytvori snimek `w` x `h` pro vystup a pripoji ho (`ATTACH`).
    fn install(&mut self, ch: Channel, caps: u32, output: u32, w: u32, h: u32, end: u64) -> Result<()> {
        if !(1..=guiproto::MAX_DIM).contains(&w) || !(1..=guiproto::MAX_DIM).contains(&h) {
            return Err(Error::NotSupported);
        }
        let size = w as u64 * h as u64 * 4;
        let vmo = Vmo::create(size)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        // Ovladac dostane snimek jen pro cteni.
        let ro = vmo.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))?;
        let mut screen = Screen {
            display: ch,
            output,
            caps,
            w,
            h,
            _vmo: vmo,
            map,
            seq: 0,
        };
        let now = libos::time::now();
        // Snimek na GPU ma pevnou velikost: zrusit, po pripojeni vytvorit znovu.
        let had_gpu = match self.gpu.take() {
            Some(g) => {
                g.destroy();
                true
            }
            None => false,
        };
        let comp = match self.comp.take() {
            Some(mut c) => {
                c.resize(w, h, now);
                c
            }
            None => Comp::new(w, h, ui_fonts(), Box::new(token)),
        };
        let mut comp = comp;
        // Pri pripojeni jen plna barva pozadi (rychly start i v emulaci;
        // hlidac sluzeb ceka na pripojeni). Cely snimek slozi dalsi takt,
        // obrazovka je cela poskozena.
        // Po startovni obrazovce je prvni snimek tentyz obraz (`splash.rs`).
        if !self.splash_first_frame(screen.pixels(), w, h) {
            let mut canvas = gfx::Canvas::new(screen.pixels(), w, h, w).ok_or(Error::NoMemory)?;
            canvas.fill(canvas.bounds(), comp.background());
        }
        comp.redraw_all();
        self.comp = Some(comp);
        let req = Request::Attach { output, w, h, stride: w * 4 };
        screen.display.write(&req.encode(), alloc::vec![ro])?;
        let mut p = Pending::default();
        let r = await_reply(&screen.display, end, &mut p)?;
        self.pending_hotplug |= p.hotplug;
        self.pending_reset |= p.reset;
        match r {
            Reply::Attached { .. } => {}
            Reply::Error { code, .. } => return Err(code),
            _ => return Err(Error::Corrupted),
        }
        self.peers.frame = Some((screen.map.addr(), w, h));
        self.screen = Some(screen);
        if had_gpu && let Err(e) = self.set_backend("gpu") {
            println!("compd: GPU backend po zmene velikosti nejde ({e}), skladani na CPU");
        }
        Ok(())
    }

    /// Takt kompozitoru podle obnovovaci frekvence vystupu.
    fn set_refresh(&mut self, mhz: u32) {
        if let Some(c) = self.comp.as_mut() {
            c.set_refresh(mhz);
        }
        if mhz > 0 {
            println!("compd: takt snimku {}.{:03} Hz", mhz / 1000, mhz % 1000);
        }
    }

    /// `mode <w> <h> [mHz]`: zmena rezimu vystupu.
    pub fn change_mode(&mut self, w: u32, h: u32, refresh_mhz: u32) -> Result<(u32, u32, u32)> {
        let sc = self.screen.as_ref().ok_or(Error::NotFound)?;
        let end = deadline();
        let mut p = Pending::default();
        let r = set_mode(&sc.display, sc.output, w, h, refresh_mhz, end, &mut p);
        self.apply_pending(p);
        let (nw, nh, hz) = r?;
        self.rebuild(nw, nh, end)?;
        self.set_refresh(hz);
        println!("compd: rezim {nw}x{nh} @ {}.{:03} Hz", hz / 1000, hz % 1000);
        Ok((nw, nh, hz))
    }

    /// Novy snimek nove velikosti pro stavajici kanal ovladace.
    fn rebuild(&mut self, w: u32, h: u32, end: u64) -> Result<()> {
        let old = self.screen.take().ok_or(Error::NotFound)?;
        self.peers.frame = None;
        let (caps, output) = (old.caps, old.output);
        // Kanal presunout, stary snimek (VMO) uvolnit az po pripojeni noveho.
        let Screen { display, _vmo, map, .. } = old;
        let r = self.install(display, caps, output, w, h, end);
        drop(map);
        drop(_vmo);
        if let Err(e) = &r {
            println!("compd: snimek {w}x{h} nelze pripojit: {e}");
        }
        r
    }

    /// `outputs`: konektory jako text (radek na konektor).
    pub fn outputs_text(&mut self) -> Result<String> {
        let sc = self.screen.as_ref().ok_or(Error::NotFound)?;
        let mut p = Pending::default();
        let cs = connectors(&sc.display, deadline(), &mut p);
        self.apply_pending(p);
        let cs = cs?.ok_or(Error::NotSupported)?;
        let mut s = String::new();
        for c in &cs {
            let cur = c.current().map(|m| format!("{}x{}@{}", m.w, m.h, m.refresh_mhz)).unwrap_or_else(|| String::from("-"));
            s.push_str(&format!("{} {} {} {} {} {}\n", c.output, modes::kind::name(c.kind), c.connected as u8, cur, c.modes.len(), c.monitor));
        }
        Ok(s)
    }

    /// Potvrzeni snimku a udalosti, ktere prisly behem synchronniho cekani.
    pub fn apply_pending(&mut self, p: Pending) {
        if let (Some(t), Some(c)) = (p.presented, self.comp.as_mut()) {
            c.presented(t);
        }
        self.pending_hotplug |= p.hotplug;
        self.pending_reset |= p.reset;
    }

    /// Po `RESET`: nova instance ovladace (po padu) nezna nas snimek -
    /// znovu vybrat vystup (muze uz umet jen rezim firmwaru) a pripojit.
    pub fn handle_reset(&mut self) {
        self.pending_reset = false;
        self.pending_hotplug = false;
        let Some(sc) = self.screen.as_ref() else { return };
        let end = deadline();
        let mut p = Pending::default();
        let r = sc.display.write(&Request::Info.encode(), Vec::new()).and_then(|_| await_reply(&sc.display, end, &mut p));
        let outputs = match r {
            Ok(Reply::Modes { outputs, caps }) => {
                if let Some(s) = self.screen.as_mut() {
                    s.caps = caps;
                }
                outputs
            }
            _ => Vec::new(),
        };
        let Some(sc) = self.screen.as_ref() else { return };
        let r = choose(&sc.display, &outputs, end, &mut p);
        self.set_refresh(p.refresh_mhz);
        self.apply_pending(Pending { hotplug: false, reset: false, ..p });
        match r {
            Ok((o, w, h)) => {
                println!("compd: ovladac displeje bezi znovu: vystup {o}, {w}x{h}");
                if let Some(s) = self.screen.as_mut() {
                    s.output = o;
                }
                let _ = self.rebuild(w, h, end);
            }
            Err(e) => println!("compd: ovladac displeje po restartu: {e}"),
        }
    }

    /// Po `HOTPLUG`: znovu vybrat vystup; jina velikost nebo vystup =
    /// novy snimek. Bez pripojeneho monitoru zustava posledni stav.
    pub fn handle_hotplug(&mut self) {
        self.pending_hotplug = false;
        let Some(sc) = self.screen.as_ref() else { return };
        let end = deadline();
        let mut p = Pending::default();
        let r = choose(&sc.display, &[], end, &mut p);
        let (cur_out, cw, chh) = (sc.output, sc.w, sc.h);
        self.set_refresh(p.refresh_mhz);
        self.apply_pending(Pending { hotplug: false, reset: p.reset, ..p });
        match r {
            Ok((o, w, h)) if (o, w, h) != (cur_out, cw, chh) => {
                println!("compd: zmena monitoru: vystup {o}, {w}x{h}");
                if let Some(s) = self.screen.as_mut() {
                    s.output = o;
                }
                let _ = self.rebuild(w, h, end);
            }
            Ok(_) => {}
            Err(Error::NotFound) => println!("compd: zadny monitor neni pripojen"),
            Err(e) => println!("compd: konektory po hotplugu: {e}"),
        }
    }
}

/// Pismo UI z tokenu: Inter (vlozene rezy) se zapnutymi vlastnostmi
/// (`font.ui_features`, napr. tabulkove cislice).
fn ui_fonts() -> Family {
    let t = theme::Theme::builtin(theme::Mode::Light);
    let feats: Vec<[u8; 4]> = t.font.ui_features.iter().filter(|f| f.1).map(|f| f.0).collect();
    Family::new(font::inter(), &feats)
}
