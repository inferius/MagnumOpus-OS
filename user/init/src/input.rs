//! Sluzba vstupu: start `inputd` a klient ridiciho protokolu.
//!
//! Init je zatim vlastnik fokusu i prijemce bezpecne zkratky (testy).
//! Pozdeji preda fokus konzoli / kompozitoru a kanal `secure` sluzbe
//! prihlaseni.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use inputproto::control::{self, Status};
use inputproto::{Record, event};
use libos::initfs::Initfs;
use libos::{Channel, Error, Handle, Process, Result, Startup, println, time};

/// Jak dlouho cekat na odpoved `inputd`.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Input {
    pub _process: Process,
    pub control: Channel,
    /// Vlastnik fokusu: sem chodi udalosti.
    pub focus: Channel,
    /// Prijemce bezpecne zkratky.
    pub secure: Channel,
}

/// Spusti `inputd`, pripoji k ni ovladace `devices` a nastavi fokus
/// a kanal `secure` na init.
pub fn start(startup: &Startup, devices: Vec<Channel>) -> Result<Input> {
    let fs = Initfs::open(startup)?;
    let image = fs.find("inputd").ok_or(Error::NotFound)?;
    let (process, control) = libos::spawn("inputd", image, Vec::new())?;
    let (focus, focus_srv) = Channel::create()?;
    let (secure, secure_srv) = Channel::create()?;
    let input = Input {
        _process: process,
        control,
        focus,
        secure,
    };
    let count = devices.len();
    for d in devices {
        input.command(control::DEVICE, vec![d.into_handle()])?;
    }
    input.command(control::FOCUS, vec![focus_srv.into_handle()])?;
    input.command(control::SECURE, vec![secure_srv.into_handle()])?;
    println!("init: sluzba vstupu bezi ({count} zarizeni)");
    Ok(input)
}

impl Input {
    /// Posle prikaz a vrati odpoved (text za `ok`) s handly; `err KOD`
    /// prevede na chybu.
    ///
    /// Odpovedi nemaji cislo pozadavku: zprava, ktera uz ceka pred
    /// odeslanim, je pozdni odpoved na prikaz, kteremu vyprsel cas, a zahodi
    /// se (jinak by se odpovedi posunuly o jednu).
    pub fn command(&self, text: &str, handles: Vec<Handle>) -> Result<(String, Vec<Handle>)> {
        drain(&self.control);
        self.control.write(text.as_bytes(), handles)?;
        let msg = self
            .control
            .read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
        let reply = core::str::from_utf8(&msg.data).map_err(|_| Error::Corrupted)?;
        if let Some(code) = reply.strip_prefix("err ") {
            return Err(libos::abi::error::ALL
                .iter()
                .copied()
                .find(|e| e.name() == code)
                .unwrap_or(Error::Corrupted));
        }
        let rest = reply.strip_prefix("ok").ok_or(Error::Corrupted)?;
        Ok((String::from(rest.trim()), msg.handles))
    }

    pub fn status(&self) -> Result<Status> {
        let (text, _) = self.command(control::STATUS, Vec::new())?;
        Status::parse(&text).ok_or(Error::Corrupted)
    }

    /// Novy zdroj umeleho vstupu (kanal protokolu `inputproto::device`).
    pub fn inject(&self) -> Result<Channel> {
        let (_, mut h) = self.command(control::INJECT, Vec::new())?;
        if h.len() != 1 {
            return Err(Error::Corrupted);
        }
        Ok(Channel::from_handle(h.remove(0)))
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
