//! Klient relace medii nad kanalem (aplikace, testy).
//!
//! Udalosti, ktere prijdou behem cekani na odpoved, se ulozi do fronty
//! a vrati je [`MediaClient::next_event`].

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::time::Duration;

use bufdesc::BufDesc;
use libos::{Channel, Handle, Rights, Vmo};

use crate::{AudioRing, Error, Event, Incoming, Info, Request, Result, decode_incoming, decode_u64, source_kind};

/// Jak dlouho cekat na odpoved (otevreni spousti proces dekoderu).
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

pub struct MediaClient {
    chan: Channel,
    events: RefCell<VecDeque<Event>>,
}

impl MediaClient {
    /// Kanal relace (z `CONNECT` na ridicim kanalu `mediad`).
    pub fn new(chan: Channel) -> Self {
        Self { chan, events: RefCell::new(VecDeque::new()) }
    }

    /// Kanal pro cekani v cizi smycce (`READABLE` = udalost nebo odpoved).
    pub fn channel(&self) -> &Channel {
        &self.chan
    }

    /// Posle pozadavek a pocka na odpoved; udalosti mezitim jdou do fronty.
    pub fn call(&self, req: &Request, handles: Vec<Handle>) -> Result<(Vec<u8>, Vec<Handle>)> {
        self.chan.write(&req.encode(), handles)?;
        let deadline = libos::time::deadline_after(CALL_TIMEOUT);
        loop {
            let m = self.chan.read(Some(deadline))?;
            match decode_incoming(&m.data)? {
                Incoming::Reply(r) => return r.map(|d| (d.to_vec(), m.handles)),
                Incoming::Event(e) => self.events.borrow_mut().push_back(e),
            }
        }
    }

    /// Novy kanal relace (jen na ridicim kanalu sluzby).
    pub fn connect(&self) -> Result<Channel> {
        let (_, mut hs) = self.call(&Request::Connect, Vec::new())?;
        let h = hs.pop().ok_or(Error::Corrupted)?;
        Ok(Channel::from_handle(h))
    }

    /// Otevre zdroj ve VMO (`size` bajtu). Sluzba dostane kopii handlu jen
    /// pro cteni.
    pub fn open_vmo(&self, vmo: &Vmo, size: u64) -> Result<Info> {
        let h = vmo.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))?;
        let (d, _) = self.call(&Request::Open { kind: source_kind::VMO, size }, alloc::vec![h])?;
        Info::decode(&d)
    }

    /// Otevre zdroj za kanalem protokolu [`crate::source`].
    pub fn open_channel(&self, source: Channel) -> Result<Info> {
        let (d, _) = self.call(&Request::Open { kind: source_kind::CHANNEL, size: 0 }, alloc::vec![source.into_handle()])?;
        Info::decode(&d)
    }

    pub fn select(&self, video: i32, audio: i32, subtitle: i32) -> Result<()> {
        self.call(&Request::Select { video, audio, subtitle }, Vec::new()).map(|_| ())
    }

    /// Pool bufferu videa: VMO v poradi popisu a plot snimku (sluzba
    /// dostane pravo plot posouvat).
    pub fn pool(&self, descs: Vec<BufDesc>, vmos: &[Vmo], fence: &libos::Fence) -> Result<()> {
        if descs.len() != vmos.len() {
            return Err(Error::InvalidArgs);
        }
        let mut hs = Vec::new();
        for v in vmos {
            hs.push(v.handle().duplicate(None)?);
        }
        hs.push(fence.handle().duplicate(None)?);
        self.call(&Request::Pool { descs }, hs).map(|_| ())
    }

    /// Kruh proudu `audiod` (VMO a udalost periody se predaji dekoderu).
    pub fn audio(&self, ring: AudioRing, vmo: Handle, event: Handle) -> Result<()> {
        self.call(&Request::Audio(ring), alloc::vec![vmo, event]).map(|_| ())
    }

    pub fn play(&self) -> Result<()> {
        self.call(&Request::Play, Vec::new()).map(|_| ())
    }

    pub fn pause(&self) -> Result<()> {
        self.call(&Request::Pause, Vec::new()).map(|_| ())
    }

    /// Vrati skutecny cas, od ktereho se pokracuje.
    pub fn seek(&self, pts_us: u64, exact: bool) -> Result<u64> {
        decode_u64(&self.call(&Request::Seek { pts_us, exact }, Vec::new())?.0)
    }

    pub fn release(&self, buffer: u8) -> Result<()> {
        self.call(&Request::Release { buffer }, Vec::new()).map(|_| ())
    }

    pub fn reopen(&self) -> Result<Info> {
        Info::decode(&self.call(&Request::Reopen, Vec::new())?.0)
    }

    pub fn close(&self) -> Result<()> {
        self.call(&Request::Close, Vec::new()).map(|_| ())
    }

    /// Dalsi udalost (z fronty, jinak z kanalu do `deadline`).
    pub fn next_event(&self, deadline: Option<u64>) -> Result<Event> {
        if let Some(e) = self.events.borrow_mut().pop_front() {
            return Ok(e);
        }
        loop {
            let m = self.chan.read(deadline)?;
            match decode_incoming(&m.data)? {
                Incoming::Event(e) => return Ok(e),
                // Odpoved bez pozadavku: protokolova chyba sluzby.
                Incoming::Reply(_) => return Err(Error::Corrupted),
            }
        }
    }

    /// Udalost bez cekani (`ShouldWait`, kdyz zadna neni).
    pub fn try_event(&self) -> Result<Event> {
        if let Some(e) = self.events.borrow_mut().pop_front() {
            return Ok(e);
        }
        let m = self.chan.try_read()?;
        match decode_incoming(&m.data)? {
            Incoming::Event(e) => Ok(e),
            Incoming::Reply(_) => Err(Error::Corrupted),
        }
    }
}
