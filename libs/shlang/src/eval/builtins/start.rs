//! `start <cesta>` a hola cesta k souboru: otevreni v aplikaci podle typu
//! pres sluzbu "otevri cestu" hostitele ([`Host::open_path`]).
//!
//! - `start` = jako dvojklik: soubor v aplikaci s asociaci, jinak ho ukaze
//!   spravce souboru; slozka ve spravci souboru.
//! - hola cesta k souboru = jen aplikace s asociaci; bez ni souhrn jako
//!   `open` a poznamka, ze pro typ neni aplikace.
//! - Hostitel bez sluzby (konzole, testy): `start` je chyba, hola cesta
//!   ukaze souhrn jako driv.

use alloc::string::String;

use super::str_arg;
use crate::error::{ShError, code};
use crate::eval::call::Args;
use crate::eval::{Flow, Interp, R};
use crate::host::{Host, Opened};
use crate::span::Span;
use crate::value::Value;

pub(crate) fn call(it: &mut Interp, host: &mut dyn Host, a: Args) -> R<Value> {
    let name = str_arg(&a, 0, "cesta")?;
    let path = it.abs(&name);
    // Neexistujici cesta: chyba hned (bez cekani na sluzbu, stejna rada
    // jako u ostatnich prikazu).
    it.fs_stat(host, &path)?;
    match open(host, &path, false, a.span)? {
        Some(_) => Ok(Value::Nothing),
        None => Err(Flow::Err(
            ShError::sys(abi::Error::NotSupported, "otevírání v aplikacích")
                .at(a.span)
                .hint("relace nemá službu „otevři cestu“ (textová konzole); obsah ukáže open"),
        )),
    }
}

/// Hola cesta k souboru (ne program, ne `.mos`): `true` = otevreno
/// v aplikaci, `false` = ukazat souhrn (bez aplikace nebo bez sluzby).
pub(crate) fn bare_file(host: &mut dyn Host, path: &str, span: Span) -> R<bool> {
    Ok(open(host, path, true, span)?.is_some_and(|o| !matches!(o, Opened::NoApp)))
}

/// Zadost sluzbe a poznamka do terminalu. `None` = hostitel sluzbu nema.
fn open(host: &mut dyn Host, path: &str, app_only: bool, span: Span) -> R<Option<Opened>> {
    let r = match host.open_path(path, app_only) {
        Ok(r) => r,
        Err(abi::Error::NotSupported) => return Ok(None),
        Err(e) => return Err(Flow::Err(ShError::sys(e, "služba „otevři cestu“").at(span))),
    };
    match &r {
        Opened::App(app) => host.note(&alloc::format!("Otevřeno v aplikaci {app}.")),
        Opened::Revealed(app) => host.note(&alloc::format!("{} Soubor ukazuje {app}.", no_app(path))),
        Opened::NoApp => host.note(&no_app(path)),
        Opened::Failed(why) => return Err(Flow::Err(failed(why).at(span))),
    }
    Ok(Some(r))
}

/// "Pro typ .xyz neni aplikace."
fn no_app(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i > 0 => alloc::format!("Pro typ .{} není aplikace.", name[i + 1..].to_lowercase()),
        _ => String::from("Pro soubory bez přípony není aplikace."),
    }
}

/// Chyba sluzby: konci-li jmenem chyby systemu (`/a.txt: NOT_FOUND`), je
/// to kod chyby (vysvetleni, rada); jinak `SH_OPEN`.
fn failed(why: &str) -> ShError {
    let sys = why.rsplit_once(": ").and_then(|(_, n)| abi::error::ALL.iter().copied().find(|e| e.name() == n));
    match sys {
        Some(e) => ShError::sys(e, String::from(why)),
        None => ShError::new(code::OPEN, alloc::format!("aplikace cestu neotevřela: {why}")),
    }
}
