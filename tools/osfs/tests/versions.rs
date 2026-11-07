//! Datasety a verze: dataset k ceste, seznam datasetu, radky rozdilu
//! a rozsirene API nad datasety (bez WinFsp, `Fs` v pameti).

use fs::{Change, ChangeKind, FormatOptions, Fs, Kind, MemDevice};
use osfs::api::{Code, Host, ListCache, handle};
use osfs::versions::{change_line, dataset_of, datasets, is_dataset_root};
use osfs_proto::{PathArg, Request, Response};

fn new_fs() -> Fs<MemDevice> {
    let dev = Fs::format(MemDevice::new(8192), &FormatOptions::default()).unwrap();
    Fs::mount(dev).unwrap()
}

/// Projekt: dataset `/projekt` se dvema soubory a snapshotem `v1`.
fn project() -> Fs<MemDevice> {
    let mut fs = new_fs();
    fs.create("/data", Kind::Dir).unwrap();
    fs.create_dataset("/data/projekt").unwrap();
    fs.create("/data/projekt/src", Kind::Dir).unwrap();
    fs.create("/data/projekt/cfg.toml", Kind::File).unwrap();
    fs.write("/data/projekt/cfg.toml", 0, b"a = 1\n").unwrap();
    fs.create("/data/projekt/src/main.rs", Kind::File).unwrap();
    fs.write("/data/projekt/src/main.rs", 0, b"fn main() {}\n")
        .unwrap();
    fs.snapshot_of("/data/projekt", "v1").unwrap();
    fs
}

#[test]
fn dataset_of_paths() {
    let mut fs = project();
    let d = dataset_of(&mut fs, "/data/projekt/src/main.rs").unwrap();
    assert_eq!(d.root, "/data/projekt");
    assert_eq!(d.inner, "/src/main.rs");
    assert_ne!(d.id, 1);
    // Cesta mimo dataset.
    let r = dataset_of(&mut fs, "/data").unwrap();
    assert_eq!((r.root.as_str(), r.id, r.inner.as_str()), ("/", 1, "/data"));
    // Koren datasetu a neexistujici zbytek cesty.
    assert_eq!(dataset_of(&mut fs, "/data/projekt").unwrap().inner, "/");
    let gone = dataset_of(&mut fs, "/data/projekt/smazano/x").unwrap();
    assert_eq!(
        (gone.root.as_str(), gone.inner.as_str()),
        ("/data/projekt", "/smazano/x")
    );
    assert!(is_dataset_root(&mut fs, "/data/projekt"));
    assert!(!is_dataset_root(&mut fs, "/data"));
    assert!(!is_dataset_root(&mut fs, "/"));
    assert!(!is_dataset_root(&mut fs, "/data/projekt/src"));
}

#[test]
fn dataset_listing_with_clone_origin() {
    let mut fs = project();
    fs.clone_snapshot("/data/projekt", "v1", "/bokem").unwrap();
    let rows = datasets(&mut fs).unwrap();
    let paths: Vec<_> = rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["/", "/bokem", "/data/projekt"]);
    assert_eq!(rows[1].origin.as_deref(), Some("/data/projekt@v1"));
    assert_eq!(rows[2].origin, None);
}

#[test]
fn change_lines() {
    let c = |kind, path: &str, old: Option<&str>, is_dir| Change {
        kind,
        path: path.into(),
        old_path: old.map(String::from),
        oid: 1,
        is_dir,
    };
    assert_eq!(
        change_line(&c(ChangeKind::Added, "/a", None, false)),
        "A  /a"
    );
    assert_eq!(
        change_line(&c(ChangeKind::Removed, "/d", None, true)),
        "D  /d/"
    );
    assert_eq!(
        change_line(&c(ChangeKind::Moved, "/b", Some("/a"), false)),
        "R  /a -> /b"
    );
    assert_eq!(
        change_line(&c(ChangeKind::MovedModified, "/b", Some("/a"), false)),
        "RM /a -> /b"
    );
}

#[test]
fn first_demo_snapshot_clone_diff_restore() {
    // Prvni ukazka verzovane slozky: snapshot, klon, zmeny, diff, obnova.
    let mut fs = project();
    fs.write("/data/projekt/cfg.toml", 0, b"a = 2\n").unwrap();
    fs.rename("/data/projekt/src/main.rs", "/data/projekt/src/app.rs")
        .unwrap();
    let lines: Vec<String> = fs
        .diff("/data/projekt", "v1", None)
        .unwrap()
        .iter()
        .map(change_line)
        .collect();
    assert_eq!(lines, ["M  /cfg.toml", "R  /src/main.rs -> /src/app.rs"]);
    fs.clone_snapshot("/data/projekt", "v1", "/zkouska")
        .unwrap();
    fs.write("/zkouska/cfg.toml", 0, b"a = 9\n").unwrap();
    let lines: Vec<String> = fs
        .diff_clone("/zkouska")
        .unwrap()
        .iter()
        .map(change_line)
        .collect();
    assert_eq!(lines, ["M  /cfg.toml"]);
    fs.restore_copy(
        "/data/projekt",
        "v1",
        "/cfg.toml",
        "/data/projekt/cfg (v1).toml",
    )
    .unwrap();
    assert_eq!(
        fs.read("/data/projekt/cfg (v1).toml", 0, 10).unwrap(),
        b"a = 1\n"
    );
    assert_eq!(
        fs.read("/data/projekt/cfg.toml", 0, 10).unwrap(),
        b"a = 2\n"
    );
}

#[derive(Default)]
struct H;

impl Host for H {
    fn mutating(&mut self) -> Result<(), i32> {
        Ok(())
    }
    fn renamed(&mut self, _: &str, _: &str) {}
    fn committed(&mut self) {}
}

fn call(fs: &mut Fs<MemDevice>, req: &Request) -> Result<Response, i32> {
    let mut cache: Option<ListCache> = None;
    let code = if req.is_write() {
        Code::Write
    } else {
        Code::Read
    };
    Response::decode(
        req.op(),
        &handle(fs, &mut H, code, &req.encode(), &mut cache, fs::Timestamp::new(1, 0)),
    )
}

#[test]
fn api_snapshots_follow_the_dataset_of_the_path() {
    let mut fs = project();
    fs.write("/data/projekt/cfg.toml", 0, b"a = 2\n").unwrap();
    let p = |s: &str| PathArg::Fs(s.into());
    // Snapshoty datasetu, ve kterem cesta lezi.
    let Response::Snapshots { list, .. } = call(
        &mut fs,
        &Request::Snapshots {
            path: p("/data/projekt/src"),
            after: 0,
        },
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(
        list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["v1"]
    );
    let Response::Snapshots { list, .. } = call(
        &mut fs,
        &Request::Snapshots {
            path: p("/data"),
            after: 0,
        },
    )
    .unwrap() else {
        panic!()
    };
    assert!(list.is_empty());
    // Cteni stare verze: cesta ve svazku, `at` = snapshot datasetu.
    let Response::Data(d) = call(
        &mut fs,
        &Request::Read {
            at: "v1".into(),
            path: p("/data/projekt/cfg.toml"),
            offset: 0,
            len: 100,
        },
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(d, b"a = 1\n");
    // Ve vypisu je dataset druhu Dataset, s vlastnim cislem.
    let Response::List(page) = call(
        &mut fs,
        &Request::List {
            at: String::new(),
            path: p("/data"),
            cursor: String::new(),
        },
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].kind, osfs_proto::Kind::Dataset);
    assert_ne!(page.entries[0].dataset, 1);
    let Response::Entry(e) = call(
        &mut fs,
        &Request::Stat {
            at: String::new(),
            path: p("/data/projekt"),
        },
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(e.kind, osfs_proto::Kind::Dataset);
    // Vypis korene datasetu ve snapshotu.
    let Response::List(page) = call(
        &mut fs,
        &Request::List {
            at: "v1".into(),
            path: p("/data/projekt"),
            cursor: String::new(),
        },
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(page.entries.len(), 2);
    // Snapshot pres API do datasetu podle cesty.
    assert!(matches!(
        call(
            &mut fs,
            &Request::SnapshotCreate {
                path: p("/data/projekt/cfg.toml"),
                name: "v2".into()
            }
        ),
        Ok(Response::Created(_))
    ));
    assert_eq!(fs.snapshots_of("/data/projekt").unwrap().len(), 2);
}
