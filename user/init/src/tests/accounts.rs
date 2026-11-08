//! Sada "ucty": accountd, prihlaseni, relace a domovy (`docs/cs/ucty.md`).
//!
//! Poradi je dulezite: zalozeni prvniho uctu, prihlaseni, PIN, domovy,
//! zamceni a odhlaseni; zpomaleni po spatnych heslech je posledni (ucet
//! pak 30 s ceka).

use alloc::format;
use alloc::string::String;

use accountproto::client::{Login, SelfService, Setup, session_of};
use accountproto::{AuthReply, Method, Secret, ToInit, flags, ids};
use fsproto::kind;
use fsproto::perm::{Acl, ACL_PROTECTED};
use libos::{Error, Startup};

use super::{TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[(&str, fn(&Startup) -> TestResult)] = &[
    ("accountd bezi", running),
    ("zalozeni prvniho uctu", create_first),
    ("prihlaseni heslem", password_login),
    ("prihlaseni PINem", pin_login),
    ("relace vidi jen svuj domov", own_home_only),
    ("init nevyrazi systemovou identitu", no_system_identity),
    ("zamceni a odhlaseni", lock_and_logout),
    ("spatne heslo a zpomaleni", wrong_password),
];

const PW: &str = "Kocka leze dirou";

fn login() -> Result<Login, String> {
    crate::accounts::channel("login").map(Login).map_err(|e| fail("kanal login", e))
}

fn alice_uid() -> Result<u64, String> {
    let l = login()?;
    let list = l.list().map_err(|e| fail("list", e))?;
    list.iter().find(|a| a.name == "alice").map(|a| a.uid).ok_or_else(|| String::from("ucet alice neni ve vypisu"))
}

fn sign_in(uid: u64) -> Result<u64, String> {
    let r = login()?.auth(uid, Method::Password, Secret::from(PW)).map_err(|e| fail("auth", e))?;
    session_of(r).map_err(|e| fail("prihlaseni heslem", e))
}

fn sys_fs() -> Result<&'static fsproto::FsClient, String> {
    super::devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| String::from("FS nebezi"))
}

fn running(_: &Startup) -> TestResult {
    let s = crate::accounts::status().map_err(|e| fail("status", e))?;
    check!(s.starts_with("ok:accounts="), &format!("status: {s}"));
    // DB je vlastni dataset, ktery vlastni accountd.
    let st = sys_fs()?.stat(crate::accounts::DB_DIR).map_err(|e| fail("stat DB", e))?;
    check_eq!(st.owner, ids::ACCOUNTD, "vlastnik DB");
    Ok(())
}

fn create_first(_: &Startup) -> TestResult {
    let setup = Setup(crate::accounts::channel("setup").map_err(|e| fail("kanal setup", e))?);
    check_eq!(setup.create_first("alice", "Alice", Secret::from("kratke")).err(), Some(Error::InvalidArgs), "kratke heslo");
    let uid = setup.create_first("alice", "Alice", Secret::from(PW)).map_err(|e| fail("CREATE_FIRST", e))?;
    check!(uid >= ids::FIRST_USER, "cislo cloveka");
    // Druhy "prvni" ucet uz nejde.
    check_eq!(setup.create_first("mallory", "M", Secret::from(PW)).err(), Some(Error::AccessDenied), "druhy prvni ucet");
    let list = login()?.list().map_err(|e| fail("list", e))?;
    let a = list.iter().find(|a| a.uid == uid).ok_or_else(|| String::from("ucet ve vypisu"))?;
    check!(a.flags & flags::ADMIN != 0, "prvni ucet ma roli spravy");
    check!(a.has(Method::Password), "metoda heslo");
    // Domov: dataset s vlastnikem, PROTECTED, zakaz spousteni.
    let fs = sys_fs()?;
    let st = fs.stat("/home/alice").map_err(|e| fail("stat domova", e))?;
    check_eq!(st.owner, uid, "vlastnik domova");
    let p = fs.permissions("/home/alice").map_err(|e| fail("opravneni domova", e))?;
    let acl = p.acl.unwrap_or_default();
    check!(acl.flags & ACL_PROTECTED != 0 && acl.grants.is_empty(), "domov PROTECTED bez grantu");
    check_eq!(fs.noexec("/home/alice").map_err(|e| fail("noexec", e))?, true, "zakaz spousteni v domove");
    Ok(())
}

fn password_login(_: &Startup) -> TestResult {
    let uid = alice_uid()?;
    let session = sign_in(uid)?;
    let fs = crate::session::connect(session).map_err(|e| fail("relace FS", e))?;
    let _ = fs.create("/home/alice/poznamka.txt", kind::FILE);
    fs.write("/home/alice/poznamka.txt", 0, b"ahoj").map_err(|e| fail("zapis do domova", e))?;
    let st = fs.stat("/home/alice/poznamka.txt").map_err(|e| fail("stat", e))?;
    check_eq!(st.owner, uid, "soubor patri uzivateli");
    let me = SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?);
    let w = me.whoami().map_err(|e| fail("whoami", e))?;
    check_eq!(w.summary.uid, uid, "whoami uid");
    check_eq!(w.home.as_str(), "/home/alice", "whoami domov");
    check_eq!(w.session, session, "whoami relace");
    me.set_phrase("modra konvice").map_err(|e| fail("fraze", e))?;
    let phrase = login()?.phrase(uid).map_err(|e| fail("phrase", e))?;
    check_eq!(phrase.as_str(), "modra konvice", "fraze");
    // Ukony spravy potvrzuje az systemovy dialog (vlakno GUI).
    check_eq!(me.elevate(1, alloc::vec![]).err(), Some(Error::NotSupported), "ELEVATE bez dialogu");
    // Sifrovani domova: FS ho zatim neumi, nic se neulozi.
    check_eq!(me.encrypt_home(Secret::from(PW)).err(), Some(Error::NotSupported), "sifrovani domova");
    me.logout().map_err(|e| fail("odhlaseni", e))?;
    Ok(())
}

fn pin_login(_: &Startup) -> TestResult {
    let uid = alice_uid()?;
    let session = sign_in(uid)?;
    let me = SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?);
    check_eq!(me.set_pin(Secret::from("spatne heslo"), Secret::from("4711")).err(), Some(Error::AccessDenied), "PIN se spatnym heslem");
    me.set_pin(Secret::from(PW), Secret::from("4711")).map_err(|e| fail("nastaveni PINu", e))?;
    let l = login()?;
    let a = l.list().map_err(|e| fail("list", e))?.into_iter().find(|a| a.uid == uid).ok_or_else(|| String::from("ucet"))?;
    check!(a.has(Method::Pin), "metoda PIN");
    check!(a.flags & flags::PIN_WEAK != 0, "bez TPM varovani u PINu");
    match l.auth(uid, Method::Pin, Secret::from("0000")).map_err(|e| fail("spatny PIN", e))? {
        AuthReply::Denied { pin_left, .. } => check_eq!(pin_left, 4, "zbyvajici pokusy PINu"),
        r => return Err(format!("spatny PIN: {r:?}")),
    }
    let s2 = session_of(l.auth(uid, Method::Pin, Secret::from("4711")).map_err(|e| fail("PIN", e))?).map_err(|e| fail("prihlaseni PINem", e))?;
    check!(s2 != session, "nova relace");
    let fs = crate::session::connect(s2).map_err(|e| fail("relace FS", e))?;
    fs.stat("/home/alice").map_err(|e| fail("domov z relace PINem", e))?;
    me.logout().map_err(|e| fail("odhlaseni", e))?;
    SelfService(crate::session::take_self(s2).ok_or_else(|| String::from("kanal self 2"))?)
        .logout()
        .map_err(|e| fail("odhlaseni 2", e))?;
    Ok(())
}

fn own_home_only(_: &Startup) -> TestResult {
    let uid = alice_uid()?;
    // Cizi domov: dataset jineho uzivatele (jako by ho zalozil accountd).
    let sys = sys_fs()?;
    match sys.create_dataset("/home/bob") {
        Ok(_) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("domov boba", e)),
    }
    sys.set_owner("/home/bob", uid + 1).map_err(|e| fail("vlastnik boba", e))?;
    sys.set_acl("/home/bob", Some(&Acl { flags: ACL_PROTECTED, grants: alloc::vec::Vec::new() })).map_err(|e| fail("acl boba", e))?;
    let _ = sys.create("/home/bob/tajne.txt", kind::FILE);
    let session = sign_in(uid)?;
    let fs = crate::session::connect(session).map_err(|e| fail("relace FS", e))?;
    fs.stat("/home/alice").map_err(|e| fail("vlastni domov", e))?;
    check_eq!(fs.stat("/home/bob").err(), Some(Error::NotFound), "cizi domov");
    check_eq!(fs.stat("/home/bob/tajne.txt").err(), Some(Error::NotFound), "soubor v cizim domove");
    check_eq!(fs.read("/home/bob/tajne.txt", 0, 1).err(), Some(Error::NotFound), "cteni z ciziho domova");
    // /home jde jen projit, ne vypsat (jmena domovu se neprozradi).
    match fs.list("/home") {
        Ok(l) => check!(!l.iter().any(|e| e.name == "bob"), "vypis /home neukaze cizi domov"),
        Err(e) => check_eq!(e, Error::AccessDenied, "vypis /home"),
    }
    // DB uctu je neviditelna.
    check_eq!(fs.stat(crate::accounts::DB_DIR).err(), Some(Error::NotFound), "DB uctu");
    check_eq!(fs.stat("/system/accounts/users").err(), Some(Error::NotFound), "zaznamy uctu");
    SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?)
        .logout()
        .map_err(|e| fail("odhlaseni", e))?;
    Ok(())
}

fn no_system_identity(_: &Startup) -> TestResult {
    // I kdyby accountd zadal o relaci systemu nebo servisniho uctu, init
    // ji nevyrazi.
    let fs = sys_fs()?;
    for uid in [0, ids::ACCOUNTD, 999] {
        let m = ToInit::OpenSession { uid, groups: alloc::vec![ids::GROUP_USERS], home: String::from("/") };
        check_eq!(crate::session::handle(fs, m, alloc::vec::Vec::new()).err(), Some(Error::AccessDenied), "relace pro systemovy ucet");
    }
    let m = ToInit::OpenSession { uid: 1000, groups: alloc::vec![0], home: String::from("/") };
    check_eq!(crate::session::handle(fs, m, alloc::vec::Vec::new()).err(), Some(Error::AccessDenied), "skupina systemu");
    let m = ToInit::CreateHome { uid: 0, name: String::from("root") };
    check_eq!(crate::session::handle(fs, m, alloc::vec::Vec::new()).err(), Some(Error::InvalidArgs), "domov systemu");
    Ok(())
}

fn lock_and_logout(_: &Startup) -> TestResult {
    let uid = alice_uid()?;
    let session = sign_in(uid)?;
    let me = SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?);
    me.lock().map_err(|e| fail("zamceni", e))?;
    check_eq!(crate::session::locked(session), Some(true), "zamceno");
    let l = login()?;
    check!(matches!(l.unlock(session, Method::Password, Secret::from("spatne heslo")), Ok(AuthReply::Denied { .. })), "odemceni spatnym heslem");
    check_eq!(crate::session::locked(session), Some(true), "porad zamceno");
    check_eq!(l.unlock(session, Method::Password, Secret::from(PW)).ok(), Some(AuthReply::Ok { session }), "odemceni");
    check_eq!(crate::session::locked(session), Some(false), "odemceno");
    me.logout().map_err(|e| fail("odhlaseni", e))?;
    check_eq!(crate::session::locked(session), None, "relace po odhlaseni");
    check_eq!(crate::session::connect(session).err(), Some(Error::NotFound), "relace FS po odhlaseni");
    check!(me.whoami().is_err(), "kanal self po odhlaseni");
    Ok(())
}

fn wrong_password(_: &Startup) -> TestResult {
    let uid = alice_uid()?;
    let l = login()?;
    check!(matches!(l.auth(999_000, Method::Password, Secret::from(PW)), Ok(AuthReply::Denied { .. })), "neznamy ucet");
    for i in 0..2 {
        check_eq!(
            l.auth(uid, Method::Password, Secret::from("tohle neni ono")).map_err(|e| fail("auth", e))?,
            AuthReply::Denied { retry_after_ms: 0, pin_left: 0 },
            &format!("spatne heslo {}", i + 1)
        );
    }
    match l.auth(uid, Method::Password, Secret::from("tohle neni ono")).map_err(|e| fail("auth", e))? {
        AuthReply::Denied { retry_after_ms, .. } => check!(retry_after_ms > 25_000 && retry_after_ms <= 30_000, "po 3. chybe cekat 30 s"),
        r => return Err(format!("3. spatne heslo: {r:?}")),
    }
    // V dobe cekani neprojde ani spravne heslo (a nic se nepocita).
    check!(matches!(l.auth(uid, Method::Password, Secret::from(PW)), Ok(AuthReply::LockedOut { .. })), "spravne heslo v dobe cekani");
    Ok(())
}
