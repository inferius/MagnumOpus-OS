//! Shell a sluzba relaci (milnik 6b) v systemu: termd z initfs, FS pres
//! fsd, nativni protokol terminalu.
//!
//! Test hraje roli okna terminalu a registru relaci: pripoji se, posila
//! radky, odpoji se a znovu pripoji. Overuje zapis do FS (koncept radku
//! v klonu bokem), undo pres pohled fsd, preruseni (Ctrl-C), prevod zapisu,
//! prehrani bloku po odpojeni a to, ze relace (proces `shsess`) i jeji
//! uloha na pozadi preziji pad termd a novy termd je prevezme z registru.
//! Ulohy `async` bezi ve vlaknech shsess (soubezne, vlastni undo, termin,
//! zruseni).

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::bootstrap::kind;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Handle, Process, Startup, time};
use term::ctl::{RegMsg, Registry};
use shlang::value::Value;
use term::proto::{ClientMsg, Item, ServerMsg};

use super::{Test, TestResult, devices, fail};

pub const TESTS: &[Test] = &[
    ("termd: relace, soubory, undo, odpojeni", session),
    ("console: displej, fokus, psani a vystup", super::console::console),
];

pub(super) struct Window {
    chan: Channel,
}

impl Window {
    pub(super) fn send(&self, m: ClientMsg) -> TestResult {
        self.chan.write(&m.encode(), Vec::new()).map_err(|e| fail("zapis do termd", e))
    }

    pub(super) fn recv(&self) -> Result<ServerMsg, String> {
        let msg = self
            .chan
            // V plnem behu je fsd po predchozich testech pomaly (radek se
            // zapisem i nekolik sekund, uklid klonu po nem dalsi): rezerva.
            .read(Some(time::deadline_after(Duration::from_secs(60))))
            .map_err(|e| fail("cteni z termd", e))?;
        ServerMsg::decode(&msg.data).map_err(|_| String::from("neplatna zprava od termd"))
    }

    /// Pocka na odpoved, kterou vybere `pick`; ostatni zpravy (udalosti
    /// jinych bloku) preskoci. Vlastni limit 60 s.
    fn reply<T>(&self, pick: impl Fn(&ServerMsg) -> Option<T>) -> Result<T, String> {
        let end = time::deadline_after(Duration::from_secs(60));
        loop {
            let m = self.recv()?;
            if let Some(v) = pick(&m) {
                return Ok(v);
            }
            if time::now() > end {
                return Err(alloc::format!("odpoved neprisla, posledni zprava: {m:?}"));
            }
        }
    }

    /// Spusti radek; vrati polozky jeho bloku a vysledek (bloky uloh
    /// `async`, ktere radek spusti, se preskoci).
    pub(super) fn run(&self, session: u64, line: &str) -> Result<(Vec<Item>, bool), String> {
        self.send(ClientMsg::Run { session, line: line.into() })?;
        let mut items = Vec::new();
        let mut ours = None;
        loop {
            match self.recv()? {
                ServerMsg::BlockStart { block, .. } if ours.is_none() => ours = Some(block),
                ServerMsg::Item { item, block, .. } if Some(block) == ours => items.push(item),
                ServerMsg::BlockEnd { ok, block, .. } if Some(block) == ours => return Ok((items, ok)),
                ServerMsg::BlockStart { .. }
                | ServerMsg::Item { .. }
                | ServerMsg::BlockEnd { .. }
                | ServerMsg::ItemUpdate { .. }
                // Adresar relace po kazdem bloku (vyzva okna).
                | ServerMsg::Cwd { .. } => {}
                other => return Err(alloc::format!("necekana zprava: {other:?}")),
            }
        }
    }

    pub(super) fn ok(&self, session: u64, line: &str) -> Result<Vec<Item>, String> {
        let (items, ok) = self.run(session, line)?;
        if !ok {
            return Err(alloc::format!("'{line}' selhal: {items:?}"));
        }
        Ok(items)
    }

    /// Hodnota jedineho vystupu radku.
    pub(super) fn value(&self, session: u64, line: &str) -> Result<Value, String> {
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

/// Registr relaci (roli supervisoru tu hraje test): drzi nahradni konce
/// kanalu relaci, aby prezily konec termd.
pub(super) struct Reg {
    pub(super) store: Registry<Handle>,
    pub(super) chan: Option<Channel>,
}

impl Reg {
    /// Zpracuje zpravy termd; `until_claim` = pockat na `Claim` (start).
    fn pump(&mut self, until_claim: bool) -> TestResult {
        let Some(chan) = &self.chan else { return Ok(()) };
        loop {
            let m = if until_claim {
                chan.read(Some(time::deadline_after(Duration::from_secs(10))))
            } else {
                chan.try_read()
            };
            let m = match m {
                Ok(m) => m,
                Err(libos::Error::ShouldWait) => return Ok(()),
                // termd skoncil: konec kanalu, uschovane konce zustavaji.
                Err(libos::Error::PeerClosed) if !until_claim => {
                    self.chan = None;
                    return Ok(());
                }
                Err(e) => return Err(fail("cteni registru", e)),
            };
            let msg = RegMsg::decode(&m.data).map_err(|_| String::from("neplatna zprava registru"))?;
            let claim = msg == RegMsg::Claim;
            if let Some((reply, hs)) = self.store.handle(msg, m.handles) {
                chan.write(&reply.encode(), hs).map_err(|e| fail("odpoved registru", e))?;
            }
            if claim && until_claim {
                return Ok(());
            }
        }
    }
}

/// Spusti termd s FS, initfs a novym kanalem registru; vrati okno.
fn start_termd(s: &Startup, reg: &mut Reg) -> Result<(Process, Window), String> {
    start_termd_with(s, reg, None)
}

/// Jako `start_termd`, navic s kanalem ke zvukove sluzbe (treti handle).
pub(super) fn start_termd_with(s: &Startup, reg: &mut Reg, audio: Option<Channel>) -> Result<(Process, Window), String> {
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    let own = fs.connect().map_err(|e| fail("nova relace fsd", e))?;
    let initfs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = initfs.find("termd").ok_or("termd neni v initfs")?;
    let vmo = s.initfs.as_ref().ok_or("initfs chybi")?;
    let (here, there) = Channel::create().map_err(|e| fail("kanal registru", e))?;
    let mut handles = alloc::vec![
        StartHandle::user(own.into_channel().into_handle()),
        StartHandle::user(there.into_handle()),
        StartHandle::new(kind::INITFS, 0, vmo.handle().duplicate(None).map_err(|e| fail("initfs", e))?),
    ];
    if let Some(a) = audio {
        // Zvuk je volitelny handle se stitkem (poradi nehraje roli).
        handles.push(StartHandle::tagged(term::ctl::tag::AUDIO, a.into_handle()));
    }
    let (proc, chan) = libos::spawn("termd", image, handles).map_err(|e| fail("spusteni termd", e))?;
    reg.chan = Some(here);
    reg.pump(true)?;
    let w = Window { chan };
    w.send(ClientMsg::Hello { version: term::proto::VERSION, cols: 80, rows: 25 })?;
    if !matches!(w.recv()?, ServerMsg::Welcome { .. }) {
        return Err("termd neposlal Welcome".into());
    }
    Ok((proc, w))
}

fn session(s: &Startup) -> TestResult {
    let mut reg = Reg { store: Registry::default(), chan: None };
    let (proc, w) = start_termd(s, &mut reg)?;
    w.send(ClientMsg::Attach { session: 0 })?;
    let id = match w.recv()? {
        ServerMsg::Attached { session, .. } => session,
        other => return Err(alloc::format!("cekal jsem Attached: {other:?}")),
    };

    // Zapis do FS a cteni zpet (B i C).
    w.ok(id, "mkdir -p /shell-test && echo ahoj > /shell-test/a.txt")?;
    crate::check_eq!(w.value(id, "open /shell-test/a.txt | from text")?, Value::str("ahoj"), "obsah souboru");
    // Doplnovani cesty za rourou (absolutni cesta, pak obsah slozky).
    for (line, want) in [
        ("ls | select name | to json | save /shell-te", "/shell-test/"),
        ("ls | to json | save /shell-test/", "/shell-test/a.txt"),
    ] {
        w.send(ClientMsg::Complete { session: id, line: line.into(), cursor: line.len() as u32, seq: 77 })?;
        let items = w.reply(|m| match m {
            ServerMsg::Completions { seq: 77, items, .. } => Some(items.clone()),
            _ => None,
        })?;
        crate::check!(items.iter().any(|(t, _, _)| t == want), alloc::format!("doplneni '{line}': {items:?}"));
    }
    crate::check_eq!(
        w.value(id, "ls(\"/shell-test\").where(it => it.ext == \"txt\").count()")?,
        Value::Int(1),
        "zapis C"
    );
    // Undo vrati posledni zmenu (snapshot fsd).
    w.ok(id, "echo zmena > /shell-test/a.txt")?;
    w.ok(id, "undo")?;
    crate::check_eq!(w.value(id, "open /shell-test/a.txt | from text")?, Value::str("ahoj"), "obsah po undo");

    // Prepsani souboru nesmi sahnout na cizi soubor se jmenem, ktere by
    // mohlo byt docasne (drive pevne `a.~termd`; ted vyhradne jmeno fsd).
    // (Kazdy radek se zapisem stoji v ladicim fsd ~5 s: dva soubory staci.)
    w.ok(id, "echo cizi > /shell-test/a.txt.~termd; echo cizi1 > /shell-test/.~a.txt.1.tmp")?;
    w.ok(id, "echo prepsano > /shell-test/a.txt")?;
    crate::check_eq!(w.value(id, "open /shell-test/a.txt | from text")?, Value::str("prepsano"), "prepsani");
    crate::check_eq!(w.value(id, "open /shell-test/a.txt.~termd | from text")?, Value::str("cizi"), "cizi soubor zustal");
    crate::check_eq!(
        w.value(id, "open /shell-test/.~a.txt.1.tmp | from text")?,
        Value::str("cizi1"),
        "soubor se jmenem docasneho zustal"
    );
    w.ok(id, "undo")?;
    crate::check_eq!(w.value(id, "open /shell-test/a.txt | from text")?, Value::str("ahoj"), "undo prepsani");
    crate::check_eq!(w.value(id, "open /shell-test/a.txt.~termd | from text")?, Value::str("cizi"), "cizi soubor po undo");

    // async: ulohy ve vlaknech shsess bezi soubezne, vysledky v poradi.
    let t0 = time::now();
    crate::check_eq!(
        w.value(id, "let a = async { sleep 1s; 1 }
let b = async { sleep 1s; 2 }
await all [a, b]")?,
        Value::List(alloc::vec![Value::Int(1), Value::Int(2)]),
        "await all"
    );
    let took = time::now() - t0;
    if took > 1_800_000_000 {
        return Err(alloc::format!("ulohy nebezely soubezne ({} ms)", took / 1_000_000));
    }
    // Uloha zapisuje vlastni relaci fsd; zmenu vrati undo relace.
    w.ok(id, "let c = async { echo uloha > /shell-test/t.txt }
await c")?;
    crate::check_eq!(w.value(id, "open /shell-test/t.txt | from text")?, Value::str("uloha"), "zapis ulohy");
    w.ok(id, "undo")?;
    crate::check_eq!(
        w.value(id, "try { open /shell-test/t.txt } else e => e.code")?,
        Value::str("NOT_FOUND"),
        "undo ulohy"
    );
    // Termin: uloha bezi dal, cancel ji zrusi a jeji koncept zahodi.
    crate::check_eq!(
        w.value(id, "let s = async { echo x > /shell-test/s.txt; sleep 5s }
let v = try await s timeout 300ms
cancel $s
v.error.code")?,
        Value::str("TIMED_OUT"),
        "await timeout"
    );
    w.ok(id, "sleep 500ms")?;
    crate::check_eq!(
        w.value(id, "try { open /shell-test/s.txt } else e => e.code")?,
        Value::str("NOT_FOUND"),
        "zrusena uloha nic nezapsala"
    );

    // Chyba se siri a ma kod; try ji zachyti.
    let (items, ok) = w.run(id, "rm /shell-test/neni.txt")?;
    if ok || !items.iter().any(|i| matches!(i, Item::Error { code, .. } if code == "NOT_FOUND")) {
        return Err(alloc::format!("rm neexistujiciho souboru: {items:?}"));
    }
    crate::check_eq!(w.value(id, "try { rm /shell-test/neni.txt } else e => e.code")?, Value::str("NOT_FOUND"), "try");

    // Ctrl-C ukonci nekonecny cyklus. Preruseni az po zacatku jeho bloku
    // (jinak by pod zatezi mohlo prijit drive a cyklus by bezel dal) a konec
    // jen toho bloku - udalosti jinych bloku (async, bg) se preskoci.
    w.send(ClientMsg::Run { session: id, line: "while true { let x = 1 }".into() })?;
    let looping = loop {
        if let ServerMsg::BlockStart { block, line, .. } = w.recv()?
            && line.starts_with("while true")
        {
            break block;
        }
    };
    w.send(ClientMsg::Interrupt { session: id })?;
    loop {
        if let ServerMsg::BlockEnd { block, ok, .. } = w.recv()?
            && block == looping
        {
            if ok {
                return Err("preruseny cyklus skoncil uspechem".into());
            }
            break;
        }
    }

    // Prevod zapisu.
    // Odpoved se paruje podle `seq`; udalosti bloku (pozdni konec zrusene
    // ulohy, bg) mezitim preskocit.
    w.send(ClientMsg::Format { session: id, line: "ls / -r | where size > 1KB".into(), style: 1, seq: 77 })?;
    let text = w.reply(|m| match m {
        ServerMsg::Formatted { seq: 77, text } => Some(text.clone()),
        _ => None,
    })?;
    crate::check_eq!(text, String::from("ls(\"/\", recursive: true).where(it => it.size > 1KB)"), "fmt --style c");

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

    // Pad termd: relace (vlastni proces) i uloha na pozadi bezi dal.
    w.ok(id, "bg { sleep 2s; echo prezil > /shell-test/bg.txt }")?;
    reg.pump(false)?;
    if reg.store.is_empty() {
        return Err("termd neuschoval nahradni konec relace".into());
    }
    proc.kill().map_err(|e| fail("ukonceni termd", e))?;
    let _ = proc.wait_exit(Some(time::deadline_after(Duration::from_secs(5))));
    drop(w);
    reg.pump(false)?;
    let (proc2, w) = start_termd(s, &mut reg)?;
    w.send(ClientMsg::Attach { session: id })?;
    loop {
        match w.recv()? {
            ServerMsg::Attached { session, .. } if session == id => break,
            ServerMsg::Fail { message } => return Err(alloc::format!("relace se neprevzala: {message}")),
            _ => {}
        }
    }
    w.ok(id, "sleep 3s")?;
    crate::check_eq!(
        w.value(id, "open /shell-test/bg.txt | from text")?,
        Value::str("prezil"),
        "uloha dobehla bez termd"
    );
    // Klony bokem se po radcich uklizi.
    crate::check_eq!(w.value(id, "ls -a /tmp/.aside | count")?, Value::Int(0), "zadne zbyle klony");
    w.ok(id, "rm -r /shell-test")?;
    drop(w);
    let _ = proc2.wait_exit(Some(time::deadline_after(Duration::from_secs(5))));
    Ok(())
}
