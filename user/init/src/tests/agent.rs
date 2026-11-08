//! Agent AI: relace v `agentd` s falesnym poskytovatelem (deterministicky
//! skript odpovedi a volani nastroju, zadna sit) pres branu initu.
//!
//! Overuje prava (zapis mimo prostor bez potvrzeni selze, i primo pres
//! capability `agentd`), potvrzeny zapis provede klient, pracovni prostor
//! (commit za krok, vraceni na krok a cele relace), audit, cesty, ktere
//! neopusti pocitac, limity utraty, navrh prikazu shellu overeny parserem,
//! systemove nastroje a registraci ciziho poskytovatele nastroju.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use agentproto::action::{Action, ActionFs, Outcome, apply};
use agentproto::policy::Mode;
use agentproto::price::Limits;
use agentproto::proto::{AgentMsg, ClientMsg, OpenSpec};
use agentproto::registry::ProviderMsg;
use agentproto::tool::{Effect, Schema, ToolSpec};
use fsproto::perm::{EVERYONE, Mount, mount_flags};
use fsproto::{FsClient, kind};
use libos::{Channel, Error, Startup, time};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("agentd bezi a brana vyda relaci", gate),
    ("cteni: vysledek nastroje dorazi k modelu", read_tool),
    ("zapis mimo prostor bez potvrzeni selze", write_outside_denied),
    ("potvrzeny zapis mimo prostor provede klient", write_outside_confirmed),
    ("prostor: zapis bez potvrzeni, commit za krok, vraceni", workspace_steps),
    ("audit obsahuje volani a rozhodnuti", audit_log),
    ("cesta, ktera neopusti pocitac, se neprecte", never_leave),
    ("limity utraty a poctu volani", limits),
    ("shell: navrh prikazu overi parser", shell_propose),
    ("diagnostika: systemove nastroje a cizi poskytovatel", sys_and_provider),
    ("shell: prikaz ai pres branu terminalu", super::agent_shell::ai_command),
];

const ALICE: u64 = 4242;
const BASE: &str = "/agent-t";
const WS: &str = "/agent-t/ws";
const WAIT: Duration = Duration::from_secs(30);

fn fs() -> Result<&'static FsClient, String> {
    devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| "sluzba fsd nebezi".into())
}

fn put(f: &FsClient, path: &str, data: &[u8]) -> Result<(), String> {
    match f.create(path, kind::FILE) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail(path, e)),
    }
    f.truncate(path, 0).map_err(|e| fail("truncate", e))?;
    f.write(path, 0, data).map_err(|e| fail("zapis", e))?;
    f.set_owner(path, ALICE).map_err(|e| fail("vlastnik", e))
}

fn get(f: &FsClient, path: &str) -> Result<Vec<u8>, String> {
    let st = f.stat(path).map_err(|e| fail(path, e))?;
    f.read(path, 0, st.size as usize).map_err(|e| fail("cteni", e))
}

/// Pripravi strom testu (vlastnik alice) a cisty dataset prostoru.
fn setup() -> Result<&'static FsClient, String> {
    let f = fs()?;
    crate::system::grant_traverse(f, "/", EVERYONE).map_err(|e| fail("pruchod /", e))?;
    for d in [BASE, "/agent-t/.ssh"] {
        match f.create(d, kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(fail(d, e)),
        }
        f.set_owner(d, ALICE).map_err(|e| fail("vlastnik", e))?;
    }
    if f.stat(WS).is_err() {
        f.create_dataset(WS).map_err(|e| fail("dataset prostoru", e))?;
        f.set_owner(WS, ALICE).map_err(|e| fail("vlastnik prostoru", e))?;
    }
    put(f, "/agent-t/ws/a.txt", b"puvodni\n")?;
    put(f, "/agent-t/mimo.txt", b"mimo\n")?;
    put(f, "/agent-t/.ssh/id", b"TAJNE\n")?;
    f.fsync().map_err(|e| fail("fsync", e))?;
    Ok(f)
}

fn spec(mode: Mode, script: &str, ws: bool) -> OpenSpec {
    let mut s = OpenSpec::new(ALICE, mode, "fake");
    s.script = String::from(script);
    if ws {
        s.workspace = String::from(WS);
    }
    s
}

fn open(s: OpenSpec, sys: bool) -> Result<Channel, String> {
    crate::agent::open(ALICE, &[], s, sys).map(|(c, _)| c).map_err(|e| fail("otevreni relace", e))
}

fn recv(ch: &Channel) -> Result<AgentMsg, String> {
    let m = ch.read(Some(time::deadline_after(WAIT))).map_err(|e| fail("odpoved agentd", e))?;
    AgentMsg::decode(&m.data).map_err(|e| fail("zprava agentd", e))
}

/// Klient: provede potvrzenou akci se svymi pravy (relace alice RW).
struct AliceFs(FsClient);

impl ActionFs for AliceFs {
    fn read(&mut self, path: &str) -> libos::Result<Vec<u8>> {
        let st = self.0.stat(path)?;
        self.0.read(path, 0, st.size as usize)
    }
    fn write(&mut self, path: &str, data: &[u8]) -> libos::Result<()> {
        match self.0.create(path, kind::FILE) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
        self.0.truncate(path, 0)?;
        self.0.write(path, 0, data)
    }
    fn mkdir(&mut self, path: &str) -> libos::Result<()> {
        self.0.create(path, kind::DIR)
    }
    fn trash(&mut self, path: &str) -> libos::Result<()> {
        self.0.trash(path).map(|_| ())
    }
    fn rename(&mut self, from: &str, to: &str) -> libos::Result<()> {
        self.0.rename(from, to)
    }
}

fn alice_rw() -> Result<AliceFs, String> {
    let (_, c) = fs()?.connect_as(ALICE, &[], &[Mount::new("/", "/", 0)]).map_err(|e| fail("relace alice", e))?;
    Ok(AliceFs(c))
}

/// Dotaz az do konce; na potvrzeni odpovi `answer` (None = odmitnout).
fn ask(ch: &Channel, text: &str, mut answer: Option<&mut AliceFs>) -> Result<Vec<AgentMsg>, String> {
    ch.write(&ClientMsg::Ask { text: String::from(text) }.encode(), Vec::new()).map_err(|e| fail("dotaz", e))?;
    let mut out = Vec::new();
    loop {
        let m = recv(ch)?;
        if let AgentMsg::Confirm { id, action, .. } = &m {
            let outcome = match answer.as_deref_mut() {
                Some(fs) => match apply(fs, action) {
                    Ok(t) => Outcome::Done(t),
                    Err(e) => Outcome::Failed(e, String::new()),
                },
                None => Outcome::Declined,
            };
            ch.write(&ClientMsg::Confirmed { id: *id, outcome }.encode(), Vec::new()).map_err(|e| fail("potvrzeni", e))?;
        }
        let end = m.ends_turn();
        out.push(m);
        if end {
            return Ok(out);
        }
    }
}

fn text_of(msgs: &[AgentMsg]) -> String {
    msgs.iter().filter_map(|m| if let AgentMsg::Text { text } = m { Some(text.as_str()) } else { None }).collect()
}

fn decisions(msgs: &[AgentMsg]) -> Vec<String> {
    msgs.iter().filter_map(|m| if let AgentMsg::Tool { decision, .. } = m { Some(decision.clone()) } else { None }).collect()
}

fn request(ch: &Channel, m: ClientMsg) -> Result<AgentMsg, String> {
    ch.write(&m.encode(), Vec::new()).map_err(|e| fail("zprava", e))?;
    recv(ch)
}

fn audit_lines(ch: &Channel) -> Result<Vec<String>, String> {
    match request(ch, ClientMsg::Audit { from: 0, max: 500 })? {
        AgentMsg::AuditPage { lines, .. } => Ok(lines),
        other => Err(format!("audit: {other:?}")),
    }
}

fn gate(_: &Startup) -> TestResult {
    setup()?;
    check!(crate::agent::AGENTD.lock().is_some(), "agentd nebezi");
    let ch = open(spec(Mode::Chat, "say ahoj {user}", false), false)?;
    let msgs = ask(&ch, "svete", None)?;
    check_eq!(text_of(&msgs), String::from("ahoj svete"), "odpoved falesneho modelu");
    check!(matches!(msgs.last(), Some(AgentMsg::TurnEnd { .. })), "konec dotazu");
    // Neexistujici prostor: relace se neotevre (prostor musi byt dataset).
    let mut s = spec(Mode::Chat, "say x", false);
    s.workspace = String::from(BASE);
    let ch = open(s, false)?;
    let r = recv(&ch)?;
    check!(matches!(r, AgentMsg::Error { code: Error::NotSupported, .. }), &format!("prostor mimo dataset odmitnut: {r:?}"));
    Ok(())
}

fn read_tool(_: &Startup) -> TestResult {
    setup()?;
    let script = "tool fs.read {\"path\":\"/agent-t/ws/a.txt\"}\n---\nsay obsah: {result}";
    let ch = open(spec(Mode::Chat, script, false), false)?;
    let msgs = ask(&ch, "precti a.txt", None)?;
    check_eq!(decisions(&msgs), vec![String::from("allow")], "cteni volne");
    check!(text_of(&msgs).contains("obsah: puvodni"), "vysledek nastroje u modelu");
    Ok(())
}

fn write_outside_denied(_: &Startup) -> TestResult {
    let f = setup()?;
    let script = "tool fs.write {\"path\":\"/agent-t/mimo.txt\",\"content\":\"zmeneno\\n\"}\n---\nsay {result}";
    // 1. Klient potvrzeni odmitne: soubor se nezmeni.
    let ch = open(spec(Mode::Chat, script, true), false)?;
    let msgs = ask(&ch, "zmen mimo.txt", None)?;
    let preview = msgs.iter().find_map(|m| if let AgentMsg::Confirm { preview, action, .. } = m { Some((preview.clone(), action.clone())) } else { None });
    let (preview, action) = preview.ok_or("chybi dotaz na potvrzeni")?;
    check!(preview.contains("-mimo") && preview.contains("+zmeneno"), "nahled zmeny");
    check!(matches!(action, Action::FsWrite { .. }), "akce zapisu");
    check!(text_of(&msgs).contains("odmitl"), "model vi, ze uzivatel odmitl");
    check_eq!(get(f, "/agent-t/mimo.txt")?, b"mimo\n".to_vec(), "soubor beze zmeny");
    // 2. Klient bez UI: odmitnuto bez dotazu.
    let mut s = spec(Mode::Chat, script, true);
    s.can_confirm = false;
    let ch = open(s, false)?;
    let msgs = ask(&ch, "zmen mimo.txt", None)?;
    check_eq!(decisions(&msgs), vec![String::from("deny")], "bez potvrzeni odmitnuto");
    check!(!msgs.iter().any(|m| matches!(m, AgentMsg::Confirm { .. })), "zadny dotaz");
    check_eq!(get(f, "/agent-t/mimo.txt")?, b"mimo\n".to_vec(), "soubor beze zmeny");
    // 3. Capability, kterou brana vydava agentd pro cteni: zapis neprojde
    //    ani primo (kdyby pravidla v agentd selhala).
    let (_, ro) = f.connect_as(ALICE, &[], &[Mount::new("/", "/", mount_flags::READ_ONLY)]).map_err(|e| fail("relace", e))?;
    check!(ro.write("/agent-t/mimo.txt", 0, b"X").is_err(), "zapis pres capability jen pro cteni");
    check!(ro.create("/agent-t/novy.txt", kind::FILE).is_err(), "vytvoreni pres capability jen pro cteni");
    // Capability prostoru nevidi nic mimo prostor.
    let (_, wsfs) = f.connect_as(ALICE, &[], &[Mount::new(WS, WS, 0)]).map_err(|e| fail("relace prostoru", e))?;
    check!(wsfs.write("/agent-t/mimo.txt", 0, b"X").is_err(), "zapis mimo prostor pres capability prostoru");
    check_eq!(get(f, "/agent-t/mimo.txt")?, b"mimo\n".to_vec(), "soubor beze zmeny");
    Ok(())
}

fn write_outside_confirmed(_: &Startup) -> TestResult {
    let f = setup()?;
    let script = "tool fs.write {\"path\":\"/agent-t/mimo.txt\",\"content\":\"potvrzeno\\n\"}\n---\nsay {result}";
    let ch = open(spec(Mode::Chat, script, false), false)?;
    let mut alice = alice_rw()?;
    let msgs = ask(&ch, "zmen mimo.txt", Some(&mut alice))?;
    check_eq!(decisions(&msgs), vec![String::from("confirm")], "potvrzeni");
    check!(text_of(&msgs).contains("potvrdil"), "model dostal vysledek");
    check_eq!(get(f, "/agent-t/mimo.txt")?, b"potvrzeno\n".to_vec(), "zapsal klient");
    Ok(())
}

fn workspace_steps(_: &Startup) -> TestResult {
    let f = setup()?;
    let _ = f.trash("/agent-t/ws/b.txt");
    let script = concat!(
        "tool fs.write {\"path\":\"/agent-t/ws/b.txt\",\"content\":\"novy\"}\n",
        "tool fs.write {\"path\":\"/agent-t/ws/a.txt\",\"content\":\"upraveno\\n\"}\n---\n",
        "tool fs.mkdir {\"path\":\"/agent-t/ws/sub\"}\n---\n",
        "say hotovo\n",
    );
    let ch = open(spec(Mode::Dev, script, true), false)?;
    let msgs = ask(&ch, "uprav projekt", None)?;
    check!(!msgs.iter().any(|m| matches!(m, AgentMsg::Confirm { .. })), "v prostoru bez potvrzeni");
    check_eq!(decisions(&msgs), vec![String::from("allow-ws"); 3], "zapis v prostoru volne");
    check_eq!(get(f, "/agent-t/ws/a.txt")?, b"upraveno\n".to_vec(), "zapsano");
    check!(f.stat("/agent-t/ws/sub").is_ok(), "slozka");
    let steps = match request(&ch, ClientMsg::Steps)? {
        AgentMsg::Steps(s) => s,
        other => return Err(format!("kroky: {other:?}")),
    };
    check_eq!(steps.iter().map(|s| s.step).collect::<Vec<_>>(), vec![0, 1, 2], "zacatek + commit za krok");
    let diff = match request(&ch, ClientMsg::StepDiff { step: 1 })? {
        AgentMsg::Diff(d) => d,
        other => return Err(format!("rozdil: {other:?}")),
    };
    check!(diff.iter().any(|c| c.kind == 1 && c.path.ends_with("/b.txt")), "krok 1: b.txt pridano");
    check!(diff.iter().any(|c| c.kind == 3 && c.path.ends_with("/a.txt")), "krok 1: a.txt zmeneno");
    check!(!diff.iter().any(|c| c.path.ends_with("/sub")), "krok 1 bez slozky z kroku 2");
    // Navrat na krok 1: slozka z kroku 2 zmizi, soubory zustanou.
    check!(matches!(request(&ch, ClientMsg::Revert { to: 1 })?, AgentMsg::Reverted { step: 1, .. }), "vraceno na krok 1");
    check!(f.stat("/agent-t/ws/sub").is_err(), "slozka z kroku 2 pryc");
    check_eq!(get(f, "/agent-t/ws/b.txt")?, b"novy".to_vec(), "b.txt z kroku 1");
    // Cela relace: stav pred agentem.
    let r = request(&ch, ClientMsg::Revert { to: 0 })?;
    check!(matches!(r, AgentMsg::Reverted { step: 0, .. }), &format!("vraceno vse: {r:?}"));
    check!(f.stat("/agent-t/ws/b.txt").is_err(), "pridany soubor v kosi");
    check_eq!(get(f, "/agent-t/ws/a.txt")?, b"puvodni\n".to_vec(), "puvodni obsah");
    Ok(())
}

fn audit_log(_: &Startup) -> TestResult {
    setup()?;
    let script = concat!(
        "tool fs.read {\"path\":\"/agent-t/mimo.txt\"}\n",
        "tool fs.write {\"path\":\"/agent-t/ws/c.txt\",\"content\":\"TAJNY OBSAH\"}\n",
        "tool fs.write {\"path\":\"/agent-t/mimo.txt\",\"content\":\"x\"}\n---\nsay ok",
    );
    let ch = open(spec(Mode::Chat, script, true), false)?;
    ask(&ch, "audit", None)?;
    let lines = audit_lines(&ch)?;
    let has = |what: &str, decision: &str| lines.iter().any(|l| l.contains(what) && l.contains(decision));
    check!(has("\tfs.read\t", "\tallow\t"), "cteni v auditu");
    check!(has("\tfs.write\t", "\tallow-ws\t"), "zapis v prostoru v auditu");
    check!(has("\tfs.write\t", "\tconfirm-no\t"), "odmitnuty zapis v auditu");
    check!(lines.iter().any(|l| l.contains("\tmodel\t") && l.contains("fake/fake-1") && l.contains("odeslano=")), "volani modelu s odeslanymi bajty");
    check!(lines.iter().any(|l| l.contains("\tsession\t") && l.contains("\topen\t")), "otevreni relace");
    check!(!lines.iter().any(|l| l.contains("TAJNY OBSAH")), "obsah zapisu v auditu neni");
    // Audit je mimo dosah uzivatele i agenta (/system/agent).
    let (_, alice) = fs()?.connect_as(ALICE, &[], &[Mount::new("/", "/", 0)]).map_err(|e| fail("relace", e))?;
    check!(alice.list(crate::agent::STATE_DIR).is_err(), "uzivatel stav agentd nevidi");
    Ok(())
}

fn never_leave(_: &Startup) -> TestResult {
    setup()?;
    let script = "tool fs.read {\"path\":\"/agent-t/.ssh/id\"}\n---\nsay {result}";
    let ch = open(spec(Mode::Chat, script, false), false)?;
    let msgs = ask(&ch, "precti klic", None)?;
    check_eq!(decisions(&msgs), vec![String::from("deny")], "odmitnuto");
    let t = text_of(&msgs);
    check!(t.contains("nesmi opustit") && !t.contains("TAJNE"), "obsah se k modelu nedostal");
    Ok(())
}

fn limits(_: &Startup) -> TestResult {
    setup()?;
    // Limit utraty relace: odhad prvniho volani je vetsi -> odmitnuto predem.
    let mut s = spec(Mode::Chat, "say nikdy", false);
    s.limits = Limits { max_tokens: 1000, max_calls: 0, session: 10, day: 0 };
    let ch = open(s, false)?;
    let msgs = ask(&ch, "ahoj", None)?;
    check!(matches!(msgs.last(), Some(AgentMsg::Error { code: Error::NoResources, .. })), "limit utraty");
    check!(text_of(&msgs).is_empty(), "model se nevolal");
    // Limit poctu volani modelu na dotaz.
    let lp = "tool fs.stat {\"path\":\"/agent-t\"}\n---\n";
    let mut s = spec(Mode::Chat, &lp.repeat(5), false);
    s.limits = Limits { max_tokens: 100, max_calls: 2, session: 0, day: 0 };
    let ch = open(s, false)?;
    let msgs = ask(&ch, "smycka", None)?;
    check!(matches!(msgs.last(), Some(AgentMsg::Error { code: Error::OutOfRange, .. })), "limit volani");
    check_eq!(decisions(&msgs).len(), 2, "dve volani modelu");
    match request(&ch, ClientMsg::Spend)? {
        AgentMsg::Spend { spend, limits } => {
            check!(spend.session > 0, "utrata relace zapocitana");
            check_eq!(limits.max_calls, 2, "limity relace");
        }
        other => return Err(format!("utrata: {other:?}")),
    }
    Ok(())
}

fn shell_propose(_: &Startup) -> TestResult {
    setup()?;
    let script = concat!(
        "tool shell.propose {\"command\":\"ls | where (\",\"explanation\":\"x\"}\n---\n",
        "say chyba: {result}\n",
        "tool shell.propose {\"command\":\"ls /agent-t | where size > 1MB\",\"explanation\":\"velke soubory\",\"steps\":[\"rm /agent-t/stare\"]}\n---\n",
        "say {result}\n",
        "tool fs.write {\"path\":\"/agent-t/x\",\"content\":\"x\"}\n---\nsay konec",
    );
    let ch = open(spec(Mode::Shell, script, false), false)?;
    let msgs = ask(&ch, "najdi velke soubory", None)?;
    let t = text_of(&msgs);
    check!(t.contains("chyba: navrh neprosel parserem"), "neplatny navrh vracen modelu");
    let props: Vec<_> = msgs.iter().filter_map(|m| if let AgentMsg::Proposal { command, other, risky, .. } = m { Some((command.clone(), other.clone(), *risky)) } else { None }).collect();
    check_eq!(props.len(), 2, "dva overene kroky");
    check!(props[0].0.starts_with("ls /agent-t") && props[0].1.contains("ls(") && !props[0].2, "krok 1 v obou zapisech");
    check!(props[1].2, "rm je riskantni");
    check!(t.contains("navrh overen"), "model vi, ze navrh prosel");
    // Rezim shell nema zapis (zadne potvrzeni ani zapis).
    check!(decisions(&msgs).last().is_some_and(|d| d == "deny"), "zapis v rezimu shell odmitnut");
    check!(!msgs.iter().any(|m| matches!(m, AgentMsg::Confirm { .. })), "bez dotazu");
    Ok(())
}

/// Cizi poskytovatel nastroju: odpovi ozvenou a rekne, zda dostal capability.
fn echo_provider(ch: Channel) {
    let tool = ToolSpec::new("test.echo", "Ozvena.", Effect::Read, Schema::object().string("text", "text", true).build());
    if ch.write(&ProviderMsg::Register { tools: vec![tool] }.encode(), Vec::new()).is_err() {
        return;
    }
    while let Ok(m) = ch.read(None) {
        if let Ok(ProviderMsg::Call { id, input, .. }) = ProviderMsg::decode(&m.data) {
            // Capability relace: FS s identitou uzivatele (alice vidi svuj soubor).
            let cap = m.handles.into_iter().next().map(|h| FsClient::new(Channel::from_handle(h)));
            let sees = cap.as_ref().is_some_and(|c| c.stat("/agent-t/mimo.txt").is_ok());
            let content = format!("echo {input} cap={}", if sees { "ano" } else { "ne" });
            let _ = ch.write(&ProviderMsg::Result { id, is_error: false, content }.encode(), Vec::new());
        }
    }
}

fn sys_and_provider(_: &Startup) -> TestResult {
    setup()?;
    {
        let sup = crate::supervisor::SUPERVISOR.lock();
        crate::agent::publish(&sup.status(), &[]);
    }
    let script = "tool sys.services {}\n---\nsay {result}";
    let ch = open(spec(Mode::Diag, script, false), true)?;
    let msgs = ask(&ch, "jak je na tom system", None)?;
    check_eq!(decisions(&msgs), vec![String::from("allow")], "systemove nastroje jen cteni");
    check!(text_of(&msgs).contains("fsd stav="), "stav sluzeb ze supervisoru");
    // Prehled uloh z taskd (identita uzivatele relace, jen cteni).
    let ch = open(spec(Mode::Diag, "tool sys.tasks {}\n---\nsay ulohy: {result}", false), true)?;
    let t = text_of(&ask(&ch, "jake ulohy bezi", None)?);
    check!(t.starts_with("ulohy: ") && !t.contains("taskd") && !t.contains("nebezi"), &format!("prehled uloh: {t}"));
    let (ours, theirs) = Channel::create().map_err(|e| fail("kanal", e))?;
    libos::thread::spawn("agent-test-echo", move || echo_provider(theirs)).map_err(|e| fail("vlakno", e))?;
    crate::agent::register_provider("test", ours).map_err(|e| fail("registrace", e))?;
    let ch = open(spec(Mode::Chat, "tool test.echo {\"text\":\"ahoj\"}\n---\nsay {result}", false), false)?;
    let msgs = ask(&ch, "ozvena", None)?;
    let t = text_of(&msgs);
    check!(t.contains("echo {\"text\":\"ahoj\"}") && t.contains("cap=ano"), "nastroj sluzby s capability relace");
    // Neplatny vstup podle schematu se sluzbe vubec neposle.
    let ch = open(spec(Mode::Chat, "tool test.echo {\"text\":1}\n---\nsay {result}", false), false)?;
    check!(text_of(&ask(&ch, "ozvena", None)?).contains("neplatny vstup"), "schema overeno pred volanim");
    Ok(())
}
