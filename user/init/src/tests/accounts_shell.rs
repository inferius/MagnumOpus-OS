//! Sada "ucty": shell bezneho uzivatele zapisuje (klony "bokem" v jeho
//! soukrome `/tmp/<uid>`, `docs/cs/ucty.md`). Driv relace uzivatele
//! zapsat nesla: shell zakladal klon ve spolecnem `/tmp`.

use alloc::format;
use alloc::string::String;

use accountproto::client::SelfService;
use fsproto::perm::{ACL_PROTECTED, user_tmp};
use libos::{Error, Startup};
use shlang::value::Value;
use term::proto::{ClientMsg, ServerMsg};

use super::accounts::{alice_uid, sign_in};
use super::shell::{Reg, start_termd_fs};
use super::{TestResult, devices, fail};
use crate::{check, check_eq};

pub fn shell_writes(s: &Startup) -> TestResult {
    let uid = alice_uid()?;
    let session = sign_in(uid)?;
    let r = writes_in(s, uid, session);
    let out = SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?).logout();
    r?;
    out.map_err(|e| fail("odhlaseni", e))
}

fn writes_in(s: &Startup, uid: u64, session: u64) -> TestResult {
    let sys = devices().and_then(|d| d.fs.as_ref()).ok_or("FS nebezi")?;
    // Soukroma docasna slozka: vlastni uzivatel, zadne granty.
    let tmp = user_tmp(uid);
    let st = sys.stat(&tmp).map_err(|e| fail("soukromy /tmp", e))?;
    check_eq!(st.owner, uid, "vlastnik soukromeho /tmp");
    let acl = sys.permissions(&tmp).map_err(|e| fail("popis soukromeho /tmp", e))?.acl;
    check!(acl.as_ref().is_some_and(|a| a.flags & ACL_PROTECTED != 0 && a.grants.is_empty()), format!("popis {tmp}: {acl:?}"));
    // Do spolecneho /tmp uzivatel nezapise.
    let fs = crate::session::connect(session).map_err(|e| fail("relace FS", e))?;
    check_eq!(fs.create("/tmp/cizi.txt", fsproto::kind::FILE).err(), Some(Error::AccessDenied), "zapis do spolecneho /tmp");
    // Shell relace uzivatele: zapis do domova a cteni zpet.
    let mut reg = Reg { store: Default::default(), chan: None };
    reg.store.home = String::from("/home/alice");
    let (proc, w) = start_termd_fs(s, &mut reg, fs, None, None)?;
    let run = (|| -> TestResult {
        w.send(ClientMsg::Attach { session: 0 })?;
        let id = match w.recv()? {
            ServerMsg::Attached { session, .. } => session,
            other => return Err(format!("cekal jsem Attached: {other:?}")),
        };
        w.ok(id, "echo zapis-uzivatele | save ~/zapis.txt")?;
        let v = w.value(id, "open ~/zapis.txt | from text")?;
        check!(matches!(&v, Value::Str(t) if t.trim() == "zapis-uzivatele"), format!("obsah souboru: {v:?}"));
        w.ok(id, "rm ~/zapis.txt")?;
        Ok(())
    })();
    let _ = proc.kill();
    run
}
