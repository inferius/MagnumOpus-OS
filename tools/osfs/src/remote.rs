//! Prikazy nad pripojenou jednotkou pres rozsirene API (`osfs-proto`
//! klient) - ukazka a test toho, co muze pouzit kazda aplikace.
//!
//! `osfs info Q:`, `osfs ls Q: [cesta]`, `osfs cat Q: <cesta>`,
//! `osfs snapshots Q:`, `osfs snapshot Q: <jmeno>`. Cesty jsou v nasem FS
//! (`/a/b`), takze jdou i jmena, ktera Windows nezobrazi.

/// Je argument pismeno jednotky (`Q:`), ne cesta k obrazu?
pub fn is_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

#[cfg(windows)]
mod imp {
    use std::io::Write;
    use std::path::PathBuf;

    use osfs_proto::client::Client;
    use osfs_proto::{Hidden, Kind, PathArg};

    fn connect(drive: &str) -> Result<Client, String> {
        let root = PathBuf::from(format!("{drive}\\"));
        if !Client::is_osfs(&root) {
            return Err(format!("{drive} neni pripojeny svazek MagnumOpusFS"));
        }
        Client::open(&root).map_err(|e| format!("{drive}: {e}"))
    }

    fn hidden_note(h: Hidden) -> &'static str {
        match h {
            Hidden::Visible => "",
            Hidden::PrivateUse => "   [ve Windows skryte: znak U+F0xx]",
            Hidden::Reserved => "   [ve Windows skryte: rezervovane jmeno]",
            Hidden::TrailingDotOrSpace => "   [ve Windows skryte: koncova tecka/mezera]",
            Hidden::TooLong => "   [ve Windows skryte: prilis dlouhe]",
            Hidden::Symlink => "   [ve Windows skryte: odkaz]",
            Hidden::Other => "   [ve Windows skryte]",
        }
    }

    pub fn info(drive: &str) -> Result<(), String> {
        let mut c = connect(drive)?;
        let h = c.hello().map_err(|e| e.to_string())?;
        let v = c.volume_info().map_err(|e| e.to_string())?;
        println!(
            "server: {} (protokol {}, schopnosti 0x{:x})",
            h.server, h.version, h.features
        );
        println!(
            "svazek '{}': {} bloku po {} B, volnych {}, generace {}{}",
            v.label,
            v.block_count,
            v.block_size,
            v.free_blocks,
            v.generation,
            if v.read_only { ", jen pro cteni" } else { "" }
        );
        Ok(())
    }

    pub fn ls(drive: &str, path: &str, at: &str) -> Result<(), String> {
        let mut c = connect(drive)?;
        let list = c
            .list_all(at, &PathArg::Fs(path.into()))
            .map_err(|e| format!("{path}: {e}"))?;
        for e in list {
            let (kind, size) = match e.kind {
                Kind::Dir => ("dir ", e.tree_bytes),
                Kind::File => ("file", e.size),
                Kind::Symlink => ("link", e.size),
            };
            let mut line = format!("{kind} {size:>12}  {}", e.name);
            if !e.link_target.is_empty() {
                line.push_str(&format!(" -> {}", e.link_target));
            }
            line.push_str(hidden_note(e.hidden));
            println!("{line}");
        }
        Ok(())
    }

    pub fn cat(drive: &str, path: &str, at: &str) -> Result<(), String> {
        let mut c = connect(drive)?;
        let p = PathArg::Fs(path.into());
        let size = c.stat(at, &p).map_err(|e| format!("{path}: {e}"))?.size;
        let mut out = std::io::stdout().lock();
        let mut off = 0u64;
        while off < size {
            let n = (size - off).min(1 << 20) as usize;
            let d = c.read(at, &p, off, n).map_err(|e| format!("{path}: {e}"))?;
            if d.is_empty() {
                break;
            }
            out.write_all(&d).map_err(|e| e.to_string())?;
            off += d.len() as u64;
        }
        out.flush().map_err(|e| e.to_string())
    }

    pub fn snapshots(drive: &str) -> Result<(), String> {
        let mut c = connect(drive)?;
        for s in c.snapshots().map_err(|e| e.to_string())? {
            println!("{:>4}  {}  (generace {})", s.id, s.name, s.generation);
        }
        Ok(())
    }

    pub fn snapshot(drive: &str, name: &str) -> Result<(), String> {
        let mut c = connect(drive)?;
        let id = c
            .snapshot_create(name)
            .map_err(|e| format!("snapshot {name}: {e}"))?;
        println!("snapshot '{name}' vytvoren (id {id})");
        Ok(())
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
mod imp {
    fn no() -> Result<(), String> {
        Err("prikazy nad pripojenou jednotkou jsou zatim jen pro Windows".into())
    }
    pub fn info(_: &str) -> Result<(), String> {
        no()
    }
    pub fn ls(_: &str, _: &str, _: &str) -> Result<(), String> {
        no()
    }
    pub fn cat(_: &str, _: &str, _: &str) -> Result<(), String> {
        no()
    }
    pub fn snapshots(_: &str) -> Result<(), String> {
        no()
    }
    pub fn snapshot(_: &str, _: &str) -> Result<(), String> {
        no()
    }
}

#[cfg(not(windows))]
pub use imp::*;
