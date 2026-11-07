//! Behove prostredi aplikaci v OS: okno nad kanalem ke kompozitoru.
//!
//! [`Window`] je nizka vrstva (buffery ve VMO, vrstvy, transakce, udalosti),
//! kterou pouzije i aplikace kreslici sama (okno terminalu). [`run`] nad
//! ni pusti aplikaci s rozhranim z knihovny ([`crate::App`]).
//!
//! Buffery jsou dvojite: kresli se do toho, ktery kompozitor zrovna
//! nepouziva (`RELEASED`), takze aplikace na kompozitor nikdy neceka.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use gfx::text::Text;
use guiproto::a11y::Node;
use guiproto::action::{Arg, Value};
use guiproto::window::{ClientMsg, InputRecord, LayerKind, LayerProps, Mode, ServerMsg, commit, prop};
use guiproto::{Rect, window};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Error, Mapping, Result, Rights, Vmo};

use crate::{App, ROOT_LAYER, Ui, scroll_layers};

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
}

struct Buf {
    id: u32,
    w: u32,
    h: u32,
    _vmo: Vmo,
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
}

/// Dvojice bufferu jedne vrstvy.
struct Pair {
    bufs: [Option<Buf>; 2],
    front: usize,
}

pub struct Window {
    ch: Channel,
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
    a11y_ids: Vec<u32>,
    groups: BTreeMap<u32, Rect>,
}

fn read(ch: &Channel) -> Result<ServerMsg> {
    let m = ch.read(None)?;
    ServerMsg::decode(&m.data)
}

impl Window {
    /// Pozdravi kompozitor a vytvori okno (ceka na prvni `CONFIGURE`).
    pub fn open(ch: Channel, mode: Mode, w: u32, h: u32, title: &str) -> Result<Self> {
        ch.write(&ClientMsg::Hello { version: guiproto::VERSION }.encode(), Vec::new())?;
        let (screen, app) = loop {
            match read(&ch)? {
                ServerMsg::Welcome { w, h, app, .. } => break ((w, h), app),
                ServerMsg::Error { code, .. } => return Err(code),
                _ => {}
            }
        };
        let msg = ClientMsg::WindowCreate { window: 1, mode, w, h, title: title.into() };
        ch.write(&msg.encode(), Vec::new())?;
        let mut win = Self {
            ch,
            w,
            h,
            screen,
            app,
            next_buffer: 1,
            pairs: BTreeMap::new(),
            queue: Vec::new(),
            ack: None,
            txn: 0,
            frame_pending: false,
            a11y_ids: Vec::new(),
            groups: BTreeMap::new(),
        };
        loop {
            match read(&win.ch)? {
                ServerMsg::Configure { w, h, serial, .. } => {
                    win.w = w;
                    win.h = h;
                    win.ack = Some(serial);
                    break;
                }
                ServerMsg::Error { code, .. } => return Err(code),
                _ => {}
            }
        }
        win.send(ClientMsg::LayerCreate { layer: ROOT_LAYER, window: 1, parent: 0, kind: LayerKind::Buffer })?;
        win.pairs.insert(ROOT_LAYER, Pair { bufs: [None, None], front: 1 });
        Ok(win)
    }

    fn send(&self, m: ClientMsg) -> Result<()> {
        self.ch.write(&m.encode(), Vec::new())
    }

    pub fn channel(&self) -> &Channel {
        &self.ch
    }

    fn new_buffer(&mut self, w: u32, h: u32) -> Result<Buf> {
        let size = (w as u64 * h as u64 * 4).max(4);
        let vmo = Vmo::create(size)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        let id = self.next_buffer;
        self.next_buffer += 1;
        // Kompozitor dostane buffer jen pro cteni.
        let ro = vmo.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))?;
        let msg = ClientMsg::BufferCreate { buffer: id, w, h, stride: w };
        self.ch.write(&msg.encode(), vec![ro])?;
        Ok(Buf { id, w, h, _vmo: vmo, map, busy: false })
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
                    Some(Event::Resized { w, h })
                } else {
                    None
                }
            }
            ServerMsg::Frame { .. } => {
                self.frame_pending = false;
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
            ServerMsg::A11yAction { node, action, value, .. } => Some(Event::A11yAction { node, action, value }),
            ServerMsg::ActionInvoke { call, action, args, caller, synthetic } => Some(Event::Action { call, id: action, args, caller, synthetic }),
            _ => None,
        })
    }

    /// Dalsi udalost (blokuje az do `deadline`).
    pub fn next(&mut self, deadline: Option<u64>) -> Result<Event> {
        if !self.queue.is_empty() {
            return Ok(self.queue.remove(0));
        }
        loop {
            let m = self.ch.read(deadline)?;
            let Ok(msg) = ServerMsg::decode(&m.data) else { continue };
            if let ServerMsg::Error { op, code } = &msg {
                libos::println!("{}: kompozitor odmitl operaci {op}: {code}", self.app);
                continue;
            }
            if let Some(e) = self.handle(msg)? {
                return Ok(e);
            }
        }
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
            let m = read(&self.ch)?;
            if let Some(e) = self.handle(m)? {
                self.queue.push(e);
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

    /// Nakresli cele okno.
    pub fn draw(&mut self, f: impl FnOnce(&mut Canvas<'_>)) -> Result<()> {
        let r = Rect::new(0, 0, self.w, self.h);
        self.draw_layer(ROOT_LAYER, r, f)
    }

    /// Posouvana oblast `index`: skupina s orezem na `rect` a obsah vysky
    /// `content_h`, posunuty o `offset`.
    pub fn draw_scroll(&mut self, index: usize, rect: Rect, content_h: u32, offset: i32, f: impl FnOnce(&mut Canvas<'_>)) -> Result<()> {
        let (g, c) = scroll_layers(index);
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
    pub fn set_scroll(&mut self, index: usize, offset: i32) -> Result<()> {
        let (g, _) = scroll_layers(index);
        self.send(ClientMsg::LayerSet { layer: g, props: LayerProps { mask: prop::SCROLL, scroll: (0, offset), ..Default::default() } })
    }

    /// Odstrani posouvane oblasti, ktere uz nejsou.
    pub fn retain_scrolls(&mut self, keep: &[usize]) -> Result<()> {
        let gone: Vec<u32> = self.groups.keys().copied().filter(|g| !keep.iter().any(|&i| scroll_layers(i).0 == *g)).collect();
        for g in gone {
            self.groups.remove(&g);
            self.pairs.remove(&(g + 1));
            self.send(ClientMsg::LayerDestroy { layer: g })?;
        }
        Ok(())
    }

    /// Strom pristupnosti (cely; zmizele uzly odebere).
    pub fn set_a11y(&mut self, nodes: &[Node]) -> Result<()> {
        let ids: Vec<u32> = nodes.iter().map(|n| n.id).collect();
        let gone: Vec<u32> = self.a11y_ids.iter().copied().filter(|i| !ids.contains(i)).collect();
        if !gone.is_empty() {
            self.send(ClientMsg::A11yRemove { window: 1, ids: gone })?;
        }
        for m in window::a11y_messages(1, nodes) {
            self.ch.write(&m, Vec::new())?;
        }
        self.a11y_ids = ids;
        Ok(())
    }

    /// Uplatni vse od posledniho commitu. `frame` = chci vedet, kdy kreslit
    /// dalsi snimek.
    pub fn commit(&mut self, frame: bool) -> Result<()> {
        self.txn += 1;
        let mut flags = if frame { commit::FRAME } else { 0 };
        let serial = match self.ack.take() {
            Some(s) => {
                flags |= commit::ACK_CONFIGURE;
                s
            }
            None => 0,
        };
        if frame {
            self.frame_pending = true;
        }
        self.send(ClientMsg::Commit { txn: self.txn, flags, window: 1, serial, token: 0 })
    }

    /// Odpoved na volani akce.
    pub fn action_result(&self, call: u32, r: core::result::Result<Option<Value>, String>) -> Result<()> {
        let (ok, value, message) = match r {
            Ok(v) => (true, v, String::new()),
            Err(m) => (false, None, m),
        };
        self.send(ClientMsg::ActionResult { call, ok, value, message })
    }
}

/// Spusti aplikaci s rozhranim z knihovny v okne. Vraci po zavreni okna.
pub fn run<A: App>(ch: Channel, mut app: A, mode: Mode, w: u32, h: u32) -> Result<()> {
    let mut win = Window::open(ch, mode, w, h, &app.title())?;
    let mut ui: Ui<A::Msg> = Ui::new(Text::system(), win.w, win.h);
    let mut dirty = true;
    loop {
        if dirty && !win.frame_pending {
            ui.rebuild(app.title(), app.view());
            render(&mut win, &mut ui)?;
            dirty = false;
        }
        let ev = match win.next(None) {
            Ok(e) => e,
            Err(Error::PeerClosed) => return Ok(()),
            Err(e) => return Err(e),
        };
        match ev {
            Event::Input(r) => {
                let resp = ui.event(r.event, r.mods);
                let changed = !resp.msgs.is_empty();
                for m in resp.msgs {
                    app.update(m);
                }
                for (i, off) in &resp.scrolls {
                    win.set_scroll(*i, *off)?;
                }
                if !resp.scrolls.is_empty() && !changed && !resp.redraw {
                    win.commit(false)?;
                }
                dirty |= changed || resp.redraw;
            }
            Event::Resized { w, h } => {
                ui.resize(w, h);
                dirty = true;
            }
            Event::Close => return Ok(()),
            Event::A11yAction { node, action, value } => {
                let resp = ui.a11y_action(node, action, &value);
                for m in resp.msgs {
                    app.update(m);
                }
                dirty = true;
            }
            Event::Action { call, id, args, .. } => {
                let r = app.action(&id, &args);
                win.action_result(call, r)?;
                dirty = true;
            }
            Event::Frame => {}
        }
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
        win.draw_scroll(a.index, a.rect, a.content_h, a.offset, |c| ui.draw(c, a.index + 1))?;
    }
    win.retain_scrolls(&areas.iter().map(|a| a.index).collect::<Vec<_>>())?;
    win.set_a11y(&ui.a11y())?;
    win.commit(true)
}
