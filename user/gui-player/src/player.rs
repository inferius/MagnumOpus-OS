//! Stav prehravace: fronta dekodovanych snimku, hodiny (zvuk, jinak
//! monotonni cas), vyber snimku k vymene kompozitoru, vraceni bufferu.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use audioproto::conn::Stream;
use libos::{Fence, println};
use mediaproto::clock::{AudioAnchor, Decision, WallClock, audio_now_us, decide};
use mediaproto::conn::MediaClient;
use mediaproto::{Event, Incoming, decode_incoming};
use ui::{App, Node, VideoShow, VideoTick, button, column, label, row, slider, video};

#[derive(Clone)]
pub enum Msg {
    Toggle,
    Seek(i32),
}

/// Dekodovany snimek cekajici na zobrazeni.
struct Queued {
    slot: u8,
    pts_us: u64,
    duration_us: u32,
    serial: u64,
}

pub struct Player {
    media: MediaClient,
    stream: Option<Stream>,
    /// Plot uvolneni povrchu (posouva kompozitor).
    release: Fence,
    duration_us: u64,
    size: (u32, u32),
    queue: VecDeque<Queued>,
    /// Ukazane buffery cekajici na plot uvolneni: (buffer, hodnota).
    shown: Vec<(u8, u64)>,
    anchor: Option<AudioAnchor>,
    wall: WallClock,
    playing: bool,
    ended: bool,
    /// Cas medii posledniho ukazaneho snimku.
    position_us: u64,
    status: String,
    /// Sekunda na posuvniku pri poslednim prekresleni.
    drawn_second: u64,
}

fn mmss(us: u64) -> String {
    let s = us / 1_000_000;
    format!("{}:{:02}", s / 60, s % 60)
}

impl Player {
    pub fn new(media: MediaClient, stream: Option<Stream>, release: Fence, duration_us: u64, size: (u32, u32)) -> Self {
        Self {
            media,
            stream,
            release,
            duration_us,
            size,
            queue: VecDeque::new(),
            shown: Vec::new(),
            anchor: None,
            wall: WallClock::new(0),
            playing: false,
            ended: false,
            position_us: 0,
            status: String::new(),
            drawn_second: 0,
        }
    }

    /// Udalost `mediad` (z kanalu ve smycce nebo z fronty klienta).
    fn on_event(&mut self, e: Event) -> bool {
        match e {
            Event::Frame { buffer, pts_us, duration_us, serial } => {
                self.queue.push_back(Queued { slot: buffer, pts_us, duration_us, serial });
                false
            }
            Event::AudioBase { ring_pos, pts_us } => {
                self.anchor = Some(AudioAnchor { ring_pos, pts_us });
                false
            }
            Event::End => {
                self.ended = true;
                false
            }
            Event::Error { code, decoder_lost } => {
                self.status = if decoder_lost { String::from("Dekodér skončil, obnovuji") } else { format!("Chyba dekodéru {code}") };
                if decoder_lost {
                    self.recover();
                }
                true
            }
        }
    }

    /// Udalosti, ktere klient zaradil behem cekani na odpoved.
    fn drain(&mut self) {
        while let Ok(e) = self.media.try_event() {
            self.on_event(e);
        }
    }

    /// Po padu dekoderu: novy se stejnym stavem, pokracovat od pozice.
    fn recover(&mut self) {
        self.queue.clear();
        self.shown.clear();
        let r = self.media.reopen().and_then(|_| self.media.seek(self.position_us, true));
        match r {
            Ok(pts) => {
                self.wall.seek(pts, libos::time::now());
                if self.playing {
                    let _ = self.media.play();
                }
                self.status.clear();
            }
            Err(e) => {
                self.status = format!("Obnova dekodéru selhala: {e}");
                self.playing = false;
            }
        }
        self.drain();
    }

    fn now_us(&self, at_ns: u64) -> u64 {
        match (self.anchor, self.stream.as_ref()) {
            (Some(a), Some(s)) => {
                let ring = s.ring();
                let ahead = at_ns.saturating_sub(libos::time::now()) / 1000;
                audio_now_us(a, ring.played(), ring.latency_us(), s.info().format.rate) + ahead
            }
            _ => self.wall.now_us(at_ns),
        }
    }

    /// Buffery, ktere kompozitor uz pustil, vratit dekoderu.
    fn release_done(&mut self) {
        let Ok(v) = self.release.query().map(|i| i.value) else { return };
        let mut i = 0;
        while i < self.shown.len() {
            if self.shown[i].1 <= v {
                let (slot, _) = self.shown.swap_remove(i);
                let _ = self.media.release(slot);
            } else {
                i += 1;
            }
        }
    }

    fn toggle(&mut self) {
        let now = libos::time::now();
        if self.playing {
            let _ = self.media.pause();
            if let Some(s) = self.stream.as_ref() {
                let _ = s.stop();
            }
            self.wall.pause(now);
            self.playing = false;
        } else {
            if self.ended {
                self.seek(0);
            }
            match self.media.play() {
                Ok(()) => {
                    if let Some(s) = self.stream.as_ref() {
                        let _ = s.start();
                    }
                    self.wall.play(now);
                    self.playing = true;
                }
                Err(e) => self.status = format!("Nelze přehrát: {e}"),
            }
        }
        self.drain();
    }

    fn seek(&mut self, pts_us: u64) {
        // Snimky ve fronte patri staremu casu: vratit je dekoderu.
        for q in self.queue.drain(..) {
            let _ = self.media.release(q.slot);
        }
        self.anchor = None;
        self.ended = false;
        match self.media.seek(pts_us, false) {
            Ok(pts) => {
                self.position_us = pts;
                self.wall.seek(pts, libos::time::now());
            }
            Err(e) => self.status = format!("Skok selhal: {e}"),
        }
        self.drain();
        // Snimky dekodovane pred skokem (prisly behem cekani) zahodit.
        let keep = self.position_us;
        let old: Vec<u8> = self.queue.iter().filter(|q| q.pts_us < keep).map(|q| q.slot).collect();
        self.queue.retain(|q| q.pts_us >= keep);
        for slot in old {
            let _ = self.media.release(slot);
        }
    }
}

impl App for Player {
    type Msg = Msg;

    fn title(&self) -> String {
        String::from("Přehrávač")
    }

    fn view(&self, _cx: &ui::Context) -> Node<Msg> {
        let total = (self.duration_us / 1_000_000) as i32;
        let pos = (self.position_us / 1_000_000) as i32;
        let mut col = alloc::vec![
            video(0, self.size.0, self.size.1, "Video").grow(),
            row([
                button(if self.playing { "Pozastavit" } else { "Přehrát" }).on_click(Msg::Toggle),
                slider(pos, 0, total.max(1)).on_value(Msg::Seek).grow(),
                label(&format!("{} / {}", mmss(self.position_us), mmss(self.duration_us))),
            ]),
        ];
        if !self.status.is_empty() {
            col.push(label(&self.status));
        }
        column(col)
    }

    fn update(&mut self, m: Msg) {
        match m {
            Msg::Toggle => self.toggle(),
            Msg::Seek(s) => self.seek(s.max(0) as u64 * 1_000_000),
        }
    }

    fn external_from(&mut self, _index: usize, data: &[u8]) -> bool {
        match decode_incoming(data) {
            Ok(Incoming::Event(e)) => self.on_event(e),
            Ok(Incoming::Reply(_)) => {
                println!("gui-player: odpoved mimo pozadavek");
                false
            }
            Err(_) => false,
        }
    }

    fn wants_frames(&self) -> bool {
        self.playing
    }

    fn video_frame(&mut self, _now_ns: u64, deadline_ns: u64) -> VideoTick {
        self.drain();
        self.release_done();
        let at_ns = if deadline_ns != 0 { deadline_ns } else { libos::time::now() };
        let at = self.now_us(at_ns);
        let mut tick = VideoTick::default();
        while let Some(q) = self.queue.front() {
            match decide(q.pts_us, q.duration_us, at) {
                Decision::Hold => break,
                Decision::Drop => {
                    let q = self.queue.pop_front().unwrap();
                    let _ = self.media.release(q.slot);
                }
                Decision::Show | Decision::Resync => {
                    let q = self.queue.pop_front().unwrap();
                    self.position_us = q.pts_us;
                    tick.shows.push(VideoShow { id: 0, slot: q.slot, acquire: q.serial });
                    break;
                }
            }
        }
        if self.ended && self.queue.is_empty() && self.position_us + 100_000 >= self.duration_us.saturating_sub(40_000) {
            self.playing = false;
            if let Some(s) = self.stream.as_ref() {
                let _ = s.stop();
            }
            tick.redraw = true;
        }
        let second = self.position_us / 1_000_000;
        if second != self.drawn_second {
            self.drawn_second = second;
            tick.redraw = true;
        }
        tick
    }

    fn video_shown(&mut self, _id: u32, slot: u8, release: u64) {
        self.shown.push((slot, release));
    }
}
