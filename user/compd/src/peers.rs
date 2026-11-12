//! Kanaly protistran: klienti (okna), dialogy, volajici akci, inspekce.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use comp::{ClientId, Comp, Out, PeerId, Pixels};
use guiproto::control::{CallerMsg, DialogRequest, InspectReply, InspectRequest};
use guiproto::window::{ClientMsg, ServerMsg};
use libos::{Channel, Handle, Mapping, Signals, Vmo};

/// Buffer aplikace namapovany jen pro cteni.
pub struct VmoPixels {
    pub _vmo: Vmo,
    pub map: Mapping,
    pub len: usize,
}

impl Pixels for VmoPixels {
    fn pixels(&self) -> Option<&[u32]> {
        // SAFETY: mapovani `len` pixelu (jen cteni) zije s `self`. Aplikace
        // muze pamet soucasne menit; nejhorsi vysledek je roztrzeny obsah
        // jejiho vlastniho okna.
        Some(unsafe { core::slice::from_raw_parts(self.map.addr() as *const u32, self.len) })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Client(ClientId),
    Dialog,
    Caller,
    Inspect,
}

struct Peer {
    kind: Kind,
    ch: Channel,
    /// Zpravy, ktere se nevesly do fronty kanalu (posilaji se pozdeji ve
    /// stejnem poradi; RELEASED, FRAME, CONFIGURE se nesmi ztratit).
    backlog: alloc::collections::VecDeque<Vec<u8>>,
    /// Volajici akci je automat (agent).
    automat: bool,
    name: String,
    closed: bool,
}

/// Nejvic odlozenych zprav; klient, ktery tolik nevycte, se odpoji.
const BACKLOG_MAX: usize = 4096;

impl Peer {
    fn push_backlog(&mut self, d: Vec<u8>) {
        if self.backlog.len() >= BACKLOG_MAX {
            self.closed = true;
        } else {
            self.backlog.push_back(d);
        }
    }
}

#[derive(Default)]
pub struct Peers {
    peers: BTreeMap<PeerId, Peer>,
    next: PeerId,
    /// Posledni pixel snimku pro inspekci `Pixel` (sluzba ho vyplni).
    pub frame: Option<(u64, u32, u32)>,
}

/// Nejvic zprav od jedne protistrany za jedno kolo (ferovost).
const BATCH: usize = 64;

impl Peers {
    fn add(&mut self, kind: Kind, ch: Channel, name: String) -> PeerId {
        self.next += 1;
        self.peers.insert(self.next, Peer { kind, ch, name, closed: false, backlog: Default::default(), automat: false });
        self.next
    }

    pub fn add_client(&mut self, id: ClientId, ch: Channel) {
        self.add(Kind::Client(id), ch, String::new());
    }

    pub fn add_dialog(&mut self, ch: Channel) {
        self.add(Kind::Dialog, ch, String::new());
    }

    pub fn add_caller(&mut self, ch: Channel, name: String, automat: bool) {
        let id = self.add(Kind::Caller, ch, name);
        self.peers.get_mut(&id).unwrap().automat = automat;
    }

    pub fn add_inspect(&mut self, ch: Channel) {
        self.add(Kind::Inspect, ch, String::new());
    }

    pub fn wait_items<'a>(&'a self, items: &mut Vec<(&'a Handle, Signals)>) {
        for p in self.peers.values() {
            items.push((p.ch.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        }
    }

    /// Obslouzi protistrany podle signalu (stejne poradi jako `wait_items`).
    pub fn serve(&mut self, obs: &[Signals], c: &mut Comp, now: u64) {
        let ids: Vec<PeerId> = self.peers.keys().copied().collect();
        for (i, id) in ids.into_iter().enumerate() {
            let s = obs.get(i).copied().unwrap_or(Signals(0));
            if s.intersects(Signals::READABLE) {
                for _ in 0..BATCH {
                    let p = self.peers.get_mut(&id).unwrap();
                    let Ok(m) = p.ch.try_read() else { break };
                    let (kind, name, automat) = (p.kind, p.name.clone(), p.automat);
                    self.message(id, kind, &name, automat, m.data, m.handles, c, now);
                }
            } else if s.intersects(Signals::PEER_CLOSED) {
                self.peers.get_mut(&id).unwrap().closed = true;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn message(&mut self, id: PeerId, kind: Kind, name: &str, automat: bool, data: Vec<u8>, handles: Vec<Handle>, c: &mut Comp, now: u64) {
        match kind {
            Kind::Client(cid) => match ClientMsg::decode(&data) {
                Ok(m) => {
                    let px = match &m {
                        // Rozmery overit pred mapovanim (mapovani stoji pamet
                        // jadra na ucet kompozitoru).
                        ClientMsg::BufferCreate { w, h, stride, .. } if comp::scene::buffer_ok(*w, *h, *stride) => handles
                            .into_iter()
                            .next()
                            .and_then(|h2| VmoPixels::map(Vmo::from_handle(h2), *stride as u64 * *h as u64).ok())
                            .map(|p| Box::new(p) as Box<dyn Pixels>),
                        _ => None,
                    };
                    c.client_msg_sized(cid, m, px, data.len(), now);
                }
                Err(e) => {
                    let op = ClientMsg::op_of(&data);
                    let _ = self.peers[&id].ch.write(&ServerMsg::Error { op, code: e }.encode(), Vec::new());
                }
            },
            Kind::Dialog => {
                if let Ok(r) = DialogRequest::decode(&data) {
                    c.dialog_request(id, r, now);
                }
            }
            Kind::Caller => {
                if let Ok(m) = CallerMsg::decode(&data) {
                    c.caller_msg(id, name, automat, m, now);
                }
            }
            Kind::Inspect => match InspectRequest::decode(&data) {
                Ok(InspectRequest::Pixel { x, y }) => {
                    let reply = match self.frame {
                        Some((addr, w, h)) if x < w && y < h => {
                            // SAFETY: `addr` je mapovani snimku `w * h` pixelu
                            // (zije po celou dobu sluzby), index je v mezich.
                            InspectReply::Pixel(unsafe { *((addr as *const u32).add((y * w + x) as usize)) })
                        }
                        _ => InspectReply::Error(libos::Error::InvalidArgs),
                    };
                    let _ = self.peers[&id].ch.write(&reply.encode(), Vec::new());
                }
                Ok(r) => c.inspect(id, r),
                Err(_) => {}
            },
        }
    }

    /// Posle zpravu z jadra kompozitoru prislusne protistrane.
    pub fn send(&mut self, o: Out) {
        let mut droppable = false;
        let (target, data) = match o {
            Out::Client(cid, m) => {
                let Some((&id, _)) = self.peers.iter().find(|(_, p)| p.kind == Kind::Client(cid)) else { return };
                // Pri plne fronte smi vypadnout jen pohyb ukazatele, kolecko
                // a ping; tlacitka, klavesy a fokus ne (aplikace by mela
                // "zaseknute" tlacitko nebo fokus).
                droppable = match &m {
                    ServerMsg::Input { records, .. } => records.iter().all(|r| {
                        matches!(r.event, guiproto::window::InputEvent::PointerMove { .. } | guiproto::window::InputEvent::Scroll { .. })
                    }),
                    ServerMsg::Ping { .. } => true,
                    _ => false,
                };
                (id, m.encode())
            }
            Out::Dialog(id, mut r) => {
                // Odpoved s heslem: kopie v kompozitoru hned prepsat.
                let b = r.encode();
                if let guiproto::control::DialogReply::Answer { text, .. } = &mut r {
                    wipe(unsafe_bytes(text));
                }
                (id, b)
            }
            Out::Caller(id, r) => (id, r.encode()),
            Out::Inspect(id, r) => (id, r.encode()),
            _ => return,
        };
        let Some(p) = self.peers.get_mut(&target) else { return };
        if !p.backlog.is_empty() {
            if !droppable {
                p.push_backlog(data);
            }
            return;
        }
        let mut data = data;
        match p.ch.write(&data, Vec::new()) {
            Ok(()) => wipe(&mut data),
            Err(libos::Error::PeerClosed) => {
                p.closed = true;
                wipe(&mut data);
            }
            // Plna fronta: klient nestiha cist.
            Err(_) if !droppable => p.push_backlog(data),
            Err(_) => wipe(&mut data),
        }
    }

    pub fn has_backlog(&self) -> bool {
        self.peers.values().any(|p| !p.backlog.is_empty())
    }

    /// Zkusi odeslat odlozene zpravy (volat v kazdem kole smycky).
    pub fn flush_backlog(&mut self) {
        for p in self.peers.values_mut() {
            while let Some(d) = p.backlog.front() {
                match p.ch.write(d, Vec::new()) {
                    Ok(()) => {
                        if let Some(mut d) = p.backlog.pop_front() {
                            wipe(&mut d);
                        }
                    }
                    Err(libos::Error::PeerClosed) => {
                        p.closed = true;
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
    }

    /// Odstrani zavrene protistrany a da o nich vedet jadru kompozitoru.
    pub fn reap(&mut self, c: &mut Comp) {
        let gone: Vec<PeerId> = self.peers.iter().filter(|(_, p)| p.closed).map(|(&i, _)| i).collect();
        for id in gone {
            let p = self.peers.remove(&id).unwrap();
            match p.kind {
                Kind::Client(cid) => c.remove_client(cid),
                Kind::Dialog => c.dialog_peer_gone(id),
                Kind::Caller => c.caller_gone(id),
                Kind::Inspect => {}
            }
        }
    }
}

/// Prepise bajty nulami (odeslane zpravy mohou obsahovat heslo z dialogu).
fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: platny jedinecny odkaz na bajt; volatile zapis prekladac
        // nevynecha.
        unsafe { core::ptr::write_volatile(x, 0) };
    }
}

/// Bajty retezce k prepsani nulami (nuly jsou platne UTF-8).
fn unsafe_bytes(s: &mut alloc::string::String) -> &mut [u8] {
    // SAFETY: zapisujeme jen nuly, retezec zustane platne UTF-8.
    unsafe { s.as_bytes_mut() }
}
