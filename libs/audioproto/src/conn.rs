//! Klient zvukove sluzby pro programy (feature `client`).
//!
//! ```ignore
//! let audio = AudioClient::new(chan);
//! let s = audio.open(Direction::Output, Format::new(48_000, 2, Sample::S16), 50_000)?;
//! s.write_all(&pcm, deadline)?;   // vzorky jdou rovnou do sdileneho bufferu
//! s.start()?;
//! s.drain()?;
//! ```
//!
//! Volani jsou postupna (jeden pozadavek, jedna odpoved); klient neni
//! sdileny mezi vlakny.

use alloc::vec::Vec;
use core::cell::Cell;
use core::time::Duration;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Event, Handle, Mapping, Signals, Vmo, time};

use crate::client::{Opened, Request, Status, StreamInfo};
use crate::ring::{self, Ring};
use crate::{Direction, Error, Format, Result, decode_reply};

/// Jak dlouho cekat na odpoved sluzby (krome `DRAIN`).
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

pub struct AudioClient {
    chan: Channel,
}

impl AudioClient {
    pub fn new(chan: Channel) -> Self {
        Self { chan }
    }

    pub fn channel(&self) -> &Channel {
        &self.chan
    }

    fn call_until(&self, req: &Request, deadline: u64) -> Result<(Vec<u8>, Vec<Handle>)> {
        // Pozdni odpoved na pozadavek, kteremu driv vyprsel cas, zahodit.
        while self.chan.try_read().is_ok() {}
        self.chan.write(&req.encode(), Vec::new())?;
        let m = self.chan.read(Some(deadline))?;
        let data = decode_reply(&m.data)?.to_vec();
        Ok((data, m.handles))
    }

    fn call(&self, req: &Request) -> Result<(Vec<u8>, Vec<Handle>)> {
        self.call_until(req, time::deadline_after(REPLY_TIMEOUT))
    }

    pub fn status(&self) -> Result<Status> {
        Status::decode(&self.call(&Request::Status)?.0)
    }

    pub fn master_volume(&self, percent: u8) -> Result<()> {
        self.call(&Request::MasterVolume { percent }).map(|_| ())
    }

    pub fn mute(&self, on: bool) -> Result<()> {
        self.call(&Request::Mute { on }).map(|_| ())
    }

    /// Novy samostatny kanal ke sluzbe (pro jiny proces).
    pub fn connect(&self) -> Result<Channel> {
        let (_, hs) = self.call(&Request::Connect)?;
        hs.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)
    }

    /// Otevre stream; `latency_us` = velikost bufferu klienta.
    pub fn open(&self, dir: Direction, format: Format, latency_us: u32) -> Result<Stream<'_>> {
        let (data, hs) = self.call(&Request::Open { dir, format, latency_us })?;
        let info = Opened::decode(&data)?;
        let mut hs = hs.into_iter();
        let (Some(vmo), Some(event)) = (hs.next(), hs.next()) else {
            return Err(Error::Corrupted);
        };
        let vmo = Vmo::from_handle(vmo);
        let size = ring::vmo_size(info.capacity, info.format.frame_bytes()) as u64;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        // SAFETY: mapovani ma `size` bajtu, zacina na strance a zije v `Stream`.
        let ring = unsafe { Ring::attach(map.addr() as *mut u8, map.len() as usize)? };
        Ok(Stream { client: self, info, ring, _map: map, event: Event::from_handle(event), closed: false, started: Cell::new(false) })
    }
}

/// Otevreny stream. Pri zruseni se zavre (`CLOSE`).
pub struct Stream<'a> {
    client: &'a AudioClient,
    info: Opened,
    ring: Ring,
    _map: Mapping,
    event: Event,
    closed: bool,
    /// Poslali jsme START (a od te doby ne STOP ani dohrany DRAIN)?
    started: Cell<bool>,
}

impl Stream<'_> {
    pub fn info(&self) -> &Opened {
        &self.info
    }

    pub fn ring(&self) -> &Ring {
        &self.ring
    }

    pub fn start(&self) -> Result<()> {
        self.client.call(&Request::Start { stream: self.info.stream })?;
        self.started.set(true);
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.started.set(false);
        self.client.call(&Request::Stop { stream: self.info.stream }).map(|_| ())
    }

    pub fn volume(&self, percent: u8) -> Result<()> {
        self.client.call(&Request::StreamVolume { stream: self.info.stream, percent }).map(|_| ())
    }

    pub fn stream_info(&self) -> Result<StreamInfo> {
        StreamInfo::decode(&self.client.call(&Request::StreamInfo { stream: self.info.stream })?.0)
    }

    /// Pocka, az zazni vsechno zapsane (stream se pak zastavi).
    pub fn drain(&self) -> Result<()> {
        let left = self.info.format.us_for_frames(self.ring.capacity() as u64);
        let deadline = time::deadline_after(Duration::from_micros(left) + REPLY_TIMEOUT);
        // Po dohrani sluzba stream zastavi.
        self.started.set(false);
        self.client.call_until(&Request::Drain { stream: self.info.stream }, deadline).map(|_| ())
    }

    /// Pocka na udalost sluzby (perioda dohrala / nova data), nejdele do
    /// `deadline`.
    pub fn wait(&self, deadline: Option<u64>) -> Result<()> {
        libos::wait_one(self.event.handle(), Signals::SIGNALED, deadline)?;
        self.event.reset()
    }

    /// Zapise vsechna data (cele snimky); kdyz je buffer plny, ceka na
    /// misto. `start_when_full = true`: stream se spusti, jakmile se buffer
    /// poprve naplni (nebo na konci dat). `false`: stream se nespousti -
    /// volajici ho uz spustil, nebo ho spusti sam (`start`); data se musi
    /// vejit do bufferu, jinak cekani na misto vyprsi.
    pub fn write_all(&self, mut data: &[u8], start_when_full: bool, deadline: Option<u64>) -> Result<()> {
        let fb = self.ring.frame_bytes() as usize;
        // Uz bezici stream se znovu nespousti (zadny START na kazdy kousek).
        let mut started = !start_when_full || self.started.get();
        while data.len() >= fb {
            let _ = self.event.reset();
            let n = self.ring.write(data) as usize;
            data = &data[n * fb..];
            if data.len() < fb {
                break;
            }
            if !started {
                self.start()?;
                started = true;
            }
            if n == 0 {
                match libos::wait_one(self.event.handle(), Signals::SIGNALED, deadline) {
                    Ok(_) | Err(Error::ShouldWait) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        if !started {
            self.start()?;
        }
        Ok(())
    }

    /// Precte dostupna data (nahravani); vrati pocet snimku.
    pub fn read(&self, dst: &mut [u8]) -> u32 {
        self.ring.read(dst)
    }

    pub fn close(mut self) -> Result<()> {
        self.closed = true;
        self.client.call(&Request::Close { stream: self.info.stream }).map(|_| ())
    }
}

impl Drop for Stream<'_> {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.client.call(&Request::Close { stream: self.info.stream });
        }
    }
}
