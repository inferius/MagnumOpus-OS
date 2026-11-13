//! `install`: textovy pruvodce instalaci (kroky a texty z
//! `libs/installplan`, stejne jako budouci GUI). Mluvi se sluzbou
//! `installd` pres hostitele (`Host::installer`). `--odpovedi <soubor>` =
//! instalace bez dotazu (skripty, test v QEMU).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use installplan::config::Settings;
use installplan::proto::{AccountInput, DiskEntry, KeyboardEntry, Reply, Request};
use installplan::{Mode, NO_ENCRYPTION, PROGRESS, STEPS};

use super::super::call::Args;
use super::super::{Flow, Interp, R};
use crate::error::{ShError, code};
use crate::host::Host;
use crate::value::Value;

fn err(a: &Args, text: impl Into<String>) -> Flow {
    Flow::Err(ShError::new(code::USER, text).at(a.span))
}

fn cancelled(a: &Args) -> Flow {
    Flow::Err(ShError::new(code::REJECTED, "instalace zrušena, disk se nezměnil").at(a.span))
}

/// Pozadavek na installd; `Failed` = chyba prikazu.
fn call(host: &mut dyn Host, a: &Args, req: &Request) -> R<Reply> {
    let mut steps: Vec<(u32, String)> = Vec::new();
    let raw = host
        .installer(&req.encode(), &mut |b| {
            if let Ok(Reply::Progress { step, text }) = Reply::decode(b) {
                steps.push((step, text));
            }
        })
        .map_err(|e| Flow::Err(ShError::sys(e, "instalátor není dostupný (instalace jde jen ze živého USB)").at(a.span)))?;
    for (step, text) in steps {
        host.progress(1, step as u64 + 1, PROGRESS.len() as u64, &text);
        host.note(&format!("[{}/{}] {text}", step + 1, PROGRESS.len()));
    }
    match Reply::decode(&raw) {
        Ok(Reply::Failed(_, text)) => Err(err(a, text)),
        Ok(r) => Ok(r),
        Err(_) => Err(err(a, "neplatná odpověď instalátoru")),
    }
}

fn step(host: &mut dyn Host, i: usize) {
    host.note(&format!("\n== Krok {}/{}: {} ==", i + 1, STEPS.len(), STEPS[i]));
}

fn ask(host: &mut dyn Host, a: &Args, q: &str, secret: bool, default: &str) -> R<String> {
    host.ask(q, secret, default).ok_or_else(|| cancelled(a))
}

fn disks(host: &mut dyn Host, a: &Args) -> R<Vec<DiskEntry>> {
    match call(host, a, &Request::Disks)? {
        Reply::Disks(d) => Ok(d),
        _ => Err(err(a, "neplatná odpověď instalátoru")),
    }
}

/// Vstupni bod prikazu.
pub(crate) fn run(it: &mut Interp, host: &mut dyn Host, a: &Args) -> R<Value> {
    if let Some(Value::Str(path)) = a.named("odpovedi") {
        let p = it.paths(&Value::Str(path.clone()))?.into_iter().next().unwrap_or_default();
        let text = String::from_utf8(it.fs_read(host, &p)?).map_err(|_| err(a, "soubor odpovědí není UTF-8"))?;
        return unattended(host, a, &text);
    }
    interactive(host, a)
}

fn unattended(host: &mut dyn Host, a: &Args, text: &str) -> R<Value> {
    let ans = installplan::answers::parse(text).map_err(|e| err(a, e))?;
    let list = disks(host, a)?;
    let d = list
        .iter()
        .find(|d| d.model == ans.disk || format!("{}", d.index) == ans.disk)
        .ok_or_else(|| err(a, format!("disk '{}' nenalezen", ans.disk)))?;
    host.note(&format!("instalace bez dotazů na '{}' ({})", d.model, ans.mode.key()));
    if let Reply::Preview { text, .. } = call(host, a, &Request::Preview { disk: d.index, mode: ans.mode })? {
        host.note(&text);
    }
    let req = Request::Install { disk: d.index, mode: ans.mode, confirm: ans.confirm, settings: ans.settings, account: ans.account };
    finish(host, a, &req)
}

fn finish(host: &mut dyn Host, a: &Args, req: &Request) -> R<Value> {
    match call(host, a, req)? {
        Reply::Done(summary) => {
            host.note(&summary);
            Ok(Value::Str(summary))
        }
        _ => Err(err(a, "neplatná odpověď instalátoru")),
    }
}

fn interactive(host: &mut dyn Host, a: &Args) -> R<Value> {
    // 1. Jazyk a klavesnice.
    step(host, 0);
    host.note("Jazyk systému: čeština (další jazyky se připravují).");
    let kbs: Vec<KeyboardEntry> = match call(host, a, &Request::Keyboards)? {
        Reply::Keyboards(k) => k,
        _ => Vec::new(),
    };
    for k in &kbs {
        host.note(&format!("{} - {}:\n  {}", k.name, k.description, k.rows.join("\n  ")));
    }
    let names: Vec<String> = kbs.iter().map(|k| format!("{} ({})", k.description, k.name)).collect();
    let first = host.choose("Hlavní rozložení klávesnice", &names).ok_or_else(|| cancelled(a))?;
    let mut settings = Settings { keyboards: alloc::vec![kbs[first].name.clone()], ..Settings::default() };
    loop {
        let rest: Vec<usize> = (0..kbs.len()).filter(|i| !settings.keyboards.contains(&kbs[*i].name)).collect();
        if rest.is_empty() {
            break;
        }
        let mut opts: Vec<String> = rest.iter().map(|&i| format!("přidat {}", names[i])).collect();
        opts.push(String::from("hotovo"));
        let c = host.choose("Další rozložení (přepínání levý Alt + levý Shift)", &opts).ok_or_else(|| cancelled(a))?;
        if c >= rest.len() {
            break;
        }
        settings.keyboards.push(kbs[rest[c]].name.clone());
    }
    settings.zone = ask(host, a, "Časové pásmo", false, "Europe/Prague")?;

    // 2. Kontrola pocitace.
    step(host, 1);
    let list = disks(host, a)?;
    for d in &list {
        host.note(&format!(
            "disk {}: {} ({}){}\n  {}",
            d.index,
            d.model,
            installplan::human(d.bytes),
            if d.live { " - živé USB" } else { "" },
            d.parts.join("\n  ")
        ));
    }
    host.note("Secure Boot ve firmwaru musí být vypnutý, jinak počítač nainstalovaný systém nespustí.");

    // 3. Kam nainstalovat.
    step(host, 2);
    let targets: Vec<&DiskEntry> = list.iter().filter(|d| !d.live).collect();
    if targets.is_empty() {
        return Err(err(a, "žádný disk pro instalaci (kromě živého USB)"));
    }
    let tn: Vec<String> = targets.iter().map(|d| format!("{} ({})", d.model, installplan::human(d.bytes))).collect();
    let disk = targets[host.choose("Disk", &tn).ok_or_else(|| cancelled(a))?];
    for c in disk.choices.iter().filter(|c| !c.available) {
        host.note(&format!("(nejde) {}: {}", c.title, c.detail));
    }
    let avail: Vec<_> = disk.choices.iter().filter(|c| c.available).collect();
    if avail.is_empty() {
        return Err(err(a, "na tento disk teď instalovat nejde (viz důvody výše)"));
    }
    let an: Vec<String> = avail.iter().map(|c| format!("{} - {}", c.title, c.detail)).collect();
    let mode: Mode = avail[host.choose("Způsob instalace", &an).ok_or_else(|| cancelled(a))?].mode;

    // 4. Nahled zmen.
    step(host, 3);
    let (type_name, other_os) = match call(host, a, &Request::Preview { disk: disk.index, mode })? {
        Reply::Preview { text, type_name, other_os } => {
            host.note(&text);
            (type_name, other_os)
        }
        _ => return Err(err(a, "neplatná odpověď instalátoru")),
    };
    let confirm = match &type_name {
        Some(name) => {
            host.note("Všechno na disku se smaže, včetně jiných systémů.");
            ask(host, a, &format!("Napiš název disku pro potvrzení ({name})"), false, "")?
        }
        None => {
            if !host.confirm("Zapsat tyto změny disku?", &Value::Nothing) {
                return Err(cancelled(a));
            }
            String::new()
        }
    };
    if other_os {
        let c = host
            .choose("Výchozí systém při startu", &[String::from("MagnumOpus"), String::from("druhý systém")])
            .ok_or_else(|| cancelled(a))?;
        settings.default_magnum = c == 0;
        settings.rtc_local = host.confirm("Je druhý systém Windows (hodiny v místním čase)?", &Value::Nothing);
    }

    // 5. Ucet (u preinstalace zustanou stavajici ucty).
    step(host, 4);
    let mut account = AccountInput { name: String::new(), display: String::new(), password: String::new(), pin: String::new() };
    if mode == Mode::Reinstall {
        host.note("Přeinstalace: účty a soubory z předchozí instalace zůstanou.");
    } else {
        host.note("Účet je jen na tomto počítači. Nic se neposílá na internet.");
        loop {
            account.name = ask(host, a, "Jméno pro přihlášení (malá písmena, číslice, - a _)", false, "")?;
            if installplan::valid_user_name(&account.name) {
                break;
            }
            host.note("Jméno: a-z, 0-9, - a _, začíná písmenem, nejvýš 32 znaků.");
        }
        account.display = ask(host, a, "Zobrazované jméno", false, &account.name)?;
        loop {
            account.password = ask(host, a, "Heslo (aspoň 10 znaků nebo 4 slova)", true, "")?;
            if !installplan::password_ok(&account.password) {
                host.note("Heslo je moc krátké.");
                continue;
            }
            if ask(host, a, "Heslo znovu", true, "")? == account.password {
                break;
            }
            host.note("Hesla se neshodují.");
        }
        account.pin = ask(host, a, "PIN pro rychlé přihlášení (prázdné = bez PIN)", true, "")?;
    }

    // 6. Zabezpeceni.
    step(host, 5);
    host.note(NO_ENCRYPTION);

    // 7. Souhrn.
    step(host, 6);
    host.note(&format!(
        "Klávesnice: {}\nČasové pásmo: {}\nDisk: {} ({})\nÚčet: {}\nVýchozí systém: {}",
        settings.keyboards.join(", "),
        settings.zone,
        disk.model,
        mode.key(),
        if account.name.is_empty() { String::from("stávající") } else { account.name.clone() },
        installplan::default_os_text(settings.default_magnum)
    ));
    if !host.confirm("Spustit instalaci?", &Value::Nothing) {
        return Err(cancelled(a));
    }

    // 8. Prubeh.
    step(host, 7);
    let req = Request::Install { disk: disk.index, mode, confirm, settings: settings.to_toml(), account };
    finish(host, a, &req)
}
