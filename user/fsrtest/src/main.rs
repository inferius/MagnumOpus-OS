//! fsrtest: klient FS pro test "zabij fsd uprostred prace"
//! (`user/init/src/tests/fs_recovery.rs`, `docs/cs/obnova-sluzeb.md`).
//!
//! Handly startovni zpravy: kanal relace `fsd` a listek. Prikazy na
//! bootstrap (odpoved `ok <text>` nebo `err <JMENO_CHYBY>`):
//!
//! - `write <cesta> <text>` - vytvori (kdyz neni) a zapise od zacatku,
//! - `read <cesta>` - cely obsah (do 4 KiB),
//! - `open <cesta>` - otevre pro cteni a zapis, vrati cislo handlu,
//! - `readh <handle>` - obsah pres handle (do 4 KiB),
//! - `writeh <handle> <text>` - zapis pres handle od zacatku,
//! - `lock <handle>` - vylucny zamek celeho souboru,
//! - `close <handle>`,
//! - `gen` - generace instance, se kterou spojeni naposledy obnovilo relaci,
//! - `mapw <cesta>` - soubor 16 B tecek, sdilene mapovani okna 0 se zapisem,
//! - `pokew <offset> <text>` - zapis primo do mapovani (bez fsd),
//! - `recoverw` - obnova po padu fsd (`FsClient::recover_window`): vrati
//!   prvnich 16 B noveho okna.
//!
//! Klient bezi v jinem procesu nez supervisor (ten v testu ridi init).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::FsClient;
use fsproto::v2::{Identity, access, map_flags, share};
use libos::{Channel, Error, Startup, Vmo};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let mut h = startup.handles.into_iter();
    let (Some(session), Some(lease)) = (h.next(), h.next()) else {
        return 7;
    };
    let fs = FsClient::with_lease(Channel::from_handle(session), Channel::from_handle(lease));
    let control = startup.bootstrap;
    let mut mapped: Option<Mapped> = None;
    loop {
        let m = match control.read(None) {
            Ok(m) => m,
            Err(_) => return 0,
        };
        let cmd = String::from_utf8_lossy(&m.data).into_owned();
        let text = match command(&fs, &cmd, &mut mapped) {
            Ok(t) => format!("ok {t}"),
            Err(e) => format!("err {}", e.name()),
        };
        if control.write(text.as_bytes(), Vec::new()).is_err() {
            return 0;
        }
    }
}

/// Sdilene mapovani pro test obnovy oken (Q5).
struct Mapped {
    addr: u64,
    vmo: Vmo,
    id: Identity,
}

fn num(s: Option<&str>) -> libos::Result<u64> {
    s.and_then(|x| x.parse().ok()).ok_or(Error::InvalidArgs)
}

/// Namapuje okno pro cteni a zapis; vraci adresu.
fn map_window(vmo: &Vmo) -> libos::Result<u64> {
    let page = libos::vm::page_size();
    let flags = libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE;
    libos::sys::vm_map(libos::rt::self_process(), vmo.handle().raw(), 0, page, flags, 0)
}

fn command(fs: &FsClient, cmd: &str, mapped: &mut Option<Mapped>) -> libos::Result<String> {
    let mut w = cmd.splitn(3, ' ');
    match (w.next(), w.next(), w.next()) {
        (Some("mapw"), Some(path), None) => {
            match fs.create(path, fsproto::kind::FILE) {
                Ok(()) | Err(Error::AlreadyExists) => {}
                Err(e) => return Err(e),
            }
            fs.write(path, 0, b"................")?;
            let o = fs.open(path, access::READ | access::WRITE, share::ALL, 0)?;
            let win = fs.map_vmo(o.handle, 0, map_flags::WRITE | map_flags::SHARED)?;
            let addr = map_window(&win.vmo)?;
            *mapped = Some(Mapped { addr, vmo: win.vmo, id: fs.identity(o.handle)? });
            Ok(String::new())
        }
        (Some("pokew"), off, Some(text)) => {
            let m = mapped.as_ref().ok_or(Error::BadState)?;
            let off = num(off)? as usize;
            if off + text.len() > 16 {
                return Err(Error::InvalidArgs);
            }
            // SAFETY: [addr, addr + stranka) je namapovane okno se zapisem.
            unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), (m.addr + off as u64) as *mut u8, text.len()) };
            Ok(String::new())
        }
        (Some("recoverw"), None, None) => {
            let m = mapped.as_mut().ok_or(Error::BadState)?;
            let win = fs.recover_window(&m.id, 0, &m.vmo)?;
            let page = libos::vm::page_size();
            let _ = libos::sys::vm_unmap(libos::rt::self_process(), m.addr, page);
            m.addr = map_window(&win.vmo)?;
            m.vmo = win.vmo;
            // SAFETY: prave namapovane nove okno, aspon 16 B souboru.
            let b = unsafe { core::slice::from_raw_parts(m.addr as *const u8, 16) };
            Ok(String::from_utf8_lossy(b).into_owned())
        }
        (Some("write"), Some(path), Some(text)) => {
            match fs.create(path, fsproto::kind::FILE) {
                Ok(()) | Err(Error::AlreadyExists) => {}
                Err(e) => return Err(e),
            }
            fs.write(path, 0, text.as_bytes())?;
            Ok(String::new())
        }
        (Some("read"), Some(path), None) => Ok(String::from_utf8_lossy(&fs.read(path, 0, 4096)?).into_owned()),
        (Some("open"), Some(path), None) => {
            let o = fs.open(path, access::READ | access::WRITE, share::ALL, 0)?;
            Ok(format!("{}", o.handle))
        }
        (Some("readh"), h, None) => Ok(String::from_utf8_lossy(&fs.read_h(num(h)?, 0, 4096)?).into_owned()),
        (Some("writeh"), h, Some(text)) => {
            let n = fs.write_h(num(h)?, 0, text.as_bytes()).map_err(|(_, e)| e)?;
            Ok(format!("{n}"))
        }
        (Some("lock"), h, None) => {
            fs.lock(num(h)?, 0, 0, fsproto::v2::lock::EXCLUSIVE)?;
            Ok(String::new())
        }
        (Some("close"), h, None) => {
            fs.close(num(h)?)?;
            Ok(String::new())
        }
        (Some("gen"), None, None) => Ok(format!("{}", fs.conn().generation())),
        _ => Err(Error::InvalidArgs),
    }
}
