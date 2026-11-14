//! Behove prostredi aplikaci v OS: okno nad kanalem ke kompozitoru.
//!
//! [`Window`] je nizka vrstva (buffery ve VMO, vrstvy, transakce, udalosti),
//! kterou pouzije i aplikace kreslici sama (okno terminalu). [`run`] nad
//! ni pusti aplikaci s rozhranim z knihovny ([`crate::App`]).
//!
//! Buffery jsou dvojite: kresli se do toho, ktery kompozitor zrovna
//! nepouziva (`RELEASED`), takze aplikace na kompozitor nikdy neceka.
//!
//! **Obnova po padu kompozitoru** (`restore.rs`, protokol
//! `docs/cs/obnova-sluzeb.md`): okno mluvi s kompozitorem pres
//! `svcclient::Conn` s listkem od supervisoru ([`Link`]). Vse, co okno
//! posle, se zaznamena (`guiproto::replay`). Kdyz kanal zanikne
//! (`PEER_CLOSED`), okno pozada o novou relaci (stejna identita z
//! manifestu) a posle znovu posledni potvrzeny stav: okno, buffery (VMO
//! drzi aplikace), vrstvy, oblasti titulku a strom pristupnosti. Aplikace
//! dostane [`Event::Restored`]. Rozpracovana transakce a odpoved na akci
//! vrati `SERVICE_RESTARTED` ([`run`] pak okno nakresli znovu).

mod clip;
mod size;
mod restore;
mod video;

pub use video::VideoSurface;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::a11y::Node;
use guiproto::action::{Arg, Value};
use guiproto::replay::Replay;
use guiproto::window::{ClientMsg, InputRecord, LayerKind, LayerProps, Mode, ServerMsg, commit, prop};
use guiproto::{Rect, window};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Error, Handle, Mapping, Result, Rights, Signals, Vmo};
use svcclient::Conn;

use crate::{App, ROOT_LAYER, Ui, scroll_layers};

/// Vrstva popoveru (nabidka a bublina nad okny), korenova.
const POPOVER_LAYER: u32 = 2;

/// Vrstva kurzoru textoveho pole: blikani meni jen jeji viditelnost
/// (kompozitor poskodi jen jeji obdelnik, okno se neprekresluje).
const CARET_LAYER: u32 = 3;

/// Udalost okna pro aplikaci.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Input(InputRecord),
    /// Nova velikost (uz platna pro dalsi kresleni).
    Resized { w: u32, h: u32 },
    /// Uzivatel klikl na zavrit.
    Close,
    A11yAction { node: u32, action: u32, value: String },
    Action { call: u32, id: String, args: Vec<Arg>, caller: String, synthetic: bool },
    /// Termin dalsiho snimku (aplikace o nej pozadala).
    Frame,
    /// Kompozitor spadl a okno je zpet na jeho nove instanci (generace)
    /// s poslednim potvrzenym obsahem. Rozpracovana zmena se ztratila:
    /// aplikace, ktera kresli sama, at okno nakresli znovu.
    Restored { generation: u64 },
    /// Rezim barev systemu (prekreslit stejnym motivem).
    Theme(theme::Mode),
    /// Zprava schranky; zpracuje ji [`Window::clipboard_event`].
    Clipboard(clipproto::ServerMsg),
    /// Novy kontext okna (obrazovka, pracovni plocha, skala): uz ulozeny
    /// v [`Window::context`].
    Context,
    /// Obsah okna se posunul nebo zmenil velikost (obdelnik na obrazovce).
    Place(Rect),
    /// Rozpracovany text u kurzoru (mrtva klavesa; `ServerMsg::Compose`).
    Compose { text: String, cursor_begin: u32, cursor_end: u32, flags: u32 },
}

/// Kontext okna od kompozitoru (`WINDOW_CONTEXT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinContext {
    pub output: u32,
    pub screen: Rect,
    pub work_area: Rect,
    pub scale: u32,
    pub refresh_mhz: u32,
    pub generation: u32,
    pub ext: guiproto::context::ContextExt,
}

impl WinContext {
    fn from_msg(m: &ServerMsg) -> Option<Self> {
        match m {
            ServerMsg::WindowContext { generation, output, screen, work_area, scale, refresh_mhz, ext, .. } => {
                Some(Self { output: *output, screen: *screen, work_area: *work_area, scale: *scale, refresh_mhz: *refresh_mhz, generation: *generation, ext: ext.clone() })
            }
            _ => None,
        }
    }
}

/// Vysledek pozdravu: obrazovka, aplikace, prvni `CONFIGURE` (w, h,
/// serial) a kontext okna, pokud prisel pred nim.
struct Greeting {
    screen: (u32, u32),
    app: String,
    size: (u32, u32, u32),
    context: Option<WinContext>,
}

/// Kanal okna a listek k jeho obnove ze startovni zpravy aplikace.
///
/// Poradi handlu aplikace s oknem: 0 = kanal okna, 1 = listek, 2+ =
/// handly aplikace (napr. `gui-tasks`: kanal na `taskd`).
pub struct Link {
    pub window: Channel,
    /// `None` = bez obnovy (pad kompozitoru = konec okna).
    pub lease: Option<Channel>,
}

impl Link {
    /// Prvni dva handly startovni zpravy (okno, listek).
    pub fn from_handles(it: &mut impl Iterator<Item = Handle>) -> Option<Self> {
        let window = Channel::from_handle(it.next()?);
        let lease = it.next().map(Channel::from_handle);
        Some(Self { window, lease })
    }
}

impl From<Channel> for Link {
    fn from(window: Channel) -> Self {
        Self { window, lease: None }
    }
}

struct Buf {
    id: u32,
    w: u32,
    h: u32,
    vmo: Vmo,
    map: Mapping,
    busy: bool,
}

impl Buf {
    fn canvas(&mut self) -> Canvas<'_> {
        // SAFETY: mapovani `w * h` pixelu (cteni a zapis) zije s bufferem;
        // kompozitor ho cte jen mezi COMMIT a RELEASED (`busy`).
        let px = unsafe { core::slice::from_raw_parts_mut(self.map.addr() as *mut u32, (self.w * self.h) as usize) };
        Canvas::new(px, self.w, self.h, self.w).unwrap()
    }

    /// Handle VMO pro kompozitor (jen cteni).
    fn readonly(&self) -> Result<Handle> {
        self.vmo.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))
    }
}

/// Dvojice bufferu jedne vrstvy.
struct Pair {
    bufs: [Option<Buf>; 2],
    front: usize,
}

pub struct Window {
    conn: Conn,
    pub w: u32,
    pub h: u32,
    pub screen: (u32, u32),
    /// Jmeno aplikace, jak ho vidi system.
    pub app: String,
    next_buffer: u32,
    pairs: BTreeMap<u32, Pair>,
    /// Neprectene udalosti (prisly behem cekani na RELEASED).
    queue: Vec<Event>,
    ack: Option<u32>,
    txn: u32,
    /// Commit s FRAME odeslan, FRAME jeste neprisel.
    pub frame_pending: bool,
    /// Cas pristi vymeny z posledniho FRAME (ns, 0 = nezname).
    pub frame_deadline: u64,
    /// Titulek, ktery kompozitor zna (zmenu posle `render` s commitem).
    title: String,
    /// Posledni poslane menu okna (`MENU_UPDATE` jen pri zmene).
    pub(crate) menu: Vec<guiproto::menu::MenuItem>,
    a11y_ids: alloc::collections::BTreeSet<u32>,
    groups: BTreeMap<u32, Rect>,
    /// Platny stav okna pro obnovu po padu kompozitoru.
    replay: Replay,
    /// Data, ktera okno zkopirovalo (format, bajty; posle je na zadost).
    clip_owned: Vec<(String, Vec<u8>)>,
    clip_serial: u32,
    /// Cislo gesta posledni vstupni udalosti s fyzickym stiskem
    /// (`InputRecord::serial`): jde se zadosti o schranku.
    gesture: u32,
    /// Cislo pozadavku na vlozeni do textoveho pole a od aplikace.
    field_paste: u32,
    app_paste: u32,
    /// Formaty ve schrance (zprava `Changed`).
    clip_formats: Vec<String>,
    /// Kontext okna (posledni `WINDOW_CONTEXT`).
    pub context: Option<WinContext>,
    /// Vrstva popoveru je videt.
    popover_shown: bool,
    /// Vrstva kurzoru: nakresleny obdelnik a barva, viditelnost.
    caret: Option<(Rect, u32)>,
    caret_shown: bool,
}

/// Zapis, ktery pri plne fronte kanalu chvili pocka (kompozitor zrovna
/// nestiha cist); po 2 s to vzda.
fn write(ch: &Channel, data: &[u8], handles: Vec<Handle>) -> Result<()> {
    let end = libos::time::deadline_after(core::time::Duration::from_secs(2));
    let mut handles = Some(handles);
    loop {
        // Handly se pri neuspechu zapisu nevraceji: zkusit znovu jen bez nich.
        let h = handles.take().unwrap_or_default();
        let had = !h.is_empty();
        match ch.write(data, h) {
            Err(Error::ShouldWait | Error::NoResources) if !had && libos::time::now() < end => {
                libos::time::sleep(core::time::Duration::from_millis(1));
            }
            r => return r,
        }
    }
}

fn read(ch: &Channel, deadline: Option<u64>) -> Result<ServerMsg> {
    let m = ch.read(deadline)?;
    ServerMsg::decode(&m.data)
}

/// Pozdrav a vytvoreni okna na (nove) instanci kompozitoru: `HELLO` ->
/// `WELCOME`, pak `zbytek` zprav (vytvoreni okna, obnova), pak prvni
/// `CONFIGURE` (pred nim kontext okna).
fn handshake(ch: &Channel, rest: &[(ClientMsg, Vec<Handle>)], deadline: Option<u64>) -> Result<Greeting> {
    ch.write(&ClientMsg::Hello { version: guiproto::VERSION }.encode(), Vec::new())?;
    let (screen, app) = loop {
        match read(ch, deadline)? {
            ServerMsg::Welcome { w, h, app, .. } => break ((w, h), app),
            ServerMsg::Error { code, .. } => return Err(code),
            _ => {}
        }
    };
    for (m, h) in rest {
        let dup = h.iter().map(|x| x.duplicate(None)).collect::<Result<Vec<_>>>()?;
        write(ch, &m.encode(), dup)?;
    }
    let mut context = None;
    loop {
        match read(ch, deadline)? {
            ServerMsg::Configure { w, h, serial, .. } => return Ok(Greeting { screen, app, size: (w, h, serial), context }),
            ServerMsg::Error { code, .. } => return Err(code),
            m => {
                if let Some(c) = WinContext::from_msg(&m) {
                    context = Some(c);
                }
            }
        }
    }
}

impl Window {
    /// Pozdravi kompozitor a vytvori okno (ceka na prvni `CONFIGURE`).
    /// `link` = kanal okna (bez obnovy) nebo [`Link`] s listkem.
    pub fn open(link: impl Into<Link>, mode: Mode, w: u32, h: u32, title: &str) -> Result<Self> {
        let link = link.into();
        let create = ClientMsg::WindowCreate { window: 1, mode, w, h, title: title.into() };
        let g = handshake(&link.window, &[(create.clone(), Vec::new())], None)?;
        let (screen, app, (cw, ch, serial), context) = (g.screen, g.app, g.size, g.context);
        let mut replay = Replay::new();
        replay.record(&create);
        replay.configured(1, cw, ch);
        let mut win = Self {
            conn: Conn::new("compd", link.window, link.lease),
            w: cw,
            h: ch,
            screen,
            app,
            next_buffer: 1,
            pairs: BTreeMap::new(),
            queue: Vec::new(),
            ack: Some(serial),
            txn: 0,
            frame_pending: false,
            frame_deadline: 0,
            title: title.into(),
            menu: Vec::new(),
            a11y_ids: Default::default(),
            groups: BTreeMap::new(),
            replay,
            clip_owned: Vec::new(),
            clip_serial: 0,
            gesture: 0,
            field_paste: 0,
            app_paste: 0,
            clip_formats: Vec::new(),
            context,
            popover_shown: false,
            caret: None,
            caret_shown: false,
        };
        win.send(ClientMsg::LayerCreate { layer: ROOT_LAYER, window: 1, parent: 0, kind: LayerKind::Buffer })?;
        win.pairs.insert(ROOT_LAYER, Pair { bufs: [None, None], front: 1 });
        Ok(win)
    }

    /// Posle zpravu kompozitoru (a zaznamena ji pro obnovu). Po padu
    /// kompozitoru okno obnovi; zprava rozpracovane transakce (vrstvy,
    /// pristupnost, titulek bez `COMMIT`) pak vrati `SERVICE_RESTARTED`.
    pub fn send(&mut self, m: ClientMsg) -> Result<()> {
        self.send_with(m, Vec::new())
    }

    /// Ceka udalost ve fronte (prisla behem kresleni)? Pak `next` vrati
    /// hned a cekat na kanal nema smysl.
    pub fn has_queued(&self) -> bool {
        !self.queue.is_empty()
    }

    fn send_with(&mut self, m: ClientMsg, handles: Vec<Handle>) -> Result<()> {
        // Predchozi obnova selhala: nejdriv okno (jinak by zprava sla na
        // instanci, ktera o okne nevi).
        self.ensure()?;
        self.replay.record(&m);
        let data = m.encode();
        match self.conn.with_channel(|ch| write(ch, &data, handles)) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(Error::PeerClosed)) | Err(Error::PeerClosed) => {
                // Zprava je v zaznamu: obnova ji posle s platnym stavem
                // (buffer, `COMMIT`), nebo zahodi (rozpracovana transakce).
                self.restore().map(|_| ())
            }
            Ok(Err(e)) | Err(e) => Err(e),
        }
    }

    /// Generace instance kompozitoru, na ktere okno je (0 = prvni,
    /// bez obnovy).
    pub fn generation(&self) -> u64 {
        self.conn.generation()
    }

    /// Kanal okna pro cekani spolu s jinymi kanaly (jen cteni signalu;
    /// po obnove je jiny).
    pub fn with_channel<R>(&mut self, f: impl FnOnce(&Channel) -> R) -> Result<R> {
        match self.ensure() {
            Ok(()) | Err(Error::ServiceRestarted) => {}
            Err(e) => return Err(e),
        }
        self.conn.with_channel(f)
    }

    fn new_buffer(&mut self, w: u32, h: u32) -> Result<Buf> {
        let size = (w as u64 * h as u64 * 4).max(4);
        let vmo = Vmo::create(size)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        let id = self.next_buffer;
        self.next_buffer += 1;
        let b = Buf { id, w, h, vmo, map, busy: false };
        // Kompozitor dostane buffer jen pro cteni.
        let ro = b.readonly()?;
        // Buffer je v zaznamu i pri padu (obnova ho posle znovu).
        self.send_with(ClientMsg::BufferCreate { buffer: id, w, h, stride: w }, vec![ro])?;
        Ok(b)
    }

    /// Zpracuje zpravu kompozitoru; udalosti pro aplikaci vrati.
    fn handle(&mut self, m: ServerMsg) -> Result<Option<Event>> {
        Ok(match m {
            ServerMsg::Released { buffer } => {
                for p in self.pairs.values_mut() {
                    for b in p.bufs.iter_mut().flatten() {
                        if b.id == buffer {
                            b.busy = false;
                        }
                    }
                }
                None
            }
            ServerMsg::Ping { serial } => {
                self.send(ClientMsg::Pong { serial })?;
                None
            }
            ServerMsg::Configure { w, h, serial, .. } => {
                self.ack = Some(serial);
                if (w, h) != (self.w, self.h) {
                    self.w = w;
                    self.h = h;
                    self.replay.configured(1, w, h);
                    Some(Event::Resized { w, h })
                } else {
                    None
                }
            }
            ServerMsg::Frame { deadline, .. } => {
                self.frame_pending = false;
                self.frame_deadline = deadline;
                Some(Event::Frame)
            }
            ServerMsg::Input { records, .. } => {
                // Vic zaznamu = vic udalosti: dalsi do fronty.
                let mut it = records.into_iter();
                let first = it.next().map(Event::Input);
                self.queue.extend(it.map(Event::Input));
                first
            }
            ServerMsg::Close { .. } => Some(Event::Close),
            ServerMsg::Theme { mode } => Some(Event::Theme(match mode {
                1 => theme::Mode::Dark,
                2 => theme::Mode::Contrast,
                _ => theme::Mode::Light,
            })),
            ServerMsg::A11yAction { node, action, value, .. } => Some(Event::A11yAction { node, action, value }),
            ServerMsg::Clipboard { data } => clipproto::ServerMsg::decode(&data).ok().map(Event::Clipboard),
            ServerMsg::WindowPlace { rect, .. } => Some(Event::Place(rect)),
            ServerMsg::Compose { text, cursor_begin, cursor_end, flags, .. } => Some(Event::Compose { text, cursor_begin, cursor_end, flags }),
            ServerMsg::WindowContext { generation, output, screen, work_area, scale, refresh_mhz, ext, .. } => {
                self.context = Some(WinContext { output, screen, work_area, scale, refresh_mhz, generation, ext });
                Some(Event::Context)
            }
            ServerMsg::ActionInvoke { call, action, args, caller, synthetic } => Some(Event::Action { call, id: action, args, caller, synthetic }),
            _ => None,
        })
    }

    /// Precte zpravu kompozitoru (nejdel do `deadline`). Po padu
    /// kompozitoru okno obnovi a vrati `Ok(None)` (udalost `Restored` je
    /// ve fronte).
    fn read_msg(&mut self, deadline: Option<u64>) -> Result<Option<ServerMsg>> {
        self.ensure()?;
        match self.conn.with_channel(|ch| read(ch, deadline)) {
            Ok(Ok(m)) => Ok(Some(m)),
            Ok(Err(Error::PeerClosed)) | Err(Error::PeerClosed) => self.restore().map(|_| None),
            // Neplatna zprava: preskocit.
            Ok(Err(Error::InvalidArgs | Error::NotSupported)) => Ok(None),
            Ok(Err(e)) | Err(e) => Err(e),
        }
    }

    /// Dalsi udalost (blokuje az do `deadline`).
    pub fn next(&mut self, deadline: Option<u64>) -> Result<Event> {
        loop {
            if let Some(e) = self.next_one(deadline)? {
                return Ok(e);
            }
        }
    }

    /// Nejvys jedna zprava kompozitoru (do `deadline`): udalost, nebo
    /// `None`, kdyz zprava zadnou nevytvorila (napr. `RELEASED`). Smycka
    /// s dalsimi kanaly pak muze znovu cekat na vsechny (jinak by zprava
    /// sluzby cekala, dokud v okne neco nenastane).
    pub fn next_one(&mut self, deadline: Option<u64>) -> Result<Option<Event>> {
        let e = self.next_raw(deadline)?;
        // Stisk z mistniho fyzickeho zarizeni: jeho cislo plati pro
        // schranku (kopirovat / vlozit v reakci na nej).
        if let Some(Event::Input(r)) = &e
            && r.serial != 0
        {
            self.gesture = r.serial;
        }
        Ok(e)
    }

    /// Cislo gesta posledni dorucene udalosti s fyzickym stiskem (0 =
    /// zadne); posila se v `Offer` / `Request` schranky.
    pub fn gesture(&self) -> u32 {
        self.gesture
    }

    fn next_raw(&mut self, deadline: Option<u64>) -> Result<Option<Event>> {
        if !self.queue.is_empty() {
            return Ok(Some(self.queue.remove(0)));
        }
        let Some(msg) = self.read_msg(deadline)? else { return Ok(None) };
        if let ServerMsg::Error { op, code } = &msg {
            libos::println!("{}: kompozitor odmitl operaci {op}: {code}", self.app);
            return Ok(None);
        }
        self.handle(msg)
    }

    /// Volny buffer vrstvy `layer` velikosti `w` x `h` (pocka na RELEASED).
    fn back(&mut self, layer: u32, w: u32, h: u32) -> Result<usize> {
        let back = 1 - self.pairs[&layer].front;
        let needs_new = self.pairs[&layer].bufs[back].as_ref().is_none_or(|b| b.w != w || b.h != h);
        if needs_new {
            if let Some(old) = self.pairs.get_mut(&layer).unwrap().bufs[back].take() {
                self.send(ClientMsg::BufferDestroy { buffer: old.id })?;
            }
            let b = self.new_buffer(w, h)?;
            self.pairs.get_mut(&layer).unwrap().bufs[back] = Some(b);
        }
        while self.pairs[&layer].bufs[back].as_ref().unwrap().busy {
            match self.read_msg(None)? {
                Some(m) => {
                    if let Some(e) = self.handle(m)? {
                        self.queue.push(e);
                    }
                }
                // Obnova: buffery jsou volne, ale transakce se ztratila.
                None => return Err(Error::ServiceRestarted),
            }
        }
        Ok(back)
    }

    /// Nakresli obsah vrstvy (`f` dostane platno velikosti vrstvy) a vymeni
    /// buffery. Plati az po [`Window::commit`].
    fn draw_layer(&mut self, layer: u32, rect: Rect, f: impl FnOnce(&mut Canvas<'_>)) -> Result<()> {
        let back = self.back(layer, rect.w.max(1), rect.h.max(1))?;
        let pair = self.pairs.get_mut(&layer).unwrap();
        let b = pair.bufs[back].as_mut().unwrap();
        f(&mut b.canvas());
        b.busy = true;
        pair.front = back;
        let id = b.id;
        self.send(ClientMsg::LayerSet {
            layer,
            props: LayerProps {
                mask: prop::BUFFER | prop::RECT,
                rect,
                buffer: id,
                src: Rect::new(0, 0, rect.w, rect.h),
                ..Default::default()
            },
        })
    }

    /// Vrstva popoveru (nabidka, bublina) v obdelniku `rect` souradnic
    /// okna: kompozitor ji kresli nad vsemi okny a neorezava ji oknem.
    pub fn draw_popover(&mut self, rect: Rect, f: impl FnOnce(&mut Canvas<'_>)) -> Result<()> {
        if !self.pairs.contains_key(&POPOVER_LAYER) {
            self.send(ClientMsg::LayerCreate { layer: POPOVER_LAYER, window: 1, parent: 0, kind: LayerKind::Buffer })?;
            self.pairs.insert(POPOVER_LAYER, Pair { bufs: [None, None], front: 1 });
        }
        self.send(ClientMsg::LayerSet {
            layer: POPOVER_LAYER,
            props: LayerProps { mask: prop::POPOVER | prop::VISIBLE | prop::ORDER, popover: true, visible: true, order: 10, ..Default::default() },
        })?;
        self.popover_shown = true;
        self.draw_layer(POPOVER_LAYER, rect, f)
    }

    /// Vrstva kurzoru podle `Ui::caret`: novy rozmer nebo barva = nove
    /// pixely, jen posun = vlastnost `RECT`, blikani = jen `VISIBLE`.
    /// Vraci, zda neco poslala (pak je treba commit).
    pub fn set_caret(&mut self, c: Option<(Rect, u32, bool)>) -> Result<bool> {
        let Some((r, color, on)) = c else {
            if self.caret_shown {
                self.caret_shown = false;
                self.send(ClientMsg::LayerSet { layer: CARET_LAYER, props: LayerProps { mask: prop::VISIBLE, visible: false, ..Default::default() } })?;
                return Ok(true);
            }
            return Ok(false);
        };
        let mut sent = false;
        if !self.pairs.contains_key(&CARET_LAYER) {
            self.send(ClientMsg::LayerCreate { layer: CARET_LAYER, window: 1, parent: 0, kind: LayerKind::Buffer })?;
            self.send(ClientMsg::LayerSet { layer: CARET_LAYER, props: LayerProps { mask: prop::ORDER, order: 5, ..Default::default() } })?;
            self.pairs.insert(CARET_LAYER, Pair { bufs: [None, None], front: 1 });
        }
        match self.caret {
            Some((old, oc)) if (old.w, old.h, oc) == (r.w, r.h, color) => {
                if (old.x, old.y) != (r.x, r.y) {
                    self.send(ClientMsg::LayerSet { layer: CARET_LAYER, props: LayerProps { mask: prop::RECT, rect: r, ..Default::default() } })?;
                    sent = true;
                }
            }
            _ => {
                self.draw_layer(CARET_LAYER, r, |cv| cv.fill(cv.bounds(), color))?;
                sent = true;
            }
        }
        self.caret = Some((r, color));
        if on != self.caret_shown {
            self.caret_shown = on;
            self.send(ClientMsg::LayerSet { layer: CARET_LAYER, props: LayerProps { mask: prop::VISIBLE, visible: on, ..Default::default() } })?;
            sent = true;
        }
        Ok(sent)
    }

    /// Skryje vrstvu popoveru (nic otevreneho).
    pub fn hide_popover(&mut self) -> Result<()> {
        if !self.popover_shown {
            return Ok(());
        }
        self.popover_shown = false;
        self.send(ClientMsg::LayerSet { layer: POPOVER_LAYER, props: LayerProps { mask: prop::VISIBLE, visible: false, ..Default::default() } })
    }

    /// Nakresli cele okno.
    pub fn draw(&mut self, f: impl FnOnce(&mut Canvas<'_>)) -> Result<()> {
        let r = Rect::new(0, 0, self.w, self.h);
        self.draw_layer(ROOT_LAYER, r, f)
    }

    /// Posouvana oblast se stabilnim id `id`: skupina s orezem na `rect`
    /// a obsah vysky `content_h`, posunuty o `offset`.
    pub fn draw_scroll(&mut self, id: u32, rect: Rect, content_h: u32, offset: i32, f: impl FnOnce(&mut Canvas<'_>)) -> Result<()> {
        let (g, c) = scroll_layers(id);
        if !self.pairs.contains_key(&c) {
            self.send(ClientMsg::LayerCreate { layer: g, window: 1, parent: 0, kind: LayerKind::Group })?;
            self.send(ClientMsg::LayerCreate { layer: c, window: 1, parent: g, kind: LayerKind::Buffer })?;
            self.pairs.insert(c, Pair { bufs: [None, None], front: 1 });
        }
        self.groups.insert(g, rect);
        self.send(ClientMsg::LayerSet {
            layer: g,
            props: LayerProps {
                mask: prop::RECT | prop::CLIP | prop::SCROLL | prop::ORDER,
                rect,
                clip: true,
                scroll: (0, offset),
                order: 1,
                ..Default::default()
            },
        })?;
        self.draw_layer(c, Rect::new(0, 0, rect.w, content_h), f)
    }

    /// Jen posun oblasti (kompozitor posune obsah bez prekresleni).
    pub fn set_scroll(&mut self, id: u32, offset: i32) -> Result<()> {
        let (g, _) = scroll_layers(id);
        self.send(ClientMsg::LayerSet { layer: g, props: LayerProps { mask: prop::SCROLL, scroll: (0, offset), ..Default::default() } })
    }

    /// Odstrani posouvane oblasti, ktere uz nejsou.
    pub fn retain_scrolls(&mut self, keep: &[u32]) -> Result<()> {
        let gone: Vec<u32> = self.groups.keys().copied().filter(|g| !keep.iter().any(|&i| scroll_layers(i).0 == *g)).collect();
        for g in gone {
            self.groups.remove(&g);
            self.send(ClientMsg::LayerDestroy { layer: g })?;
            // Buffery obsahu uvolnit i v kompozitoru (jinak by se hromadily).
            if let Some(p) = self.pairs.remove(&(g + 1)) {
                for b in p.bufs.into_iter().flatten() {
                    self.send(ClientMsg::BufferDestroy { buffer: b.id })?;
                }
            }
        }
        Ok(())
    }

    /// Strom pristupnosti (cely; zmizele uzly odebere).
    pub fn set_a11y(&mut self, nodes: &[Node]) -> Result<()> {
        let ids: alloc::collections::BTreeSet<u32> = nodes.iter().map(|n| n.id).collect();
        let gone: Vec<u32> = self.a11y_ids.iter().copied().filter(|i| !ids.contains(i)).collect();
        if !gone.is_empty() {
            self.send(ClientMsg::A11yRemove { window: 1, ids: gone })?;
        }
        for m in window::a11y_chunks(1, nodes) {
            self.send(m)?;
        }
        self.a11y_ids = ids;
        Ok(())
    }

    /// Uplatni vse od posledniho commitu. `frame` = chci vedet, kdy kreslit
    /// dalsi snimek. Cekajici `CONFIGURE` potvrdi jen commit po prekresleni
    /// v nove velikosti (jinak by kompozitor ukazal novy ram se starym
    /// obsahem) - viz [`Window::commit_redrawn`].
    pub fn commit(&mut self, frame: bool) -> Result<()> {
        self.commit_inner(frame, false)
    }

    /// Commit po prekresleni celeho okna (potvrdi cekajici `CONFIGURE`).
    pub fn commit_redrawn(&mut self, frame: bool) -> Result<()> {
        self.commit_inner(frame, true)
    }

    fn commit_inner(&mut self, frame: bool, redrawn: bool) -> Result<()> {
        self.txn += 1;
        let mut flags = if frame { commit::FRAME } else { 0 };
        let serial = match self.ack.take_if(|_| redrawn) {
            Some(s) => {
                flags |= commit::ACK_CONFIGURE;
                s
            }
            None => 0,
        };
        if frame {
            self.frame_pending = true;
        }
        // Commit je v zaznamu: po padu ho obnova uplatni (stav okna je pak
        // stejny, jako kdyby ho stara instance prijala).
        self.send(ClientMsg::Commit { txn: self.txn, flags, window: 1, serial, token: 0 })
    }

    /// Odpoved na volani akce. Volani patri instanci kompozitoru, ktera ho
    /// poslala: po jejim padu `SERVICE_RESTARTED` (volajici odpoved
    /// nedostal, akce ale probehla - rozhodne aplikace).
    pub fn action_result(&mut self, call: u32, r: core::result::Result<Option<Value>, String>) -> Result<()> {
        let (ok, value, message) = match r {
            Ok(v) => (true, v, String::new()),
            Err(m) => (false, None, m),
        };
        let generation = self.conn.generation();
        self.send(ClientMsg::ActionResult { call, ok, value, message })?;
        if self.conn.generation() != generation {
            return Err(Error::ServiceRestarted);
        }
        Ok(())
    }
}

/// Spusti aplikaci s rozhranim z knihovny v okne. Vraci po zavreni okna.
pub fn run<A: App>(link: impl Into<Link>, app: A, mode: Mode, w: u32, h: u32) -> Result<()> {
    run_with(link, app, mode, w, h, None)
}

/// Provede ucinky aplikace (schranka) po jejich zpravach.
fn effects<A: App>(win: &mut Window, app: &mut A) -> Result<()> {
    for e in app.effects() {
        match e {
            crate::Effect::Copy { formats } => win.copy(formats)?,
            crate::Effect::Paste { formats } => win.app_paste = win.request_paste_formats(&formats)?,
        }
    }
    Ok(())
}

/// Jako [`run`], navic cte zpravy z kanalu `extra` (sluzba, od ktere
/// aplikace bere data) a predava je [`App::external`].
pub fn run_with<A: App>(link: impl Into<Link>, app: A, mode: Mode, w: u32, h: u32, extra: Option<Channel>) -> Result<()> {
    run_channels(link, app, mode, w, h, extra.into_iter().collect())
}

/// Jako [`run`] s vic dalsimi kanaly (sluzba, pomocny proces kopirovani):
/// zpravy z kanalu `extras[i]` dostane [`App::external_from`] s indexem `i`.
pub fn run_channels<A: App>(link: impl Into<Link>, app: A, mode: Mode, w: u32, h: u32, extras: Vec<Channel>) -> Result<()> {
    run_video(link, app, mode, w, h, extras, Vec::new())
}

/// Jako [`run_channels`] s povrchy videa pro plochy `node::video`:
/// klient tika se snimky kompozitoru, dokud [`App::wants_frames`], a ukazuje
/// snimky z [`App::video_frame`] bez prekresleni okna.
pub fn run_video<A: App>(link: impl Into<Link>, mut app: A, mode: Mode, w: u32, h: u32, extras: Vec<Channel>, surfaces: Vec<VideoSurface>) -> Result<()> {
    let mut videos = surfaces.into_iter().map(video::Video::new).collect::<Result<Vec<_>>>()?;
    let mut win = Window::open(link, mode, w, h, &app.title())?;
    // Velikost podle aplikace pred prvnim zobrazenim (bez probliknuti).
    if let Some((sw, sh, fw, fh)) = app.size_hint() {
        win.request_size(sw, sh, fw, fh)?;
    }
    let mut ui: Ui<A::Msg> = Ui::new(crate::Look::ui_fonts(), win.w, win.h);
    ui.use_overlay_layer(true);
    ui.use_caret_layer(true);
    set_context(&win, &mut ui);
    let mut dirty = true;
    // Trasovani vstupu: klik, ktery zmenil rozhrani, ceka na nakresleni
    // a na snimek kompozitoru (cela cesta klik -> obraz).
    let mut drawn: Option<u64> = None;
    loop {
        // Ucinky aplikace (schranka) z predchozich zprav.
        match effects(&mut win, &mut app) {
            Ok(()) | Err(Error::ServiceRestarted) => {}
            Err(Error::PeerClosed) => return Ok(()),
            Err(e) => return Err(e),
        }
        // Prehravani: udrzet tok snimku kompozitoru (prazdny commit s FRAME).
        if !dirty && !win.frame_pending && app.wants_frames() {
            match win.commit(true) {
                Ok(()) => {}
                Err(Error::ServiceRestarted) => dirty = true,
                Err(Error::PeerClosed) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
        if dirty && !win.frame_pending {
            ui.refresh(&app);
            match render(&mut win, &mut ui).and_then(|_| place_videos(&mut win, &ui, &mut videos)) {
                Ok(()) => {
                    dirty = false;
                    // Menu aplikace (paleta prikazu): jen pri zmene.
                    let menu = app.menu();
                    if menu != win.menu {
                        match win.send(ClientMsg::MenuUpdate { window: 1, items: menu.clone() }) {
                            Ok(()) => win.menu = menu,
                            Err(e) => libos::println!("ui: menu okna: {e}"),
                        }
                    }
                    let id = CHANGED_ID.swap(0, core::sync::atomic::Ordering::Relaxed);
                    if id != 0 {
                        libos::println!("vstup: id={id} ui nakresleno a odeslano kompozitoru +{} ms", since_ms(id));
                        drawn = Some(id);
                    }
                }
                // Kompozitor spadl uprostred snimku: okno je obnovene,
                // nakreslit ho znovu.
                Err(Error::ServiceRestarted) => continue,
                Err(Error::PeerClosed) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
        // Zpravy z dalsich kanalu (bez blokovani okna).
        if !extras.is_empty() && win.queue.is_empty() {
            let rd = Signals::READABLE | Signals::PEER_CLOSED;
            let obs = match win.with_channel(|ch| {
                let mut hs = alloc::vec![(ch.handle(), rd)];
                hs.extend(extras.iter().map(|x| (x.handle(), Signals::READABLE)));
                libos::wait_many(&hs, ui.next_deadline())
            }) {
                Ok(Ok((_, o))) => o,
                Ok(Err(Error::TimedOut)) => {
                    if ui.timer(libos::time::now()) {
                        dirty = true;
                    }
                    if !dirty && !blink(&mut win, &mut ui)? {
                        dirty = true;
                    }
                    for m in ui.take_msgs() {
                        app.update(m);
                        dirty = true;
                    }
                    continue;
                }
                Ok(Err(e)) | Err(e) => return Err(e),
            };
            for (k, x) in extras.iter().enumerate() {
                if obs[k + 1].intersects(Signals::READABLE) {
                    while let Ok(m) = x.try_read() {
                        dirty |= app.external_from(k, &m.data);
                    }
                }
            }
            if !obs[0].intersects(rd) {
                continue;
            }
        }
        // Bublina: probudit se v case jejiho zobrazeni.
        let deadline = ui.next_deadline().map(|t| t / 1000 * 1000);
        // S dalsimi kanaly jen jedna zprava okna (kanal je citelny), pak
        // znovu cekani na vsechny kanaly.
        let next = if extras.is_empty() { win.next(deadline).map(Some) } else { win.next_one(None) };
        let ev = match next {
            Ok(Some(e)) => e,
            Ok(None) => continue,
            Err(Error::TimedOut) => {
                if ui.timer(libos::time::now()) {
                    dirty = true;
                }
                if !dirty && !blink(&mut win, &mut ui)? {
                    dirty = true;
                }
                for m in ui.take_msgs() {
                    app.update(m);
                    dirty = true;
                }
                continue;
            }
            Err(Error::PeerClosed) => return Ok(()),
            Err(e) => return Err(e),
        };
        if matches!(ev, Event::Frame)
            && let Some(id) = drawn.take()
        {
            libos::println!("vstup: id={id} ui kompozitor slozil snimek se zmenou (FRAME) +{} ms", since_ms(id));
        }
        let frame = matches!(ev, Event::Frame);
        let r = handle_event(&mut win, &mut ui, &mut app, ev, &mut dirty);
        if frame && r.is_ok() && app.wants_frames() {
            match video_tick(&mut win, &ui, &mut app, &mut videos, &mut dirty) {
                Ok(()) => {}
                Err(Error::ServiceRestarted) => dirty = true,
                Err(Error::PeerClosed) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
        match r {
            Ok(true) => {}
            Ok(false) => return Ok(()),
            Err(Error::ServiceRestarted) => dirty = true,
            Err(Error::PeerClosed) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

/// Snimek kompozitoru pri prehravani: snimky videa od aplikace do vrstev.
/// Bez prekresleni okna je commit tady (s FRAME = dalsi tik); jinak ho
/// udela `render` v teze transakci.
fn video_tick<A: App>(win: &mut Window, ui: &Ui<A::Msg>, app: &mut A, videos: &mut [video::Video], dirty: &mut bool) -> Result<()> {
    let tick = app.video_frame(libos::time::now(), win.frame_deadline);
    *dirty |= tick.redraw;
    let rects = ui.video_rects();
    for s in tick.shows {
        let Some(v) = videos.iter_mut().find(|v| v.id() == s.id) else { continue };
        let Some(&(_, r)) = rects.iter().find(|(id, _)| *id == s.id) else { continue };
        let release = win.video_show(v, s.slot, s.acquire, r)?;
        app.video_shown(s.id, s.slot, release);
    }
    if !*dirty {
        win.commit(true)?;
    }
    Ok(())
}

/// Po prekresleni: vrstvy videa na obdelniky z rozvrzeni.
fn place_videos<M: Clone>(win: &mut Window, ui: &Ui<M>, videos: &mut [video::Video]) -> Result<()> {
    if videos.is_empty() {
        return Ok(());
    }
    let rects = ui.video_rects();
    let mut sent = false;
    for v in videos.iter_mut() {
        let r = rects.iter().find(|(id, _)| *id == v.id()).map(|&(_, r)| r);
        sent |= win.video_place(v, r)?;
    }
    if sent {
        win.commit(false)?;
    }
    Ok(())
}

/// Blikani kurzoru bez prekresleni: jen viditelnost vrstvy kurzoru
/// a commit. `Ok(false)` = kompozitor se restartoval (prekreslit cele).
fn blink<M: Clone>(win: &mut Window, ui: &mut Ui<M>) -> Result<bool> {
    match win.set_caret(ui.caret()).and_then(|sent| if sent { win.commit(false) } else { Ok(()) }) {
        Ok(()) => Ok(true),
        Err(Error::ServiceRestarted) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Id udalosti vstupu, kterou aplikace prave zpracovava (0 = zadna):
/// aplikace ho pripoji k trasovani pozadavku na sluzby ("vstup: id=...").
static INPUT_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Posledni udalost vstupu, ktera zmenila rozhrani (ceka na nakresleni).
static CHANGED_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Milisekundy od udalosti `id` (cas z inputd v us, stejne hodiny).
fn since_ms(id: u64) -> u64 {
    (libos::time::now() / 1000).saturating_sub(id) / 1000
}

/// Id zpracovavane udalosti vstupu pro trasovani (0 = zadna).
pub fn input_id() -> u64 {
    INPUT_ID.load(core::sync::atomic::Ordering::Relaxed)
}

/// Udalost okna v [`run_with`]; `Ok(false)` = zavrit.
fn handle_event<A: App>(win: &mut Window, ui: &mut Ui<A::Msg>, app: &mut A, ev: Event, dirty: &mut bool) -> Result<bool> {
    match ev {
        Event::Input(r) => {
            ui.set_input_class(r.class);
            let resp = ui.event_at(r.event, r.mods, Some(r.time));
            // Schranka: Ctrl+C nabidne text, Ctrl+V si ho vyzada (data
            // prijdou jako `Event::Clipboard`).
            match &resp.clipboard {
                Some(crate::event::Clip::Copy(t)) => win.copy_text(t.clone())?,
                Some(crate::event::Clip::Paste) => {
                    win.field_paste = win.request_paste()?;
                }
                None => {}
            }
            // Klavesa, kterou prvky nespotrebovaly: zkratka aplikace.
            let shortcut = resp.unhandled_key.and_then(|(code, ch, mods)| app.key(code, ch, mods));
            let changed = !resp.msgs.is_empty();
            // Trasovani vstupu (cil logu `vstup`): id = cas zaznamu z inputd.
            let id = r.time / 1000;
            INPUT_ID.store(id, core::sync::atomic::Ordering::Relaxed);
            for l in &resp.trace {
                libos::println!("vstup: id={id} ui {l}");
            }
            if changed {
                libos::println!("vstup: id={id} aplikace zpracuje {} zprav", resp.msgs.len());
            }
            let changed = changed || shortcut.is_some() || !resp.actions.is_empty();
            for m in resp.msgs.into_iter().chain(shortcut) {
                app.update(m);
            }
            // Polozky nabidky s akci z manifestu.
            for (a, args) in &resp.actions {
                if let Err(e) = app.action(a, args) {
                    libos::println!("ui: akce {a} z nabidky: {e}");
                }
            }
            INPUT_ID.store(0, core::sync::atomic::Ordering::Relaxed);
            if changed {
                CHANGED_ID.store(id, core::sync::atomic::Ordering::Relaxed);
            }
            // Dalsi udalost (rychle psani pred snimkem) musi videt novy
            // stav: rozvrhnout hned, kreslit az na snimek.
            if changed {
                ui.refresh(app);
            }
            *dirty |= changed || resp.redraw;
            for (i, off) in &resp.scrolls {
                win.set_scroll(*i, *off)?;
            }
            if !resp.scrolls.is_empty() && !changed && !resp.redraw {
                win.commit(false)?;
            }
        }
        Event::Resized { w, h } => {
            ui.resize(w, h);
            *dirty = true;
        }
        Event::Context => {
            set_context(win, ui);
            *dirty = true;
        }
        // Poloha okna: jen pro umisteni popoveru (rozvrzeni se nemeni).
        Event::Place(r) => ui.set_window_rect(r),
        Event::Compose { text, cursor_begin, cursor_end, flags } => {
            if ui.compose(&text, cursor_begin, cursor_end, flags) {
                *dirty = true;
            }
        }
        Event::Close => return Ok(false),
        Event::A11yAction { node, action, value } => {
            let resp = ui.a11y_action(node, action, &value);
            for m in resp.msgs {
                app.update(m);
            }
            ui.refresh(app);
            *dirty = true;
        }
        Event::Action { call, id, args, .. } => {
            let r = app.action(&id, &args);
            ui.refresh(app);
            *dirty = true;
            // Volajici patril spadle instanci: odpoved nema kam, akce
            // probehla (stav aplikace je novy).
            match win.action_result(call, r) {
                Err(Error::ServiceRestarted) => {}
                o => o?,
            }
        }
        // Obnovene okno ma posledni potvrzeny obsah; nakreslit aktualni.
        Event::Restored { .. } => *dirty = true,
        Event::Theme(m) => {
            ui.look.set_mode(m);
            ui.refresh(app);
            *dirty = true;
        }
        Event::Clipboard(m) => {
            match win.clipboard_event_any(m)? {
                // Do textoveho pole (Ctrl+V v poli).
                Some(p) if p.serial == win.field_paste && p.format == clipproto::TEXT => {
                    let resp = ui.paste(&String::from_utf8_lossy(&p.data));
                    let changed = !resp.msgs.is_empty();
                    for m in resp.msgs {
                        app.update(m);
                    }
                    if changed {
                        ui.refresh(app);
                    }
                    *dirty |= changed || resp.redraw;
                }
                // Aplikaci (`Effect::Paste`).
                Some(p) if p.serial == win.app_paste => {
                    if app.pasted(&p.format, &p.data) {
                        ui.refresh(app);
                        *dirty = true;
                    }
                }
                _ => {}
            }
            // Formaty ve schrance pro `Context::clipboard` (polozka Vlozit).
            if ui.context().clipboard != win.clip_formats {
                ui.set_clipboard_formats(win.clip_formats.clone());
                *dirty = true;
            }
        }
        // Snimek: krok plynuleho rolovani tabulek.
        Event::Frame => {
            if ui.tick() {
                *dirty = true;
            }
        }
    }
    Ok(true)
}

/// Prenese kontext okna do `Ui` (aplikace ho dostane ve `view`).
fn set_context<M: Clone>(win: &Window, ui: &mut Ui<M>) {
    if let Some(c) = &win.context {
        ui.set_context(c.output, c.screen, c.work_area, c.scale, c.refresh_mhz, c.generation);
        ui.set_context_ext(&c.ext);
    }
}

/// Vykresli rozhrani do vrstev okna a uplatni ho jednou transakci.
fn render<M: Clone>(win: &mut Window, ui: &mut Ui<M>) -> Result<()> {
    if (ui.w, ui.h) != (win.w, win.h) {
        ui.resize(win.w, win.h);
    }
    win.draw(|c| ui.draw(c, 0))?;
    let areas = ui.scroll_areas();
    for a in &areas {
        win.draw_scroll(a.id, a.rect, a.content_h, a.offset, |c| ui.draw(c, a.index + 1))?;
    }
    win.retain_scrolls(&areas.iter().map(|a| a.id).collect::<Vec<_>>())?;
    // Nabidka a bublina ve vrstve popoveru (muze presahovat okno).
    match ui.overlay_rect() {
        Some(r) => win.draw_popover(r, |c| ui.draw_overlay(c, r))?,
        None => win.hide_popover()?,
    }
    win.set_caret(ui.caret())?;
    win.set_a11y(&ui.a11y())?;
    // Titulek podle stavu aplikace ("Správce úloh (20 běží)"): v teze
    // transakci jako obsah. Ostatni vlastnosti okna vychozi (jako po
    // `WindowCreate`).
    if ui.title != win.title {
        win.send(ClientMsg::WindowSet {
            window: 1,
            title: ui.title.clone(),
            min_w: 64,
            min_h: 32,
            side: guiproto::window::ButtonSide::Right,
            button_color: 0,
        })?;
        win.title = ui.title.clone();
    }
    win.commit_redrawn(true)
}
