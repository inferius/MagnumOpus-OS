use super::*;

#[test]
fn requests_roundtrip() {
    let mut raw = Mode::cooked();
    raw.make_raw();
    for r in [
        Request::GetMode,
        Request::SetMode { when: when::FLUSH, mode: raw },
        Request::GetSize,
        Request::SetSize(Size { rows: 40, cols: 120, xpixel: 0, ypixel: 0 }),
        Request::Flush(queue::BOTH),
        Request::Drain,
        Request::WatchSize,
    ] {
        assert_eq!(Request::decode(&r.encode()), Ok(r));
    }
    assert_eq!(Request::decode(&99u32.to_le_bytes()), Err(Error::NotSupported));
    let mut bad = Request::Flush(0).encode();
    *bad.last_mut().unwrap() = 7;
    assert_eq!(Request::decode(&bad), Err(Error::InvalidArgs));
    let mut long = Request::Drain.encode();
    long.push(0);
    assert_eq!(Request::decode(&long), Err(Error::InvalidArgs));
}

#[test]
fn replies_roundtrip() {
    let m = Mode::cooked();
    let b = encode_reply(Ok(Body::Mode(m)));
    assert_eq!(decode_reply(&Request::GetMode, &b), Ok(Body::Mode(m)));
    let s = Size { rows: 25, cols: 80, xpixel: 640, ypixel: 400 };
    assert_eq!(decode_reply(&Request::GetSize, &encode_reply(Ok(Body::Size(s)))), Ok(Body::Size(s)));
    assert_eq!(decode_reply(&Request::Drain, &encode_reply(Ok(Body::Empty))), Ok(Body::Empty));
    assert_eq!(decode_reply(&Request::GetMode, &encode_reply(Err(Error::AccessDenied))), Err(Error::AccessDenied));
}

#[test]
fn cooked_and_raw_modes() {
    let c = Mode::cooked();
    assert_ne!(c.lflag & lflag::ICANON, 0);
    assert_ne!(c.lflag & lflag::ECHO, 0);
    assert_eq!(c.cc[cc::VINTR], 3);
    assert_eq!(c.cc[cc::VEOF], 4);
    let mut r = c;
    r.make_raw();
    assert_eq!(r.lflag & (lflag::ICANON | lflag::ECHO | lflag::ISIG), 0);
    assert_eq!(r.iflag & iflag::ICRNL, 0);
    assert_eq!(r.oflag & oflag::OPOST, 0);
    assert_eq!(r.cflag & cflag::CSIZE, cflag::CS8);
    assert_eq!((r.cc[cc::VMIN], r.cc[cc::VTIME]), (1, 0));
}

#[test]
fn terminal_state() {
    let mut t = Tty::new(Size { rows: 24, cols: 80, ..Size::default() });
    let mut m = Mode::cooked();
    m.make_raw();
    m.iflag |= iflag::IXANY;
    m.cflag |= cflag::PARENB;
    let Ok(Body::Mode(got)) = t.handle(&Request::SetMode { when: when::NOW, mode: m }) else { panic!() };
    assert_eq!(got.iflag & iflag::IXANY, 0, "nepodporovany priznak se vynuluje");
    assert_eq!(got.cflag & cflag::PARENB, 0);
    assert_eq!(got.lflag & lflag::ICANON, 0);
    assert_eq!(t.handle(&Request::GetMode), Ok(Body::Mode(got)));
    let big = Size { rows: 50, cols: 132, ..Size::default() };
    assert_eq!(t.handle(&Request::SetSize(big)), Err(Error::AccessDenied));
    t.client_resize = true;
    assert_eq!(t.handle(&Request::SetSize(big)), Ok(Body::Empty));
    assert_eq!(t.handle(&Request::GetSize), Ok(Body::Size(big)));
    assert!(!t.resize(big));
    assert!(t.resize(Size { rows: 10, ..big }));
}
