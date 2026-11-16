//! Vymenna media (docs/cs/media.md): pro kazdy oddil media (`block-media
//! <fs> <index> <model>` od ovladace pres devmgr) init spusti proces
//! `volfs` jen s kanalem oddilu a kanalem k fsd a medium ohlasi fsd
//! (`MEDIA_ADD`). fsd ho pak ukaze v `/media/<jmeno>` relacim uzivatelu.
//!
//! Media nahlasena pri startu (pred fsd) cekaji, az fsd pobezi
//! ([`attach_pending`]). Pad procesu media = medium zmizi (fsd to pozna na
//! kanalu), system bezi dal. Odpojeni media (Odpojit v Souborech) dela fsd
//! (`MEDIA_REMOVE`), proces media pak skonci sam.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use libos::initfs::Initfs;
use libos::params::Params;
use libos::spawn::StartHandle;
use libos::{Channel, Handle, Result, Vmo, println};

use crate::system::Devices;

/// Oddil media cekajici na fsd.
struct Offer {
    ch: Channel,
    fs: String,
    model: String,
}

static PENDING: spin::Mutex<Vec<Offer>> = spin::Mutex::new(Vec::new());
/// Bezici procesy media (drzet handle = proces zije; init je neuklizi,
/// skonci samy po odpojeni nebo vytazeni).
static RUNNING: spin::Mutex<Vec<libos::Process>> = spin::Mutex::new(Vec::new());

/// Zprava `block-media ...` od devmgr. Bez fsd ceka.
pub fn offer(d: Option<&Devices>, data: &[u8], handles: Vec<Handle>) {
    let text = core::str::from_utf8(data).unwrap_or("");
    let mut parts = text.splitn(4, ' ');
    let (_, fs, _index, model) = (parts.next(), parts.next().unwrap_or(""), parts.next(), parts.next().unwrap_or("").trim());
    for h in handles {
        let offer = Offer { ch: Channel::from_handle(h), fs: String::from(fs), model: String::from(model) };
        match d.filter(|d| d.fs.is_some()) {
            Some(d) => attach(d, offer),
            None => PENDING.lock().push(offer),
        }
    }
}

/// Media nahlasena pred startem fsd.
pub fn attach_pending(d: &Devices) {
    let list = core::mem::take(&mut *PENDING.lock());
    for o in list {
        attach(d, o);
    }
}

fn attach(d: &Devices, o: Offer) {
    if let Err(e) = try_attach(d, o) {
        println!("init: medium nejde pripojit: {e}");
    }
}

fn try_attach(d: &Devices, o: Offer) -> Result<()> {
    let fs = d.fs.as_ref().ok_or(libos::Error::NotFound)?;
    let vmo = d.initfs.as_ref().ok_or(libos::Error::NotFound)?;
    let initfs = Initfs::from_vmo(vmo)?;
    let image = initfs.find("volfs").ok_or(libos::Error::NotFound)?;
    let (ours, theirs) = Channel::create()?;
    let params = Params {
        cwd: b"/".to_vec(),
        args: vec![b"volfs".to_vec(), o.fs.as_bytes().to_vec(), b"rw".to_vec()],
        env: Vec::new(),
        flags: 0,
        fds: Vec::new(),
    };
    let handles = vec![params.start_handle()?, StartHandle::user(o.ch.into_handle()), StartHandle::user(theirs.into_handle())];
    let (process, _bootstrap) = libos::spawn("volfs", image, handles)?;
    RUNNING.lock().retain(|p| p.wait_exit(Some(0)).is_err());
    RUNNING.lock().push(process);
    let name = if o.model.is_empty() { "USB disk" } else { o.model.as_str() };
    fs.media_add(name, false, ours)?;
    println!("init: medium {} ({}) predano fsd", name, o.fs);
    Ok(())
}

/// Kopie VMO initfs pro spousteni procesu media mimo start (`Devices`).
pub fn initfs_vmo(startup: &libos::Startup) -> Option<Vmo> {
    startup.initfs.as_ref().and_then(|v| v.handle().duplicate(None).ok()).map(Vmo::from_handle)
}
