//! Streamy klientu: sdileny buffer, prevod formatu a frekvence, mix do
//! DMA bufferu zarizeni (prehravani) a rozvod nahraneho zvuku
//! (nahravani).

use alloc::vec;
use alloc::vec::Vec;

use audioproto::client::{Opened, StreamInfo};
use audioproto::ring::{self, Ring};
use audioproto::{Direction, Error, Format, Result, volume_gain};
use audiomix::Frame;
use audiomix::convert::{frame_to_stereo, stereo_to_frame};
use audiomix::mix::Mixer;
use audiomix::src::Resampler;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Event, Handle, Mapping, Vmo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Stopped,
    Running,
    /// Ceka na dohrani; po nem odpoved na `DRAIN` a zastaveni.
    Draining,
}

/// Udaje zarizeni, ktere stream potrebuje (frekvence a perioda).
#[derive(Debug, Clone, Copy)]
pub struct DevClock {
    pub rate: u32,
    pub period: u32,
    /// Latence od mixu po reproduktor (us).
    pub latency_us: u32,
}

pub struct Stream {
    pub id: u32,
    /// Index klienta (vlastnika).
    pub owner: usize,
    pub dir: Direction,
    pub format: Format,
    _vmo: Vmo,
    _map: Mapping,
    pub ring: Ring,
    pub event: Event,
    pub state: State,
    pub volume: u8,
    src: Resampler,
    /// Snimky klienta prectene z bufferu (prehravani) nebo zapsane do nej
    /// (nahravani) celkem.
    consumed: u64,
    /// Pozice zarizeni za poslednim skutecnym vzorkem streamu.
    last_data: u64,
    /// Mel stream v minule periode data (pro pocitani podteceni).
    fed: bool,
    pub underruns: u64,
    pub overruns: u64,
    /// Ceka nekdo na `DRAIN`?
    pub drain_pending: bool,
    /// Kolik snimku ticha jeste dat prevodniku frekvence pri dohravani:
    /// posledni vstupy jsou v jeho historii (a ve zpozdeni filtru).
    flush: u32,
    scratch: Vec<u8>,
    frames: Vec<Frame>,
}

impl Stream {
    /// Novy stream: buffer na `latency_us` (zaokrouhleno na periody
    /// zarizeni, min. 2). Vraci stream, odpoved a handly pro klienta.
    pub fn open(id: u32, owner: usize, dev: DevClock, dir: Direction, format: Format, latency_us: u32) -> Result<(Self, Vec<u8>, Vec<Handle>)> {
        let lat = latency_us.clamp(audioproto::client::MIN_LATENCY_US, audioproto::client::MAX_LATENCY_US);
        // Perioda zarizeni ve snimcich klienta.
        let period = (dev.period as u64 * format.rate as u64).div_ceil(dev.rate as u64) as u32;
        let want = format.frames_for_us(lat as u64) as u32;
        let capacity = want.div_ceil(period).max(2) * period;
        let fb = format.frame_bytes();
        if capacity as usize * fb as usize > ring::MAX_DATA {
            return Err(Error::InvalidArgs);
        }
        let size = ring::vmo_size(capacity, fb) as u64;
        let vmo = Vmo::create(size)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        // SAFETY: mapovani o `size` bajtech zije v `Stream`.
        let ring = unsafe { Ring::init(map.addr() as *mut u8, map.len() as usize, capacity, fb, dir)? };
        let latency = format.us_for_frames(capacity as u64) as u32 + dev.latency_us;
        ring.set_latency_us(latency as u64);
        let event = Event::create()?;
        let handles = vec![vmo.handle().duplicate(None)?, event.handle().duplicate(None)?];
        let reply = Opened { stream: id, format, capacity, period, latency_us: latency }.encode();
        let src = match dir {
            Direction::Output => Resampler::new(format.rate, dev.rate),
            Direction::Input => Resampler::new(dev.rate, format.rate),
        };
        let s = Self {
            id,
            owner,
            dir,
            format,
            _vmo: vmo,
            _map: map,
            ring,
            event,
            state: State::Stopped,
            volume: 100,
            flush: src.flush_frames(),
            src,
            consumed: 0,
            last_data: 0,
            fed: false,
            underruns: 0,
            overruns: 0,
            drain_pending: false,
            scratch: Vec::new(),
            frames: vec![[0; 2]; dev.period as usize + 8],
        };
        Ok((s, reply, handles))
    }

    /// Zarizeni se zmenilo (vyber jineho): prevodnik frekvence na novou
    /// frekvenci, velikost periody.
    pub fn retarget(&mut self, dev: DevClock) {
        self.src = match self.dir {
            Direction::Output => Resampler::new(self.format.rate, dev.rate),
            Direction::Input => Resampler::new(dev.rate, self.format.rate),
        };
        self.flush = self.src.flush_frames();
        self.frames = vec![[0; 2]; dev.period as usize + 8];
        self.last_data = 0;
    }

    pub fn start(&mut self) {
        if self.state == State::Stopped {
            self.src.reset();
            self.fed = false;
        }
        self.state = State::Running;
        self.flush = self.src.flush_frames();
    }

    pub fn stop(&mut self) {
        self.state = State::Stopped;
        if self.dir == Direction::Output {
            self.ring.discard();
        }
        self.drain_pending = false;
    }

    pub fn info(&self) -> StreamInfo {
        StreamInfo { running: self.state != State::Stopped, played: self.ring.played(), underruns: self.underruns.max(self.overruns), volume: self.volume }
    }

    /// Prehravani: smicha `n` snimku na pozici zarizeni `pos` do `mixer`.
    pub fn mix(&mut self, mixer: &mut Mixer, n: usize, pos: u64) {
        if self.dir != Direction::Output || self.state == State::Stopped || n == 0 {
            return;
        }
        if !self.ring.consistent() {
            // Klient zapsal nesmyslnou pozici: zahodit, co tam je.
            self.ring.resync();
        }
        let fb = self.format.frame_bytes() as usize;
        let need = self.src.input_needed(n as u32).min(self.ring.readable()) as usize;
        self.scratch.resize(need * fb, 0);
        let got = self.ring.peek(&mut self.scratch) as usize;
        let mut k = 0usize;
        let (fmt, scratch) = (self.format, &self.scratch);
        let draining = self.state == State::Draining && self.ring.readable() as usize == got;
        let flush = &mut self.flush;
        if self.frames.len() < n {
            self.frames.resize(n, [0; 2]);
        }
        let out = &mut self.frames[..n];
        let made = self.src.process(out, || {
            if k >= got {
                // Dohravani: historii prevodniku vytlacit tichem.
                if draining && *flush > 0 {
                    *flush -= 1;
                    return Some([0, 0]);
                }
                return None;
            }
            let f = frame_to_stereo(&fmt, &scratch[k * fb..(k + 1) * fb]);
            k += 1;
            Some(f)
        });
        self.ring.consume(k as u32);
        self.consumed += k as u64;
        if made > 0 {
            mixer.add(&self.frames[..made], volume_gain(self.volume));
            self.last_data = pos + made as u64;
        }
        if made < n {
            if self.state == State::Running && self.fed {
                self.underruns += 1;
                self.ring.add_underrun();
            }
            self.fed = false;
        } else {
            self.fed = true;
        }
        if k > 0 {
            let _ = self.event.signal();
        }
    }

    /// Nahravani: prevede snimky zarizeni na format klienta (s hlasitosti
    /// streamu) a zapise je do bufferu. Kdyz klient nestiha cist, zbytek
    /// se zahodi (preteceni).
    pub fn capture(&mut self, input: &[Frame]) {
        if self.dir != Direction::Input || self.state == State::Stopped || input.is_empty() {
            return;
        }
        let fb = self.format.frame_bytes() as usize;
        let gain = volume_gain(self.volume) as i64;
        let mut k = 0usize;
        let mut lost = false;
        loop {
            let out_len = self.frames.len();
            let made = self.src.process(&mut self.frames[..out_len], || {
                let f = input.get(k).copied();
                k += 1;
                f
            });
            if made > 0 {
                self.scratch.resize(made * fb, 0);
                for (i, f) in self.frames[..made].iter().enumerate() {
                    let g = [(f[0] as i64 * gain >> 16) as i32, (f[1] as i64 * gain >> 16) as i32];
                    stereo_to_frame(&self.format, g, &mut self.scratch[i * fb..(i + 1) * fb]);
                }
                let wrote = self.ring.write(&self.scratch) as usize;
                self.consumed += wrote as u64;
                if wrote < made {
                    lost = true;
                }
            }
            if made < out_len {
                break; // vstup dosel
            }
        }
        if lost {
            self.overruns += 1;
            self.ring.add_overrun();
        }
        self.ring.set_played(self.consumed);
        let _ = self.event.signal();
    }

    /// Po mixu: odhad prehranych snimku a konec `DRAIN` (`true` = stream
    /// prave dohral a ma se odpovedet).
    pub fn after_mix(&mut self, hw: u64, written: u64, dev_rate: u32) -> bool {
        if self.dir != Direction::Output || self.state == State::Stopped {
            return false; // pozice zastaveneho streamu uz se nemeni
        }
        // Smichane, ale jeste nezaznele snimky zarizeni -> snimky klienta.
        let pending_dev = written.saturating_sub(hw);
        let pending = pending_dev * self.format.rate as u64 / dev_rate as u64;
        self.ring.set_played(self.consumed.saturating_sub(pending));
        if self.state == State::Draining && self.ring.readable() == 0 && self.flush == 0 && hw >= self.last_data {
            self.state = State::Stopped;
            self.ring.set_played(self.consumed);
            return true;
        }
        false
    }

    pub fn active(&self) -> bool {
        self.state != State::Stopped
    }
}
