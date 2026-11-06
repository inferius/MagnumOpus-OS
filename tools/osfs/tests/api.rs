//! Serverova cast rozsireneho API nad `Fs` v pameti (bez WinFsp).

use fs::{FormatOptions, Fs, Kind, MemDevice};
use osfs::api::{Code, Host, ListCache, handle};
use osfs_proto::{Hidden, ListPage, PathArg, Request, Response, error};

#[derive(Default)]
struct TestHost {
    read_only: bool,
    dirty: bool,
    renamed: Vec<(String, String)>,
}

impl Host for TestHost {
    fn mutating(&mut self) -> Result<(), i32> {
        if self.read_only {
            return Err(error::READ_ONLY);
        }
        self.dirty = true;
        Ok(())
    }
    fn renamed(&mut self, from: &str, to: &str) {
        self.renamed.push((from.into(), to.into()));
    }
    fn committed(&mut self) {
        self.dirty = false;
    }
}

struct Server {
    fs: Fs<MemDevice>,
    host: TestHost,
    cache: Option<ListCache>,
}

impl Server {
    fn new() -> Self {
        let dev = Fs::format(MemDevice::new(8192), &FormatOptions::default()).unwrap();
        Self {
            fs: Fs::mount(dev).unwrap(),
            host: TestHost::default(),
            cache: None,
        }
    }

    fn call_code(&mut self, code: Code, req: &Request) -> Result<Response, i32> {
        let out = handle(
            &mut self.fs,
            &mut self.host,
            code,
            &req.encode(),
            &mut self.cache,
            1,
        );
        assert!(out.len() <= osfs_proto::MAX_MESSAGE);
        Response::decode(req.op(), &out)
    }

    fn call(&mut self, req: &Request) -> Result<Response, i32> {
        let code = if req.is_write() {
            Code::Write
        } else {
            Code::Read
        };
        self.call_code(code, req)
    }

    fn list_all(&mut self, at: &str, path: &str) -> Vec<osfs_proto::EntryInfo> {
        let mut out = Vec::new();
        let mut cursor = String::new();
        let mut pages = 0;
        loop {
            let Response::List(ListPage { entries, next }) = self
                .call(&Request::List {
                    at: at.into(),
                    path: PathArg::Fs(path.into()),
                    cursor: cursor.clone(),
                })
                .unwrap()
            else {
                panic!("cekam List");
            };
            pages += 1;
            out.extend(entries);
            if next.is_empty() {
                break;
            }
            cursor = next;
            assert!(pages < 10_000);
        }
        out
    }
}

fn fs_path(p: &str) -> PathArg {
    PathArg::Fs(p.into())
}

#[test]
fn error_codes_match_abi() {
    use abi::Error as E;
    for (e, c) in [
        (E::AccessDenied, error::ACCESS_DENIED),
        (E::WrongType, error::WRONG_TYPE),
        (E::NoMemory, error::NO_MEMORY),
        (E::InvalidArgs, error::INVALID_ARGS),
        (E::NotFound, error::NOT_FOUND),
        (E::NotSupported, error::NOT_SUPPORTED),
        (E::Corrupted, error::CORRUPTED),
        (E::BufferTooSmall, error::BUFFER_TOO_SMALL),
        (E::OutOfRange, error::OUT_OF_RANGE),
        (E::AlreadyExists, error::ALREADY_EXISTS),
        (E::NotADirectory, error::NOT_A_DIRECTORY),
        (E::IsADirectory, error::IS_A_DIRECTORY),
        (E::NotEmpty, error::NOT_EMPTY),
        (E::NoSpace, error::NO_SPACE),
        (E::IoError, error::IO_ERROR),
        (E::ReadOnly, error::READ_ONLY),
    ] {
        assert_eq!(e.code(), c, "{e:?}");
    }
}

#[test]
fn hidden_entries_and_tree_sizes() {
    let mut s = Server::new();
    s.fs.create("/d", Kind::Dir).unwrap();
    for n in ["CON.txt", "a:b", "x.", "Normal.txt"] {
        s.fs.create(&format!("/d/{n}"), Kind::File).unwrap();
        s.fs.write(&format!("/d/{n}"), 0, b"12345").unwrap();
    }
    s.fs.symlink("/d/odkaz", "/d/Normal.txt").unwrap();
    let list = s.list_all("", "/d");
    let get = |n: &str| list.iter().find(|e| e.name == n).unwrap().clone();
    assert_eq!(list.len(), 5);
    assert_eq!(get("CON.txt").hidden, Hidden::Reserved);
    assert_eq!(get("CON.txt").windows_name, "");
    assert_eq!(get("a:b").windows_name, "a\u{F03A}b");
    assert_eq!(get("x.").hidden, Hidden::TrailingDotOrSpace);
    assert_eq!(get("odkaz").hidden, Hidden::Symlink);
    assert_eq!(get("odkaz").link_target, "/d/Normal.txt");
    assert_eq!(get("Normal.txt").windows_name, "Normal.txt");
    // Okamzita velikost podstromu.
    let Response::Entry(d) = s
        .call(&Request::Stat {
            at: String::new(),
            path: fs_path("/d"),
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(d.name, "d");
    assert_eq!(d.tree_bytes, 20);
    assert_eq!(d.tree_files, 5);
    // Stat vrati ulozene jmeno (velikost pismen), i kdyz se pta jinak.
    let Response::Entry(e) = s
        .call(&Request::Stat {
            at: String::new(),
            path: PathArg::Windows("\\D\\normal.TXT".into()),
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(e.name, "Normal.txt");
    // Skryty soubor jde precist podle jmena v nasem FS.
    let Response::Data(b) = s
        .call(&Request::Read {
            at: String::new(),
            path: fs_path("/d/CON.txt"),
            offset: 1,
            len: 100,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(b, b"2345");
    // Z Windows skryte jmeno nejde zadat.
    assert_eq!(
        s.call(&Request::Stat {
            at: String::new(),
            path: PathArg::Windows("\\d\\CON.txt".into())
        }),
        Err(error::NOT_FOUND)
    );
}

#[test]
fn listing_pages_cover_everything_once() {
    let mut s = Server::new();
    s.fs.create("/big", Kind::Dir).unwrap();
    // Dlouha jmena, at je stranek hodne.
    let n = 400;
    for i in 0..n {
        s.fs.create(&format!("/big/{i:04}-{}", "x".repeat(200)), Kind::File)
            .unwrap();
    }
    let list = s.list_all("", "/big");
    assert_eq!(list.len(), n);
    let mut names: Vec<_> = list.iter().map(|e| e.name.clone()).collect();
    names.dedup();
    assert_eq!(names.len(), n);
    // Zmena behem strankovani (smazani) neprinese duplikaty ani chybu.
    let first = match s
        .call(&Request::List {
            at: String::new(),
            path: fs_path("/big"),
            cursor: String::new(),
        })
        .unwrap()
    {
        Response::List(p) => p,
        _ => panic!(),
    };
    assert!(!first.next.is_empty());
    let victim = format!("/big/{}", list[n - 1].name);
    s.fs.remove(&victim).unwrap();
    let mut seen = first.entries.len();
    let mut cursor = first.next;
    while !cursor.is_empty() {
        let Response::List(p) = s
            .call(&Request::List {
                at: String::new(),
                path: fs_path("/big"),
                cursor: cursor.clone(),
            })
            .unwrap()
        else {
            panic!()
        };
        seen += p.entries.len();
        cursor = p.next;
    }
    assert_eq!(seen, n - 1);
}

#[test]
fn snapshots_and_reading_old_versions() {
    let mut s = Server::new();
    s.fs.create("/a.txt", Kind::File).unwrap();
    s.fs.write("/a.txt", 0, b"stara").unwrap();
    let Response::Created(id) = s
        .call(&Request::SnapshotCreate {
            path: fs_path("/"),
            name: "vcera".into(),
        })
        .unwrap()
    else {
        panic!()
    };
    s.fs.write("/a.txt", 0, b"NOVA!").unwrap();
    let Response::Snapshots { list: v, .. } = s
        .call(&Request::Snapshots {
            path: fs_path("/"),
            after: 0,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].id, v[0].name.as_str()), (id, "vcera"));
    let Response::Data(old) = s
        .call(&Request::Read {
            at: "vcera".into(),
            path: fs_path("/a.txt"),
            offset: 0,
            len: 10,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(old, b"stara");
    assert_eq!(s.list_all("vcera", "/").len(), 1);
    assert_eq!(
        s.call(&Request::Stat {
            at: "neni".into(),
            path: fs_path("/a.txt")
        }),
        Err(error::NOT_FOUND)
    );
}

#[test]
fn write_ops_need_write_code_and_writable_volume() {
    let mut s = Server::new();
    s.fs.create("/CON.txt", Kind::File).unwrap();
    let rename = Request::Rename {
        from: fs_path("/CON.txt"),
        to: PathArg::Windows("\\con_.txt".into()),
    };
    // Zapisova operace kodem pro cteni.
    assert_eq!(s.call_code(Code::Read, &rename), Err(error::ACCESS_DENIED));
    assert_eq!(
        s.call_code(Code::Read, &Request::Commit),
        Err(error::ACCESS_DENIED)
    );
    // Jen pro cteni.
    s.host.read_only = true;
    assert_eq!(s.call(&rename), Err(error::READ_ONLY));
    s.host.read_only = false;
    // Prejmenovani skryteho jmena na viditelne.
    assert!(matches!(s.call(&rename), Ok(Response::Done(_))));
    assert_eq!(
        s.host.renamed,
        [("/CON.txt".to_string(), "/con_.txt".to_string())]
    );
    assert!(s.host.dirty);
    assert!(s.fs.stat("/con_.txt").is_ok());
    // Cil z Windows s neplatnym jmenem.
    let bad = Request::Rename {
        from: fs_path("/con_.txt"),
        to: PathArg::Windows("\\NUL".into()),
    };
    assert_eq!(s.call(&bad), Err(error::INVALID_ARGS));
    // Commit vynuluje priznak zmeny.
    assert!(matches!(s.call(&Request::Commit), Ok(Response::Done(_))));
    assert!(!s.host.dirty);
}

#[test]
fn garbage_input_gives_error_reply() {
    let mut s = Server::new();
    for input in [
        &b""[..],
        b"x",
        &[0u8; 100][..],
        &Request::Hello.encode()[..7],
    ] {
        let out = handle(&mut s.fs, &mut s.host, Code::Read, input, &mut s.cache, 1);
        assert_eq!(out.len(), osfs_proto::RESPONSE_HEADER);
    }
    // Relativni cesta v nasem FS.
    assert_eq!(
        s.call(&Request::Stat {
            at: String::new(),
            path: fs_path("a")
        }),
        Err(error::INVALID_ARGS)
    );
    // Hello a info.
    assert!(matches!(s.call(&Request::Hello), Ok(Response::Hello(h)) if h.version == 1));
    assert!(matches!(
        s.call(&Request::VolumeInfo),
        Ok(Response::VolumeInfo(_))
    ));
}

#[test]
fn very_long_names_page_by_index_and_fit() {
    let mut s = Server::new();
    s.fs.create("/d", Kind::Dir).unwrap();
    // Slozene jmeno delsi nez MAX_CURSOR: kurzor po poradi ve snimku.
    let n = 12;
    for i in 0..n {
        s.fs.create(&format!("/d/{i:02}{}", "é".repeat(700)), Kind::File)
            .unwrap();
    }
    // Jmeno skoro 4 KiB + odkaz s 4 KiB cilem: se nevejde ani samo.
    s.fs.symlink(
        &format!("/d/z{}", "\u{1F600}".repeat(1000)),
        &"y".repeat(4000),
    )
    .unwrap();
    let list = s.list_all("", "/d");
    assert_eq!(list.len(), n + 1);
    let mut names: Vec<_> = list.iter().map(|e| e.name.clone()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), n + 1);
    let big = list.iter().find(|e| e.name.starts_with('z')).unwrap();
    assert!(big.truncated);
    assert!(list.iter().filter(|e| e.truncated).count() == 1);
}

#[test]
fn snapshot_list_pages() {
    let mut s = Server::new();
    let n = 150u64;
    for i in 0..n {
        s.fs.snapshot(&format!("snapshot-cislo-{i:04}-{}", "s".repeat(30)))
            .unwrap();
    }
    let mut all = Vec::new();
    let mut after = 0;
    let mut pages = 0;
    loop {
        let Response::Snapshots { list, next } = s
            .call(&Request::Snapshots {
                path: fs_path("/"),
                after,
            })
            .unwrap()
        else {
            panic!()
        };
        pages += 1;
        all.extend(list);
        if next == 0 {
            break;
        }
        after = next;
    }
    assert!(pages > 1);
    assert_eq!(all.len() as u64, n);
}
