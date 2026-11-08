//! Sluzba logu a zaznamu padu (`logd`): start, hlaseni padu sluzeb
//! a balicek pro vyvojare.
//!
//! logd dostane Resource jen s pravem `DIAG` (cteni logu jadra a zaznamu
//! padu, zadny pristup k hardwaru) a relaci FS servisniho uctu
//! [`LOGD_USER`], ktera vidi jen `/system/log` (`/log`) a `/system/pady`
//! (`/pady`). Relace ma listek, takze logd prezije pad fsd (log mezitim
//! drzi v RAM).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::perm::Mount;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Rights, Startup, println, time};

use crate::system::{Devices, grant_traverse};

/// Servisni ucet logd.
pub const LOGD_USER: u64 = 902;
pub const LOG_DIR: &str = "/system/log";
pub const CRASH_DIR: &str = "/system/pady";

pub struct Logd {
    /// Handle procesu (hlida ho supervisor).
    #[allow(dead_code)]
    pub process: Process,
    control: spin::Mutex<Channel>,
}

pub static LOGD: spin::Once<Logd> = spin::Once::new();

fn mkdir(fs: &fsproto::FsClient, path: &str) -> Result<()> {
    match fs.create(path, fsproto::kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Spusti logd (po pripojeni FS). Vraci cislo bootu.
pub fn start(startup: &Startup, devices: &Devices, sup: &mut crate::supervisor::Supervisor) -> Result<u64> {
    let fs = devices.fs.as_ref().ok_or(Error::NotFound)?;
    mkdir(fs, "/system")?;
    for dir in [LOG_DIR, CRASH_DIR] {
        mkdir(fs, dir)?;
        fs.set_owner(dir, LOGD_USER)?;
    }
    grant_traverse(fs, "/", LOGD_USER)?;
    grant_traverse(fs, "/system", LOGD_USER)?;
    let recipe = crate::fsd::Recipe::User {
        user: LOGD_USER,
        groups: Vec::new(),
        mounts: vec![Mount::new("/log", LOG_DIR, 0), Mount::new("/pady", CRASH_DIR, 0)],
    };
    let (session, lease) = sup.session("fsd", recipe.encode())?;
    let root = startup.root_resource.as_ref().ok_or(Error::NotFound)?;
    // Jen diagnostika: bez MANAGE nejde prostredkem sahat na hardware.
    let diag = root.handle().duplicate(Some(Rights::BASIC | Rights::DIAG))?;
    let initfs = Initfs::open(startup)?;
    let image = initfs.find("logd").ok_or(Error::NotFound)?;
    let handles = vec![
        StartHandle::user(diag),
        StartHandle::user(session.into_handle()),
        StartHandle::user(lease.into_handle()),
    ];
    let (process, control) = libos::spawn("logd", image, handles)?;
    control.write(b"start", Vec::new())?;
    let msg = control.read(Some(time::deadline_after(Duration::from_secs(20))))?;
    let text = core::str::from_utf8(&msg.data).unwrap_or("");
    let boot = text.strip_prefix("ok ").and_then(|n| n.trim().parse().ok()).ok_or(Error::IoError)?;
    sup.watch("logd", &process, None);
    LOGD.call_once(|| Logd {
        process,
        control: spin::Mutex::new(control),
    });
    Ok(boot)
}

/// Pad sluzby (supervisor): logd zapise vypis do `/system/pady`.
pub fn proc_crash(name: &str, why: &str) {
    if name == "logd" {
        return;
    }
    if let Some(l) = LOGD.get() {
        let _ = l.control.lock().write(format!("proc-crash {name} {why}").as_bytes(), Vec::new());
    }
}

/// Balicek pro vyvojare: (cesta, nahled obsahu).
pub fn report() -> Result<(String, String)> {
    let l = LOGD.get().ok_or(Error::NotFound)?;
    let ctl = l.control.lock();
    ctl.write(b"report", Vec::new())?;
    let msg = ctl.read(Some(time::deadline_after(Duration::from_secs(60))))?;
    let text = String::from_utf8_lossy(&msg.data).into_owned();
    match text.strip_prefix("ok ") {
        Some(rest) => {
            let (path, preview) = rest.split_once('\n').unwrap_or((rest, ""));
            Ok((String::from(path), String::from(preview)))
        }
        None => {
            println!("init: logd: {text}");
            Err(Error::IoError)
        }
    }
}
