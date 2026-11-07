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
    name: String,
    closed: bool,
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
        self.peers.insert(self.next, Peer { kind, ch, name, closed: false });
        self.next
    }

    pub fn add_client(&mut self, id: ClientId, ch: Channel) {
        self.add(Kind::Client(id), ch, String::new());
    }

    pub fn add_dialog(&mut self, ch: Channel) {
        self.add(Kind::Dialog, ch, String::new());
    }

    pub fn add_caller(&mut self, ch: Channel, name: String) {
        self.add(Kind::Caller, ch, name);
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
                    let (kind, name) = (p.kind, p.name.clone());
                    self.message(id, kind, &name, m.data, m.handles, c, now);
                }
            } else if s.intersects(Signals::PEER_CLOSED) {
                self.peers.get_mut(&id).unwrap().closed = true;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn message(&mut self, id: PeerId, kind: Kind, name: &str, data: Vec<u8>, handles: Vec<Handle>, c: &mut Comp, now: u64) {
        match kind {
            Kind::Client(cid) => match ClientMsg::decode(&data) {
                Ok(m) => {
                    let px = match &m {
                        ClientMsg::BufferCreate { w: _, h, stride, .. } => handles
                            .into_iter()
                            .next()
                            .and_then(|h2| VmoPixels::map(Vmo::from_handle(h2), *stride as u64 * *h as u64).ok())
                            .map(|p| Box::new(p) as Box<dyn Pixels>),
                        _ => None,
                    };
                    c.client_msg(cid, m, px, now);
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
                    c.caller_msg(id, name, m);
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
        let (target, data) = match o {
            Out::Client(cid, m) => {
                let Some((&id, _)) = self.peers.iter().find(|(_, p)| p.kind == Kind::Client(cid)) else { return };
                (id, m.encode())
            }
            Out::Dialog(id, r) => (id, r.encode()),
            Out::Caller(id, r) => (id, r.encode()),
            Out::Inspect(id, r) => (id, r.encode()),
            _ => return,
        };
        if let Some(p) = self.peers.get_mut(&target)
            && let Err(e) = p.ch.write(&data, Vec::new())
        {
            // Plna fronta: klient nestiha cist. Zavreny kanal: konec.
            if e == libos::Error::PeerClosed {
                p.closed = true;
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
