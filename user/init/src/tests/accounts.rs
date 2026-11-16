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
    ("shell uzivatele zapisuje (soukromy /tmp)", super::accounts_shell::shell_writes),
    ("init nevyrazi systemovou identitu", no_system_identity),
    ("zamceni a odhlaseni", lock_and_logout),
    ("udalosti a job relace", session_events),
    ("zvyseni prav dialogem", elevate),
    ("systemove soubory pres ELEVATE", system_files),
    ("spatne heslo a zpomaleni", wrong_password),
];

const PW: &str = "Kocka leze dirou";

fn login() -> Result<Login, String> {
    crate::accounts::channel("login").map(Login).map_err(|e| fail("kanal login", e))
}

pub(super) fn alice_uid() -> Result<u64, String> {
    let l = login()?;
    let list = l.list().map_err(|e| fail("list", e))?;
    list.iter().find(|a| a.name == "alice").map(|a| a.uid).ok_or_else(|| String::from("ucet alice neni ve vypisu"))
}

pub(super) fn sign_in(uid: u64) -> Result<u64, String> {
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
    // Kos spravy pro domovy smazanych uctu (jako `session::delete_home`).
    match sys.create(crate::session::DELETED_DIR, kind::DIR) {
        Ok(()) => sys
            .set_acl(crate::session::DELETED_DIR, Some(&Acl { flags: ACL_PROTECTED, grants: alloc::vec::Vec::new() }))
            .map_err(|e| fail("acl kose spravy", e))?,
        Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("kos spravy", e)),
    }
    let session = sign_in(uid)?;
    let fs = crate::session::connect(session).map_err(|e| fail("relace FS", e))?;
    fs.stat("/home/alice").map_err(|e| fail("vlastni domov", e))?;
    check_eq!(fs.stat("/home/bob").err(), Some(Error::NotFound), "cizi domov");
    check_eq!(fs.stat("/home/bob/tajne.txt").err(), Some(Error::NotFound), "soubor v cizim domove");
    check_eq!(fs.read("/home/bob/tajne.txt", 0, 1).err(), Some(Error::NotFound), "cteni z ciziho domova");
    // /home jde vypsat, ale vidi se v nem jen vlastni domov: cizi domovy
    // a kos spravy nemaji grant, takze je vypis vynecha.
    let names: alloc::vec::Vec<String> = fs.list("/home").map_err(|e| fail("vypis /home", e))?.into_iter().map(|e| e.name).collect();
    check!(names.iter().any(|n| n == "alice"), "vypis /home ukaze vlastni domov");
    check!(!names.iter().any(|n| n == "bob"), "vypis /home neukaze cizi domov");
    check!(!names.iter().any(|n| n == ".smazane"), "vypis /home neukaze kos spravy");
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

fn drain_events() -> alloc::vec::Vec<crate::session::Event> {
    let mut v = alloc::vec::Vec::new();
    if let Some(rx) = crate::session::events() {
        while let Ok(m) = rx.try_read() {
            v.extend(crate::session::Event::decode(&m.data));
        }
    }
    v
}

fn session_events(startup: &Startup) -> TestResult {
    use crate::session::Event;
    let uid = alice_uid()?;
    drain_events();
    let session = sign_in(uid)?;
    check_eq!(drain_events(), alloc::vec![Event::Opened(session)], "udalost otevreni");
    let name = crate::session::name(session);
    check_eq!(name.as_deref(), Some("alice"), "jmeno relace");
    check_eq!(crate::session::active(), Some(session), "aktivni relace");
    // Procesy relace bezi v jejim jobu.
    let job = crate::session::job(session).map_err(|e| fail("job relace", e))?;
    let app = crate::apps::spawn_in(startup, "echo", alloc::vec::Vec::new(), Some(&job)).map_err(|e| fail("proces v jobu relace", e))?;
    app.control.write(b"sleep", alloc::vec::Vec::new()).map_err(|e| fail("sleep", e))?;
    let me = SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?);
    me.lock().map_err(|e| fail("zamceni", e))?;
    check_eq!(crate::session::active(), None, "zamcena relace neni aktivni");
    check_eq!(login()?.unlock(session, Method::Password, Secret::from(PW)).ok(), Some(AuthReply::Ok { session }), "odemceni");
    me.logout().map_err(|e| fail("odhlaseni", e))?;
    check_eq!(drain_events(), alloc::vec![Event::Locked(session), Event::Unlocked(session), Event::Closed(session)], "udalosti relace");
    check_eq!(crate::session::job(session).err(), Some(Error::NotFound), "job po odhlaseni");
    // Odhlaseni ukonci i procesy, ktere init nesleduje (JOB_KILL).
    let code = app.process.wait_exit(Some(libos::time::deadline_after(core::time::Duration::from_secs(5))));
    check_eq!(code, Ok(libos::abi::process::EXIT_KILLED), "proces relace po odhlaseni");
    check_eq!(crate::apps::spawn_in(startup, "echo", alloc::vec::Vec::new(), Some(&job)).err(), Some(Error::BadState), "job relace uzavreny");
    Ok(())
}

/// Zadost ELEVATE na kanal self bez cekani na odpoved.
fn send_elevate(me: &SelfService, action: u32, params: alloc::vec::Vec<u8>) -> TestResult {
    me.0.write(&accountproto::Request::Elevate { action, params }.encode(), alloc::vec::Vec::new()).map_err(|e| fail("ELEVATE", e))
}

/// Odpoved na kanalu self (ELEVATE).
fn elevate_reply(me: &SelfService) -> Result<alloc::vec::Vec<u8>, Error> {
    let m = me.0.read(Some(libos::time::deadline_after(core::time::Duration::from_secs(20))))?;
    accountproto::parse_reply(&m.data).map(<[u8]>::to_vec)
}

/// Test hraje kompozitor: precte frazi a dialog, odpovi volbou a heslem.
fn answer_dialog(dialog: &libos::Channel, choice: u32, secret: &str) -> Result<alloc::string::String, String> {
    use guiproto::control::{DialogKind, DialogReply, DialogRequest};
    let deadline = Some(libos::time::deadline_after(core::time::Duration::from_secs(20)));
    loop {
        let m = dialog.read(deadline).map_err(|e| fail("dialog", e))?;
        match DialogRequest::decode(&m.data).map_err(|e| fail("zprava dialogu", e))? {
            DialogRequest::Phrase { .. } => continue,
            DialogRequest::Ask { id, kind, text, options, .. } => {
                check_eq!(kind, DialogKind::Elevate, "druh dialogu");
                check_eq!(options.len(), 2, "tlacitka Povolit/Zrusit");
                let r = DialogReply::Answer { id, choice, text: String::from(secret) };
                dialog.write(&r.encode(), alloc::vec::Vec::new()).map_err(|e| fail("odpoved dialogu", e))?;
                return Ok(text);
            }
            DialogRequest::Cancel { .. } => return Err(String::from("dialog zrusen")),
            // Stav rezimu vyvojare (jen vyvojove sestaveni).
            #[allow(unreachable_patterns)]
            _ => continue,
        }
    }
}

fn elevate(_: &Startup) -> TestResult {
    use accountproto::elevate::{Params, action, encode_check};
    let uid = alice_uid()?;
    let session = sign_in(uid)?;
    let me = SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?);
    let (ours, theirs) = libos::Channel::create().map_err(|e| fail("kanal", e))?;
    crate::accounts::give_dialog(theirs).map_err(|e| fail("dialog pro accountd", e))?;
    let bob = Params::AccountCreate { name: "bob".into(), display: "Bob".into(), password: Secret::from("Bobovo heslo"), admin: false };
    // Spatne heslo potvrzujiciho: nic se nestane.
    send_elevate(&me, action::ACCOUNT_CREATE, bob.encode())?;
    let text = answer_dialog(&ours, 0, "spatne heslo")?;
    check!(text.contains("Přidat účet bob") && text.contains("Potvrzuje: Alice"), &format!("text dialogu: {text}"));
    check_eq!(elevate_reply(&me).err(), Some(Error::AccessDenied), "spatne heslo");
    // Zrusit.
    send_elevate(&me, action::ACCOUNT_CREATE, bob.encode())?;
    answer_dialog(&ours, 1, "")?;
    check_eq!(elevate_reply(&me).err(), Some(Error::Canceled), "zruseno");
    // Povolit s rezimem spravy: ucet vznikne.
    send_elevate(&me, action::ACCOUNT_CREATE, bob.encode())?;
    answer_dialog(&ours, 2, PW)?;
    let b = elevate_reply(&me).map_err(|e| fail("pridani uctu", e))?;
    let bob_uid = u64::from_le_bytes(b.get(..8).and_then(|x| x.try_into().ok()).ok_or_else(|| String::from("uid"))?);
    check!(login()?.list().map_err(|e| fail("list", e))?.iter().any(|a| a.uid == bob_uid && a.name == "bob"), "bob ve vypisu");
    // V rezimu spravy dalsi ukon bez dialogu.
    send_elevate(&me, action::ROLE_SET, Params::RoleSet { uid: bob_uid, on: true }.encode())?;
    elevate_reply(&me).map_err(|e| fail("role v rezimu spravy", e))?;
    check!(ours.try_read().is_err(), "v rezimu spravy zadny dialog");
    // Ukon jine sluzby: grant jen jednou a jen na stejne parametry.
    send_elevate(&me, 7, b"disk0".to_vec())?;
    let token = elevate_reply(&me).map_err(|e| fail("grant", e))?;
    check_eq!(token.len(), 16, "token grantu");
    let grants = crate::accounts::channel("grants").map_err(|e| fail("kanal grants", e))?;
    let check = |params: &[u8]| -> Result<(), Error> {
        grants.write(&encode_check(&token, 7, params), alloc::vec::Vec::new())?;
        let m = grants.read(Some(libos::time::deadline_after(core::time::Duration::from_secs(5))))?;
        accountproto::parse_reply(&m.data).map(|_| ())
    };
    check_eq!(check(b"disk1"), Err(Error::AccessDenied), "grant na jine parametry");
    check_eq!(check(b"disk0"), Ok(()), "grant");
    check_eq!(check(b"disk0"), Err(Error::AccessDenied), "grant podruhe");
    me.logout().map_err(|e| fail("odhlaseni", e))?;
    drop(ours);
    Ok(())
}

/// Vysledek zadosti `system` (kanal z `sysfiles::from_shell`).
fn system_result(ch: &libos::Channel) -> Result<(), Error> {
    let m = ch.read(Some(libos::time::deadline_after(core::time::Duration::from_secs(20))))?;
    match powerproto::Reply::decode(&m.data)? {
        powerproto::Reply::Ok => Ok(()),
        powerproto::Reply::Failed(e) => Err(e),
        _ => Err(Error::Corrupted),
    }
}

/// /system jde zmenit jen ukonem spravy: init po dialogu a grantu zapise,
/// smaze a zalozi slozku svou relaci FS; /system/accounts nikdy.
/// Ve vydani accountd zadost o rezim vyvojare nezna.
fn system_files(_: &Startup) -> TestResult {
    use accountproto::elevate::FileOp;
    let uid = alice_uid()?;
    let session = sign_in(uid)?;
    let (ours, theirs) = libos::Channel::create().map_err(|e| fail("kanal", e))?;
    crate::accounts::give_dialog(theirs).map_err(|e| fail("dialog pro accountd", e))?;
    let sys = sys_fs()?;
    // Primo z relace uzivatele do /system nic.
    let fs = crate::session::connect(session).map_err(|e| fail("relace FS", e))?;
    check!(fs.create("/system/zkouska", kind::DIR).is_err(), "relace uzivatele zaklada v /system");
    // Spatne cesty: chyba hned, zadny dialog.
    let now = |op: FileOp, path: &str| match crate::sysfiles::from_shell(Some(session), op.to_u8(), path, alloc::vec::Vec::new()) {
        Ok(_) => None,
        Err(powerproto::Reply::Failed(e)) => Some(e),
        Err(_) => Some(Error::Corrupted),
    };
    check_eq!(now(FileOp::Remove, "/system/accounts/users"), Some(Error::AccessDenied), "databaze uctu");
    check_eq!(now(FileOp::Remove, "/system/x/../accounts"), Some(Error::InvalidArgs), "cesta s ..");
    check_eq!(now(FileOp::Mkdir, "/home/alice/x"), Some(Error::InvalidArgs), "cesta mimo /system");
    check!(ours.try_read().is_err(), "spatna cesta bez dialogu");
    // Slozka, soubor (spatne heslo nic nezmeni), smazani.
    let ask = |op: FileOp, path: &str, data: &[u8], pw: &str| -> Result<(String, Result<(), Error>), String> {
        let ch = crate::sysfiles::from_shell(Some(session), op.to_u8(), path, data.to_vec()).map_err(|_| String::from("zadost odmitnuta"))?;
        let text = answer_dialog(&ours, 0, pw)?;
        Ok((text, system_result(&ch)))
    };
    let _ = sys.remove("/system/zkouska/a.txt");
    let _ = sys.remove("/system/zkouska");
    let (text, r) = ask(FileOp::Mkdir, "/system/zkouska", b"", PW)?;
    check!(text.contains("Vytvořit systémovou složku /system/zkouska"), &format!("text dialogu: {text}"));
    check_eq!(r, Ok(()), "slozka");
    let (text, r) = ask(FileOp::Write, "/system/zkouska/a.txt", b"ahoj", "spatne heslo")?;
    check!(text.contains("Zapsat systémový soubor /system/zkouska/a.txt (4 B)"), &format!("text dialogu: {text}"));
    check_eq!(r, Err(Error::AccessDenied), "spatne heslo");
    check_eq!(sys.stat("/system/zkouska/a.txt").err(), Some(Error::NotFound), "po spatnem hesle nic");
    let (_, r) = ask(FileOp::Write, "/system/zkouska/a.txt", b"ahoj", PW)?;
    check_eq!(r, Ok(()), "zapis");
    check_eq!(sys.read("/system/zkouska/a.txt", 0, 16).map_err(|e| fail("cteni", e))?, b"ahoj".to_vec(), "obsah");
    let (_, r) = ask(FileOp::Remove, "/system/zkouska/a.txt", b"", PW)?;
    check_eq!(r, Ok(()), "smazani souboru");
    let (_, r) = ask(FileOp::Remove, "/system/zkouska", b"", PW)?;
    check_eq!(r, Ok(()), "smazani slozky");
    check_eq!(sys.stat("/system/zkouska").err(), Some(Error::NotFound), "slozka smazana");
    // Vydani: operaci rezimu vyvojare accountd nezna (a nic nepotvrdi).
    #[cfg(not(feature = "dev-mode"))]
    {
        let me = SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?);
        let mut w = wire::Writer::op(accountproto::op::DEV_MODE);
        w.u8(accountproto::DEV_ON);
        let r = me.0.write(&w.finish(), alloc::vec::Vec::new()).and_then(|_| elevate_reply(&me));
        crate::session::put_self(session, me.0);
        check_eq!(r.err(), Some(Error::NotSupported), "rezim vyvojare ve vydani");
    }
    SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?)
        .logout()
        .map_err(|e| fail("odhlaseni", e))?;
    drop(ours);
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
