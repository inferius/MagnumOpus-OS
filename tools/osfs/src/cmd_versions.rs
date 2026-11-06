//! Prikazy pro datasety a verze nad souborem s obrazem:
//! `dataset create|list|delete`, `snapshot`, `snapshots`, `clone`, `diff`,
//! `restore`.
//!
//! Vsechny obraz otviraji vylucne (`image::open_exclusive`): nad pripojenym
//! obrazem bezpecne odmitnou. Cesty: dataset a cil obnovy jsou cesty ve
//! svazku (`/projekt`), snapshot, zdroj obnovy a vypis rozdilu pracuji
//! s cestou uvnitr datasetu (`/cfg.toml`).

use std::path::Path;

use osfs::image;
use osfs::versions::{change_line, datasets};

use crate::{Args, remote};

fn err(what: &str, e: fs::Error) -> String {
    format!("{what}: {e}")
}

/// `osfs dataset create|list|delete <obraz> [cesta]`
pub fn cmd_dataset(raw: &[String]) -> Result<(), String> {
    let usage = "osfs dataset create|list|delete <obraz> [cesta]";
    let a = Args::parse(raw, &[])?;
    a.only(&[])?;
    let sub = a.pos.first().ok_or(format!("pouziti: {usage}"))?.as_str();
    match sub {
        "create" | "delete" => {
            a.need(3, 3, usage)?;
            let path = &a.pos[2];
            let (mut fs, _lock) = image::open_exclusive(Path::new(&a.pos[1]), false)?;
            if sub == "create" {
                let id = fs.create_dataset(path).map_err(|e| err(path, e))?;
                println!("{path}: dataset vytvoren (cislo {id})");
            } else {
                fs.delete_dataset(path).map_err(|e| match e {
                    fs::Error::BadState => format!(
                        "{path}: z jeho snapshotu existuje klon - nejdriv smaz klon \
                         (osfs dataset list ukaze, ktery)"
                    ),
                    fs::Error::NotEmpty => {
                        format!("{path}: obsahuje vnoreny dataset - nejdriv smaz ten")
                    }
                    e => err(path, e),
                })?;
                println!("{path}: dataset smazan i se snapshoty");
            }
            // Operace s datasety commituji samy; commit pro jistotu.
            fs.commit().map_err(|e| err("commit", e))
        }
        "list" => {
            a.need(2, 2, usage)?;
            let (mut fs, _lock) = image::open_exclusive(Path::new(&a.pos[1]), true)?;
            for d in datasets(&mut fs).map_err(|e| err("datasety", e))? {
                let snaps = fs.snapshots_of(&d.path).map(|v| v.len()).unwrap_or(0);
                let mut line = format!("{:>4}  {}  (snapshotu: {snaps})", d.id, d.path);
                if let Some(o) = d.origin {
                    line.push_str(&format!("  klon z {o}"));
                }
                println!("{line}");
            }
            Ok(())
        }
        other => Err(format!(
            "neznamy podprikaz dataset {other} (pouziti: {usage})"
        )),
    }
}

/// `osfs snapshot <obraz|X:> <dataset> <jmeno>`
pub fn cmd_snapshot(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &[])?;
    a.only(&[])?;
    a.need(3, 3, "osfs snapshot <obraz|X:> <dataset> <jmeno>")?;
    let (dataset, name) = (&a.pos[1], &a.pos[2]);
    if remote::is_drive(&a.pos[0]) {
        return remote::snapshot(&a.pos[0], dataset, name);
    }
    let (mut fs, _lock) = image::open_exclusive(Path::new(&a.pos[0]), false)?;
    let id = fs
        .snapshot_of(dataset, name)
        .map_err(|e| err(&format!("snapshot {dataset}@{name}"), e))?;
    println!("{dataset}@{name}: snapshot vytvoren (id {id})");
    Ok(())
}

/// `osfs snapshots <obraz|X:> [dataset]` - bez datasetu vsechny datasety.
pub fn cmd_snapshots(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &[])?;
    a.only(&[])?;
    a.need(1, 2, "osfs snapshots <obraz|X:> [dataset]")?;
    if remote::is_drive(&a.pos[0]) {
        let ds = a.pos.get(1).map(String::as_str).unwrap_or("/");
        return remote::snapshots(&a.pos[0], ds);
    }
    let (mut fs, _lock) = image::open_exclusive(Path::new(&a.pos[0]), true)?;
    let list: Vec<String> = match a.pos.get(1) {
        Some(d) => vec![d.clone()],
        None => datasets(&mut fs)
            .map_err(|e| err("datasety", e))?
            .into_iter()
            .map(|d| d.path)
            .collect(),
    };
    for ds in list {
        for s in fs.snapshots_of(&ds).map_err(|e| err(&ds, e))? {
            println!("{ds}@{}  (id {}, generace {})", s.name, s.id, s.generation);
        }
    }
    Ok(())
}

/// `osfs clone <obraz> <dataset> <snapshot> <cesta>`
pub fn cmd_clone(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &[])?;
    a.only(&[])?;
    a.need(4, 4, "osfs clone <obraz> <dataset> <snapshot> <cesta>")?;
    let (dataset, snap, path) = (&a.pos[1], &a.pos[2], &a.pos[3]);
    let (mut fs, _lock) = image::open_exclusive(Path::new(&a.pos[0]), false)?;
    let id = fs
        .clone_snapshot(dataset, snap, path)
        .map_err(|e| err(&format!("klon {dataset}@{snap}"), e))?;
    println!("{path}: klon {dataset}@{snap} (dataset {id}), zapisovatelny, sdili bloky");
    Ok(())
}

/// `osfs diff <obraz> <dataset> <from> [to]` nebo `osfs diff <obraz> --clone <cesta>`
pub fn cmd_diff(raw: &[String]) -> Result<(), String> {
    let usage = "osfs diff <obraz> <dataset> <from> [to]  |  osfs diff <obraz> --clone <cesta>";
    let a = Args::parse(raw, &["clone"])?;
    a.only(&["clone"])?;
    let changes = if let Some(clone) = a.value("clone") {
        a.need(1, 1, usage)?;
        let (mut fs, _lock) = image::open_exclusive(Path::new(&a.pos[0]), true)?;
        fs.diff_clone(clone).map_err(|e| err(clone, e))?
    } else {
        a.need(3, 4, usage)?;
        let (dataset, from) = (&a.pos[1], &a.pos[2]);
        let to = a.pos.get(3).map(String::as_str);
        let (mut fs, _lock) = image::open_exclusive(Path::new(&a.pos[0]), true)?;
        fs.diff(dataset, from, to)
            .map_err(|e| err(&format!("diff {dataset}@{from}"), e))?
    };
    if changes.is_empty() {
        println!("bez zmen");
    }
    for c in &changes {
        println!("{}", change_line(c));
    }
    Ok(())
}

/// `osfs restore <obraz> <dataset> <snapshot> <cesta> <cil>` - obnova jako
/// kopie vedle originalu (cil nesmi existovat).
pub fn cmd_restore(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &[])?;
    a.only(&[])?;
    a.need(
        5,
        5,
        "osfs restore <obraz> <dataset> <snapshot> <cesta> <cil>",
    )?;
    let (dataset, snap, path, dest) = (&a.pos[1], &a.pos[2], &a.pos[3], &a.pos[4]);
    let (mut fs, _lock) = image::open_exclusive(Path::new(&a.pos[0]), false)?;
    let n = fs
        .restore_copy(dataset, snap, path, dest)
        .map_err(|e| err(&format!("obnova {dataset}@{snap}:{path}"), e))?;
    fs.commit().map_err(|e| err("commit", e))?;
    println!("{dataset}@{snap}:{path} -> {dest}: obnoveno {n} B (kopie, original beze zmeny)");
    Ok(())
}
