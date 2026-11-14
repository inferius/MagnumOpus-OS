//! Protokol okna: aplikace <-> kompozitor.
//!
//! Stav okna je dvoji: rozpracovany (zpravy od posledniho `COMMIT`)
//! a platny (ten se sklada). `COMMIT` prevede rozpracovany stav na platny
//! atomicky. Handly (VMO bufferu) jdou v handlech zpravy kanalu.

use alloc::string::String;
use alloc::vec::Vec;

use crate::a11y::Node;
use crate::action::{self, Arg};
use crate::{Error, MAX_MESSAGE, ReadExt, Reader, Rect, Result, WriteExt, Writer, read_count};

/// Kody operaci.
pub mod op {
    // Aplikace -> kompozitor.
    pub const HELLO: u32 = 1;
    pub const WINDOW_CREATE: u32 = 2;
    pub const WINDOW_SET: u32 = 3;
    pub const WINDOW_DESTROY: u32 = 4;
    pub const BUFFER_CREATE: u32 = 5;
    pub const BUFFER_DESTROY: u32 = 6;
    pub const LAYER_CREATE: u32 = 7;
    pub const LAYER_SET: u32 = 8;
    pub const LAYER_DESTROY: u32 = 9;
    pub const DAMAGE: u32 = 10;
    pub const ANIMATE: u32 = 11;
    pub const A11Y_UPDATE: u32 = 12;
    pub const A11Y_REMOVE: u32 = 13;
    pub const REGIONS: u32 = 14;
    pub const COMMIT: u32 = 15;
    pub const SYNC_GROUP: u32 = 16;
    pub const PONG: u32 = 17;
    pub const ACTION_RESULT: u32 = 18;
    /// Deklarace menu okna (`crate::menu`).
    pub const MENU_UPDATE: u32 = 19;
    /// Obalka zpravy schranky (`clipproto::ClientMsg`).
    pub const CLIPBOARD: u32 = 20;
    /// Pozadovana velikost okna (`WindowSize`).
    pub const WINDOW_SIZE: u32 = 21;
    /// Buffer s popisem `bufdesc` (BGRA, NV12, I420; `BufferCreateDesc`).
    pub const BUFFER_CREATE_DESC: u32 = 22;
    /// Ploty pristiho `COMMIT` okna (`SyncPoints`).
    pub const SYNC_POINTS: u32 = 23;
    // Kompozitor -> aplikace.
    pub const WELCOME: u32 = 100;
    pub const CONFIGURE: u32 = 101;
    pub const FRAME: u32 = 102;
    pub const PRESENTED: u32 = 103;
    pub const RELEASED: u32 = 104;
    pub const INPUT: u32 = 105;
    pub const CLOSE: u32 = 106;
    pub const PING: u32 = 107;
    pub const SYNC_TOKEN: u32 = 108;
    pub const ANIMATION_DONE: u32 = 109;
    pub const A11Y_ACTION: u32 = 110;
    pub const ACTION_INVOKE: u32 = 111;
    pub const ERROR: u32 = 112;
    /// Rezim barev systemu (0 svetly, 1 tmavy, 2 vysoky kontrast).
    pub const THEME: u32 = 113;
    /// Obalka zpravy schranky (`clipproto::ServerMsg`).
    pub const CLIPBOARD_EVENT: u32 = 114;
    /// Kontext okna: vystup, pracovni plocha, skala (`WindowContext`).
    pub const WINDOW_CONTEXT: u32 = 115;
    /// Poloha obsahu okna na obrazovce (`WindowPlace`).
    pub const WINDOW_PLACE: u32 = 116;
}

/// Rezim okna (kdo kresli dekorace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Ram a titulek kresli system.
    Standard,
    /// Aplikace kresli i plochu titulku; odznak a tlacitka system.
    CustomTitle,
    /// Bez ramu (odznak u okraje pri najeti a pri zmene opravneni).
    Frameless,
    Fullscreen,
}

impl Mode {
    pub const fn to_u8(self) -> u8 {
        match self {
            Mode::Standard => 0,
            Mode::CustomTitle => 1,
            Mode::Frameless => 2,
            Mode::Fullscreen => 3,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Mode::Standard,
            1 => Mode::CustomTitle,
            2 => Mode::Frameless,
            3 => Mode::Fullscreen,
            _ => return Err(Error::InvalidArgs),
        })
    }

    /// Kresli system titulek s textem (ne jen odznak a tlacitka)?
    pub const fn system_title(self) -> bool {
        matches!(self, Mode::Standard)
    }

    /// Ma okno ram (titulek nebo aspon tlacitka a odznak)?
    pub const fn has_frame(self) -> bool {
        matches!(self, Mode::Standard | Mode::CustomTitle)
    }
}

/// Druh vrstvy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// Jen deti (volitelne orez a posun obsahu).
    Group,
    /// Vyrez bufferu aplikace.
    Buffer,
    /// Jednobarevny obdelnik.
    Solid,
}

impl LayerKind {
    pub const fn to_u8(self) -> u8 {
        match self {
            LayerKind::Group => 0,
            LayerKind::Buffer => 1,
            LayerKind::Solid => 2,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => LayerKind::Group,
            1 => LayerKind::Buffer,
            2 => LayerKind::Solid,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Bity masky vlastnosti v `LAYER_SET`: posila se jen to, co je v masce.
pub mod prop {
    pub const RECT: u32 = 1 << 0;
    pub const ORDER: u32 = 1 << 1;
    pub const OPACITY: u32 = 1 << 2;
    pub const VISIBLE: u32 = 1 << 3;
    pub const CLIP: u32 = 1 << 4;
    pub const SCROLL: u32 = 1 << 5;
    pub const BUFFER: u32 = 1 << 6;
    pub const COLOR: u32 = 1 << 7;
    /// Korenova vrstva jako popover: kresli se nad vsemi okny (vrstva
    /// `layer.popover`), okno ji neorezava, vstup v jeji plose jde oknu.
    pub const POPOVER: u32 = 1 << 8;
    pub const ALL: u32 = (1 << 9) - 1;
}

/// Vlastnosti vrstvy. V `LAYER_SET` plati jen pole z `mask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerProps {
    pub mask: u32,
    /// Obdelnik v souradnicich rodice.
    pub rect: Rect,
    /// Poradi mezi sourozenci (vetsi = nahore).
    pub order: i32,
    /// 0 = pruhledna, 255 = nepruhledna.
    pub opacity: u8,
    pub visible: bool,
    /// Orezat deti na obdelnik vrstvy.
    pub clip: bool,
    /// Posun obsahu (deti) - scroll.
    pub scroll: (i32, i32),
    /// Buffer a zdrojovy vyrez (vrstva `Buffer`).
    pub buffer: u32,
    pub src: Rect,
    /// Barva 0xAARRGGBB (premultiplied) pro vrstvu `Solid`.
    pub color: u32,
    /// Popover (jen korenova vrstva okna), viz `prop::POPOVER`.
    pub popover: bool,
}

impl Default for LayerProps {
    fn default() -> Self {
        Self {
            mask: 0,
            rect: Rect::default(),
            order: 0,
            opacity: 255,
            visible: true,
            clip: false,
            scroll: (0, 0),
            buffer: 0,
            src: Rect::default(),
            color: 0,
            popover: false,
        }
    }
}

impl LayerProps {
    /// Vlastnosti z `o`, ktere jsou v jeho masce, prepisou tyto.
    pub fn apply(&mut self, o: &LayerProps) {
        let m = o.mask;
        if m & prop::RECT != 0 {
            self.rect = o.rect;
        }
        if m & prop::ORDER != 0 {
            self.order = o.order;
        }
        if m & prop::OPACITY != 0 {
            self.opacity = o.opacity;
        }
        if m & prop::VISIBLE != 0 {
            self.visible = o.visible;
        }
        if m & prop::CLIP != 0 {
            self.clip = o.clip;
        }
        if m & prop::SCROLL != 0 {
            self.scroll = o.scroll;
        }
        if m & prop::BUFFER != 0 {
            self.buffer = o.buffer;
            self.src = o.src;
        }
        if m & prop::COLOR != 0 {
            self.color = o.color;
        }
        if m & prop::POPOVER != 0 {
            self.popover = o.popover;
        }
        self.mask |= m;
    }

    fn encode(&self, w: &mut Writer) {
        let m = self.mask & prop::ALL;
        w.u32(m);
        if m & prop::RECT != 0 {
            w.rect(&self.rect);
        }
        if m & prop::ORDER != 0 {
            w.i32(self.order);
        }
        if m & prop::OPACITY != 0 {
            w.u8(self.opacity);
        }
        if m & prop::VISIBLE != 0 {
            w.bool(self.visible);
        }
        if m & prop::CLIP != 0 {
            w.bool(self.clip);
        }
        if m & prop::SCROLL != 0 {
            w.i32(self.scroll.0).i32(self.scroll.1);
        }
        if m & prop::BUFFER != 0 {
            w.u32(self.buffer).rect(&self.src);
        }
        if m & prop::COLOR != 0 {
            w.u32(self.color);
        }
        if m & prop::POPOVER != 0 {
            w.bool(self.popover);
        }
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self> {
        let m = r.u32()?;
        if m & !prop::ALL != 0 {
            return Err(Error::InvalidArgs);
        }
        let mut p = LayerProps {
            mask: m,
            ..Default::default()
        };
        if m & prop::RECT != 0 {
            p.rect = r.rect()?;
        }
        if m & prop::ORDER != 0 {
            p.order = r.i32()?;
        }
        if m & prop::OPACITY != 0 {
            p.opacity = r.u8()?;
        }
        if m & prop::VISIBLE != 0 {
            p.visible = r.bool()?;
        }
        if m & prop::CLIP != 0 {
            p.clip = r.bool()?;
        }
        if m & prop::SCROLL != 0 {
            p.scroll = (r.i32()?, r.i32()?);
        }
        if m & prop::BUFFER != 0 {
            p.buffer = r.u32()?;
            p.src = r.rect()?;
        }
        if m & prop::COLOR != 0 {
            p.color = r.u32()?;
        }
        if m & prop::POPOVER != 0 {
            p.popover = r.bool()?;
        }
        Ok(p)
    }
}

/// Animovana vlastnost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimProp {
    Opacity,
    X,
    Y,
    ScrollX,
    ScrollY,
}

impl AnimProp {
    const fn to_u8(self) -> u8 {
        match self {
            AnimProp::Opacity => 0,
            AnimProp::X => 1,
            AnimProp::Y => 2,
            AnimProp::ScrollX => 3,
            AnimProp::ScrollY => 4,
        }
    }

    fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => AnimProp::Opacity,
            1 => AnimProp::X,
            2 => AnimProp::Y,
            3 => AnimProp::ScrollX,
            4 => AnimProp::ScrollY,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Krivka animace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Linear,
    EaseOut,
}

impl Curve {
    /// Prubeh 0..=1000 pro cas `t` 0..=1000 (celociselne, bez FPU).
    pub fn eval(self, t: u32) -> u32 {
        let t = t.min(1000) as u64;
        match self {
            Curve::Linear => t as u32,
            // 1 - (1 - t)^3
            Curve::EaseOut => {
                let u = 1000 - t;
                (1000 - u * u * u / 1_000_000) as u32
            }
        }
    }
}

/// Druh deklarovane oblasti.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Tazenim se presouva okno.
    Drag,
    /// Vyjimka uvnitr `Drag` (karta, tlacitko aplikace).
    NoDrag,
    /// Okraj pro zmenu velikosti.
    Resize,
}

impl RegionKind {
    const fn to_u8(self) -> u8 {
        match self {
            RegionKind::Drag => 0,
            RegionKind::NoDrag => 1,
            RegionKind::Resize => 2,
        }
    }

    fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => RegionKind::Drag,
            1 => RegionKind::NoDrag,
            2 => RegionKind::Resize,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Strana tlacitek okna (vlastni titulek).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonSide {
    Right,
    Left,
}

/// Priznaky `COMMIT`.
pub mod commit {
    /// Aplikace chce `FRAME` na zacatku pristi periody.
    pub const FRAME: u32 = 1 << 0;
    /// Pole `serial` potvrzuje `CONFIGURE` (synchronizovana zmena velikosti).
    pub const ACK_CONFIGURE: u32 = 1 << 1;
    /// Commit je soucast skupiny `token` (transakce pres vic aplikaci).
    pub const GROUP: u32 = 1 << 2;
}

/// Stav okna v `CONFIGURE`.
pub mod wstate {
    pub const FOCUSED: u32 = 1 << 0;
    pub const MAXIMIZED: u32 = 1 << 1;
    pub const FULLSCREEN: u32 = 1 << 2;
    /// Kompozitor okno ztlumil (aplikace nereaguje nebo je nad nim dialog).
    pub const DIMMED: u32 = 1 << 3;
}

/// Udalost vstupu v okne (souradnice v okne, tj. v obsahu bez ramu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Fyzicka klavesa (kod evdev nezavisly na rozlozeni), akce
    /// 0 uvolneni / 1 stisk / 2 opakovani, znak podle rozlozeni.
    Key { code: u16, action: u8, ch: Option<char> },
    /// Vlozeny text.
    Text(char),
    /// Ukazatel v okne.
    PointerMove { x: i32, y: i32 },
    /// Ukazatel opustil okno.
    PointerLeave,
    /// Tlacitko mysi (kod evdev) v bode.
    Button { code: u16, pressed: bool, x: i32, y: i32 },
    /// Kolecko (kroky, `dy` kladne nahoru).
    Scroll { dx: i32, dy: i32 },
    /// Okno ziskalo / ztratilo fokus klavesnice.
    Focus(bool),
}

/// Zaznam vstupu: udalost, modifikatory (`inputproto::mods`), priznak
/// umeleho vstupu a cas (ns od startu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputRecord {
    pub event: InputEvent,
    pub mods: u16,
    /// Ne z mistniho fyzickeho zarizeni (`class != Local`).
    pub synthetic: bool,
    pub time: u64,
    /// Trida zdroje: 0 mistni fyzicky, 1 umely, 2 vzdaleny
    /// (`inputproto::event::InputClass`).
    pub class: u8,
    /// Seriove cislo gesta: kompozitor ho prideli kazdemu stisku klavesy
    /// a tlacitka z mistniho fyzickeho vstupu, ktery doruci oknu (jako
    /// serial ve Waylandu); jinak 0. Aplikace ho posle se zadosti, kterou
    /// stisk vyvolal (schranka), viz `clipproto`.
    pub serial: u32,
}

/// Kody tridy zdroje v `InputRecord::class`.
pub mod input_class {
    pub const LOCAL: u8 = 0;
    pub const SYNTHETIC: u8 = 1;
    pub const REMOTE: u8 = 2;
}

impl InputRecord {
    pub const SIZE: usize = 32;

    fn encode(&self, w: &mut Writer) {
        let (k, a, b, c): (u8, u32, u32, u32) = match self.event {
            InputEvent::Key { code, action, ch } => (1, code as u32, action as u32, ch.map_or(0, |c| c as u32)),
            InputEvent::Text(ch) => (2, ch as u32, 0, 0),
            InputEvent::PointerMove { x, y } => (3, x as u32, y as u32, 0),
            InputEvent::PointerLeave => (4, 0, 0, 0),
            InputEvent::Button { code, pressed, x, y } => (5, code as u32 | (pressed as u32) << 16, x as u32, y as u32),
            InputEvent::Scroll { dx, dy } => (6, dx as u32, dy as u32, 0),
            InputEvent::Focus(f) => (7, f as u32, 0, 0),
        };
        // Trida ve stejnem bajtu jako driv priznak umeleho vstupu.
        let class = if self.class != input_class::LOCAL { self.class } else { self.synthetic as u8 };
        w.u8(k).u8(class).u16(self.mods).u64(self.time).u32(a).u32(b).u32(c).u32(self.serial);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self> {
        let k = r.u8()?;
        let class = r.u8()?;
        if class > input_class::REMOTE {
            return Err(Error::InvalidArgs);
        }
        let synthetic = class != input_class::LOCAL;
        let mods = r.u16()?;
        let time = r.u64()?;
        let (a, b, c) = (r.u32()?, r.u32()?, r.u32()?);
        let serial = r.u32()?;
        let ch = |v: u32| char::from_u32(v).ok_or(Error::InvalidArgs);
        let event = match k {
            1 => InputEvent::Key {
                code: u16::try_from(a).map_err(|_| Error::InvalidArgs)?,
                action: u8::try_from(b).map_err(|_| Error::InvalidArgs)?,
                ch: if c == 0 { None } else { Some(ch(c)?) },
            },
            2 => InputEvent::Text(ch(a)?),
            3 => InputEvent::PointerMove { x: a as i32, y: b as i32 },
            4 => InputEvent::PointerLeave,
            5 => InputEvent::Button {
                code: a as u16,
                pressed: a >> 16 & 1 != 0,
                x: b as i32,
                y: c as i32,
            },
            6 => InputEvent::Scroll { dx: a as i32, dy: b as i32 },
            7 => InputEvent::Focus(a != 0),
            _ => return Err(Error::InvalidArgs),
        };
        Ok(Self {
            event,
            mods,
            synthetic,
            time,
            class,
            serial,
        })
    }
}

/// Zprava aplikace -> kompozitor.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientMsg {
    Hello { version: u32 },
    WindowCreate { window: u32, mode: Mode, w: u32, h: u32, title: String },
    WindowSet {
        window: u32,
        title: String,
        min_w: u32,
        min_h: u32,
        side: ButtonSide,
        /// Barva tlacitek okna (vlastni titulek), 0 = vychozi.
        button_color: u32,
    },
    WindowDestroy { window: u32 },
    /// Handle VMO jde v handlech zpravy.
    BufferCreate { buffer: u32, w: u32, h: u32, stride: u32 },
    BufferDestroy { buffer: u32 },
    LayerCreate { layer: u32, window: u32, parent: u32, kind: LayerKind },
    LayerSet { layer: u32, props: LayerProps },
    LayerDestroy { layer: u32 },
    Damage { buffer: u32, rect: Rect },
    Animate { layer: u32, prop: AnimProp, target: i32, ms: u32, curve: Curve },
    A11yUpdate { window: u32, nodes: Vec<Node> },
    A11yRemove { window: u32, ids: Vec<u32> },
    Regions { window: u32, regions: Vec<(Rect, RegionKind)> },
    Commit { txn: u32, flags: u32, window: u32, serial: u32, token: u128 },
    SyncGroup { count: u32 },
    Pong { serial: u32 },
    ActionResult { call: u32, ok: bool, value: Option<action::Value>, message: String },
    /// Menu okna (cely strom; posila se pri zmene, kompozitor ho drzi).
    MenuUpdate { window: u32, items: Vec<crate::menu::MenuItem> },
    /// Zprava schranky (`clipproto`): obalka, aby sla schranka pozdeji
    /// prestehovat do vlastni sluzby bez zmeny jejich zprav.
    Clipboard { data: Vec<u8> },
    /// Pozadovana velikost okna (logicke px): prirozena `w` x `h` (0 = nechat
    /// na systemu), nejmensi `min_w` x `min_h` a strop v promile pracovni
    /// plochy (0 = bez stropu). Pred prvnim commitem urci velikost pri
    /// otevreni (bez probliknuti), pozdeji je to zadost o zmenu; kompozitor
    /// odpovi `CONFIGURE` (maximalizovanemu oknu ji zapamatuje na obnoveni).
    WindowSize { window: u32, w: u32, h: u32, min_w: u32, min_h: u32, max_frac_w: u16, max_frac_h: u16 },
    /// Buffer popsany `bufdesc` (handle VMO v handlech zpravy): BGRA
    /// (`ARGB8888`, `XRGB8888`), NV12 a I420 (`YU12`) s modifikatorem
    /// `LINEAR`. Kompozitor popis overi proti velikosti VMO; YUV prevede pri
    /// commitu na BGRA (BT.709, omezeny rozsah).
    BufferCreateDesc { buffer: u32, desc: bufdesc::BufDesc },
    /// Ploty pristiho `COMMIT` okna `window` (explicitni synchronizace).
    /// Handly zpravy: plot ziskani (pravo `WAIT`), je-li `acquire`, pak
    /// plot uvolneni (pravo `SIGNAL`), je-li `release`.
    /// - ziskani: kompozitor commit pouzije, az plot dosahne hodnoty
    ///   (producent dopsal buffery); do te doby ukazuje predchozi stav
    ///   a dalsi zpravy aplikace cekaji za commitem,
    /// - uvolneni: kompozitor plot posune, az buffery pripojene timto
    ///   commitem prestane cist (YUV hned po prevodu, BGRA po `RELEASED`).
    SyncPoints { window: u32, acquire: Option<u64>, release: Option<u64> },
}

/// Zprava kompozitor -> aplikace.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    /// Verze, rozmery obrazovky, skala v procentech, identita aplikace
    /// (jak ji vidi system).
    Welcome { version: u32, w: u32, h: u32, scale: u32, app: String },
    Configure { window: u32, w: u32, h: u32, state: u32, serial: u32 },
    Frame { seq: u64, deadline: u64, period: u64 },
    Presented { txn: u32, time: u64 },
    Released { buffer: u32 },
    Input { window: u32, records: Vec<InputRecord> },
    Close { window: u32 },
    Ping { serial: u32 },
    SyncToken { token: u128 },
    AnimationDone { layer: u32 },
    A11yAction { window: u32, node: u32, action: u32, value: String },
    ActionInvoke { call: u32, action: String, args: Vec<Arg>, caller: String, synthetic: bool },
    Error { op: u32, code: Error },
    /// Rezim barev systemu: aplikace kresli stejnym motivem (tokeny
    /// `design/tokens.toml`). Prijde po `WELCOME` a pri kazde zmene.
    Theme { mode: u8 },
    /// Zprava schranky (`clipproto::ServerMsg`).
    Clipboard { data: Vec<u8> },
    /// Kontext okna (pro kazde okno zvlast): vystup, na kterem lezi, cela
    /// obrazovka a pracovni plocha (bez listy) v logickych px, skala
    /// vystupu v procentech a obnovovaci frekvence. Prijde pred prvnim
    /// `CONFIGURE` a pri kazde zmene; `generation` roste s kazdou zmenou.
    WindowContext { window: u32, generation: u32, output: u32, screen: Rect, work_area: Rect, scale: u32, refresh_mhz: u32, ext: crate::context::ContextExt },
    /// Obdelnik obsahu okna na obrazovce (po vytvoreni a po kazdem presunu
    /// nebo zmene velikosti, behem tahu az na konci): aplikace podle nej
    /// umisti popover do pracovni plochy.
    WindowPlace { window: u32, rect: Rect },
}

/// Data obalky schranky: delka a bajty (zkracena zprava = chyba).
fn clip_r(r: &mut Reader<'_>) -> Result<Vec<u8>> {
    let n = r.u32()? as usize;
    Ok(r.take(n)?.to_vec())
}

fn u128w(w: &mut Writer, v: u128) {
    w.u64(v as u64).u64((v >> 64) as u64);
}

fn u128r(r: &mut Reader<'_>) -> Result<u128> {
    Ok(r.u64()? as u128 | (r.u64()? as u128) << 64)
}

impl ClientMsg {
    pub fn encode(&self) -> Vec<u8> {
        use ClientMsg::*;
        let mut w;
        match self {
            Hello { version } => {
                w = Writer::op(op::HELLO);
                w.u32(*version);
            }
            WindowCreate { window, mode, w: ww, h, title } => {
                w = Writer::op(op::WINDOW_CREATE);
                w.u32(*window).u8(mode.to_u8()).u32(*ww).u32(*h).str(title);
            }
            WindowSet {
                window,
                title,
                min_w,
                min_h,
                side,
                button_color,
            } => {
                w = Writer::op(op::WINDOW_SET);
                w.u32(*window)
                    .str(title)
                    .u32(*min_w)
                    .u32(*min_h)
                    .u8(matches!(side, ButtonSide::Left) as u8)
                    .u32(*button_color);
            }
            WindowDestroy { window } => {
                w = Writer::op(op::WINDOW_DESTROY);
                w.u32(*window);
            }
            BufferCreate { buffer, w: bw, h, stride } => {
                w = Writer::op(op::BUFFER_CREATE);
                w.u32(*buffer).u32(*bw).u32(*h).u32(*stride);
            }
            BufferDestroy { buffer } => {
                w = Writer::op(op::BUFFER_DESTROY);
                w.u32(*buffer);
            }
            LayerCreate { layer, window, parent, kind } => {
                w = Writer::op(op::LAYER_CREATE);
                w.u32(*layer).u32(*window).u32(*parent).u8(kind.to_u8());
            }
            LayerSet { layer, props } => {
                w = Writer::op(op::LAYER_SET);
                w.u32(*layer);
                props.encode(&mut w);
            }
            LayerDestroy { layer } => {
                w = Writer::op(op::LAYER_DESTROY);
                w.u32(*layer);
            }
            Damage { buffer, rect } => {
                w = Writer::op(op::DAMAGE);
                w.u32(*buffer).rect(rect);
            }
            Animate { layer, prop, target, ms, curve } => {
                w = Writer::op(op::ANIMATE);
                w.u32(*layer)
                    .u8(prop.to_u8())
                    .i32(*target)
                    .u32(*ms)
                    .u8(matches!(curve, Curve::EaseOut) as u8);
            }
            A11yUpdate { window, nodes } => {
                w = Writer::op(op::A11Y_UPDATE);
                w.u32(*window).u32(nodes.len() as u32);
                for n in nodes {
                    n.encode(&mut w);
                }
            }
            A11yRemove { window, ids } => {
                w = Writer::op(op::A11Y_REMOVE);
                w.u32(*window).u32(ids.len() as u32);
                for id in ids {
                    w.u32(*id);
                }
            }
            Regions { window, regions } => {
                w = Writer::op(op::REGIONS);
                w.u32(*window).u32(regions.len() as u32);
                for (r, k) in regions {
                    w.rect(r).u8(k.to_u8());
                }
            }
            Commit {
                txn,
                flags,
                window,
                serial,
                token,
            } => {
                w = Writer::op(op::COMMIT);
                w.u32(*txn).u32(*flags).u32(*window).u32(*serial);
                u128w(&mut w, *token);
            }
            SyncGroup { count } => {
                w = Writer::op(op::SYNC_GROUP);
                w.u32(*count);
            }
            Pong { serial } => {
                w = Writer::op(op::PONG);
                w.u32(*serial);
            }
            ActionResult { call, ok, value, message } => {
                w = Writer::op(op::ACTION_RESULT);
                w.u32(*call).bool(*ok).str(message).bool(value.is_some());
                if let Some(v) = value {
                    v.encode(&mut w);
                }
            }
            MenuUpdate { window, items } => {
                w = Writer::op(op::MENU_UPDATE);
                w.u32(*window);
                crate::menu::encode_menu(&mut w, items);
            }
            Clipboard { data } => {
                w = Writer::op(op::CLIPBOARD);
                w.u32(data.len() as u32).bytes(data);
            }
            WindowSize { window, w: sw, h, min_w, min_h, max_frac_w, max_frac_h } => {
                w = Writer::op(op::WINDOW_SIZE);
                w.u32(*window).u32(*sw).u32(*h).u32(*min_w).u32(*min_h).u16(*max_frac_w).u16(*max_frac_h);
            }
            BufferCreateDesc { buffer, desc } => {
                w = Writer::op(op::BUFFER_CREATE_DESC);
                w.u32(*buffer);
                desc.write(&mut w);
            }
            SyncPoints { window, acquire, release } => {
                w = Writer::op(op::SYNC_POINTS);
                w.u32(*window);
                for p in [acquire, release] {
                    match p {
                        Some(v) => w.u8(1).u64(*v),
                        None => w.u8(0).u64(0),
                    };
                }
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        use ClientMsg::*;
        let len = data.len();
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::HELLO => Hello { version: r.u32()? },
            op::WINDOW_CREATE => WindowCreate {
                window: r.u32()?,
                mode: Mode::from_u8(r.u8()?)?,
                w: r.u32()?,
                h: r.u32()?,
                title: r.str()?,
            },
            op::WINDOW_SET => WindowSet {
                window: r.u32()?,
                title: r.str()?,
                min_w: r.u32()?,
                min_h: r.u32()?,
                side: if r.bool()? { ButtonSide::Left } else { ButtonSide::Right },
                button_color: r.u32()?,
            },
            op::WINDOW_DESTROY => WindowDestroy { window: r.u32()? },
            op::BUFFER_CREATE => BufferCreate {
                buffer: r.u32()?,
                w: r.u32()?,
                h: r.u32()?,
                stride: r.u32()?,
            },
            op::BUFFER_DESTROY => BufferDestroy { buffer: r.u32()? },
            op::LAYER_CREATE => LayerCreate {
                layer: r.u32()?,
                window: r.u32()?,
                parent: r.u32()?,
                kind: LayerKind::from_u8(r.u8()?)?,
            },
            op::LAYER_SET => LayerSet {
                layer: r.u32()?,
                props: LayerProps::decode(&mut r)?,
            },
            op::LAYER_DESTROY => LayerDestroy { layer: r.u32()? },
            op::DAMAGE => Damage {
                buffer: r.u32()?,
                rect: r.rect()?,
            },
            op::ANIMATE => Animate {
                layer: r.u32()?,
                prop: AnimProp::from_u8(r.u8()?)?,
                target: r.i32()?,
                ms: r.u32()?,
                curve: if r.bool()? { Curve::EaseOut } else { Curve::Linear },
            },
            op::A11Y_UPDATE => {
                let window = r.u32()?;
                let n = read_count(&mut r, Node::MIN_SIZE, len)?;
                let mut nodes = Vec::with_capacity(n);
                for _ in 0..n {
                    nodes.push(Node::decode(&mut r)?);
                }
                A11yUpdate { window, nodes }
            }
            op::A11Y_REMOVE => {
                let window = r.u32()?;
                let n = read_count(&mut r, 4, len)?;
                let mut ids = Vec::with_capacity(n);
                for _ in 0..n {
                    ids.push(r.u32()?);
                }
                A11yRemove { window, ids }
            }
            op::REGIONS => {
                let window = r.u32()?;
                let n = read_count(&mut r, 17, len)?;
                let mut regions = Vec::with_capacity(n);
                for _ in 0..n {
                    let rect = r.rect()?;
                    regions.push((rect, RegionKind::from_u8(r.u8()?)?));
                }
                Regions { window, regions }
            }
            op::COMMIT => Commit {
                txn: r.u32()?,
                flags: r.u32()?,
                window: r.u32()?,
                serial: r.u32()?,
                token: u128r(&mut r)?,
            },
            op::SYNC_GROUP => SyncGroup { count: r.u32()? },
            op::PONG => Pong { serial: r.u32()? },
            op::ACTION_RESULT => {
                let call = r.u32()?;
                let ok = r.bool()?;
                let message = r.str()?;
                let value = if r.bool()? { Some(action::Value::decode(&mut r)?) } else { None };
                ActionResult { call, ok, value, message }
            }
            op::MENU_UPDATE => MenuUpdate { window: r.u32()?, items: crate::menu::decode_menu(&mut r, len)? },
            op::CLIPBOARD => Clipboard { data: clip_r(&mut r)? },
            op::WINDOW_SIZE => WindowSize {
                window: r.u32()?,
                w: r.u32()?,
                h: r.u32()?,
                min_w: r.u32()?,
                min_h: r.u32()?,
                max_frac_w: r.u16()?,
                max_frac_h: r.u16()?,
            },
            op::BUFFER_CREATE_DESC => BufferCreateDesc {
                buffer: r.u32()?,
                desc: bufdesc::BufDesc::read(&mut r).map_err(|_| Error::InvalidArgs)?,
            },
            op::SYNC_POINTS => {
                let window = r.u32()?;
                let mut point = || -> Result<Option<u64>> {
                    let present = r.u8()?;
                    let v = r.u64()?;
                    match present {
                        0 if v == 0 => Ok(None),
                        1 => Ok(Some(v)),
                        _ => Err(Error::InvalidArgs),
                    }
                };
                let acquire = point()?;
                let release = point()?;
                SyncPoints { window, acquire, release }
            }
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }

    /// Ma zprava cekat na `COMMIT` (meni platny stav okna)? Kompozitor ji
    /// do te doby jen odlozi; zaznam stavu okna ([`crate::replay`]) taky.
    pub fn is_staged(&self) -> bool {
        matches!(
            self,
            ClientMsg::WindowSet { .. }
                | ClientMsg::LayerCreate { .. }
                | ClientMsg::LayerSet { .. }
                | ClientMsg::LayerDestroy { .. }
                | ClientMsg::Damage { .. }
                | ClientMsg::Animate { .. }
                | ClientMsg::A11yUpdate { .. }
                | ClientMsg::A11yRemove { .. }
                | ClientMsg::Regions { .. }
        )
    }

    /// Kod operace zpravy (pro odpoved `ERROR`).
    pub fn op_of(data: &[u8]) -> u32 {
        data.get(..4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    }
}

impl ServerMsg {
    pub fn encode(&self) -> Vec<u8> {
        use ServerMsg::*;
        let mut w;
        match self {
            Welcome { version, w: sw, h, scale, app } => {
                w = Writer::op(op::WELCOME);
                w.u32(*version).u32(*sw).u32(*h).u32(*scale).str(app);
            }
            Configure {
                window,
                w: cw,
                h,
                state,
                serial,
            } => {
                w = Writer::op(op::CONFIGURE);
                w.u32(*window).u32(*cw).u32(*h).u32(*state).u32(*serial);
            }
            Frame { seq, deadline, period } => {
                w = Writer::op(op::FRAME);
                w.u64(*seq).u64(*deadline).u64(*period);
            }
            Presented { txn, time } => {
                w = Writer::op(op::PRESENTED);
                w.u32(*txn).u64(*time);
            }
            Released { buffer } => {
                w = Writer::op(op::RELEASED);
                w.u32(*buffer);
            }
            Input { window, records } => {
                w = Writer::op(op::INPUT);
                w.u32(*window).u32(records.len() as u32);
                for rec in records {
                    rec.encode(&mut w);
                }
            }
            Close { window } => {
                w = Writer::op(op::CLOSE);
                w.u32(*window);
            }
            Ping { serial } => {
                w = Writer::op(op::PING);
                w.u32(*serial);
            }
            SyncToken { token } => {
                w = Writer::op(op::SYNC_TOKEN);
                u128w(&mut w, *token);
            }
            AnimationDone { layer } => {
                w = Writer::op(op::ANIMATION_DONE);
                w.u32(*layer);
            }
            A11yAction {
                window,
                node,
                action,
                value,
            } => {
                w = Writer::op(op::A11Y_ACTION);
                w.u32(*window).u32(*node).u32(*action).str(value);
            }
            ActionInvoke {
                call,
                action,
                args,
                caller,
                synthetic,
            } => {
                w = Writer::op(op::ACTION_INVOKE);
                w.u32(*call).str(action).str(caller).bool(*synthetic);
                action::encode_args(&mut w, args);
            }
            Error { op: o, code } => {
                w = Writer::op(op::ERROR);
                w.u32(*o).i32(code.code());
            }
            Theme { mode } => {
                w = Writer::op(op::THEME);
                w.u8(*mode);
            }
            Clipboard { data } => {
                w = Writer::op(op::CLIPBOARD_EVENT);
                w.u32(data.len() as u32).bytes(data);
            }
            WindowPlace { window, rect } => {
                w = Writer::op(op::WINDOW_PLACE);
                w.u32(*window).rect(rect);
            }
            WindowContext { window, generation, output, screen, work_area, scale, refresh_mhz, ext } => {
                w = Writer::op(op::WINDOW_CONTEXT);
                w.u32(*window).u32(*generation).u32(*output).rect(screen).rect(work_area).u32(*scale).u32(*refresh_mhz);
                ext.encode(&mut w);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        use ServerMsg::*;
        let len = data.len();
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::WELCOME => Welcome {
                version: r.u32()?,
                w: r.u32()?,
                h: r.u32()?,
                scale: r.u32()?,
                app: r.str()?,
            },
            op::CONFIGURE => Configure {
                window: r.u32()?,
                w: r.u32()?,
                h: r.u32()?,
                state: r.u32()?,
                serial: r.u32()?,
            },
            op::FRAME => Frame {
                seq: r.u64()?,
                deadline: r.u64()?,
                period: r.u64()?,
            },
            op::PRESENTED => Presented {
                txn: r.u32()?,
                time: r.u64()?,
            },
            op::RELEASED => Released { buffer: r.u32()? },
            op::INPUT => {
                let window = r.u32()?;
                let n = read_count(&mut r, InputRecord::SIZE, len)?;
                let mut records = Vec::with_capacity(n);
                for _ in 0..n {
                    records.push(InputRecord::decode(&mut r)?);
                }
                Input { window, records }
            }
            op::CLOSE => Close { window: r.u32()? },
            op::PING => Ping { serial: r.u32()? },
            op::SYNC_TOKEN => SyncToken { token: u128r(&mut r)? },
            op::ANIMATION_DONE => AnimationDone { layer: r.u32()? },
            op::A11Y_ACTION => A11yAction {
                window: r.u32()?,
                node: r.u32()?,
                action: r.u32()?,
                value: r.str()?,
            },
            op::ACTION_INVOKE => {
                let call = r.u32()?;
                let action = r.str()?;
                let caller = r.str()?;
                let synthetic = r.bool()?;
                let args = action::decode_args(&mut r, len)?;
                ActionInvoke {
                    call,
                    action,
                    args,
                    caller,
                    synthetic,
                }
            }
            op::ERROR => Error {
                op: r.u32()?,
                code: crate::Error::from_code(r.i32()? as i64).ok_or(crate::Error::InvalidArgs)?,
            },
            op::THEME => Theme { mode: r.u8()? },
            op::CLIPBOARD_EVENT => Clipboard { data: clip_r(&mut r)? },
            op::WINDOW_PLACE => WindowPlace { window: r.u32()?, rect: r.rect()? },
            op::WINDOW_CONTEXT => WindowContext {
                window: r.u32()?,
                generation: r.u32()?,
                output: r.u32()?,
                screen: r.rect()?,
                work_area: r.rect()?,
                scale: r.u32()?,
                refresh_mhz: r.u32()?,
                ext: crate::context::ContextExt::decode(&mut r)?,
            },
            _ => return Err(crate::Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(crate::Error::InvalidArgs);
        }
        Ok(m)
    }
}

/// Rozdeli uzly pristupnosti do zprav `A11Y_UPDATE`, ktere se vejdou do
/// limitu zpravy (jeden uzel je nejvys ~128 KiB jen teoreticky - texty se
/// zkrati na 16 KiB).
pub fn a11y_messages(window: u32, nodes: &[Node]) -> Vec<Vec<u8>> {
    a11y_chunks(window, nodes).iter().map(ClientMsg::encode).collect()
}

/// Jako [`a11y_messages`], ale zpravy nezakodovane (zaznam stavu okna).
pub fn a11y_chunks(window: u32, nodes: &[Node]) -> Vec<ClientMsg> {
    const TEXT_MAX: usize = 16 * 1024;
    let mut out = Vec::new();
    let mut batch: Vec<Node> = Vec::new();
    let mut size = 16;
    for n in nodes {
        let mut n = n.clone();
        truncate(&mut n.name, TEXT_MAX);
        truncate(&mut n.value, TEXT_MAX);
        let l = n.encoded_len();
        if size + l > MAX_MESSAGE && !batch.is_empty() {
            out.push(ClientMsg::A11yUpdate {
                window,
                nodes: core::mem::take(&mut batch),
            });
            size = 16;
        }
        size += l;
        batch.push(n);
    }
    if !batch.is_empty() {
        out.push(ClientMsg::A11yUpdate { window, nodes: batch });
    }
    out
}

fn truncate(s: &mut String, max: usize) {
    if s.len() > max {
        let mut n = max;
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        s.truncate(n);
    }
}
