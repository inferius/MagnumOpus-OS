//! Spojeni s ovladacem zvukove karty: vystupni a vstupni port (DMA
//! buffer sdileny s ovladacem, stranka stavu, udalost periody).
//!
//! Rozpocet latence (48 kHz, perioda 256 snimku = 5,33 ms):
//!
//! | cast | doba |
//! |---|---|
//! | mix pred pozici DMA (predstih, zacina na [`LEAD_MIN`] = 2 periody) | 5,3-10,7 ms |
//! | FIFO radice a kodeku | < 0,5 ms |
//! | buffer klienta (vychozi 10 ms, min. 5 ms) | 0-10 ms |
//! | celkem od zapisu klienta po reproduktor | asi 11-21 ms |
//!
//! **Adaptivni predstih:** kdyz sluzba nestihne (pozice DMA predbehne
//! smichana data - zazni ticho a pocita se `underruns`), predstih se
//! zvetsi o periodu, nejvys na [`LEAD_MAX`] (64 ms). Po [`LEAD_RELAX`]
//! bez podteceni se zase o periodu zmensi. Na rychlem stroji tak latence
//! zustane 11-21 ms, na zatizenem (napr. QEMU na vytizenem hostiteli,
//! ktery hostovi na desitky ms sebere procesor) se zvetsi misto
//! opakovanych mezer ve zvuku. DMA buffer ma [`PERIODS`] period, aby se
//! predstih mel kam zvetsit; za smichanymi daty se vzdy nuluje jedna
//! perioda, takze pri nestihnuti zazni ticho, ne stara data.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::device::{self, Info, OpenStream, StatusPage, op};
use audioproto::{Error, Format, Result, decode_reply};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Event, Handle, Mapping, Vmo, println, time};

use crate::streams::DevClock;

/// Snimku na periodu (5,33 ms pri 48 kHz; zaokrouhluje ovladac).
pub const PERIOD_FRAMES: u32 = 256;
pub const PERIODS: u32 = 16;
/// Predstih mixu pred pozici DMA (periody): od, do.
pub const LEAD_MIN: u32 = 2;
pub const LEAD_MAX: u32 = 12;
/// Po jak dlouhe dobe bez podteceni predstih zmensit o periodu.
pub const LEAD_RELAX: Duration = Duration::from_secs(10);

const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

pub fn call(chan: &Channel, msg: &[u8]) -> Result<(Vec<u8>, Vec<Handle>)> {
    // Stare zpravy (odpoved predchozi instance sluzby) zahodit.
    while chan.try_read().is_ok() {}
    chan.write(msg, Vec::new())?;
    let m = chan.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    Ok((decode_reply(&m.data)?.to_vec(), m.handles))
}

/// Zepta se ovladace na zarizeni.
pub fn query(chan: &Channel) -> Result<Info> {
    Info::decode(&call(chan, &device::simple(op::INFO))?.0)
}

/// Jeden smer zarizeni: DMA buffer, pozice, udalost.
pub struct Port {
    pub format: Format,
    pub period: u32,
    pub buffer: u32,
    _buf_vmo: Vmo,
    buf: Mapping,
    _status_vmo: Vmo,
    _status_map: Mapping,
    pub status: StatusPage,
    pub event: Event,
    /// Prehravani: kam az je smichano. Nahravani: kam az je precteno.
    pub pos: u64,
    pub running: bool,
}

impl Port {
    fn open(chan: &Channel, input: bool, format: Format) -> Result<Self> {
        let want = OpenStream { format, period: PERIOD_FRAMES, periods: PERIODS };
        let code = if input { op::OPEN_INPUT } else { op::OPEN_OUTPUT };
        let (data, hs) = call(chan, &want.encode(code))?;
        let got = device::decode_opened(&data)?;
        let mut hs = hs.into_iter();
        let (Some(buf), Some(status), Some(event)) = (hs.next(), hs.next(), hs.next()) else {
            return Err(Error::Corrupted);
        };
        let fb = got.format.frame_bytes();
        let buffer = got.period * got.periods;
        let buf_vmo = Vmo::from_handle(buf);
        let buf_map = Mapping::new(&buf_vmo, 0, (buffer * fb) as u64, MAP_READ | MAP_WRITE)?;
        let status_vmo = Vmo::from_handle(status);
        let status_map = Mapping::new(&status_vmo, 0, device::STATUS_SIZE as u64, MAP_READ | MAP_WRITE)?;
        // SAFETY: mapovani stranky stavu zije v `Port`.
        let st = unsafe { StatusPage::new(status_map.addr() as *mut u8) };
        if !st.valid() || st.buffer_frames() != buffer as u64 {
            return Err(Error::Corrupted);
        }
        Ok(Self {
            format: got.format,
            period: got.period,
            buffer,
            _buf_vmo: buf_vmo,
            buf: buf_map,
            _status_vmo: status_vmo,
            _status_map: status_map,
            status: st,
            event: Event::from_handle(event),
            pos: 0,
            running: false,
        })
    }

    pub fn frame_bytes(&self) -> u32 {
        self.format.frame_bytes()
    }

    /// Zapise snimky na absolutni pozici `pos` (kruh).
    pub fn write_at(&self, pos: u64, data: &[u8]) {
        let fb = self.frame_bytes() as usize;
        let cap = self.buffer as usize * fb;
        let start = (pos % self.buffer as u64) as usize * fb;
        let first = data.len().min(cap - start);
        self.buf.write_bytes(start as u64, &data[..first]);
        if first < data.len() {
            self.buf.write_bytes(0, &data[first..]);
        }
    }

    /// Precte snimky z absolutni pozice `pos` (kruh).
    pub fn read_at(&self, pos: u64, out: &mut [u8]) {
        let fb = self.frame_bytes() as usize;
        let cap = self.buffer as usize * fb;
        let start = (pos % self.buffer as u64) as usize * fb;
        let first = out.len().min(cap - start);
        self.buf.read_bytes(start as u64, &mut out[..first]);
        if first < out.len() {
            self.buf.read_bytes(0, &mut out[first..]);
        }
    }
}

pub struct Device {
    chan: Channel,
    pub info: Info,
    pub out: Port,
    /// Vstup se otevira az s prvnim nahravacim streamem.
    pub inp: Option<Port>,
    pub underruns: u64,
    pub overruns: u64,
    /// Aktualni predstih mixu (periody, adaptivni).
    pub lead: u32,
    /// Kdy naposled podteklo (nebo se predstih zmensil).
    pub calm_since: u64,
}

impl Device {
    /// Otevre vystup zarizeni ve formatu, ktery hlasi `info`.
    pub fn open(chan: Channel, info: Info) -> Result<Self> {
        let out = Port::open(&chan, false, info.format)?;
        println!(
            "audiod: zarizeni {} ({}), {} Hz, {} bitu, perioda {} snimku ({} us), buffer {} period{}",
            info.name,
            info.active_names(),
            out.format.rate,
            if out.format.frame_bytes() == 8 { 24 } else { 16 },
            out.period,
            out.format.us_for_frames(out.period as u64),
            out.buffer / out.period,
            if info.input { ", nahravani" } else { "" }
        );
        Ok(Self { chan, info, out, inp: None, underruns: 0, overruns: 0, lead: LEAD_MIN, calm_since: time::now() })
    }

    pub fn channel(&self) -> &Channel {
        &self.chan
    }

    pub fn clock(&self) -> DevClock {
        DevClock { rate: self.out.format.rate, period: self.out.period, latency_us: self.latency_us() }
    }

    /// Takt vstupu (otevre ho, kdyz jeste neni).
    pub fn input_clock(&mut self) -> Result<DevClock> {
        if !self.info.input {
            return Err(Error::NotSupported);
        }
        if self.inp.is_none() {
            // Vstup: 48 kHz, stereo, 16 bitu (format, ktery ovladac prijme).
            let f = Format::new(48_000, 2, audioproto::Sample::S16);
            self.inp = Some(Port::open(&self.chan, true, f)?);
        }
        let p = self.inp.as_ref().unwrap();
        Ok(DevClock { rate: p.format.rate, period: p.period, latency_us: p.format.us_for_frames(p.period as u64) as u32 })
    }

    /// Spusti vystup: buffer vynulovat a mixovat od aktualni pozice.
    pub fn start(&mut self) -> Result<()> {
        if self.out.running {
            return Ok(());
        }
        let len = (self.out.buffer * self.out.frame_bytes()) as usize;
        self.out.write_at(0, &vec![0u8; len]);
        self.out.pos = self.out.status.position();
        call(&self.chan, &device::simple(op::START))?;
        self.out.running = true;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if !self.out.running {
            return Ok(());
        }
        self.out.running = false;
        call(&self.chan, &device::simple(op::STOP)).map(|_| ())
    }

    pub fn start_input(&mut self) -> Result<()> {
        let Some(p) = self.inp.as_mut() else { return Err(Error::BadState) };
        if p.running {
            return Ok(());
        }
        p.pos = p.status.position();
        call(&self.chan, &device::simple(op::START_INPUT))?;
        p.running = true;
        Ok(())
    }

    pub fn stop_input(&mut self) -> Result<()> {
        let Some(p) = self.inp.as_mut() else { return Ok(()) };
        if !p.running {
            return Ok(());
        }
        p.running = false;
        call(&self.chan, &device::simple(op::STOP_INPUT)).map(|_| ())
    }

    /// Zavre streamy u ovladace (prepnuti na jine zarizeni).
    pub fn close(&mut self) {
        let _ = self.stop();
        let _ = self.stop_input();
        let _ = call(&self.chan, &device::simple(op::CLOSE));
        if self.inp.take().is_some() {
            let _ = call(&self.chan, &device::simple(op::CLOSE_INPUT));
        }
    }

    /// Latence od mixu po reproduktor (us): mix napred + FIFO.
    pub fn latency_us(&self) -> u32 {
        (self.out.format.us_for_frames((self.out.period * self.lead) as u64) + 500) as u32
    }

    /// Podteceni: predstih o periodu vetsi (nejvys [`LEAD_MAX`]).
    pub fn underrun(&mut self) {
        self.underruns += 1;
        self.calm_since = time::now();
        if self.lead < LEAD_MAX {
            self.lead += 1;
            println!("audiod: nestiham, predstih mixu {} period ({} us)", self.lead, self.latency_us());
        }
    }

    /// Dlouho bez podteceni: predstih o periodu mensi (nejmene [`LEAD_MIN`]).
    pub fn relax(&mut self) {
        let now = time::now();
        if self.lead > LEAD_MIN && now.saturating_sub(self.calm_since) > LEAD_RELAX.as_nanos() as u64 {
            self.lead -= 1;
            self.calm_since = now;
        }
    }
}
