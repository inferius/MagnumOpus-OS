//! Klient zvukove sluzby pro programy (feature `client`).
//!
//! ```ignore
//! let audio = AudioClient::new(chan);
//! let s = audio.open(Direction::Output, Format::new(48_000, 2, Sample::S16), 50_000)?;
//! s.write_all(&pcm, true, None)?;   // vzorky jdou rovnou do sdileneho bufferu
//! s.drain()?;
//! ```
//!
//! Spojeni je `svcclient::Conn`: s listkem (`with_lease`) prezije restart
//! `audiod` - dalsi volani pozada supervisor o novou relaci. Otevrene
//! streamy restart neprezij: jejich volani pak vrati `SERVICE_RESTARTED`
//! nebo `NOT_FOUND` a program otevre novy stream. Klient i stream jdou
//! sdilet mezi vlakny (volani se radi za sebe).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Event, Handle, Mapping, Signals, Vmo};
use svcclient::{Conn, Kind};

use crate::client::{DeviceEntry, Opened, Request, Status, StreamInfo};
use crate::ring::{self, Ring};
use crate::{Direction, Error, Format, Result, decode_reply};

#[derive(Clone)]
pub struct AudioClient {
    conn: Arc<Conn>,
}

fn kind(req: &Request) -> Kind {
    match req {
        Request::Status | Request::StreamInfo { .. } | Request::Devices => Kind::Idempotent,
        _ => Kind::Effect,
    }
}

impl AudioClient {
    /// Spojeni bez obnovy (pad sluzby = `SERVICE_UNAVAILABLE`).
    pub fn new(chan: Channel) -> Self {
        Self { conn: Arc::new(Conn::new("audiod", chan, None)) }
    }

    /// Spojeni s listkem: po restartu sluzby se samo obnovi.
    pub fn with_lease(chan: Channel, lease: Channel) -> Self {
        Self { conn: Arc::new(Conn::new("audiod", chan, Some(lease))) }
    }

    pub fn conn(&self) -> &Conn {
        &self.conn
    }

    /// Posle pozadavek a vrati data a handly odpovedi.
    pub fn call(&self, req: &Request) -> Result<(Vec<u8>, Vec<Handle>)> {
        let m = self.conn.call(kind(req), &req.encode(), Vec::new())?;
        let data = decode_reply(&m.data)?.to_vec();
        Ok((data, m.handles))
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

    /// Zvukova zarizeni sluzby.
    pub fn devices(&self) -> Result<Vec<DeviceEntry>> {
        DeviceEntry::decode_list(&self.call(&Request::Devices)?.0)
    }

    /// Prepne vystup (a vstup) na zarizeni `index` ze seznamu.
    pub fn select(&self, index: u8) -> Result<()> {
        self.call(&Request::Select { index }).map(|_| ())
    }

    /// Novy samostatny kanal ke sluzbe (pro jiny proces, bez listku).
    pub fn connect(&self) -> Result<Channel> {
        let (_, hs) = self.call(&Request::Connect)?;
        hs.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)
    }

    /// Relace s vlastnim listkem pro jiny proces (prezije restart sluzby
    /// jako tato). Jen kdyz tato relace listek ma.
    pub fn narrow(&self) -> Result<(Channel, Channel)> {
        self.conn.narrow(&[])
    }

    /// Otevre stream; `latency_us` = velikost bufferu klienta.
    pub fn open(&self, dir: Direction, format: Format, latency_us: u32) -> Result<Stream> {
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
        Ok(Stream {
            client: self.clone(),
            info,
            ring,
            _map: map,
            event: Event::from_handle(event),
            closed: false,
            started: AtomicBool::new(false),
        })
    }
}

/// Otevreny stream. Pri zruseni se zavre (`CLOSE`).
pub struct Stream {
    client: AudioClient,
    info: Opened,
    ring: Ring,
    _map: Mapping,
    event: Event,
    closed: bool,
    /// Poslali jsme START (a od te doby ne STOP ani dohrany DRAIN)?
    started: AtomicBool,
}

impl Stream {
    pub fn info(&self) -> &Opened {
        &self.info
    }

    pub fn ring(&self) -> &Ring {
        &self.ring
    }

    pub fn start(&self) -> Result<()> {
        self.client.call(&Request::Start { stream: self.info.stream })?;
        self.started.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.started.store(false, Ordering::Relaxed);
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
        self.started.store(false, Ordering::Relaxed);
        self.client.call(&Request::Drain { stream: self.info.stream }).map(|_| ())
    }

    /// Pocka na udalost sluzby (perioda dohrala / nova data), nejdele do
    /// `deadline`.
    pub fn wait(&self, deadline: Option<u64>) -> Result<()> {
        libos::wait_one(self.event.handle(), Signals::SIGNALED, deadline)?;
        self.event.reset()
    }

    /// Zapise vsechna data (cele snimky); kdyz je buffer plny, ceka na
    /// misto. `start_when_full = true`: stream se spusti, jakmile se buffer
    /// poprve naplni (nebo na konci dat); uz bezici stream se znovu
    /// nespousti. `false`: stream se nespousti - volajici ho uz spustil,
    /// nebo ho spusti sam (`start`); data se pak musi vejit do bufferu,
    /// jinak cekani na misto vyprsi.
    pub fn write_all(&self, mut data: &[u8], start_when_full: bool, deadline: Option<u64>) -> Result<()> {
        let fb = self.ring.frame_bytes() as usize;
        let mut started = !start_when_full || self.started.load(Ordering::Relaxed);
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

impl Drop for Stream {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.client.call(&Request::Close { stream: self.info.stream });
        }
    }
}
