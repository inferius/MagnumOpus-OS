//! Prikazova radka od zacatku do konce: prvni ukazka verzovane slozky
//! (dataset, snapshot, klon, zmeny, diff, obnova jako kopie) a odmitnuti
//! prikazu nad obrazem, ktery ma otevreny jiny proces.

use std::path::{Path, PathBuf};
use std::process::Command;

fn osfs(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_osfs"))
        .args(args)
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn ok(args: &[&str]) -> String {
    let (success, text) = osfs(args);
    assert!(success, "osfs {args:?} selhal:\n{text}");
    text
}

struct Temp {
    dir: PathBuf,
}

impl Temp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("osfs-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn file(&self, name: &str, content: &[u8]) -> String {
        let p = self.dir.join(name);
        std::fs::write(&p, content).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn first_versioning_demo() {
    let t = Temp::new("demo");
    let img = t.path("v.img");
    ok(&["format", &img, "--size", "16M"]);
    assert!(ok(&["dataset", "create", &img, "/projekt"]).contains("dataset vytvoren"));
    let v1 = t.file("cfg1.toml", b"a = 1\n");
    let v2 = t.file("cfg2.toml", b"a = 2\n");
    ok(&["put", &img, &v1, "/projekt/cfg.toml"]);
    ok(&["put", &img, &v1, "/projekt/navod.txt"]);
    ok(&["snapshot", &img, "/projekt", "v1"]);
    assert!(ok(&["snapshots", &img]).contains("/projekt@v1"));

    // Zmena v zive verzi a rozdil proti snapshotu.
    ok(&["put", &img, &v2, "/projekt/cfg.toml"]);
    ok(&["put", &img, &v1, "/projekt/novy.txt"]);
    let d = ok(&["diff", &img, "/projekt", "v1"]);
    assert!(d.contains("M  /cfg.toml"), "{d}");
    assert!(d.contains("A  /novy.txt"), "{d}");

    // Zapisovatelny klon: zmeny bokem, original beze zmeny.
    ok(&["clone", &img, "/projekt", "v1", "/zkouska"]);
    ok(&["put", &img, &v2, "/zkouska/navod.txt"]);
    let d = ok(&["diff", &img, "--clone", "/zkouska"]);
    assert_eq!(d.trim(), "M  /navod.txt");
    assert_eq!(ok(&["cat", &img, "/projekt/navod.txt"]), "a = 1\n");
    let list = ok(&["dataset", "list", &img]);
    assert!(
        list.contains("/zkouska") && list.contains("klon z /projekt@v1"),
        "{list}"
    );

    // Obnova jako kopie vedle originalu.
    ok(&[
        "restore",
        &img,
        "/projekt",
        "v1",
        "/cfg.toml",
        "/projekt/cfg (v1).toml",
    ]);
    assert_eq!(ok(&["cat", &img, "/projekt/cfg (v1).toml"]), "a = 1\n");
    assert_eq!(ok(&["cat", &img, "/projekt/cfg.toml"]), "a = 2\n");
    // Cil obnovy uz existuje.
    let (success, _) = osfs(&[
        "restore",
        &img,
        "/projekt",
        "v1",
        "/cfg.toml",
        "/projekt/cfg.toml",
    ]);
    assert!(!success);

    // Snapshot ve dvou datasetech a rozdil mezi dvema snapshoty.
    ok(&["snapshot", &img, "/projekt", "v2"]);
    let d = ok(&["diff", &img, "/projekt", "v1", "v2"]);
    assert!(d.contains("A  /cfg (v1).toml"), "{d}");
    // Klon drzi snapshot: dataset /projekt nejde smazat, klon ano.
    assert!(!osfs(&["dataset", "delete", &img, "/projekt"]).0);
    ok(&["dataset", "delete", &img, "/zkouska"]);
    assert!(!ok(&["dataset", "list", &img]).contains("/zkouska"));
    assert!(ok(&["check", &img, "--scrub"]).contains("v poradku"));
}

#[test]
fn commands_refuse_an_image_open_elsewhere() {
    let t = Temp::new("lock");
    let img = t.path("l.img");
    ok(&["format", &img, "--size", "8M"]);
    ok(&["dataset", "create", &img, "/d"]);
    // Jako bezici `osfs mount`: obraz otevreny pro zapis v jinem "procesu".
    let (_fs, _lock) = osfs::image::open_exclusive(Path::new(&img), false).unwrap();
    for args in [
        vec!["snapshot", &img, "/d", "s"],
        vec!["snapshots", &img],
        vec!["dataset", "list", &img],
        vec!["dataset", "create", &img, "/e"],
        vec!["diff", &img, "/d", "s"],
        vec!["clone", &img, "/d", "s", "/k"],
        vec!["restore", &img, "/d", "s", "/x", "/y"],
        vec!["put", &img, &img, "/z"],
    ] {
        let (success, text) = osfs(&args);
        assert!(!success, "{args:?} melo selhat");
        assert!(
            text.contains("jiny proces osfs") || text.contains("pripojeny"),
            "{args:?}: {text}"
        );
    }
}
