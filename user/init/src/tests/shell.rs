//! Shell a sluzba relaci (milnik 6b) v systemu: termd z initfs, FS pres
//! fsd, nativni protokol terminalu.
//!
//! Test hraje roli okna terminalu: pripoji se, posila radky, odpoji se
//! a znovu pripoji. Overuje zapis do FS, line undo pres snapshot fsd,
//! preruseni (Ctrl-C), prevod zapisu a prehrani bloku po odpojeni.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Startup, time};
use shlang::value::Value;
use term::proto::{ClientMsg, Item, ServerMsg};

use super::{Test, TestResult, devices, fail};

pub const TESTS: &[Test] = &[("termd: relace, soubory, undo, odpojeni", session)];

struct Window {
    chan: Channel,
}

impl Window {
    fn send(&self, m: ClientMsg) -> TestResult {
        self.chan.write(&m.encode(), Vec::new()).map_err(|e| fail("zapis do termd", e))
    }

    fn recv(&self) -> Result<ServerMsg, String> {
        let msg = self
            .chan
            .read(Some(time::deadline_after(Duration::from_secs(20))))
            .map_err(|e| fail("cteni z termd", e))?;
        ServerMsg::decode(&msg.data).map_err(|_| String::from("neplatna zprava od termd"))
    }

    /// Spusti radek; vrati polozky bloku a vysledek.
    fn run(&self, session: u64, line: &str) -> Result<(Vec<Item>, bool), String> {
        self.send(ClientMsg::Run { session, line: line.into() })?;
        let mut items = Vec::new();
        loop {
            match self.recv()? {
                ServerMsg::Item { item, .. } => items.push(item),
                ServerMsg::BlockEnd { ok, .. } => return Ok((items, ok)),
                ServerMsg::BlockStart { .. } => {}
                other => return Err(alloc::format!("necekana zprava: {other:?}")),
            }
        }
    }

    fn ok(&self, session: u64, line: &str) -> Result<Vec<Item>, String> {
        let (items, ok) = self.run(session, line)?;
        if !ok {
            return Err(alloc::format!("'{line}' selhal: {items:?}"));
        }
        Ok(items)
    }

    /// Hodnota jedineho vystupu radku.
    fn value(&self, session: u64, line: &str) -> Result<Value, String> {
        let items = self.ok(session, line)?;
        items
            .into_iter()
            .find_map(|i| match i {
                Item::Value(v) => Some(v),
                _ => None,
            })
            .ok_or_else(|| alloc::format!("'{line}' nic nevypsal"))
    }
}

fn session(s: &Startup) -> TestResult {
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    let own = fs.connect().map_err(|e| fail("nova relace fsd", e))?;
    let initfs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = initfs.find("termd").ok_or("termd neni v initfs")?;
    let handles = alloc::vec![StartHandle::user(own.into_channel().into_handle())];
    let (_proc, chan) = libos::spawn("termd", image, handles).map_err(|e| fail("spusteni termd", e))?;
    let w = Window { chan };

    w.send(ClientMsg::Hello { version: term::proto::VERSION, cols: 80, rows: 25 })?;
    if !matches!(w.recv()?, ServerMsg::Welcome { .. }) {
        return Err("termd neposlal Welcome".into());
    }
    w.send(ClientMsg::Attach { session: 0 })?;
    let id = match w.recv()? {
        ServerMsg::Attached { session, .. } => session,
        other => return Err(alloc::format!("cekal jsem Attached: {other:?}")),
    };

    // Zapis do FS a cteni zpet (B i C).
    w.ok(id, "mkdir -p /shell-test && echo ahoj > /shell-test/a.txt")?;
    crate::check_eq!(w.value(id, "open /shell-test/a.txt | from text")?, Value::str("ahoj"), "obsah souboru");
    crate::check_eq!(
        w.value(id, "ls(\"/shell-test\").where(it => it.ext == \"txt\").count()")?,
        Value::Int(1),
        "zapis C"
    );
    // Undo vrati posledni zmenu (snapshot fsd).
    w.ok(id, "echo zmena > /shell-test/a.txt")?;
    w.ok(id, "undo")?;
    crate::check_eq!(w.value(id, "open /shell-test/a.txt | from text")?, Value::str("ahoj"), "obsah po undo");

    // Chyba se siri a ma kod; try ji zachyti.
    let (items, ok) = w.run(id, "rm /shell-test/neni.txt")?;
    if ok || !items.iter().any(|i| matches!(i, Item::Error { code, .. } if code == "NOT_FOUND")) {
        return Err(alloc::format!("rm neexistujiciho souboru: {items:?}"));
    }
    crate::check_eq!(w.value(id, "try { rm /shell-test/neni.txt } else e => e.code")?, Value::str("NOT_FOUND"), "try");

    // Ctrl-C ukonci nekonecny cyklus.
    w.send(ClientMsg::Run { session: id, line: "while true { let x = 1 }".into() })?;
    w.send(ClientMsg::Interrupt { session: id })?;
    loop {
        match w.recv()? {
            ServerMsg::BlockEnd { ok, .. } => {
                if ok {
                    return Err("preruseny cyklus skoncil uspechem".into());
                }
                break;
            }
            _ => {}
        }
    }

    // Prevod zapisu.
    w.send(ClientMsg::Format { session: id, line: "ls / -r | where size > 1KB".into(), style: 1 })?;
    crate::check_eq!(
        w.recv()?,
        ServerMsg::Formatted { text: "ls(\"/\", recursive: true).where(it => it.size > 1KB)".into() },
        "fmt --style c"
    );

    // Okno se zavre; prikaz dobehne bez nej a po pripojeni se prehraje.
    w.send(ClientMsg::Detach { session: id })?;
    w.send(ClientMsg::Run { session: id, line: "echo bez okna".into() })?;
    w.send(ClientMsg::Attach { session: id })?;
    let mut replayed = false;
    loop {
        match w.recv()? {
            ServerMsg::Item { item: Item::Value(Value::Str(t)), .. } if t == "bez okna" => replayed = true,
            ServerMsg::Attached { .. } => break,
            _ => {}
        }
    }
    if !replayed {
        return Err("blok spusteny bez okna se neprehral".into());
    }
    w.ok(id, "rm -r /shell-test")?;
    Ok(())
}
