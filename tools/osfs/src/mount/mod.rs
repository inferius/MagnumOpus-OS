//! `osfs mount`: pripojeni obrazu jako jednotky Windows pres WinFsp.
//!
//! - **Jeden zamek** kolem celeho `Fs` (WinFsp vola z vice vlaken, `Fs` je
//!   jednovlaknovy).
//! - **Po cestach:** kontext otevreneho souboru drzi jen cestu v nasem FS
//!   (vrstva handlu - sirotci, sdileci rezimy - prijde pozdeji). Prejmenovani
//!   prepise cesty otevrenych handlu pod prejmenovanym jmenem.
//! - **Commit** kazdych `COMMIT_EVERY` (jen kdyz se neco zmenilo) a pri
//!   odpojeni; `FlushFileBuffers` = `fsync` (intent log), flush cele
//!   jednotky = commit.
//! - Jmena preklada `crate::names` (skryta jmena z Windows nejdou zadat).

mod ffi;
mod ops;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use crate::image::{self, Volume};
use ffi::*;

/// Jak casto ukladat zmeny (commit), kdyz se neco zmenilo.
pub const COMMIT_EVERY: Duration = Duration::from_secs(5);

/// Popisovac zabezpeceni vsech objektu: plny pristup pro SYSTEM,
/// spravce a vsechny (FS zatim nema opravneni).
const SDDL: &str = "O:BAG:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;WD)";

/// Otevreny soubor nebo adresar.
struct Handle {
    /// Cesta v nasem FS (`/a/b`).
    path: String,
}

struct State {
    fs: Volume,
    handles: HashMap<usize, Handle>,
    next_handle: usize,
    /// Od posledniho commitu se neco zmenilo.
    dirty: bool,
    /// Vypisy adresaru pro dohledani skutecne velikosti pismen jmen
    /// (`ops::true_path`); klic = slozena cesta adresare. Maze se pri
    /// kazde zmene jmen (create, remove, rename).
    dir_cache: HashMap<String, Vec<fs::DirEntry>>,
    read_only: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static SD: OnceLock<Vec<u8>> = OnceLock::new();
static API: OnceLock<Api> = OnceLock::new();
static STOP: AtomicBool = AtomicBool::new(false);
static DONE: AtomicBool = AtomicBool::new(false);

/// Ladici vypis operaci na stderr (`OSFS_DEBUG=1`).
pub(crate) fn debug_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("OSFS_DEBUG").is_some_and(|v| v != "0"))
}

macro_rules! debug {
    ($($t:tt)*) => {
        if $crate::mount::debug_on() {
            eprintln!("osfs: {}", format_args!($($t)*));
        }
    };
}
pub(crate) use debug;

fn lock() -> MutexGuard<'static, Option<State>> {
    // Panika = abort (profil workspace), otraveny zamek tedy nenastane;
    // pro jistotu ho prijmeme.
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Spusti `f` nad stavem pod zamkem.
fn with_state(f: impl FnOnce(&mut State) -> NTSTATUS) -> NTSTATUS {
    let mut g = lock();
    match g.as_mut() {
        Some(st) => f(st),
        None => STATUS_UNSUCCESSFUL,
    }
}

/// Je `s` pismeno jednotky ve tvaru `X:`?
fn valid_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

unsafe extern "system" fn on_console(_event: u32) -> i32 {
    STOP.store(true, Ordering::SeqCst);
    // Pri zavreni okna Windows proces po navratu ukonci: pockat, nez hlavni
    // vlakno odpoji a ulozi (Windows dava asi 5 s).
    for _ in 0..45 {
        if DONE.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    1
}

fn commit_now(st: &mut State) {
    if st.dirty && !st.read_only {
        st.fs.set_time(image::now_ns());
        match st.fs.commit() {
            Ok(()) => st.dirty = false,
            Err(e) => eprintln!("osfs: commit selhal: {e}"),
        }
    }
}

/// Jmeno udalosti, kterou `osfs unmount X:` pozada o odpojeni (jen v ramci
/// prihlaseni uzivatele).
fn unmount_event_name(drive: &str) -> String {
    format!("Local\\osfs-unmount-{}", drive[..1].to_ascii_uppercase())
}

/// `osfs unmount X:`: pozada proces `osfs mount` o odpojeni (ulozi zmeny).
pub fn unmount(drive: &str) -> Result<(), String> {
    if !valid_drive(drive) {
        return Err(format!("{drive}: cekam pismeno jednotky ve tvaru X:"));
    }
    let name = wide(&unmount_event_name(drive));
    // SAFETY: platny retezec zakonceny nulou.
    let ev = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if ev.is_null() {
        return Err(format!(
            "{drive} neni pripojene pres osfs mount (v tomto prihlaseni)"
        ));
    }
    // SAFETY: platny handle udalosti.
    unsafe {
        SetEvent(ev);
        CloseHandle(ev);
    }
    println!("{drive}: odpojeni vyzadano (osfs mount ulozi zmeny a skonci)");
    Ok(())
}

/// Pripoji obraz jako jednotku `drive` a ceka na Ctrl+C nebo `osfs unmount`.
pub fn run(image_path: &Path, drive: &str) -> Result<(), String> {
    if !valid_drive(drive) {
        return Err(format!("{drive}: cekam pismeno jednotky ve tvaru X:"));
    }
    let api = API.get_or_init(|| match ffi::load() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("osfs: {e}");
            std::process::exit(1);
        }
    });
    let sd = security_descriptor(SDDL)?;
    let _ = SD.set(sd);

    let ev_name = wide(&unmount_event_name(drive));
    // SAFETY: platny retezec; udalost s rucnim nulovanim, na zacatku
    // nesignalizovana.
    let unmount_ev = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, ev_name.as_ptr()) };
    // SAFETY: bez parametru.
    if unmount_ev.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Err(format!("{drive}: uz ho pripojuje jiny proces osfs mount"));
    }

    let fs = image::open(image_path, false)?;
    let info = fs.info();
    if info.read_only {
        eprintln!(
            "osfs: POZOR - svazek je jen pro cteni (nejista obnova nebo necitelny log, viz osfs check)"
        );
    }
    *lock() = Some(State {
        fs,
        handles: HashMap::new(),
        next_handle: 1,
        dirty: false,
        dir_cache: HashMap::new(),
        read_only: info.read_only,
    });

    // SAFETY: VolumeParams je ciste POD, nuly jsou platny vychozi stav.
    let mut vp: VolumeParams = unsafe { std::mem::zeroed() };
    vp.Version = size_of::<VolumeParams>() as u16;
    vp.SectorSize = fs::BLOCK_SIZE as u16;
    vp.SectorsPerAllocationUnit = 1;
    vp.MaxComponentLength = crate::names::WIN_NAME_MAX as u16;
    vp.VolumeCreationTime = ops::filetime(image::now_ns());
    vp.VolumeSerialNumber = (image::now_ns() / 1_000_000) as u32;
    vp.FileInfoTimeout = 1000;
    vp.Flags = VP_CASE_PRESERVED_NAMES
        | VP_UNICODE_ON_DISK
        | VP_POST_CLEANUP_WHEN_MODIFIED_ONLY
        | VP_UM_FILE_CONTEXT_IS_USER_CONTEXT2;
    // Nas FS nerozlisuje velikost pismen stejne jako Windows; opravneni
    // zatim nema.
    debug_assert_eq!(
        vp.Flags & (VP_CASE_SENSITIVE_SEARCH | VP_PERSISTENT_ACLS),
        0
    );
    if info.read_only {
        vp.Flags |= VP_READ_ONLY_VOLUME;
    }
    for (i, c) in "MagnumOpusFS".encode_utf16().enumerate() {
        vp.FileSystemName[i] = c;
    }

    let iface: &'static Interface = Box::leak(Box::new(ops::interface()));
    let mut fsp: *mut FileSystem = std::ptr::null_mut();
    let mut dev = wide(DISK_DEVICE_NAME);
    // SAFETY: parametry a rozhrani ziji po celou dobu behu (rozhrani je
    // leaknute), ukazatel na vysledek je platny.
    let r = unsafe { (api.create)(dev.as_mut_ptr(), &vp, iface, &mut fsp) };
    if r != STATUS_SUCCESS {
        *lock() = None;
        return Err(format!("FspFileSystemCreate selhal: 0x{:08X}", r as u32));
    }
    let mut mp = wide(drive);
    // SAFETY: fsp je platny objekt z FspFileSystemCreate.
    let r = unsafe { (api.set_mount_point)(fsp, mp.as_mut_ptr()) };
    if r != STATUS_SUCCESS {
        // SAFETY: jako vyse.
        unsafe { (api.delete)(fsp) };
        *lock() = None;
        return Err(format!(
            "pripojeni na {drive} selhalo: 0x{:08X} (pismeno uz je pouzite?)",
            r as u32
        ));
    }
    // SAFETY: platny handler; 0 vlaken = vychozi pocet.
    unsafe { SetConsoleCtrlHandler(Some(on_console), 1) };
    // SAFETY: fsp je platny.
    let r = unsafe { (api.start_dispatcher)(fsp, 0) };
    if r != STATUS_SUCCESS {
        // SAFETY: jako vyse.
        unsafe { (api.delete)(fsp) };
        *lock() = None;
        return Err(format!(
            "FspFileSystemStartDispatcher selhal: 0x{:08X}",
            r as u32
        ));
    }
    println!(
        "{} pripojeno jako {drive} ('{}'{}). Odpojeni: Ctrl+C.",
        image_path.display(),
        info.label,
        if info.read_only {
            ", jen pro cteni"
        } else {
            ""
        }
    );
    println!("POZOR: FS je mlady - jen na zkouseni, ne na ostra data.");

    // Pravidelny commit.
    let committer = std::thread::spawn(|| {
        let step = Duration::from_millis(200);
        let mut waited = Duration::ZERO;
        while !STOP.load(Ordering::SeqCst) {
            std::thread::sleep(step);
            waited += step;
            if waited >= COMMIT_EVERY {
                waited = Duration::ZERO;
                if let Some(st) = lock().as_mut() {
                    commit_now(st);
                }
            }
        }
    });

    while !STOP.load(Ordering::SeqCst) {
        // SAFETY: platny handle udalosti.
        if unsafe { WaitForSingleObject(unmount_ev, 100) } == WAIT_OBJECT_0 {
            STOP.store(true, Ordering::SeqCst);
        }
    }
    println!("odpojuji {drive} ...");
    // SAFETY: fsp je platny; po StopDispatcher uz zadne callbacky nebezi.
    unsafe {
        (api.stop_dispatcher)(fsp);
        (api.delete)(fsp);
    }
    let _ = committer.join();
    let result = match lock().take() {
        Some(mut st) => {
            st.dirty = true;
            commit_now(&mut st);
            if st.dirty && !st.read_only {
                Err("posledni commit selhal - zmeny od predchoziho commitu (krome fsync) jsou ztracene".into())
            } else {
                Ok(())
            }
        }
        None => Ok(()),
    };
    if result.is_ok() {
        println!("odpojeno, zmeny ulozene");
    }
    // SAFETY: platny handle.
    unsafe { CloseHandle(unmount_ev) };
    DONE.store(true, Ordering::SeqCst);
    result
}
