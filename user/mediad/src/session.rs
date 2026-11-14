//! Jedna relace prehravani: kanal klienta, zdroj, proces dekoderu.
//!
//! Pozadavky klienta `mediad` zkontroluje a preposle dekoderu; udalosti
//! dekoderu po kontrole preposle klientovi. Behem cekani na odpoved
//! dekoderu obsluhuje jeho cteni zdroje. Pool bufferu a kruh zvuku si
//! relace pamatuje (kopie handlu), takze po padu dekoderu jde `REOPEN`
//! spustit novy se stejnym stavem.

use alloc::vec::Vec;
use core::time::Duration;

use libos::job::{Job, pool};
use libos::spawn::{Options, spawn_with};
use libos::{Channel, Error, Handle, Process, Result, Signals, Vmo, println};
use mediaproto::{AudioRing, Event, Incoming, Info, Request, decode_incoming, encode_reply, source_kind};

use crate::source::Source;

/// Limit uzivatelske pameti procesu dekoderu (buffery videa a kruh zvuku
/// patri aplikaci, dekoder je jen mapuje).
const DECODER_MEMORY: u64 = 64 << 20;
/// Limit pameti jadra dekoderu.
const DECODER_KERNEL: u64 = 4 << 20;
/// Nejdelsi odpoved dekoderu (otevreni cte hlavicku pres zdroj).
const DECODER_TIMEOUT: Duration = Duration::from_secs(10);

struct Decoder {
    ctl: Channel,
    /// Nase strana kanalu zdroje dekoderu.
    src: Channel,
    process: Process,
    job: Job,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.job.kill(0);
    }
}

/// Pool bufferu (popisy a kopie handlu) pro opakovane predani.
struct SavedPool {
    descs: Vec<bufdesc::BufDesc>,
    handles: Vec<Handle>,
}

struct Session {
    client: Channel,
    image: &'static [u8],
    source: Option<Source>,
    size: u64,
    dec: Option<Decoder>,
    info: Option<Info>,
    select: Option<(i32, i32, i32)>,
    pool: Option<SavedPool>,
    audio: Option<(AudioRing, Handle, Handle)>,
}

pub fn run(client: Channel, image: &'static [u8]) {
    let mut s = Session { client, image, source: None, size: 0, dec: None, info: None, select: None, pool: None, audio: None };
    loop {
        let mut items: Vec<(&Handle, Signals)> = Vec::new();
        items.push((s.client.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        if let Some(d) = s.dec.as_ref() {
            items.push((d.ctl.handle(), Signals::READABLE | Signals::PEER_CLOSED));
            items.push((d.src.handle(), Signals::READABLE));
            items.push((d.process.handle(), Signals::TERMINATED));
        }
        let observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(_) => return,
        };
        drop(items);
        if observed.len() > 1 {
            s.pump_decoder();
        }
        loop {
            match s.client.try_read() {
                Ok(m) => {
                    let (reply, quit) = match Request::decode(&m.data) {
                        Ok(req) => s.request(req, m.handles),
                        Err(e) => (Err(e), false),
                    };
                    if s.client.write(&encode_reply(reply), Vec::new()).is_err() || quit {
                        return;
                    }
                }
                Err(Error::ShouldWait) => break,
                // Klient zavrel kanal: konec relace (Drop zabije dekoder).
                Err(_) => return,
            }
        }
    }
}

impl Session {
    /// Obslouzi zdroj a udalosti dekoderu bez cekani; pri jeho konci to
    /// ohlasi klientovi.
    fn pump_decoder(&mut self) {
        let Some(d) = self.dec.as_ref() else { return };
        while let Ok(m) = d.src.try_read() {
            let reply = self.source.as_ref().map(|s| s.serve(&m.data)).unwrap_or_default();
            let _ = d.src.write(&reply, Vec::new());
        }
        loop {
            match d.ctl.try_read() {
                Ok(m) => {
                    // Odpoved mimo pozadavek dekoder poslat nema: zahodit.
                    if let Ok(Incoming::Event(e)) = decode_incoming(&m.data) {
                        self.forward(e);
                    }
                }
                Err(Error::ShouldWait) => break,
                Err(_) => return self.lost(),
            }
        }
        let d = self.dec.as_ref().unwrap();
        if libos::wait_one(d.process.handle(), Signals::TERMINATED, Some(0)).is_ok() {
            self.lost();
        }
    }

    /// Preposle udalost dekoderu po kontrole (dekoder neni duveryhodny).
    fn forward(&self, e: Event) {
        if let Event::Frame { buffer, .. } = e {
            if self.pool.as_ref().is_none_or(|p| buffer as usize >= p.descs.len()) {
                return;
            }
        }
        if let Event::Error { decoder_lost: true, .. } = e {
            return;
        }
        let _ = self.client.write(&e.encode(), Vec::new());
    }

    /// Dekoder skoncil (pad, zabiti): relace zustava, `REOPEN` ho obnovi.
    fn lost(&mut self) {
        if self.dec.take().is_some() {
            println!("mediad: dekoder relace skoncil");
            let e = Event::Error { code: Error::PeerClosed.code(), decoder_lost: true };
            let _ = self.client.write(&e.encode(), Vec::new());
        }
    }

    /// Vrati (odpoved, konec relace).
    fn request(&mut self, req: Request, hs: Vec<Handle>) -> (Result<Vec<u8>>, bool) {
        let r = match req {
            Request::Open { kind, size } => self.open(kind, size, hs),
            Request::Reopen => self.reopen(),
            Request::Close => {
                if self.dec.is_some() {
                    let _ = self.call(&Request::Close, Vec::new());
                }
                self.dec = None;
                return (Ok(Vec::new()), true);
            }
            Request::Connect | Request::Probe { .. } => Err(Error::NotSupported),
            Request::Select { video, audio, subtitle } => self.select(video, audio, subtitle),
            Request::Pool { ref descs } => self.pool(descs.clone(), hs),
            Request::Audio(ring) => self.audio(ring, hs),
            Request::Play | Request::Pause | Request::Seek { .. } | Request::Release { .. } => {
                if !hs.is_empty() {
                    Err(Error::InvalidArgs)
                } else {
                    self.call(&req, Vec::new())
                }
            }
        };
        (r, false)
    }

    fn select(&mut self, video: i32, audio: i32, subtitle: i32) -> Result<Vec<u8>> {
        let n = self.info.as_ref().ok_or(Error::BadState)?.streams.len();
        let ok = |i: i32| i == -1 || (i as usize) < n;
        if !(ok(video) && ok(audio) && ok(subtitle)) {
            return Err(Error::InvalidArgs);
        }
        let r = self.call(&Request::Select { video, audio, subtitle }, Vec::new())?;
        self.select = Some((video, audio, subtitle));
        Ok(r)
    }

    fn open(&mut self, kind: u8, size: u64, mut hs: Vec<Handle>) -> Result<Vec<u8>> {
        if self.source.is_some() {
            return Err(Error::AlreadyExists);
        }
        let h = match (hs.pop(), hs.is_empty()) {
            (Some(h), true) => h,
            _ => return Err(Error::InvalidArgs),
        };
        let source = match kind {
            source_kind::VMO => Source::Vmo { vmo: Vmo::from_handle(h), size },
            source_kind::CHANNEL => Source::App(Channel::from_handle(h)),
            _ => return Err(Error::InvalidArgs),
        };
        self.size = source.size()?;
        self.source = Some(source);
        let r = self.start_decoder();
        if r.is_err() {
            self.source = None;
        }
        r
    }

    /// Spusti dekoder, preda mu zdroj a vrati `Info`.
    fn start_decoder(&mut self) -> Result<Vec<u8>> {
        let job = Job::create(&libos::job::current()?)?;
        job.set_limit(pool::USER, DECODER_MEMORY)?;
        job.set_limit(pool::KERNEL, DECODER_KERNEL)?;
        let (process, ctl) = spawn_with(&job, "mediatest", self.image, Vec::new(), Options::default())?;
        let (src, theirs) = Channel::create()?;
        self.dec = Some(Decoder { ctl, src, process, job });
        let d = self.call(&Request::Probe { size: self.size }, alloc::vec![theirs.into_handle()])?;
        let info = Info::decode(&d)?;
        self.info = Some(info);
        Ok(d)
    }

    fn reopen(&mut self) -> Result<Vec<u8>> {
        if self.dec.is_some() || self.source.is_none() {
            return Err(Error::BadState);
        }
        let d = self.start_decoder()?;
        if let Some((v, a, s)) = self.select {
            self.call(&Request::Select { video: v, audio: a, subtitle: s }, Vec::new())?;
        }
        if let Some(p) = self.pool.as_ref() {
            let hs = dup_all(&p.handles)?;
            self.call(&Request::Pool { descs: p.descs.clone() }, hs)?;
        }
        if let Some((ring, vmo, ev)) = self.audio.as_ref() {
            let (ring, hs) = (*ring, alloc::vec![vmo.duplicate(None)?, ev.duplicate(None)?]);
            self.call(&Request::Audio(ring), hs)?;
        }
        Ok(d)
    }

    fn pool(&mut self, descs: Vec<bufdesc::BufDesc>, hs: Vec<Handle>) -> Result<Vec<u8>> {
        if hs.len() != descs.len() + 1 {
            return Err(Error::InvalidArgs);
        }
        let copies = dup_all(&hs)?;
        let r = self.call(&Request::Pool { descs: descs.clone() }, copies)?;
        self.pool = Some(SavedPool { descs, handles: hs });
        Ok(r)
    }

    fn audio(&mut self, ring: AudioRing, mut hs: Vec<Handle>) -> Result<Vec<u8>> {
        if hs.len() != 2 {
            return Err(Error::InvalidArgs);
        }
        let copies = dup_all(&hs)?;
        let r = self.call(&Request::Audio(ring), copies)?;
        let ev = hs.pop().unwrap();
        let vmo = hs.pop().unwrap();
        self.audio = Some((ring, vmo, ev));
        Ok(r)
    }

    /// Pozadavek dekoderu; mezitim obsluhuje jeho zdroj a preposila
    /// udalosti. Pad dekoderu = `PEER_CLOSED` (a udalost klientovi).
    fn call(&mut self, req: &Request, hs: Vec<Handle>) -> Result<Vec<u8>> {
        let d = self.dec.as_ref().ok_or(Error::BadState)?;
        d.ctl.write(&req.encode(), hs)?;
        let deadline = libos::time::deadline_after(DECODER_TIMEOUT);
        loop {
            let d = self.dec.as_ref().ok_or(Error::PeerClosed)?;
            while let Ok(m) = d.src.try_read() {
                let reply = self.source.as_ref().map(|s| s.serve(&m.data)).unwrap_or_default();
                let _ = d.src.write(&reply, Vec::new());
            }
            match d.ctl.try_read() {
                Ok(m) => match decode_incoming(&m.data) {
                    Ok(Incoming::Reply(r)) => return r.map(|d| d.to_vec()),
                    Ok(Incoming::Event(e)) => self.forward(e),
                    Err(_) => {}
                },
                Err(Error::ShouldWait) => {
                    let items = [
                        (d.ctl.handle(), Signals::READABLE | Signals::PEER_CLOSED),
                        (d.src.handle(), Signals::READABLE),
                    ];
                    match libos::wait_many(&items, Some(deadline)) {
                        Ok(_) => {}
                        Err(Error::TimedOut) => {
                            // Zaseknuty dekoder: zabit, relace jde obnovit.
                            self.lost();
                            return Err(Error::TimedOut);
                        }
                        Err(e) => return Err(e),
                    }
                }
                Err(_) => {
                    self.lost();
                    return Err(Error::PeerClosed);
                }
            }
        }
    }
}

fn dup_all(hs: &[Handle]) -> Result<Vec<Handle>> {
    hs.iter().map(|h| h.duplicate(None)).collect()
}
