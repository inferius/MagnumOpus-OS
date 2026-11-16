//! Testy protokolu a kruhoveho bufferu na hostiteli.

use alloc::vec;
use alloc::vec::Vec;

use crate::client::{Opened, Request, Status, StreamInfo};
use crate::device::{self, Info, Jack, OpenStream, Output, OutputKind, StatusPage};
use crate::ring::{self, Ring};
use crate::{Direction, Error, Format, Sample, decode_reply, encode_reply, volume_gain};

/// Zarovnana pamet pro buffer (Vec<u64> = zarovnani 8).
fn mem(bytes: usize) -> Vec<u64> {
    vec![0u64; bytes.div_ceil(8)]
}

#[test]
fn format_frame_bytes_and_validity() {
    let f = Format::new(48_000, 2, Sample::S16);
    assert_eq!(f.frame_bytes(), 4);
    assert_eq!(Format::new(44_100, 6, Sample::F32).frame_bytes(), 24);
    assert!(f.valid());
    assert!(!Format::new(1_000, 2, Sample::S16).valid());
    assert!(!Format::new(48_000, 0, Sample::S16).valid());
    assert_eq!(f.frames_for_us(10_000), 480);
    assert_eq!(f.us_for_frames(480), 10_000);
}

#[test]
fn volume_curve() {
    assert_eq!(volume_gain(100), 65_536);
    assert_eq!(volume_gain(0), 0);
    assert_eq!(volume_gain(200), 65_536);
    // 50 % = 1/8 = -18 dB.
    assert_eq!(volume_gain(50), 8_192);
}

#[test]
fn requests_roundtrip() {
    let f = Format::new(44_100, 1, Sample::S24);
    let all = [
        Request::Open { dir: Direction::Output, format: f, latency_us: 20_000 },
        Request::Open { dir: Direction::Input, format: f, latency_us: 5_000 },
        Request::Start { stream: 3 },
        Request::Stop { stream: 3 },
        Request::Close { stream: 7 },
        Request::Drain { stream: 1 },
        Request::StreamVolume { stream: 1, percent: 40 },
        Request::MasterVolume { percent: 100 },
        Request::Mute { on: true },
        Request::Status,
        Request::Connect,
        Request::StreamInfo { stream: 9 },
        Request::Devices,
        Request::Select { index: 2 },
    ];
    for r in all {
        assert_eq!(Request::decode(&r.encode()), Ok(r.clone()), "{r:?}");
    }
}

#[test]
fn bad_requests_are_errors_not_panics() {
    assert_eq!(Request::decode(&[]), Err(Error::InvalidArgs));
    assert_eq!(Request::decode(&99u32.to_le_bytes()), Err(Error::NotSupported));
    // Hlasitost nad 100 %.
    let mut m = Request::MasterVolume { percent: 10 }.encode();
    *m.last_mut().unwrap() = 101;
    assert_eq!(Request::decode(&m), Err(Error::InvalidArgs));
    // Neplatny format (0 kanalu).
    let mut m = Request::Open { dir: Direction::Output, format: Format::DEVICE_DEFAULT, latency_us: 1 }.encode();
    m[9] = 0;
    assert_eq!(Request::decode(&m), Err(Error::InvalidArgs));
    // Zkracene zpravy.
    let m = Request::Open { dir: Direction::Output, format: Format::DEVICE_DEFAULT, latency_us: 1 }.encode();
    for n in 0..m.len() {
        assert!(Request::decode(&m[..n]).is_err());
    }
}

#[test]
fn replies_roundtrip() {
    let o = Opened { stream: 2, format: Format::DEVICE_DEFAULT, capacity: 4096, period: 256, latency_us: 31_000 };
    assert_eq!(Opened::decode(&o.encode()), Ok(o));
    let s = Status {
        device: "HDA 8086:2668".into(),
        outputs: "line-out".into(),
        format: Format::DEVICE_DEFAULT,
        master: 80,
        muted: false,
        streams: 2,
        running: 1,
        hw_pos: 123_456,
        irqs: 77,
        device_underruns: 1,
        latency_us: 10_666,
        period: 256,
        input: true,
        device_index: 1,
        capturing: alloc::vec![alloc::string::String::from("SDL testaudio")],
    };
    assert_eq!(Status::decode(&s.encode()), Ok(s));
    let list = vec![
        crate::client::DeviceEntry { index: 0, name: "HDA a".into(), outputs: "line-out".into(), input: true, active: true },
        crate::client::DeviceEntry { index: 1, name: "HDA b".into(), outputs: "".into(), input: false, active: false },
    ];
    assert_eq!(crate::client::DeviceEntry::decode_list(&crate::client::DeviceEntry::encode_list(&list)), Ok(list));
    let i = StreamInfo { running: true, played: 99, underruns: 0, volume: 100 };
    assert_eq!(StreamInfo::decode(&i.encode()), Ok(i));
    let r = encode_reply(Ok(vec![1, 2, 3]));
    assert_eq!(decode_reply(&r), Ok(&[1u8, 2, 3][..]));
    assert_eq!(decode_reply(&encode_reply(Err(Error::NotSupported))), Err(Error::NotSupported));
    assert_eq!(decode_reply(&[1]), Err(Error::Corrupted));
}

#[test]
fn device_messages_roundtrip() {
    let info = Info {
        name: "QEMU".into(),
        outputs: vec![
            Output { kind: OutputKind::LineOut, jack: Jack::Plugged, active: true },
            Output { kind: OutputKind::Digital, jack: Jack::Unknown, active: false },
        ],
        format: Format::DEVICE_DEFAULT,
        input: false,
    };
    assert_eq!(Info::decode(&info.encode()), Ok(info.clone()));
    assert!(info.has_analog());
    assert_eq!(info.active_names(), "line-out");
    let o = OpenStream { format: Format::DEVICE_DEFAULT, period: 256, periods: 4 };
    let m = o.encode(device::op::OPEN_OUTPUT);
    let mut r = wire::Reader::new(&m[4..]);
    assert_eq!(OpenStream::decode(&mut r), Ok(o));
    assert_eq!(device::decode_opened(&device::encode_opened(&o)), Ok(o));
    // Prilis velky buffer a malo period.
    let big = OpenStream { format: Format::DEVICE_DEFAULT, period: 1 << 20, periods: 4 };
    assert!(device::decode_opened(&device::encode_opened(&big)).is_err());
    let one = OpenStream { format: Format::DEVICE_DEFAULT, period: 256, periods: 1 };
    assert!(device::decode_opened(&device::encode_opened(&one)).is_err());
}

#[test]
fn status_page() {
    let mut m = mem(device::STATUS_SIZE);
    // SAFETY: pamet ma STATUS_SIZE bajtu a zije do konce testu.
    let p = unsafe { StatusPage::new(m.as_mut_ptr() as *mut u8) };
    assert!(!p.valid());
    p.init(1024);
    assert!(p.valid());
    assert_eq!(p.buffer_frames(), 1024);
    p.set_position(500, 7);
    p.add_irq();
    p.set_flags(device::flags::RUNNING, 0);
    assert_eq!((p.position(), p.updated_ns(), p.irqs()), (500, 7, 1));
    p.set_flags(device::flags::ERROR, device::flags::RUNNING);
    assert_eq!(p.flags(), device::flags::ERROR);
}

fn ring_pair(cap: u32, fb: u32) -> (Vec<u64>, Ring, Ring) {
    let len = ring::vmo_size(cap, fb);
    let mut m = mem(len);
    let base = m.as_mut_ptr() as *mut u8;
    // SAFETY: pamet zije v Vec, ktery vracime spolu s pohledy.
    let server = unsafe { Ring::init(base, len, cap, fb, Direction::Output).unwrap() };
    let client = unsafe { Ring::attach(base, len).unwrap() };
    (m, server, client)
}

#[test]
fn ring_write_read_wraps() {
    let (_m, server, client) = ring_pair(8, 4);
    assert_eq!(client.writable(), 8);
    let data: Vec<u8> = (0..24).collect();
    assert_eq!(client.write(&data), 6);
    assert_eq!(server.readable(), 6);
    let mut out = [0u8; 16];
    assert_eq!(server.read(&mut out), 4);
    assert_eq!(&out, &data[..16]);
    // Zapis pres konec kruhu.
    let more: Vec<u8> = (100..124).collect();
    assert_eq!(client.write(&more), 6);
    assert_eq!(client.writable(), 0);
    let mut all = [0u8; 32];
    assert_eq!(server.read(&mut all), 8);
    assert_eq!(&all[..8], &data[16..24]);
    assert_eq!(&all[8..], &more[..]);
    // Necele snimky se ignoruji.
    assert_eq!(client.write(&[1, 2, 3]), 0);
}

#[test]
fn ring_peek_consume_discard() {
    let (_m, server, client) = ring_pair(4, 2);
    client.write(&[1, 1, 2, 2, 3, 3]);
    let mut out = [0u8; 4];
    assert_eq!(server.peek(&mut out), 2);
    assert_eq!(server.readable(), 3);
    server.consume(1);
    assert_eq!(server.readable(), 2);
    server.consume(10);
    assert_eq!(server.readable(), 0);
    client.write(&[9, 9]);
    server.discard();
    assert_eq!(server.readable(), 0);
}

#[test]
fn ring_rejects_hostile_positions() {
    let (m, server, client) = ring_pair(8, 4);
    client.write(&[0u8; 8]);
    // Klient zapise nesmyslnou write_pos (daleko napred): nic ke cteni.
    let mut m = m;
    m[8] = 1_000_000; // offset 64 = write_pos
    assert_eq!(server.readable(), 0);
    assert!(!server.consistent());
    server.resync();
    assert!(server.consistent());
    assert_eq!(server.readable(), 0);
    // write_pos pred read_pos.
    m[16] = 2_000_000; // offset 128 = read_pos
    assert_eq!(server.readable(), 0);
    assert_eq!(client.writable(), 0);
}

#[test]
fn ring_attach_validates_header() {
    let len = ring::vmo_size(16, 4);
    let mut m = mem(len);
    let base = m.as_mut_ptr() as *mut u8;
    // SAFETY: pamet zije v `m`.
    unsafe {
        assert!(Ring::attach(base, len).is_err());
        Ring::init(base, len, 16, 4, Direction::Output).unwrap();
        assert!(Ring::attach(base, len - 1).is_err());
        assert!(Ring::attach(base, len).is_ok());
        assert!(Ring::init(base, len, 17, 4, Direction::Output).is_err());
    }
}

#[test]
fn ring_service_counters() {
    let (_m, server, client) = ring_pair(4, 4);
    server.add_underrun();
    server.set_played(42);
    server.set_latency_us(20_000);
    assert_eq!((client.underruns(), client.played(), client.latency_us()), (1, 42, 20_000));
}

#[test]
fn overview_apps_and_new_requests_roundtrip() {
    use crate::client::{AppEntry, Overview, Request, Status};
    for req in [
        Request::Apps,
        Request::AppVolume { app: "org.winehq.wine".into(), percent: 40 },
        Request::Overview,
        Request::SelectInput { index: 1 },
        Request::SelectInput { index: 255 },
    ] {
        assert_eq!(Request::decode(&req.encode()).unwrap(), req);
    }
    // Prazdna identita a hlasitost nad 100 jsou chyba.
    assert!(Request::decode(&Request::AppVolume { app: alloc::string::String::new(), percent: 10 }.encode()).is_err());
    assert!(Request::decode(&Request::AppVolume { app: "x".into(), percent: 101 }.encode()).is_err());
    let apps = alloc::vec![
        AppEntry { app_id: "org.winehq.wine".into(), name: "Wine".into(), volume: 40, playing: true, capturing: false },
        AppEntry { app_id: "org.magnumopus.player".into(), name: "Přehrávač".into(), volume: 100, playing: false, capturing: true },
    ];
    assert_eq!(AppEntry::decode_list(&AppEntry::encode_list(&apps)).unwrap(), apps);
    let status = Status {
        device: "HDA".into(),
        outputs: "line-out".into(),
        format: crate::Format::DEVICE_DEFAULT,
        master: 60,
        muted: true,
        streams: 1,
        running: 1,
        hw_pos: 1,
        irqs: 2,
        device_underruns: 0,
        latency_us: 1,
        period: 256,
        input: true,
        device_index: 0,
        capturing: alloc::vec![],
    };
    let devices = alloc::vec![crate::client::DeviceEntry { index: 0, name: "HDA".into(), outputs: "line-out".into(), input: true, active: true }];
    let o = Overview { status, devices, apps, input: Some(0) };
    assert_eq!(Overview::decode(&o.encode()).unwrap(), o);
    let o2 = Overview { input: None, ..o };
    assert_eq!(Overview::decode(&o2.encode()).unwrap(), o2);
    // Aplikace (relace s receptem) je nesmi.
    for req in [Request::Apps, Request::Overview, Request::SelectInput { index: 0 }, Request::AppVolume { app: "a".into(), percent: 1 }] {
        assert!(!crate::session::app_allowed(&req));
    }
}
