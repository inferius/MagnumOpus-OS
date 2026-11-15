//! Knihovny z disku (DL-4c): relace FS procesu ze zpravy zavadece
//! (`abi::bootstrap::kind::FS`) a nacteni souboru pro libd.
//!
//! Zavadec cte soubor sam, relaci se stejnou identitou a cestami jako
//! program (pravo `READ | EXECUTE`, jako `launchd` u programu), a obsah
//! preda libd (`OpenImage`). libd disk necte, takze knihovnu dostane jen
//! proces, ktery by soubor precetl sam. Relaci si zavadec drzi do konce
//! procesu (`dlopen` z disku za behu).

use alloc::string::String;

use fsproto::FsClient;
use fsproto::v2::{access, share};
use libdproto::Library;
use libos::sync::Mutex;
use libos::{Channel, Error, Handle, Vmo};

/// Relace FS zavadece (`None` = program ji nedostal: knihovny jen
/// z `/system/lib`).
static FS: Mutex<Option<FsClient>> = Mutex::new(None);

/// Kolik bajtu cist najednou (vic zprav `READ_H` po `fsproto::MAX_IO`).
const CHUNK: usize = 1 << 20;

/// Prevezme relaci FS ze zpravy zavadece.
pub fn set_session(raw: u32) {
    *FS.lock() = Some(FsClient::new(Channel::from_handle(Handle::from_raw(raw))));
}

/// Ma zavadec relaci FS?
pub fn available() -> bool {
    FS.lock().is_some()
}

/// Nacte knihovnu `path` (absolutni normalizovana cesta) a da ji libd:
/// sablona (sdilena podle obsahu). `Err(NotFound)` = soubor neni (zkusit
/// dalsi misto), jine chyby se hlasi.
pub fn fetch(path: &str) -> Result<(Library, alloc::vec::Vec<Vmo>), (Error, String)> {
    let guard = FS.lock();
    let fs = guard.as_ref().ok_or((Error::NotFound, String::from("proces nema relaci FS (knihovny jen z /system/lib)")))?;
    let opened = fs.open(path, access::READ | access::EXECUTE, share::READ, 0).map_err(|e| (e, alloc::format!("{path}: {e}")))?;
    let r = (|| {
        let size = opened.stat.size;
        if opened.stat.kind != fsproto::kind::FILE {
            return Err((Error::NotFound, alloc::format!("{path}: neni soubor")));
        }
        if size == 0 || size > libdproto::MAX_FILE {
            return Err((Error::Corrupted, alloc::format!("{path}: velikost {size} B")));
        }
        let vmo = Vmo::create(size).map_err(|e| (e, alloc::format!("{path}: pamet")))?;
        let mut at = 0u64;
        while at < size {
            let n = (size - at).min(CHUNK as u64) as usize;
            let data = fs.read_h(opened.handle, at, n).map_err(|e| (e, alloc::format!("{path}: cteni: {e}")))?;
            if data.is_empty() {
                return Err((Error::Corrupted, alloc::format!("{path}: soubor se behem cteni zkratil")));
            }
            vmo.write(at, &data).map_err(|e| (e, alloc::format!("{path}: pamet")))?;
            at += data.len() as u64;
        }
        let name = dynload::search::file_name(path);
        let label = if libdproto::valid_name(name) { name } else { "knihovna" };
        libos::libd::open_image(label, &vmo, size).map_err(|e| (e, alloc::format!("{path}: libd: {e}")))
    })();
    let _ = fs.close(opened.handle);
    r
}
