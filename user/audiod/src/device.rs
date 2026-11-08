//! Spojeni s ovladacem zvukove karty: otevreni vystupu, DMA buffer
//! sdileny s ovladacem, pozice a takt period.
//!
//! Rozpocet latence (48 kHz, perioda 256 snimku = 5,33 ms, 4 periody):
//!
//! | cast | doba |
//! |---|---|
//! | mix pred pozici DMA ([`LEAD_PERIODS`] = 2 periody) | 5,3-10,7 ms |
//! | FIFO radice a kodeku | < 0,5 ms |
//! | buffer klienta (vychozi 10 ms, min. 5 ms) | 0-10 ms |
//! | celkem od zapisu klienta po reproduktor | asi 11-21 ms |
//!
//! Treti a ctvrta perioda bufferu jsou rezerva: za smichanymi daty se
//! vzdy nuluje jedna perioda, takze kdyz sluzba nestihne, zazni ticho
//! (a pocita se `device_underruns`), ne stara data z minuleho kola.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::device::{self, Info, OpenStream, StatusPage, op};
use audioproto::{Error, Format, Result, decode_reply};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Event, Handle, Mapping, Vmo, println, time};

/// Snimku na periodu (5,33 ms pri 48 kHz; zaokrouhluje ovladac).
pub const PERIOD_FRAMES: u32 = 256;
pub const PERIODS: u32 = 4;
/// O kolik period napred se mixuje.
pub const LEAD_PERIODS: u32 = 2;

const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Device {
    chan: Channel,
    pub info: Info,
    pub format: Format,
    pub period: u32,
    pub buffer: u32,
    _buf_vmo: Vmo,
    buf: Mapping,
    _status_vmo: Vmo,
    _status_map: Mapping,
    pub status: StatusPage,
    pub event: Event,
    /// Absolutni pozice (snimky), kam az je smichano.
    pub written: u64,
    pub running: bool,
    pub underruns: u64,
}

fn call(chan: &Channel, msg: &[u8]) -> Result<(Vec<u8>, Vec<Handle>)> {
    while chan.try_read().is_ok() {}
    chan.write(msg, Vec::new())?;
    let m = chan.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    Ok((decode_reply(&m.data)?.to_vec(), m.handles))
}

/// Zepta se ovladace na zarizeni.
pub fn query(chan: &Channel) -> Result<Info> {
    Info::decode(&call(chan, &device::simple(op::INFO))?.0)
}

impl Device {
    /// Otevre vystup zarizeni ve formatu, ktery hlasi `info`.
    pub fn open(chan: Channel, info: Info) -> Result<Self> {
        let want = OpenStream { format: info.format, period: PERIOD_FRAMES, periods: PERIODS };
        let (data, hs) = call(&chan, &want.encode(op::OPEN_OUTPUT))?;
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
        // SAFETY: mapovani stranky stavu zije v `Device`.
        let st = unsafe { StatusPage::new(status_map.addr() as *mut u8) };
        if !st.valid() || st.buffer_frames() != buffer as u64 {
            return Err(Error::Corrupted);
        }
        println!(
            "audiod: zarizeni {} ({}), {} Hz, perioda {} snimku ({} us), buffer {} period",
            info.name,
            info.active_names(),
            got.format.rate,
            got.period,
            got.format.us_for_frames(got.period as u64),
            got.periods
        );
        Ok(Self {
            chan,
            info,
            format: got.format,
            period: got.period,
            buffer,
            _buf_vmo: buf_vmo,
            buf: buf_map,
            _status_vmo: status_vmo,
            _status_map: status_map,
            status: st,
            event: Event::from_handle(event),
            written: 0,
            running: false,
            underruns: 0,
        })
    }

    /// Kanal k ovladaci (zavreni = ovladac skoncil).
    pub fn channel(&self) -> &Channel {
        &self.chan
    }

    pub fn frame_bytes(&self) -> u32 {
        self.format.frame_bytes()
    }

    /// Spusti DMA: buffer vynulovat a mixovat od aktualni pozice.
    pub fn start(&mut self) -> Result<()> {
        if self.running {
            return Ok(());
        }
        let len = (self.buffer * self.frame_bytes()) as usize;
        self.buf.write_bytes(0, &vec![0u8; len]);
        self.written = self.status.position();
        call(&self.chan, &device::simple(op::START))?;
        self.running = true;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if !self.running {
            return Ok(());
        }
        self.running = false;
        call(&self.chan, &device::simple(op::STOP)).map(|_| ())
    }

    /// Zapise snimky (s16 stereo) na absolutni pozici `pos` (kruh).
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

    /// Latence od mixu po reproduktor (us): mix napred + FIFO.
    pub fn latency_us(&self) -> u32 {
        (self.format.us_for_frames((self.period * LEAD_PERIODS) as u64) + 500) as u32
    }
}
