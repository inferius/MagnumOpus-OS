use super::*;

#[test]
fn request_and_reply_roundtrip() {
    let mut w = request(42, op::READ);
    w.str("/a/b.txt").u64(4096).u32(512);
    let m = w.finish();
    let (id, o, mut r) = parse_request(&m).unwrap();
    assert_eq!((id, o), (42, op::READ));
    assert_eq!((r.str().unwrap().as_str(), r.u64().unwrap(), r.u32().unwrap()), ("/a/b.txt", 4096, 512));

    let mut body = Writer::new();
    body.bytes(b"data");
    let rep = reply(42, Ok(body));
    assert_eq!(parse_reply(&rep).unwrap(), (42, Ok(&b"data"[..])));
    let rep = reply(7, Err(Error::NotFound));
    assert_eq!(parse_reply(&rep).unwrap(), (7, Err(Error::NotFound)));
}

#[test]
fn info_and_stat_roundtrip() {
    let i = Info { version: VERSION, fs: "fat".into(), label: "USB".into(), total_bytes: 1 << 30, free_bytes: 5, read_only: true };
    let mut w = Writer::new();
    i.put(&mut w);
    assert_eq!(Info::get(&mut Reader::new(&w.finish())).unwrap(), i);
    let s = Stat { kind: kind::FILE, size: 9, mtime: -5, btime: 1_700_000_000, read_only: false };
    let mut w = Writer::new();
    s.put(&mut w);
    assert_eq!(Stat::get(&mut Reader::new(&w.finish())).unwrap(), s);
}

#[test]
fn normalize_paths() {
    assert_eq!(normalize("").unwrap(), "/");
    assert_eq!(normalize("/").unwrap(), "/");
    assert_eq!(normalize("a//b/./c/").unwrap(), "/a/b/c");
    assert_eq!(normalize("/Dlouhe jmeno souboru.txt").unwrap(), "/Dlouhe jmeno souboru.txt");
    for bad in ["/a/../b", "/a:b", "/x\\y", "/q?", "/a\u{1}"] {
        assert_eq!(normalize(bad), Err(Error::InvalidArgs), "{bad}");
    }
}

#[test]
fn garbage_is_error_not_panic() {
    assert!(parse_request(&[1, 2]).is_err());
    assert!(parse_reply(&[]).is_err());
    assert!(Info::get(&mut Reader::new(&[0; 3])).is_err());
}
