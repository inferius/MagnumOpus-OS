//! `osfs` - prikazova radka. Popis prikazu: `osfs help` nebo
//! `docs/cs/osfs.md`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fs::{Algo, Kind};
use osfs::image::{self, DEFAULT_SIZE};

mod cmd_versions;
mod remote;

const HELP: &str = "\
osfs - souborovy system MagnumOpus na hostiteli (zatim jen soubory s obrazem)

  osfs format <obraz> [--size N] [--label X] [--blake3]
        vytvori NOVY soubor s obrazem a naformatuje ho (N napr. 64M, 1G;
        vychozi 256M). Existujici soubor se neprepise.
  osfs check <obraz> [--scrub]
        kontrola konzistence bez zapisu; --scrub overi i data.
        Vypise i polozky, ktere se ve Windows nezobrazi.
  osfs ls <obraz> [cesta]          vypis adresare (vychozi /)
  osfs cat <obraz> <cesta>         obsah souboru na standardni vystup
  osfs put <obraz> <soubor> <cesta> zkopiruje mistni soubor do obrazu
  osfs mount <obraz> <X:>          pripoji obraz jako jednotku (WinFsp);
                                   bezi, dokud se neodpoji (Ctrl+C)
  osfs unmount <X:>                odpoji jednotku z jineho okna

Datasety a verze (slozka s vlastni historii; obraz nesmi byt pripojeny):
  osfs dataset create <obraz> <cesta>       novy dataset (slozka)
  osfs dataset list <obraz>                 seznam datasetu
  osfs dataset delete <obraz> <cesta>       smaze dataset i se snapshoty
  osfs snapshot <obraz> <dataset> <jmeno>   snapshot datasetu
  osfs snapshots <obraz> [dataset]          seznam snapshotu
  osfs clone <obraz> <dataset> <snapshot> <cesta>
                                            zapisovatelny klon snapshotu
  osfs diff <obraz> <dataset> <od> [do]     zmeny mezi snapshoty (bez [do]
                                            proti zive verzi)
  osfs diff <obraz> --clone <cesta>         co zmenil klon
  osfs restore <obraz> <dataset> <snapshot> <cesta> <cil>
                                            obnova jako kopie vedle originalu
  Cesty <cesta> u snapshotu a obnovy jsou uvnitr datasetu (/cfg.toml),
  dataset a <cil> jsou cesty ve svazku (/projekt).

Nad pripojenou jednotkou (rozsirene API, cesty v nasem FS, i skryta jmena):
  osfs info <X:>                   server a svazek
  osfs ls <X:> [cesta] [--at S]    vypis (--at = ze snapshotu S)
  osfs cat <X:> <cesta> [--at S]   obsah souboru
  osfs snapshots <X:> [dataset]    seznam snapshotu datasetu (vychozi /)
  osfs snapshot <X:> <dataset> <jmeno>  vytvori snapshot

Cesty v obrazu pisou se s '/', napr. /dokumenty/a.txt.
Skutecne disky osfs zatim odmita: formatovani disku je nevratne.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("osfs: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Rozdeli argumenty na pozicni a volby `--x [hodnota]`.
struct Args {
    pos: Vec<String>,
    opts: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(raw: &[String], with_value: &[&str]) -> Result<Self, String> {
        let mut pos = Vec::new();
        let mut opts = Vec::new();
        let mut it = raw.iter();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                if with_value.contains(&name) {
                    let v = it
                        .next()
                        .ok_or_else(|| format!("--{name} potrebuje hodnotu"))?;
                    opts.push((name.to_string(), Some(v.clone())));
                } else {
                    opts.push((name.to_string(), None));
                }
            } else {
                pos.push(a.clone());
            }
        }
        Ok(Self { pos, opts })
    }

    fn flag(&self, name: &str) -> bool {
        self.opts.iter().any(|(n, _)| n == name)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.opts
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    fn only(&self, allowed: &[&str]) -> Result<(), String> {
        for (n, _) in &self.opts {
            if !allowed.contains(&n.as_str()) {
                return Err(format!("neznama volba --{n}"));
            }
        }
        Ok(())
    }

    fn need(&self, min: usize, max: usize, usage: &str) -> Result<(), String> {
        if self.pos.len() < min || self.pos.len() > max {
            return Err(format!("pouziti: {usage}"));
        }
        Ok(())
    }
}

fn run(raw: &[String]) -> Result<(), String> {
    let Some(cmd) = raw.first() else {
        print!("{HELP}");
        return Ok(());
    };
    let rest = &raw[1..];
    match cmd.as_str() {
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            Ok(())
        }
        "format" => cmd_format(rest),
        "check" => cmd_check(rest),
        "ls" => cmd_ls(rest),
        "cat" => cmd_cat(rest),
        "put" => cmd_put(rest),
        "mount" => cmd_mount(rest),
        "info" => cmd_info(rest),
        "snapshots" => cmd_versions::cmd_snapshots(rest),
        "snapshot" => cmd_versions::cmd_snapshot(rest),
        "dataset" => cmd_versions::cmd_dataset(rest),
        "clone" => cmd_versions::cmd_clone(rest),
        "diff" => cmd_versions::cmd_diff(rest),
        "restore" => cmd_versions::cmd_restore(rest),
        "unmount" => cmd_unmount(rest),
        other => Err(format!("neznamy prikaz {other} (osfs help)")),
    }
}

fn cmd_format(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &["size", "label"])?;
    a.only(&["size", "label", "blake3"])?;
    a.need(
        1,
        1,
        "osfs format <obraz> [--size N] [--label X] [--blake3]",
    )?;
    let size = match a.value("size") {
        Some(s) => image::parse_size(s)?,
        None => DEFAULT_SIZE,
    };
    let label = a.value("label").unwrap_or("");
    let algo = if a.flag("blake3") {
        Algo::Blake3
    } else {
        Algo::Xxh3
    };
    let path = PathBuf::from(&a.pos[0]);
    image::create_image(&path, size, label, algo)?;
    println!(
        "{}: naformatovano, {} MiB, kontrolni soucty {:?}",
        path.display(),
        size >> 20,
        algo
    );
    Ok(())
}

fn cmd_check(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &[])?;
    a.only(&["scrub"])?;
    a.need(1, 1, "osfs check <obraz> [--scrub]")?;
    let (mut fs, _lock) = image::open(Path::new(&a.pos[0]), true)?;
    let info = fs.info();
    println!(
        "svazek '{}': {} bloku po {} B, volnych {}, generace {}, soucty {:?}",
        info.label, info.block_count, info.block_size, info.free_blocks, info.generation, info.algo
    );
    if info.fallback > 0 {
        println!(
            "POZOR: pripojeni se vratilo o {} generaci (poskozeny novejsi stav)",
            info.fallback
        );
    }
    if info.unreadable_superblocks > 0 {
        println!(
            "POZOR: necitelnych slotu superbloku: {}",
            info.unreadable_superblocks
        );
    }
    if info.newest_uncertain {
        println!("POZOR: nejnovejsi stav nejde bezpecne urcit");
    }
    if info.log_unreadable {
        println!("POZOR: intent log nesel precist cely");
    }
    if info.replayed > 0 {
        println!(
            "z intent logu prehrano {} operaci (jen v pameti)",
            info.replayed
        );
    }
    let r = fs
        .check(a.flag("scrub"))
        .map_err(|e| format!("kontrola: {e}"))?;
    println!(
        "generace {}: uzly stromu {} bloku, data {} bloku, metadata {} bloku",
        r.generation, r.tree_blocks, r.data_blocks, r.meta_blocks
    );
    if r.live_not_checked {
        println!("(zmeny prehrane z logu jen v pameti zkontrolovane nejsou)");
    }
    let hidden = image::hidden_entries(&mut fs).map_err(|e| format!("pruchod stromem: {e}"))?;
    if !hidden.is_empty() {
        println!("ve Windows se nezobrazi ({}):", hidden.len());
        for h in &hidden {
            println!("  {}  - {}", h.path, h.why);
        }
    }
    if r.problems_total == 0 {
        println!("v poradku");
        Ok(())
    } else {
        for p in &r.problems {
            println!("PROBLEM: {p}");
        }
        Err(format!("nalezeno problemu: {}", r.problems_total))
    }
}

fn cmd_ls(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &["at"])?;
    a.only(&["at"])?;
    a.need(1, 2, "osfs ls <obraz|X:> [cesta]")?;
    let path = a.pos.get(1).map(String::as_str).unwrap_or("/");
    if remote::is_drive(&a.pos[0]) {
        return remote::ls(&a.pos[0], path, a.value("at").unwrap_or(""));
    }
    if a.value("at").is_some() {
        return Err("--at zatim jen nad pripojenou jednotkou".into());
    }
    let (mut fs, _lock) = image::open(Path::new(&a.pos[0]), true)?;
    for e in fs.list(path).map_err(|e| format!("{path}: {e}"))? {
        let full = image::join(path, &e.name);
        let st = fs.stat(&full).map_err(|e| format!("{full}: {e}"))?;
        let (kind, size) = match e.kind {
            Kind::Dir => ("dir ", st.tree_bytes),
            Kind::File => ("file", st.size),
            Kind::Symlink => ("link", st.size),
            // Slozka s vlastni historii (stat vraci jeji korenovy adresar).
            Kind::Dataset => ("dset", st.tree_bytes),
        };
        let mut line = format!("{kind} {size:>12}  {}", e.name);
        if e.kind == Kind::Symlink
            && let Ok(t) = fs.readlink(&full)
        {
            line.push_str(&format!(" -> {t}"));
        }
        if osfs::names::to_windows(&e.name).is_none() {
            line.push_str("   [ve Windows skryte]");
        }
        println!("{line}");
    }
    Ok(())
}

fn cmd_cat(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &["at"])?;
    a.only(&["at"])?;
    a.need(2, 2, "osfs cat <obraz|X:> <cesta>")?;
    let path = &a.pos[1];
    if remote::is_drive(&a.pos[0]) {
        return remote::cat(&a.pos[0], path, a.value("at").unwrap_or(""));
    }
    if a.value("at").is_some() {
        return Err("--at zatim jen nad pripojenou jednotkou".into());
    }
    let (mut fs, _lock) = image::open(Path::new(&a.pos[0]), true)?;
    let size = fs.stat(path).map_err(|e| format!("{path}: {e}"))?.size;
    let mut out = std::io::stdout().lock();
    let mut off = 0u64;
    const CHUNK: usize = 1 << 20;
    while off < size {
        let n = (size - off).min(CHUNK as u64) as usize;
        let data = fs.read(path, off, n).map_err(|e| format!("{path}: {e}"))?;
        if data.is_empty() {
            break;
        }
        out.write_all(&data).map_err(|e| e.to_string())?;
        off += data.len() as u64;
    }
    out.flush().map_err(|e| e.to_string())
}

fn cmd_put(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &[])?;
    a.only(&[])?;
    a.need(3, 3, "osfs put <obraz> <soubor> <cesta>")?;
    let (img, local, path) = (&a.pos[0], &a.pos[1], &a.pos[2]);
    let mut src = std::fs::File::open(local).map_err(|e| format!("{local}: {e}"))?;
    let (mut fs, _lock) = image::open(Path::new(img), false)?;
    if fs.info().read_only {
        return Err("svazek je pripojeny jen pro cteni (viz osfs check)".into());
    }
    match fs.stat(path) {
        Ok(st) if st.kind == Kind::File => {
            fs.truncate(path, 0).map_err(|e| format!("{path}: {e}"))?
        }
        Ok(_) => return Err(format!("{path}: existuje a neni to soubor")),
        Err(fs::Error::NotFound) => fs
            .create(path, Kind::File)
            .map_err(|e| format!("{path}: {e}"))?,
        Err(e) => return Err(format!("{path}: {e}")),
    }
    let mut buf = vec![0u8; 1 << 20];
    let mut off = 0u64;
    loop {
        let n = src.read(&mut buf).map_err(|e| format!("{local}: {e}"))?;
        if n == 0 {
            break;
        }
        fs.write(path, off, &buf[..n])
            .map_err(|e| format!("{path}: {e}"))?;
        off += n as u64;
    }
    fs.commit().map_err(|e| format!("commit: {e}"))?;
    println!("{path}: zapsano {off} B");
    Ok(())
}

#[cfg(windows)]
fn cmd_mount(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &[])?;
    a.only(&[])?;
    a.need(2, 2, "osfs mount <obraz> <X:>")?;
    osfs::mount::run(Path::new(&a.pos[0]), &a.pos[1])
}

fn cmd_info(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &[])?;
    a.only(&[])?;
    a.need(1, 1, "osfs info <X:>")?;
    remote::info(&a.pos[0])
}

#[cfg(windows)]
fn cmd_unmount(raw: &[String]) -> Result<(), String> {
    let a = Args::parse(raw, &[])?;
    a.only(&[])?;
    a.need(1, 1, "osfs unmount <X:>")?;
    osfs::mount::unmount(&a.pos[0])
}

#[cfg(not(windows))]
fn cmd_unmount(_raw: &[String]) -> Result<(), String> {
    Err("osfs unmount je zatim jen pro Windows (WinFsp)".into())
}

#[cfg(not(windows))]
fn cmd_mount(_raw: &[String]) -> Result<(), String> {
    Err("osfs mount je zatim jen pro Windows (WinFsp)".into())
}
