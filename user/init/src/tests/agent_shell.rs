//! Agent AI v shellu: skutecny `termd` a `shsess` z initfs s branou
//! terminalu; radek `ai ...` s falesnym poskytovatelem vrati overeny navrh
//! prikazu (parser shlang) a odpoved modelu.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::bootstrap::kind;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Handle, Startup, time};
use shlang::value::Value;
use term::ctl::{RegMsg, Registry};
use term::proto::{ClientMsg, Item, ServerMsg};

use super::{TestResult, devices, fail};
use crate::check;

fn recv(ch: &Channel) -> Result<ServerMsg, String> {
    let m = ch.read(Some(time::deadline_after(Duration::from_secs(60)))).map_err(|e| fail("cteni z termd", e))?;
    ServerMsg::decode(&m.data).map_err(|_| String::from("neplatna zprava od termd"))
}

fn send(ch: &Channel, m: ClientMsg) -> TestResult {
    ch.write(&m.encode(), Vec::new()).map_err(|e| fail("zapis do termd", e))
}

/// Radek shellu: polozky jeho bloku a vysledek.
fn run(ch: &Channel, session: u64, line: &str) -> Result<(Vec<Item>, bool), String> {
    send(ch, ClientMsg::Run { session, line: line.into() })?;
    let mut items = Vec::new();
    let mut ours = None;
    loop {
        match recv(ch)? {
            ServerMsg::BlockStart { block, .. } if ours.is_none() => ours = Some(block),
            ServerMsg::Item { item, block, .. } if Some(block) == ours => items.push(item),
            ServerMsg::BlockEnd { ok, block, .. } if Some(block) == ours => return Ok((items, ok)),
            _ => {}
        }
    }
}

fn field<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    match v {
        Value::Record(r) => r.iter().find(|(n, _)| n == k).map(|(_, v)| v),
        _ => None,
    }
}

pub fn ai_command(s: &Startup) -> TestResult {
    let gate = crate::agent::terminal_gate().ok_or("brana terminalu nebezi")?;
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    let own = fs.connect().map_err(|e| fail("relace fsd", e))?;
    let initfs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = initfs.find("termd").ok_or("termd neni v initfs")?;
    let vmo = s.initfs.as_ref().ok_or("initfs chybi")?;
    let (reg, there) = Channel::create().map_err(|e| fail("kanal", e))?;
    // Poradi handlu termd: FS, registr, (initfs), napajeni; brana agenta
    // se stitkem.
    // Kanal napajeni test jen drzi (pozadavky relaci nevyrizuje).
    let (_power, power_there) = Channel::create().map_err(|e| fail("kanal", e))?;
    let handles = alloc::vec![
        StartHandle::user(own.into_channel().into_handle()),
        StartHandle::user(there.into_handle()),
        StartHandle::new(kind::INITFS, 0, vmo.handle().duplicate(None).map_err(|e| fail("initfs", e))?),
        StartHandle::user(power_there.into_handle()),
        StartHandle::tagged(term::ctl::tag::AGENT, gate),
    ];
    let (proc, w) = libos::spawn("termd", image, handles).map_err(|e| fail("spusteni termd", e))?;
    // Registr relaci: odpovedet na Claim (zadne relace k prevzeti).
    let mut store: Registry<Handle> = Registry::default();
    let m = reg.read(Some(time::deadline_after(Duration::from_secs(10)))).map_err(|e| fail("registr", e))?;
    let msg = RegMsg::decode(&m.data).map_err(|_| String::from("registr"))?;
    if let Some((reply, hs)) = store.handle(msg, m.handles) {
        reg.write(&reply.encode(), hs).map_err(|e| fail("registr", e))?;
    }
    send(&w, ClientMsg::Hello { version: term::proto::VERSION, cols: 80, rows: 25 })?;
    check!(matches!(recv(&w)?, ServerMsg::Welcome { .. }), "Welcome");
    send(&w, ClientMsg::Attach { session: 0 })?;
    let id = match recv(&w)? {
        ServerMsg::Attached { session, .. } => session,
        other => return Err(alloc::format!("cekal jsem Attached: {other:?}")),
    };
    let line = concat!(
        r#"ai "velke soubory" --poskytovatel=fake --skript='tool shell.propose {"command":"ls / | where size > 1MB","explanation":"velke"}\n---\nsay hotovo'"#
    );
    let (items, ok) = run(&w, id, line)?;
    // Udrzovat registr (Park relace) - jinak termd ceka.
    while let Ok(m) = reg.try_read() {
        if let Ok(msg) = RegMsg::decode(&m.data)
            && let Some((reply, hs)) = store.handle(msg, m.handles)
        {
            let _ = reg.write(&reply.encode(), hs);
        }
    }
    let _ = proc.kill();
    check!(ok, &alloc::format!("ai selhal: {items:?}"));
    let v = items.into_iter().find_map(|i| if let Item::Value(v) = i { Some(v) } else { None }).ok_or("ai nic nevypsal")?;
    check!(field(&v, "odpoved") == Some(&Value::str("hotovo")), &alloc::format!("odpoved: {v:?}"));
    let Some(Value::List(n)) = field(&v, "navrhy") else { return Err(alloc::format!("navrhy: {v:?}")) };
    check!(n.len() == 1, "jeden navrh");
    let other = field(&n[0], "druhy_zapis");
    check!(matches!(other, Some(Value::Str(s)) if s.contains("ls(")), &alloc::format!("navrh v zapisu C: {other:?}"));
    Ok(())
}
