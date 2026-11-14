//! gui-player: Prehravac nad knihovnou `ui` a sluzbou `mediad`
//! (`docs/cs/multimedia.md`, etapa F4 s testovacim dekoderem).
//!
//! Startovni zprava (kind USER): kanal okna, listek, kanal relace
//! `mediad`, volitelne kanal k `audiod` (bez nej hraje bez zvuku podle
//! monotonniho casu). Dokud neni vyber souboru a FFmpeg, prehrava
//! vestavenou ukazku v testovacim formatu `MTM1` (pohyblivy pruh a ton).

#![no_std]
#![no_main]

extern crate alloc;

mod player;

use alloc::vec::Vec;

use audioproto::conn::AudioClient;
use audioproto::{Direction, Format, Sample};
use bufdesc::{BufDesc, format};
use guiproto::window::Mode;
use libos::{Channel, Fence, Startup, Vmo, println};
use mediaproto::conn::MediaClient;
use mediaproto::testmedia::{self, Header};
use mediaproto::{AudioRing, Kind};
use ui::client::VideoSurface;

use player::Player;

libos::entry!(main);

/// Pocet bufferu videa (dekoder muze byt napred o tolik snimku).
const POOL: usize = 4;
/// Latence zvuku (buffer kruhu audiod).
const AUDIO_LATENCY_US: u32 = 100_000;

/// Vestavena ukazka: 10 s, 640x360 @ 25 fps, ton 440 Hz.
fn demo() -> Header {
    Header { width: 640, height: 360, fps_num: 25, fps_den: 1, frames: 250, rate: 48_000, channels: 2, tone_hz: 440, crash_at: 0 }
}

fn main(startup: Startup) -> i64 {
    let mut it = startup.handles.into_iter();
    let Some(link) = ui::client::Link::from_handles(&mut it) else { return 1 };
    let Some(media) = it.next().map(Channel::from_handle) else {
        println!("gui-player: chybi kanal k mediad");
        return 1;
    };
    let audio = it.next().map(|h| AudioClient::new(Channel::from_handle(h)));
    match setup(media, audio) {
        Ok((player, surface, events)) => match ui::client::run_video(link, player, Mode::Standard, 680, 470, alloc::vec![events], alloc::vec![surface]) {
            Ok(()) => 0,
            Err(e) => {
                println!("gui-player: {e}");
                2
            }
        },
        Err(e) => {
            println!("gui-player: nelze otevrit ukazku: {e}");
            3
        }
    }
}

/// Otevre ukazku, pool bufferu, zvuk a povrch videa. Vrati prehravac,
/// povrch a kopii kanalu relace pro smycku (udalosti `mediad`).
fn setup(media: Channel, audio: Option<AudioClient>) -> libos::Result<(Player, VideoSurface, Channel)> {
    let events = Channel::from_handle(media.handle().duplicate(None)?);
    let m = MediaClient::new(media);
    let h = demo();
    let file = testmedia::build(&h);
    let vmo = Vmo::create(file.len() as u64)?;
    vmo.write(0, &file)?;
    let info = m.open_vmo(&vmo, file.len() as u64)?;
    let video = info.first(Kind::Video).ok_or(libos::Error::NotSupported)?;
    let (vw, vh) = (info.streams[video].width, info.streams[video].height);

    // Zvuk: proud audiod ve frekvenci souboru, kruh preda dekoderu.
    let mut stream = None;
    let audio_index = info.first(Kind::Audio);
    if let (Some(a), Some(ai)) = (audio, audio_index) {
        let s = &info.streams[ai];
        match a.open(Direction::Output, Format::new(s.rate, 2, Sample::S16), AUDIO_LATENCY_US) {
            Ok(st) => {
                let (rv, ev) = st.producer_handles()?;
                let ring = AudioRing { rate: s.rate, channels: 2, sample: Sample::S16 as u8, capacity: st.info().capacity };
                m.audio(ring, rv, ev)?;
                stream = Some(st);
            }
            Err(e) => println!("gui-player: bez zvuku ({e})"),
        }
    }
    let audio_sel = if stream.is_some() { audio_index.map_or(-1, |i| i as i32) } else { -1 };
    m.select(video as i32, audio_sel, -1)?;

    let desc = BufDesc::linear(format::NV12, vw, vh, 64).map_err(|_| libos::Error::InvalidArgs)?;
    let size = desc.min_size().map_err(|_| libos::Error::InvalidArgs)?;
    let mut vmos = Vec::new();
    for _ in 0..POOL {
        vmos.push(Vmo::create(size)?);
    }
    let frames = Fence::create(0)?;
    let release = Fence::create(0)?;
    m.pool(alloc::vec![desc.clone(); POOL], &vmos, &frames)?;
    let surface = VideoSurface {
        id: 0,
        descs: alloc::vec![desc; POOL],
        vmos: vmos.iter().map(|v| v.handle().duplicate(None).map(Vmo::from_handle)).collect::<libos::Result<_>>()?,
        acquire: frames.for_waiter()?,
        release: Fence::from_handle(release.handle().duplicate(None)?),
    };
    let player = Player::new(m, stream, release, info.duration_us, (vw, vh));
    Ok((player, surface, events))
}
