//! Stav systemu v liste: sit (`netd`, relace jen s pravem `INFO`)
//! a hlasitost (`audiod`). Kompozitor se nesmi zablokovat na cizi sluzbe:
//! dotaz se posle, odpoved se precte, az prijde (cekani spolu s ostatnimi
//! kanaly), dalsi dotaz za [`PERIOD`].

use alloc::vec::Vec;

use libos::{Channel, Error};

use crate::compd_now;

/// Jak casto se ptat na stav.
pub const PERIOD: u64 = 2_000_000_000;
/// Bez odpovedi tak dlouho: zkusit znovu (sluzba mohla zpravu zahodit).
const RETRY: u64 = 10_000_000_000;

/// Jeden zdroj stavu: kanal a pozadavek, ktery se opakuje.
pub struct Source {
    pub ch: Channel,
    request: Vec<u8>,
    /// Kdy se zeptat znovu; `pending` = dotaz je na ceste od `sent`.
    next: u64,
    pending: Option<u64>,
}

impl Source {
    pub fn new(ch: Channel, request: Vec<u8>) -> Self {
        Self { ch, request, next: 0, pending: None }
    }

    /// Posle zpravu sluzbe (zadost panelu) a stav precte co nejdriv.
    pub fn send(&mut self, msg: &[u8]) -> bool {
        let ok = self.ch.write(msg, Vec::new()).is_ok();
        self.pending = None;
        self.next = 0;
        ok
    }

    /// Posle dotaz, kdyz je cas; vraci, kdy se probudit.
    pub fn poll(&mut self, now: u64) -> Option<u64> {
        if let Some(sent) = self.pending {
            if now < sent + RETRY {
                return Some(sent + RETRY);
            }
            self.pending = None;
        }
        if now >= self.next {
            if self.ch.write(&self.request, Vec::new()).is_ok() {
                self.pending = Some(now);
            }
            self.next = now + PERIOD;
        }
        Some(self.next)
    }

    /// Precte odpoved (bez cekani): `Ok(Some)` = novy stav, `Ok(None)` =
    /// nic neprislo, `Err` = sluzba skoncila.
    pub fn read<T>(&mut self, parse: fn(&[u8]) -> Option<T>) -> Result<Option<T>, ()> {
        let mut out = None;
        loop {
            match self.ch.try_read() {
                Ok(m) => {
                    self.pending = None;
                    self.next = compd_now() + PERIOD;
                    if let Some(v) = parse(&m.data) {
                        out = Some(v);
                    }
                }
                Err(Error::ShouldWait) => return Ok(out),
                Err(_) => return Err(()),
            }
        }
    }
}

/// Dotaz `INFO` (netproto v2: hlavicka s cislem dotazu). Cislo je pevne:
/// dalsi dotaz jde az po odpovedi nebo po `RETRY`; opakovany dotaz
/// s cislem, ktere jeste ceka, sluzba odmitne a stav prijde z prvniho.
pub fn net_request() -> Vec<u8> {
    let h = netproto::Header { op: netproto::op::INFO, id: 1, deadline: netproto::NO_DEADLINE };
    netproto::msg::request(h, &[])
}

/// Prehled pro listu a panel Zvuk (`OVERVIEW`: stav, zarizeni, aplikace).
pub fn audio_request() -> Vec<u8> {
    audioproto::client::Request::Overview.encode()
}

pub fn net_state(data: &[u8]) -> Option<comp::render::NetState> {
    let mut r = netproto::parse_reply(data).ok()?;
    let i = netproto::Info::get(&mut r).ok()?;
    // Bez rozhrani netd hlasi nulovou MAC (karta chybi nebo ji neumime).
    Some(if i.mac == [0; 6] {
        comp::render::NetState::NoDevice
    } else if !i.link_up {
        comp::render::NetState::Offline
    } else if i.configured {
        comp::render::NetState::Online
    } else {
        comp::render::NetState::Connecting
    })
}

/// Hlasitost, ztlumeni a nahravajici aplikace ze stavu `audiod`.
/// Hlasitost, kdo nahrava a co ukazat v panelu Zvuk. Odpovedi na zadosti
/// panelu (bez tela) a jine zpravy vrati `None`.
pub fn volume(data: &[u8]) -> Option<AudioState> {
    let body = audioproto::decode_reply(data).ok()?;
    let o = audioproto::client::Overview::decode(body).ok()?;
    let info = comp::render::AudioInfo {
        devices: o
            .devices
            .into_iter()
            .map(|d| comp::render::AudioDev { index: d.index, name: d.name, outputs: d.outputs, input: d.input, active: d.active })
            .collect(),
        apps: o
            .apps
            .into_iter()
            .map(|a| comp::render::AudioApp { id: a.app_id, name: a.name, volume: a.volume, playing: a.playing, capturing: a.capturing })
            .collect(),
        input: o.input,
    };
    Some(((o.status.master, o.status.muted), o.status.capturing, info))
}

pub type AudioState = ((u8, bool), alloc::vec::Vec<alloc::string::String>, comp::render::AudioInfo);

/// Zadost panelu Zvuk jako zprava sluzbe zvuku.
pub fn audio_control(req: &comp::AudioReq) -> Vec<u8> {
    use audioproto::client::Request;
    match req {
        comp::AudioReq::Master(p) => Request::MasterVolume { percent: *p },
        comp::AudioReq::Mute(on) => Request::Mute { on: *on },
        comp::AudioReq::Output(i) => Request::Select { index: *i },
        comp::AudioReq::Input(i) => Request::SelectInput { index: *i },
        comp::AudioReq::App(app, p) => Request::AppVolume { app: app.clone(), percent: *p },
    }
    .encode()
}
