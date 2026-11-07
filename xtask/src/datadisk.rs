//! Datovy disk se souborovym systemem: vytvori ho na hostiteli stejna
//! knihovna `fs`, jakou pouziva sluzba fsd v systemu (overuje to i
//! kompatibilitu formatu mezi hostitelem a systemem).
//!
//! Disk se pri kazdem behu vytvori znovu (testy do nej zapisuji).

use std::path::{Path, PathBuf};

use fs::{Algo, FileDevice, FormatOptions, Fs, Kind};

/// Velikost datoveho disku.
const SIZE: u64 = 64 * 1024 * 1024;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fs_err(what: &str, e: fs::Error) -> Box<dyn std::error::Error> {
    format!("datovy disk: {what}: {e:?}").into()
}

/// Vytvori datovy disk `path` s ukazkovym obsahem; vraci cestu.
pub fn create(path: &Path) -> Result<PathBuf> {
    let path = path.to_path_buf();
    let file = std::fs::File::create(&path)?;
    file.set_len(SIZE)?;
    drop(file);
    let dev = FileDevice::open(&path)?;
    let opts = FormatOptions {
        label: "data".into(),
        algo: Algo::Xxh3,
        ag_blocks: None,
        uuid: Some(random_uuid()),
        time: fs::Timestamp::UNIX_EPOCH,
    };
    let dev = Fs::format(dev, &opts).map_err(|e| fs_err("formatovani", e))?;
    let mut fs = Fs::mount(dev).map_err(|e| fs_err("pripojeni", e))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| fs::Timestamp::new(d.as_secs() as i64, d.subsec_nanos() as i64))
        .unwrap_or(fs::Timestamp::UNIX_EPOCH);
    fs.set_time(now);
    let files: [(&str, &[u8]); 2] = [
        ("/vitejte.txt", b"Ahoj z hostitele!\n"),
        (
            "/dokumenty/navod.md",
            b"# Navod\n\nTento soubor vytvoril xtask na hostiteli.\n",
        ),
    ];
    fs.create("/dokumenty", Kind::Dir)
        .map_err(|e| fs_err("adresar", e))?;
    for (p, data) in files {
        fs.create(p, Kind::File).map_err(|e| fs_err(p, e))?;
        fs.write(p, 0, data).map_err(|e| fs_err(p, e))?;
    }
    fs.commit().map_err(|e| fs_err("commit", e))?;
    let report = fs.check(true).map_err(|e| fs_err("kontrola", e))?;
    if !report.problems.is_empty() {
        return Err(format!("datovy disk: kontrola konzistence: {:?}", report.problems).into());
    }
    Ok(path)
}

/// Nahodne UUID verze 4. Nahodu dava `RandomState` ze std (klice SipHash
/// seedovane z generatoru nahody operacniho systemu), takze neni potreba
/// dalsi zavislost.
fn random_uuid() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let mut u = [0u8; 16];
    for half in u.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(0);
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    u[6] = (u[6] & 0x0F) | 0x40; // verze 4
    u[8] = (u[8] & 0x3F) | 0x80; // varianta RFC 4122
    u
}
