//! Kodovani zprav: tam a zpet a odolnost proti poskozenym zpravam.

use osfs_proto::*;
use proptest::prelude::*;

fn entry(name: &str) -> EntryInfo {
    EntryInfo {
        name: name.into(),
        windows_name: name.replace(':', "\u{F03A}"),
        hidden: Hidden::Visible,
        truncated: false,
        kind: Kind::File,
        oid: 42,
        generation: 7,
        size: 1234,
        mtime: Time { secs: 1, nanos: 5 },
        ctime: Time {
            secs: -2,
            nanos: 999_999_999,
        },
        btime: Time {
            secs: -12_000_000_000,
            nanos: 0,
        },
        tree_bytes: 0,
        tree_files: 0,
        tree_dirs: 0,
        dataset: 1,
        link_target: String::new(),
    }
}

#[test]
fn requests_roundtrip() {
    let reqs = [
        Request::Hello,
        Request::VolumeInfo,
        Request::Stat {
            at: String::new(),
            path: PathArg::Fs("/a:b".into()),
        },
        Request::List {
            at: "vcera".into(),
            path: PathArg::Windows("\\a\\b".into()),
            cursor: "x".into(),
        },
        Request::Read {
            at: String::new(),
            path: PathArg::Fs("/CON.txt".into()),
            offset: 1 << 40,
            len: 4096,
        },
        Request::Rename {
            from: PathArg::Fs("/CON.txt".into()),
            to: PathArg::Windows("\\con_.txt".into()),
        },
        Request::Snapshots {
            path: PathArg::Fs("/projekt".into()),
            after: 7,
        },
        Request::SnapshotCreate {
            path: PathArg::Fs("/".into()),
            name: "pred upravou".into(),
        },
        Request::Commit,
    ];
    for r in reqs {
        assert_eq!(Request::decode(&r.encode()), Ok(r.clone()), "{r:?}");
    }
}

#[test]
fn write_ops_use_write_code() {
    assert!(Request::Commit.is_write());
    assert_eq!(Request::Commit.ioctl(), IOCTL_WRITE);
    assert_eq!(Request::Hello.ioctl(), IOCTL_READ);
    assert_ne!(IOCTL_READ, IOCTL_WRITE);
    // Bit 0x8000 v typu zarizeni (podminka WinFsp), METHOD_BUFFERED.
    assert_ne!(IOCTL_READ >> 16 & 0x8000, 0);
    assert_eq!(IOCTL_READ & 3, 0);
    assert_eq!(IOCTL_WRITE & 3, 0);
}

#[test]
fn responses_roundtrip() {
    let cases = [
        (
            op::HELLO,
            Response::Hello(HelloInfo {
                version: 1,
                features: feature::SNAPSHOTS | feature::READ,
                server: "osfs".into(),
                max_message: 4096,
            }),
        ),
        (
            op::VOLUME_INFO,
            Response::VolumeInfo(VolumeInfo {
                label: "Zkouska".into(),
                block_size: 4096,
                block_count: 1000,
                free_blocks: 900,
                deferred_blocks: 3,
                generation: 9,
                algo: 1,
                read_only: true,
                fallback: 0,
                replayed: 2,
            }),
        ),
        (op::STAT, Response::Entry(entry("a"))),
        (
            op::LIST,
            Response::List(ListPage {
                entries: vec![entry("a"), entry("b:c")],
                next: "b:c".into(),
            }),
        ),
        (op::READ, Response::Data(vec![1, 2, 3])),
        (
            op::SNAPSHOTS,
            Response::Snapshots {
                list: vec![SnapshotInfo {
                    id: 1,
                    name: "s".into(),
                    generation: 5,
                }],
                next: 1,
            },
        ),
        (op::SNAPSHOT_CREATE, Response::Created(3)),
        (op::COMMIT, Response::Done(10)),
    ];
    for (op, r) in cases {
        assert_eq!(Response::decode(op, &r.encode(op)), Ok(r.clone()), "{r:?}");
    }
}

#[test]
fn errors_and_mismatch() {
    let e = encode_error(op::STAT, error::NOT_FOUND);
    assert_eq!(Response::decode(op::STAT, &e), Err(error::NOT_FOUND));
    // Odpoved na jinou operaci = poskozena.
    let ok = Response::Done(1).encode(op::COMMIT);
    assert_eq!(Response::decode(op::RENAME, &ok), Err(error::CORRUPTED));
    // Spatna verze pozadavku = nepodporovano, spatna hlavicka = neplatne.
    let mut r = Request::Hello.encode();
    r[4] = 99;
    assert_eq!(Request::decode(&r), Err(error::NOT_SUPPORTED));
    assert_eq!(Request::decode(b"nesmysl"), Err(error::INVALID_ARGS));
    // Bajty navic za pozadavkem.
    let mut r = Request::Hello.encode();
    r.push(0);
    assert_eq!(Request::decode(&r), Err(error::INVALID_ARGS));
}

#[test]
fn oversized_entry_is_truncated_to_fit() {
    let mut e = entry(&"\u{1F600}".repeat(1000));
    e.link_target = "x".repeat(4000);
    let budget = MAX_MESSAGE - RESPONSE_HEADER - 4 - 2;
    e.fit(budget);
    assert!(e.truncated);
    assert!(e.encoded_len() <= budget);
    assert!(e.link_target.is_empty());
    // Zkraceni na hranici znaku (ctyrbajtove znaky).
    assert_eq!(e.name.len() % 4, 0);
    let mut small = entry("a");
    small.fit(budget);
    assert!(!small.truncated);
}

#[test]
fn time_and_filetime() {
    // 1. 1. 1970 a cas pred rokem 1970 (zaporne sekundy).
    assert_eq!(
        Time { secs: 0, nanos: 0 }.to_filetime(),
        116_444_736_000_000_000
    );
    let t = Time {
        secs: -86_400,
        nanos: 500,
    };
    assert_eq!(Time::from_filetime(t.to_filetime()), t);
    // Pred rokem 1601 FILETIME neumi: 0.
    assert_eq!(
        Time {
            secs: -20_000_000_000,
            nanos: 0
        }
        .to_filetime(),
        0
    );
    // Nanosekundy mimo rozsah ve zprave = poskozena zprava.
    let mut e = entry("a");
    e.mtime.nanos = 1_000_000_000;
    let b = Response::Entry(e).encode(op::STAT);
    assert_eq!(Response::decode(op::STAT, &b), Err(error::CORRUPTED));
}

#[test]
fn windows_full_path() {
    assert_eq!(
        PathArg::from_windows_full("Q:\\a\\b"),
        PathArg::Windows("\\a\\b".into())
    );
    assert_eq!(
        PathArg::from_windows_full("Q:"),
        PathArg::Windows("\\".into())
    );
    assert_eq!(
        PathArg::from_windows_full("\\\\?\\Q:\\x"),
        PathArg::Windows("\\x".into())
    );
}

proptest! {
    /// Libovolne bajty: dekodovani nesmi panikarit (vstup od ciziho procesu).
    #[test]
    fn decode_never_panics(b in prop::collection::vec(any::<u8>(), 0..400), op in 0u16..12) {
        let _ = Request::decode(&b);
        let _ = Response::decode(op, &b);
    }

    /// Poskozena platna zprava: taky bez paniky.
    #[test]
    fn corrupted_valid_message(i in 0usize..200, v in any::<u8>()) {
        let r = Response::List(ListPage { entries: vec![entry("a"), entry("bb")], next: "bb".into() });
        let mut b = r.encode(op::LIST);
        let n = b.len();
        b[i % n] = v;
        let _ = Response::decode(op::LIST, &b);
        b.truncate(i % n);
        let _ = Response::decode(op::LIST, &b);
    }
}
