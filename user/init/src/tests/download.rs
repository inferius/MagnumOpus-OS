//! Stahovani do souboru (`docs/cs/stahovani.md`): `fetch save` proti
//! pomocnym serverum hostitele (HTTP i HTTPS), soubor ve slozce Stazene,
//! puvod `sys.origin` (zapise jen fetch, uzivatel ho nepodvrhne ani
//! nesmaze), uloha v prehledu uloh, oznameni, srozumitelne chyby (404,
//! TLS s cizi CA).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use fsproto::perm::{Mount, Profile, mount_flags};
use fsproto::{FsClient, kind};
use libos::spawn::StartHandle;
use libos::{Error, Startup, time};
use notifyproto::{View, ViewReply};

use super::web::{fnv, pattern, ports, start_fetch_with};
use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[("save: soubor, puvod, uloha, oznameni a chyby", save)];

const USER: u64 = 1000;
const HOME: &str = "/stahovani-test";
const DIR: &str = "/stahovani-test/Stažené";

fn sys() -> Result<&'static FsClient, String> {
    devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| String::from("fsd nebezi"))
}

/// Dalsi oznameni prohlizece (preskoci odpovedi a stav Nerusit).
fn next_notice(v: &libos::Channel) -> Result<notifyproto::Notification, String> {
    loop {
        let m = v.read(Some(time::deadline_after(core::time::Duration::from_secs(10)))).map_err(|e| fail("oznameni", e))?;
        if let Ok(ViewReply::Added(n)) = ViewReply::decode(&m.data) {
            return Ok(n);
        }
    }
}

fn save(s: &Startup) -> TestResult {
    let Some(p) = ports(s)? else { return Ok(()) };
    let f = sys()?;
    // Domov testovaciho uzivatele se slozkou Stazene.
    for d in [HOME, DIR] {
        match f.create(d, kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(fail("slozka", e)),
        }
        f.set_owner(d, USER).map_err(|e| fail("vlastnik", e))?;
    }
    let (_, dl) = f.connect_as(USER, &[], &[Mount::new("/Stazene", DIR, mount_flags::ORIGIN)]).map_err(|e| fail("relace Stazene", e))?;
    let tasks = crate::tasks::producer("uzivatel").map_err(|e| fail("producent uloh", e))?;
    let notify = crate::notify::sender("org.magnumopus.fetch", "Stahování z webu", USER, false).map_err(|e| fail("oznameni", e))?;
    let viewer = crate::notify::viewer(USER).map_err(|e| fail("prohlizec", e))?;
    viewer.write(&View::Subscribe.encode(), Vec::new()).map_err(|e| fail("odber", e))?;
    let fetch = start_fetch_with(
        s,
        &p,
        vec![
            StartHandle::user(dl.into_channel().into_handle()),
            StartHandle::user(tasks.into_handle()),
            StartHandle::tagged(libos::abi::params::tag::NOTIFY, notify.into_handle()),
        ],
    )?;

    // 1. HTTPS (HTTP/2), vetsi soubor po castech.
    let n = 1_000_000;
    let url = format!("https://10.0.2.2:{}/stream?n={n}", p.https);
    let r = fetch.run(&format!("save {url} velký soubor.bin"))?;
    check_eq!(r, format!("ok:velký soubor.bin {n}"), "stazeno");
    let path = format!("{DIR}/velký soubor.bin");
    let mut data = Vec::new();
    while data.len() < n {
        let part = f.read(&path, data.len() as u64, 65536).map_err(|e| fail("cteni", e))?;
        if part.is_empty() {
            break;
        }
        data.extend(part);
    }
    check_eq!(fnv(data), fnv((0..n).map(pattern)), "obsah souboru");
    check!(f.list(DIR).map_err(|e| fail("vypis", e))?.iter().all(|e| !e.name.ends_with(".stahuje")), "docasny soubor pryc");
    // Puvod: adresa a cas; uzivatel ho nepodvrhne ani nesmaze.
    let origin = f.get_xattr(&path, fsproto::perm::ORIGIN_XATTR).map_err(|e| fail("puvod", e))?;
    let origin = String::from_utf8(origin).map_err(|_| String::from("puvod neni UTF-8"))?;
    check!(origin.starts_with(&format!("{url}\n")) && origin.len() > url.len() + 10, format!("puvod: {origin:?}").as_str());
    let (_, user) = f.connect_as(USER, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("relace uzivatele", e))?;
    check_eq!(user.set_xattr(&path, fsproto::perm::ORIGIN_XATTR, b"https://podvrh\n0").err(), Some(Error::AccessDenied), "uzivatel puvod nepodvrhne");
    check_eq!(user.remove_xattr(&path, fsproto::perm::ORIGIN_XATTR).err(), Some(Error::AccessDenied), "uzivatel puvod nesmaze");
    check!(user.set_xattr(&path, "user.poznamka", b"ok").is_ok(), "vlastni atributy uzivatel smi");
    // Oznameni o dokonceni.
    let note = next_notice(&viewer)?;
    check!(note.title.contains("velký soubor.bin") && note.app_id == "org.magnumopus.fetch", format!("oznameni: {}", note.title).as_str());

    // 2. Stejne jmeno podruhe: neprepise, nove jmeno. Jmeno z adresy.
    let r = fetch.run(&format!("save http://10.0.2.2:{}/hello", p.http))?;
    check_eq!(r, String::from("ok:hello 16"), "jmeno z adresy");
    let r = fetch.run(&format!("save http://10.0.2.2:{}/hello", p.http))?;
    check_eq!(r, String::from("ok:hello (2) 16"), "druhe stazeni pod novym jmenem");
    next_notice(&viewer)?;
    next_notice(&viewer)?;

    // 3. Chyby: 404, cizi CA. Zadny soubor nezustane.
    let r = fetch.run(&format!("save http://10.0.2.2:{}/neni.zip", p.http))?;
    check!(r.starts_with("err:") && r.contains("404"), format!("404: {r}").as_str());
    let fail_note = next_notice(&viewer)?;
    check!(fail_note.title.contains("selhalo") && fail_note.body.contains("404"), format!("oznameni chyby: {}", fail_note.body).as_str());
    let r = fetch.run(&format!("save https://10.0.2.2:{}/hello cizi.txt", p.untrusted))?;
    check!(r.starts_with("err:") && r.contains("Zabezpečené spojení"), format!("cizi CA: {r}").as_str());
    let names: Vec<String> = f.list(DIR).map_err(|e| fail("vypis", e))?.into_iter().map(|e| e.name).collect();
    check!(!names.iter().any(|n| n == "neni.zip" || n == "cizi.txt" || n.ends_with(".stahuje")), format!("po chybach: {names:?}").as_str());
    // Bez relace Stazene fetch neuklada.
    Ok(())
}
