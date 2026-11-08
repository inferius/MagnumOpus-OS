//! Sluzba vstupu: start `inputd`, klient ridiciho protokolu a adapter
//! supervisoru pro obnovu po padu (`docs/cs/obnova-sluzeb.md`).
//!
//! Init je zatim vlastnik fokusu i prijemce bezpecne zkratky (testy).
//! V bezne behu je preda kompozitoru (`gui.rs`, `take_input`).
//!
//! **Obnova po padu**: vsechno, co `inputd` dostala (konce kanalu
//! k ovladacum, fokus, `secure`, rozlozeni, volby, `settled`), si init
//! pamatuje ve sdilenem stavu [`Live`]. Konce kanalu drzi v
//! [`Kept`]: sluzba dostane duplikat, takze ovladac ani vlastnik
//! fokusu pad nepoznaji. Nova instance dostane vse znovu, v puvodnim poradi
//! (zarizeni ze startu pred `settled`, zarizeni pripojena za behu po nem).
//! Zarizeni, ktera mela stara instance, prijdou jako `device sync`:
//! `inputd` jim posle `ISYN` a do odpovedi zahazuje stare zpravy.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use inputproto::control::{self, Status};
use inputproto::{Record, event};
use libos::initfs::Initfs;
use libos::{Channel, Error, Handle, Process, Result, Signals, Startup, println, time};

use crate::supervisor::{Kept, Service, StartCtx, Started};

/// Jak dlouho cekat na odpoved `inputd`.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// Konec kanalu k ovladaci (zdroji vstupu), ktery init drzi.
struct DeviceEnd {
    end: Kept,
    /// Dostala ho uz nejaka instance (pak je treba synchronizace).
    given: bool,
    /// Pripojeny pred `settled` (duveryhodny bez overeni).
    boot: bool,
}

impl DeviceEnd {
    /// Ovladac uz skoncil (zarizeni odpojeno)?
    fn gone(&self) -> bool {
        match self.end.give() {
            Ok(c) => libos::wait_one(c.handle(), Signals::PEER_CLOSED, Some(0)).is_ok(),
            Err(_) => true,
        }
    }
}

/// Stav sluzby vstupu sdileny klientem ([`Input`]) a adapterem
/// ([`InputdService`]).
#[derive(Default)]
struct Live {
    /// Ridici kanal bezici instance (`None` = nebezi nebo startuje).
    control: Option<Channel>,
    /// Proces bezici instance.
    process: Option<Process>,
    devices: Vec<DeviceEnd>,
    focus: Option<Kept>,
    secure: Option<Kept>,
    layout: Option<String>,
    verify_new: Option<bool>,
    settled: bool,
    /// Startujici instance: ridici kanal a pocet odpovedi, na ktere se
    /// ceka (prikazy obnovy se posilaji najednou, adapter neblokuje).
    starting: Option<(Channel, usize)>,
}

impl Live {
    /// Zahodi konce odpojenych zarizeni.
    fn prune(&mut self) {
        self.devices.retain(|d| !d.gone());
    }
}

type Shared = Arc<spin::Mutex<Live>>;

/// Klient sluzby vstupu.
pub struct Input {
    /// Vlastnik fokusu (init): sem chodi udalosti.
    pub focus: Channel,
    /// Prijemce bezpecne zkratky (init).
    pub secure: Channel,
    live: Shared,
}

/// Spusti `inputd` (generace 1), pripoji k ni ovladace `devices` a nastavi
/// fokus a kanal `secure` na init. Vraci klienta a proces sluzby (pro
/// registraci v supervisoru, viz [`Input::service`]).
pub fn start(startup: &Startup, devices: Vec<Channel>) -> Result<(Input, Process)> {
    let (process, control) = spawn(startup)?;
    let (focus, focus_srv) = Channel::create()?;
    let (secure, secure_srv) = Channel::create()?;
    let live = Live {
        control: Some(control),
        ..Live::default()
    };
    let input = Input {
        focus,
        secure,
        live: Arc::new(spin::Mutex::new(live)),
    };
    let count = devices.len();
    for d in devices {
        input.command(control::DEVICE, vec![d.into_handle()])?;
    }
    input.command(control::FOCUS, vec![focus_srv.into_handle()])?;
    input.command(control::SECURE, vec![secure_srv.into_handle()])?;
    // Zarizeni nalezena pri startu jsou duveryhodna; dalsi klavesnice muze
    // cekat na overeni (volba zabezpeceni).
    input.command(control::SETTLED, Vec::new())?;
    println!("init: sluzba vstupu bezi ({count} zarizeni)");
    Ok((input, process))
}

fn spawn(startup: &Startup) -> Result<(Process, Channel)> {
    let fs = Initfs::open(startup)?;
    let image = fs.find("inputd").ok_or(Error::NotFound)?;
    libos::spawn("inputd", image, Vec::new())
}

/// Posle prikaz a vrati odpoved (text za `ok`) s handly; `err KOD`
/// prevede na chybu.
///
/// Odpovedi nemaji cislo pozadavku: zprava, ktera uz ceka pred odeslanim,
/// je pozdni odpoved na prikaz, kteremu vyprsel cas, a zahodi se (jinak by
/// se odpovedi posunuly o jednu).
fn request(control: &Channel, text: &str, handles: Vec<Handle>) -> Result<(String, Vec<Handle>)> {
    drain(control);
    control.write(text.as_bytes(), handles)?;
    let msg = control.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    let rest = parse_reply(&msg.data)?;
    Ok((String::from(rest), msg.handles))
}

/// Odpoved `ok [data]` -> data, `err KOD` -> chyba.
fn parse_reply(data: &[u8]) -> Result<&str> {
    let reply = core::str::from_utf8(data).map_err(|_| Error::Corrupted)?;
    if let Some(code) = reply.strip_prefix("err ") {
        return Err(libos::abi::error::ALL
            .iter()
            .copied()
            .find(|e| e.name() == code)
            .unwrap_or(Error::Corrupted));
    }
    Ok(reply.strip_prefix("ok").ok_or(Error::Corrupted)?.trim())
}

/// Jeden handle z prikazu (kanal).
fn one(mut handles: Vec<Handle>) -> Result<Channel> {
    if handles.len() != 1 {
        return Err(Error::InvalidArgs);
    }
    Ok(Channel::from_handle(handles.remove(0)))
}

impl Input {
    /// Adapter pro supervisor (restart `inputd` po padu).
    pub fn service(&self) -> Box<dyn Service> {
        Box::new(InputdService(self.live.clone()))
    }

    /// Posle prikaz a vrati odpoved (text za `ok`) s handly; `err KOD`
    /// prevede na chybu.
    ///
    /// Co se ma po padu predat znovu (`device`, `focus`, `secure`,
    /// `layout`, `policy`, `settled`), si zapamatuje. Konce kanalu
    /// `device`, `focus` a `secure` si drzi ([`Kept`]) a sluzbe posle
    /// duplikat. Kdyz sluzba prave nebezi, zapamatovany prikaz dostane az
    /// nova instance a vrati se `ok`; ostatni prikazy vrati
    /// `SERVICE_UNAVAILABLE`.
    pub fn command(&self, text: &str, handles: Vec<Handle>) -> Result<(String, Vec<Handle>)> {
        let mut live = self.live.lock();
        let mut words = text.split_whitespace();
        let name = words.next().unwrap_or("");
        match (name, words.next()) {
            (control::DEVICE, None) => {
                let end = Kept::new(one(handles)?);
                live.prune();
                let boot = !live.settled;
                let given = match &live.control {
                    Some(c) => {
                        request(c, control::DEVICE, vec![end.give()?.into_handle()])?;
                        true
                    }
                    None => false,
                };
                live.devices.push(DeviceEnd { end, given, boot });
                Ok((String::new(), Vec::new()))
            }
            (control::FOCUS | control::SECURE, None) => {
                let end = Kept::new(one(handles)?);
                if let Some(c) = &live.control {
                    request(c, name, vec![end.give()?.into_handle()])?;
                }
                if name == control::FOCUS {
                    live.focus = Some(end);
                } else {
                    live.secure = Some(end);
                }
                Ok((String::new(), Vec::new()))
            }
            _ => {
                let c = live.control.as_ref().ok_or(Error::ServiceUnavailable)?;
                let r = request(c, text, handles)?;
                match (name, words.next()) {
                    (control::LAYOUT, _) => {
                        live.layout = text.split_whitespace().nth(1).map(String::from)
                    }
                    (control::POLICY, Some(v)) if text.contains(control::VERIFY_NEW_DEVICES) => {
                        live.verify_new = Some(v == "on")
                    }
                    (control::SETTLED, _) => live.settled = true,
                    _ => {}
                }
                Ok(r)
            }
        }
    }

    pub fn status(&self) -> Result<Status> {
        let (text, _) = self.command(control::STATUS, Vec::new())?;
        Status::parse(&text).ok_or(Error::Corrupted)
    }

    /// Novy zdroj umeleho vstupu (kanal protokolu `inputproto::device`).
    /// Pad sluzby ho neprezije (kanal vytvorila sluzba): zdroj dostane
    /// `PEER_CLOSED` a pozada o novy.
    pub fn inject(&self) -> Result<Channel> {
        let (_, h) = self.command(control::INJECT, Vec::new())?;
        one(h).map_err(|_| Error::Corrupted)
    }

    /// Id zdroje, jehoz jmeno zacina `prefix` (fyzicka zarizeni).
    pub fn source_id(&self, prefix: &str) -> Result<u16> {
        let (text, _) = self.command(control::SOURCES, Vec::new())?;
        control::SourceInfo::parse_list(&text)
            .find(|s| !s.synthetic && s.name.starts_with(prefix))
            .map(|s| s.id)
            .ok_or(Error::NotFound)
    }

    pub fn set_layout(&self, name: &str) -> Result<()> {
        let cmd = alloc::format!("{} {name}", control::LAYOUT);
        self.command(&cmd, Vec::new()).map(|_| ())
    }
}

/// Adapter `inputd` pro supervisor: nova instance dostane vse, co mela
/// predchozi (viz popis modulu).
struct InputdService(Shared);

impl InputdService {
    /// Posle prikaz startujici instanci (odpoved precte `ready`).
    fn send(c: &Channel, pending: &mut usize, text: &str, handles: Vec<Handle>) -> Result<()> {
        c.write(text.as_bytes(), handles)?;
        *pending += 1;
        Ok(())
    }

    /// Zarizeni, ktera jeste zadna instance nedostala (pripojena behem
    /// startu), nebo vsechna podle `filter`.
    fn send_devices(
        live: &mut Live,
        c: &Channel,
        pending: &mut usize,
        filter: impl Fn(&DeviceEnd) -> bool,
    ) -> Result<()> {
        for d in live.devices.iter_mut().filter(|d| filter(d)) {
            let cmd = if d.given {
                alloc::format!("{} {}", control::DEVICE, control::SYNC)
            } else {
                String::from(control::DEVICE)
            };
            Self::send(c, pending, &cmd, vec![d.end.give()?.into_handle()])?;
            d.given = true;
        }
        Ok(())
    }
}

impl InputdService {
    /// Posle nove instanci vse, co mela predchozi; vraci pocet odpovedi,
    /// na ktere se ceka.
    fn replay(live: &mut Live, c: &Channel, restart: bool) -> Result<usize> {
        live.prune();
        let mut n = 0;
        // Poradi jako pri startu systemu: zarizeni ze startu, fokus,
        // `secure`, `settled`, pak zarizeni pripojena za behu (s volbou
        // overeni cekaji znovu na overeni).
        if restart {
            Self::send(c, &mut n, control::RESTARTED, Vec::new())?;
        }
        if let Some(v) = live.verify_new {
            let cmd = alloc::format!(
                "{} {} {}",
                control::POLICY,
                control::VERIFY_NEW_DEVICES,
                if v { "on" } else { "off" }
            );
            Self::send(c, &mut n, &cmd, Vec::new())?;
        }
        if let Some(l) = &live.layout {
            let cmd = alloc::format!("{} {l}", control::LAYOUT);
            Self::send(c, &mut n, &cmd, Vec::new())?;
        }
        Self::send_devices(live, c, &mut n, |d| d.boot)?;
        if let Some(f) = &live.focus {
            Self::send(c, &mut n, control::FOCUS, vec![f.give()?.into_handle()])?;
        }
        if let Some(s) = &live.secure {
            Self::send(c, &mut n, control::SECURE, vec![s.give()?.into_handle()])?;
        }
        if live.settled {
            Self::send(c, &mut n, control::SETTLED, Vec::new())?;
        }
        Self::send_devices(live, c, &mut n, |d| !d.boot)?;
        Ok(n)
    }
}

impl Service for InputdService {
    fn start(&mut self, ctx: &StartCtx) -> Result<Started> {
        let (process, control) = spawn(ctx.startup)?;
        let mut guard = self.0.lock();
        let live = &mut *guard;
        let started = Self::replay(live, &control, ctx.restart).and_then(|n| {
            let s = Started {
                process: process.handle().duplicate(None)?,
                ready: Some(control.handle().duplicate(None)?),
            };
            Ok((s, n))
        });
        match started {
            Ok((s, n)) => {
                live.process = Some(process);
                live.starting = Some((control, n));
                Ok(s)
            }
            Err(e) => {
                let _ = process.kill();
                Err(e)
            }
        }
    }

    /// Odpovedi na prikazy obnovy. Chyba jednoho zarizeni (napr. plny
    /// seznam zdroju) start nezastavi, jen se zaloguje.
    fn ready(&mut self) -> Result<bool> {
        let mut guard = self.0.lock();
        let live = &mut *guard;
        let Some((c, pending)) = &mut live.starting else {
            return Err(Error::BadState);
        };
        loop {
            match c.try_read() {
                Ok(m) => {
                    *pending = pending.saturating_sub(1);
                    if let Err(e) = parse_reply(&m.data) {
                        println!("init: obnova sluzby vstupu: prikaz selhal: {e}");
                    }
                }
                Err(Error::ShouldWait) => break,
                Err(e) => return Err(e),
            }
        }
        if *pending > 0 {
            return Ok(false);
        }
        // Zarizeni pripojena behem startu.
        let (c, mut pending) = live.starting.take().ok_or(Error::BadState)?;
        Self::send_devices(live, &c, &mut pending, |d| !d.given)?;
        if pending > 0 {
            live.starting = Some((c, pending));
            return Ok(false);
        }
        println!("init: sluzba vstupu obnovena ({} zarizeni)", live.devices.len());
        live.control = Some(c);
        Ok(true)
    }

    fn stopped(&mut self) {
        let mut live = self.0.lock();
        live.control = None;
        live.starting = None;
        live.process = None;
    }
}

/// Zahodi vsechny cekajici zpravy kanalu.
pub fn drain(ch: &Channel) {
    while ch.try_read().is_ok() {}
}

/// Precte zaznamy z kanalu az do terminu `deadline`, dokud `done` nevrati
/// `true` (pak vrati vse prectene). Po terminu `Err` s chybou a vsim, co do
/// te doby prislo (pro diagnostiku testu).
pub fn read_until(
    ch: &Channel,
    deadline: u64,
    mut done: impl FnMut(&[Record]) -> bool,
) -> core::result::Result<Vec<Record>, (Error, Vec<Record>)> {
    let mut all = Vec::new();
    loop {
        if done(&all) {
            return Ok(all);
        }
        let msg = match ch.read(Some(deadline)) {
            Ok(m) => m,
            Err(e) => return Err((e, all)),
        };
        match event::decode(&msg.data) {
            Some(r) => all.extend(r),
            None => return Err((Error::Corrupted, all)),
        }
    }
}

/// Bezici instance sluzby vstupu (pro adaptery jinych sluzeb, napr.
/// predani fokusu kompozitoru po jeho restartu). Po zapojeni obnovy
/// `inputd` vrati vzdy aktualni instanci.
pub fn with_input<R>(f: impl FnOnce(&Input) -> R) -> Option<R> {
    crate::system::devices().and_then(|d| d.input.as_ref()).map(f)
}
