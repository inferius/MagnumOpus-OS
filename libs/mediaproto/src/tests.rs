use alloc::vec;
use alloc::vec::Vec;

use bufdesc::{BufDesc, format};

use crate::clock::{AudioAnchor, Decision, WallClock, audio_now_us, decide};
use crate::source::{SourceRequest, clamp};
use crate::testmedia::{self, Header, Record};
use crate::*;

fn header() -> Header {
    Header { width: 64, height: 32, fps_num: 25, fps_den: 1, frames: 50, rate: 48_000, channels: 2, tone_hz: 440, crash_at: 0 }
}

#[test]
fn requests_roundtrip() {
    let desc = BufDesc::linear(format::NV12, 64, 32, 16).unwrap();
    let reqs = vec![
        Request::Connect,
        Request::Open { kind: source_kind::VMO, size: 1234 },
        Request::Open { kind: source_kind::CHANNEL, size: 0 },
        Request::Select { video: 0, audio: 1, subtitle: -1 },
        Request::Pool { descs: vec![desc.clone(), desc] },
        Request::Audio(AudioRing { rate: 48_000, channels: 2, sample: 2, capacity: 4800 }),
        Request::Play,
        Request::Pause,
        Request::Seek { pts_us: 5_000_000, exact: true },
        Request::Release { buffer: 3 },
        Request::Close,
        Request::Reopen,
        Request::Probe { size: 99 },
    ];
    for r in reqs {
        assert_eq!(Request::decode(&r.encode()).unwrap(), r);
    }
}

#[test]
fn bad_requests_are_rejected() {
    // Neznamy druh zdroje, pool bez bufferu, buffer mimo pool, index proudu
    // mimo rozsah, zbytek bajtu za zpravou, neznama operace.
    let mut open = Request::Open { kind: 1, size: 0 }.encode();
    open[4] = 9;
    assert_eq!(Request::decode(&open), Err(Error::InvalidArgs));
    let mut pool = wire::Writer::op(op::POOL);
    pool.u8(0);
    assert_eq!(Request::decode(&pool.finish()), Err(Error::InvalidArgs));
    let mut rel = wire::Writer::op(op::RELEASE);
    rel.u8(MAX_POOL as u8);
    assert_eq!(Request::decode(&rel.finish()), Err(Error::InvalidArgs));
    let mut sel = wire::Writer::op(op::SELECT);
    sel.i32(0).i32(-2).i32(-1);
    assert_eq!(Request::decode(&sel.finish()), Err(Error::InvalidArgs));
    let mut play = Request::Play.encode();
    play.push(0);
    assert_eq!(Request::decode(&play), Err(Error::InvalidArgs));
    assert_eq!(Request::decode(&wire::Writer::op(77).finish()), Err(Error::NotSupported));
    // Zkracena zprava: nikdy panika.
    let full = Request::Seek { pts_us: 1, exact: false }.encode();
    for n in 0..full.len() {
        assert!(Request::decode(&full[..n]).is_err());
    }
}

#[test]
fn events_and_replies() {
    let evs = [
        Event::Frame { buffer: 2, pts_us: 40_000, duration_us: 40_000, serial: 7 },
        Event::AudioBase { ring_pos: 4800, pts_us: 100_000 },
        Event::End,
        Event::Error { code: Error::PeerClosed.code(), decoder_lost: true },
    ];
    for e in evs {
        let m = e.encode();
        assert_eq!(Event::decode(&m).unwrap(), e);
        assert_eq!(decode_incoming(&m).unwrap(), Incoming::Event(e));
    }
    let ok = encode_reply(Ok(encode_u64(42)));
    match decode_incoming(&ok).unwrap() {
        Incoming::Reply(Ok(d)) => assert_eq!(decode_u64(d).unwrap(), 42),
        other => panic!("{other:?}"),
    }
    let err = encode_reply(Err(Error::NotSupported));
    assert_eq!(decode_incoming(&err).unwrap(), Incoming::Reply(Err(Error::NotSupported)));
}

#[test]
fn info_roundtrip_and_limits() {
    let info = header().info();
    assert_eq!(info.streams.len(), 2);
    assert_eq!(info.first(Kind::Audio), Some(1));
    assert_eq!(info.duration_us, 2_000_000);
    assert_eq!(Info::decode(&info.encode()).unwrap(), info);
    let mut w = wire::Writer::new();
    w.u64(0).u8((MAX_STREAMS + 1) as u8);
    assert_eq!(Info::decode(&w.finish()), Err(Error::InvalidArgs));
}

#[test]
fn source_reply_fits_channel() {
    // Odpoved na nejvetsi cteni (status i32 + data) i s rezervou.
    let full = alloc::vec![0u8; source::MAX_READ as usize];
    assert!(source::encode_reply(Ok(&full)).len() + 64 <= abi::syscall::CHANNEL_MAX_DATA);
}

#[test]
fn source_requests() {
    let r = SourceRequest::ReadAt { offset: 10, len: 100 };
    assert_eq!(SourceRequest::decode(&r.encode()).unwrap(), r);
    assert_eq!(SourceRequest::decode(&SourceRequest::Size.encode()).unwrap(), SourceRequest::Size);
    for len in [0, source::MAX_READ + 1] {
        let bad = SourceRequest::ReadAt { offset: 0, len }.encode();
        assert_eq!(SourceRequest::decode(&bad), Err(Error::InvalidArgs));
    }
    let over = SourceRequest::ReadAt { offset: u64::MAX, len: 1 }.encode();
    assert_eq!(SourceRequest::decode(&over), Err(Error::InvalidArgs));
    assert_eq!(source::decode_reply(&source::encode_reply(Ok(b"abc"))).unwrap(), b"abc");
    assert_eq!(source::decode_reply(&source::encode_reply(Err(Error::IoError))), Err(Error::IoError));
    assert_eq!(clamp(90, 20, 100), (90, 10));
    assert_eq!(clamp(100, 20, 100), (100, 0));
    assert_eq!(clamp(0, 20, 100), (0, 20));
}

#[test]
fn testmedia_file() {
    let h = header();
    let file = testmedia::build(&h);
    assert_eq!(file.len() as u64, h.file_len());
    assert_eq!(Header::parse(&file).unwrap(), h);
    let rec = |i: u32| {
        let o = testmedia::record_offset(i) as usize;
        Record::parse(&file[o..o + testmedia::RECORD_LEN as usize]).unwrap()
    };
    assert_eq!(rec(0), Record { pts_us: 0, seed: testmedia::seed(0), flags: testmedia::KEY });
    assert_eq!(rec(1).pts_us, 40_000);
    assert_eq!(rec(1).flags, 0);
    assert_eq!(rec(25).flags, testmedia::KEY);
    assert_eq!(h.frame_us(), 40_000);
    // Chybna magie, licha sirka, nulova fps.
    let mut bad = file.clone();
    bad[0] = b'X';
    assert_eq!(Header::parse(&bad), Err(Error::NotSupported));
    for (i, v) in [(8usize, 63u32), (16, 0)] {
        let mut b = file.clone();
        b[i..i + 4].copy_from_slice(&v.to_le_bytes());
        assert_eq!(Header::parse(&b), Err(Error::Corrupted));
    }
    assert!(Header::parse(&file[..10]).is_err());
}

#[test]
fn pattern_moves_and_is_deterministic() {
    let (w, h) = (64u32, 32u32);
    let desc = BufDesc::linear(format::NV12, w, h, 16).unwrap();
    let size = desc.min_size().unwrap() as usize;
    let render = |seed: u32| {
        let mut buf = vec![0u8; size];
        let (yo, yp) = (desc.planes[0].offset as usize, desc.planes[0].pitch as usize);
        let (uo, up) = (desc.planes[1].offset as usize, desc.planes[1].pitch as usize);
        let (ys, uvs) = buf.split_at_mut(uo);
        testmedia::fill_nv12(w, h, &mut ys[yo..], yp, uvs, up, seed);
        let sum = testmedia::checksum_nv12(w, h, &buf[yo..], yp, &buf[uo..], up);
        (buf, sum)
    };
    let (a, sa) = render(testmedia::seed(3));
    let (b, sb) = render(testmedia::seed(3));
    let (_, sc) = render(testmedia::seed(4));
    assert_eq!(a, b);
    assert_eq!(sa, sb);
    assert_ne!(sa, sc);
    // Pruh ma jas 235, okoli prechod pod nim.
    let row: Vec<u8> = a[..w as usize].to_vec();
    assert!(row.contains(&235));
    assert!(row.iter().all(|&p| (16..=235).contains(&p)));
}

#[test]
fn audio_clock() {
    let a = AudioAnchor { ring_pos: 48_000, pts_us: 2_000_000 };
    // Pred kotvou: cas kotvy.
    assert_eq!(audio_now_us(a, 1000, 0, 48_000), 2_000_000);
    // 0,5 s za kotvou bez latence.
    assert_eq!(audio_now_us(a, 72_000, 0, 48_000), 2_500_000);
    // Latence 100 ms posune slysene o 4800 ramcu zpet.
    assert_eq!(audio_now_us(a, 72_000, 100_000, 48_000), 2_400_000);
    assert_eq!(audio_now_us(a, 72_000, 0, 0), 2_000_000);
}

#[test]
fn wall_clock() {
    let mut c = WallClock::new(1_000_000);
    assert_eq!(c.now_us(5_000_000_000), 1_000_000);
    c.play(1_000_000_000);
    assert_eq!(c.now_us(1_500_000_000), 1_500_000);
    c.pause(2_000_000_000);
    assert_eq!(c.now_us(9_000_000_000), 2_000_000);
    c.play(10_000_000_000);
    c.seek(0, 10_000_000_000);
    assert_eq!(c.now_us(10_040_000_000), 40_000);
}

#[test]
fn frame_decisions() {
    let d = 40_000;
    assert_eq!(decide(1_000_000, d, 1_000_000), Decision::Show);
    assert_eq!(decide(1_000_000, d, 1_030_000), Decision::Show);
    assert_eq!(decide(1_000_000, d, 990_000), Decision::Show);
    // Konec intervalu snimku = pozadu.
    assert_eq!(decide(1_000_000, d, 1_040_000), Decision::Drop);
    // Vic nez pul snimku napred = podrzet.
    assert_eq!(decide(1_000_000, d, 970_000), Decision::Hold);
    // Velka odchylka obema smery.
    assert_eq!(decide(1_000_000, d, 1_300_000), Decision::Resync);
    assert_eq!(decide(1_000_000, d, 700_000), Decision::Resync);
}
