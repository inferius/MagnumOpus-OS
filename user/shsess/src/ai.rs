//! Prikaz `ai`: pomocnik agenta v shellu (navrh `docs/cs/agent.md`).
//!
//! Relaci agenta otevre brana initu (kanal od termd); shsess ji drzi pro
//! dalsi `ai` v teto relaci shellu. Shell nema dialog potvrzeni uvnitr
//! prikazu, proto relace z shellu potvrzeni neumi (`can_confirm = false`):
//! agent tu zapisuje jen v pracovnim prostoru, jinak navrhuje prikazy,
//! ktere spusti uzivatel.
//!
//! ```text
//! ai "dotaz"                      rezim shell: odpoved a overene navrhy prikazu
//! ai "dotaz" --rezim=diag         diagnostika (stav sluzeb, pady, log)
//! ai "dotaz" --prostor=~/proj     rezim dev s pracovnim prostorem (dataset)
//! ai --vysvetli="text chyby"      vysvetleni chyby (s mistni dokumentaci)
//! ai --kroky | --rozdil=N | --vrat=N | --audit | --utrata
//! ai --klic=soubor                ulozit API klic (soubor pak smazte)
//! ```
//!
//! `ai` je pro shell program bez podpisu: hodnota prepinace se pise
//! `--jmeno=hodnota` (`--jmeno hodnota` by byl prepinac a pozicni argument).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use agentproto::policy::Mode;
use agentproto::price::usd;
use agentproto::proto::{AgentMsg, ClientMsg, OpenSpec};
use libos::{Channel, time};
use shlang::host::Fs;
use shlang::{ShError, Value};

/// Otevrena relace agenta teto relace shellu.
pub struct AgentConn {
    ch: Channel,
    mode: Mode,
    workspace: String,
}

/// Stav prikazu `ai` v shsess.
#[derive(Default)]
pub struct Ai {
    /// Kanal k brane initu (jen zapis zadosti; odpoved prijde na kanal
    /// pribaleny k zadosti).
    pub gate: Option<Channel>,
    conn: Option<AgentConn>,
}

const WAIT: Duration = Duration::from_secs(600);

fn err(msg: impl Into<String>) -> ShError {
    ShError::new(shlang::error::code::USER, msg)
}

fn rec(fields: Vec<(&str, Value)>) -> Value {
    Value::Record(fields.into_iter().map(|(k, v)| (String::from(k), v)).collect())
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Int(i) => Some(format!("{i}")),
        _ => None,
    }
}

fn expand(home: &str, p: &str) -> String {
    match p.strip_prefix('~') {
        Some(rest) => format!("{}{}", home.trim_end_matches('/'), rest),
        None => String::from(p),
    }
}

impl Ai {
    pub fn new(gate: Option<Channel>) -> Self {
        Self { gate, conn: None }
    }

    fn open(&mut self, mode: Mode, workspace: &str, provider: &str, script: &str) -> Result<&AgentConn, ShError> {
        let reuse = self.conn.as_ref().is_some_and(|c| c.mode == mode && c.workspace == workspace);
        if !reuse {
            let gate = self.gate.as_ref().ok_or_else(|| err("agent AI neni k dispozici (agentd nebezi)"))?;
            let mut spec = OpenSpec::new(0, mode, provider);
            spec.workspace = String::from(workspace);
            spec.script = String::from(script);
            spec.can_confirm = false;
            let (ours, theirs) = Channel::create().map_err(|e| ShError::sys(e, "kanal"))?;
            let mut w = wire::Writer::new();
            spec.put(&mut w);
            gate.write(&w.finish(), alloc::vec![theirs.into_handle()]).map_err(|e| ShError::sys(e, "brana agenta"))?;
            let reply = ours.read(Some(time::deadline_after(Duration::from_secs(15)))).map_err(|e| ShError::sys(e, "brana agenta"))?;
            let txt = String::from_utf8_lossy(&reply.data).into_owned();
            let ch = reply.handles.into_iter().next().map(Channel::from_handle);
            match (txt.starts_with("ok"), ch) {
                (true, Some(ch)) => self.conn = Some(AgentConn { ch, mode, workspace: String::from(workspace) }),
                _ => return Err(err(format!("relaci agenta nejde otevrit: {txt}"))),
            }
        }
        Ok(self.conn.as_ref().unwrap())
    }

    fn conn(&self) -> Result<&AgentConn, ShError> {
        self.conn.as_ref().ok_or_else(|| err("zadna relace agenta (nejdriv ai \"dotaz\")"))
    }

    /// Jedna zprava a odpoved (kroky, rozdil, vraceni, audit, utrata, klic).
    fn request(&self, m: ClientMsg) -> Result<AgentMsg, ShError> {
        let c = self.conn()?;
        c.ch.write(&m.encode(), Vec::new()).map_err(|e| ShError::sys(e, "agentd"))?;
        let r = c.ch.read(Some(time::deadline_after(Duration::from_secs(60)))).map_err(|e| ShError::sys(e, "agentd"))?;
        AgentMsg::decode(&r.data).map_err(|e| ShError::sys(e, "agentd"))
    }

    fn ask(&mut self, q: &str) -> Result<Value, ShError> {
        let c = self.conn()?;
        c.ch.write(&ClientMsg::Ask { text: String::from(q) }.encode(), Vec::new()).map_err(|e| ShError::sys(e, "agentd"))?;
        let mut answer = String::new();
        let mut proposals = Vec::new();
        let mut tools = Vec::new();
        loop {
            let r = c.ch.read(Some(time::deadline_after(WAIT))).map_err(|e| ShError::sys(e, "agentd"))?;
            match AgentMsg::decode(&r.data).map_err(|e| ShError::sys(e, "agentd"))? {
                AgentMsg::Text { text } => answer.push_str(&text),
                AgentMsg::Tool { name, decision, summary } => {
                    tools.push(rec(alloc::vec![("nastroj", Value::Str(name)), ("rozhodnuti", Value::Str(decision)), ("vstup", Value::Str(summary))]))
                }
                AgentMsg::Proposal { command, style, other, risky, explanation } => proposals.push(rec(alloc::vec![
                    ("prikaz", Value::Str(command)),
                    ("zapis", Value::Str(style)),
                    ("druhy_zapis", Value::Str(other)),
                    ("riskantni", Value::Bool(risky)),
                    ("vysvetleni", Value::Str(explanation)),
                ])),
                // Relace z shellu potvrzeni neumi (agentd se nezepta).
                AgentMsg::Confirm { id, .. } => {
                    let m = ClientMsg::Confirmed { id, outcome: agentproto::action::Outcome::Declined };
                    let _ = c.ch.write(&m.encode(), Vec::new());
                }
                AgentMsg::TurnEnd { cost, step, .. } => {
                    return Ok(rec(alloc::vec![
                        ("odpoved", Value::Str(answer)),
                        ("navrhy", Value::List(proposals)),
                        ("nastroje", Value::List(tools)),
                        ("cena", Value::Str(usd(cost))),
                        ("krok", Value::Int(i64::from(step))),
                    ]));
                }
                AgentMsg::Error { code, message } => return Err(ShError::sys(code, message)),
                _ => {}
            }
        }
    }

    /// Prikaz `ai` s argumenty shellu.
    pub fn run(&mut self, args: &[(Option<String>, Value)], fs: &mut dyn Fs, home: &str) -> Result<Value, ShError> {
        // Prepinac s hodnotou zapsany bez `=` (hodnota je pak pozicni argument).
        // (`--rozdil` a `--vrat` bez hodnoty = krok 0.)
        for n in ["rezim", "prostor", "vysvetli", "klic", "poskytovatel", "skript"] {
            if args.iter().any(|(k, v)| k.as_deref() == Some(n) && *v == Value::Bool(true)) {
                return Err(err(format!("hodnotu zapiste jako --{n}=hodnota")));
            }
        }
        let named = |n: &str| args.iter().find(|(k, _)| k.as_deref() == Some(n)).map(|(_, v)| v);
        let flag = |n: &str| named(n).is_some();
        let pos: Vec<String> = args.iter().filter(|(k, _)| k.is_none()).filter_map(|(_, v)| text(v)).collect();
        let num = |n: &str| named(n).and_then(text).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        if flag("kroky") {
            return match self.request(ClientMsg::Steps)? {
                AgentMsg::Steps(s) => Ok(Value::List(
                    s.into_iter().map(|s| rec(alloc::vec![("krok", Value::Int(i64::from(s.step))), ("zprava", Value::Str(s.message))])).collect(),
                )),
                AgentMsg::Error { code, message } => Err(ShError::sys(code, message)),
                _ => Err(err("neocekavana odpoved")),
            };
        }
        if flag("rozdil") {
            return match self.request(ClientMsg::StepDiff { step: num("rozdil") })? {
                AgentMsg::Diff(d) => Ok(Value::List(
                    d.into_iter().map(|c| rec(alloc::vec![("druh", Value::Int(i64::from(c.kind))), ("cesta", Value::Str(c.path))])).collect(),
                )),
                AgentMsg::Error { code, message } => Err(ShError::sys(code, message)),
                _ => Err(err("neocekavana odpoved")),
            };
        }
        if flag("vrat") {
            return match self.request(ClientMsg::Revert { to: num("vrat") })? {
                AgentMsg::Reverted { files, step } => Ok(Value::Str(format!("vraceno na krok {step} ({files} polozek)"))),
                AgentMsg::Error { code, message } => Err(ShError::sys(code, message)),
                _ => Err(err("neocekavana odpoved")),
            };
        }
        if flag("audit") {
            self.open(Mode::Shell, "", "claude", "")?;
            return match self.request(ClientMsg::Audit { from: 0, max: 200 })? {
                AgentMsg::AuditPage { lines, .. } => Ok(Value::List(lines.into_iter().map(Value::Str).collect())),
                AgentMsg::Error { code, message } => Err(ShError::sys(code, message)),
                _ => Err(err("neocekavana odpoved")),
            };
        }
        if flag("utrata") {
            self.open(Mode::Shell, "", "claude", "")?;
            return match self.request(ClientMsg::Spend)? {
                AgentMsg::Spend { spend, limits } => Ok(rec(alloc::vec![
                    ("relace", Value::Str(usd(spend.session))),
                    ("dnes", Value::Str(usd(spend.day))),
                    ("limit_relace", Value::Str(usd(limits.session))),
                    ("limit_den", Value::Str(usd(limits.day))),
                ])),
                _ => Err(err("neocekavana odpoved")),
            };
        }
        if let Some(path) = named("klic").and_then(text) {
            let path = expand(home, &path);
            let data = fs.read(&path).map_err(|e| ShError::sys(e, format!("klic {path}")))?;
            let key: Vec<u8> = data.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
            self.open(Mode::Shell, "", "claude", "")?;
            return match self.request(ClientMsg::SetKey { provider: String::from("claude"), key })? {
                AgentMsg::KeyStored => Ok(Value::Str(format!("klic ulozen zasifrovane; smazte soubor: rm {path}"))),
                AgentMsg::Error { code, message } => Err(ShError::sys(code, message)),
                _ => Err(err("neocekavana odpoved")),
            };
        }
        let workspace = named("prostor").and_then(text).map(|p| expand(home, &p)).unwrap_or_default();
        let mode = match named("rezim").and_then(text).as_deref() {
            Some("diag") => Mode::Diag,
            Some("chat") => Mode::Chat,
            Some("dev") => Mode::Dev,
            Some("shell") | None if workspace.is_empty() => Mode::Shell,
            Some("shell") => Mode::Shell,
            _ => Mode::Dev,
        };
        let provider = named("poskytovatel").and_then(text).unwrap_or_else(|| String::from("claude"));
        // Skript falesneho modelu (vyvoj, testy): `\n` = novy radek.
        let script = named("skript").and_then(text).unwrap_or_default().replace("\\n", "\n");
        let mut question = pos.join(" ");
        if let Some(e) = named("vysvetli").and_then(text) {
            question = format!("Vysvetli tuto chybu shellu a navrhni opravu:\n{e}");
            if let Some(doc) = agentsh::explain_context(&e) {
                question.push_str("\n\nMistni dokumentace:\n");
                question.push_str(&doc);
            }
        }
        if question.trim().is_empty() {
            return Err(err("ai \"dotaz\" (nebo --kroky, --rozdil N, --vrat N, --audit, --utrata, --klic soubor)"));
        }
        self.open(mode, &workspace, &provider, &script)?;
        self.ask(&question)
    }
}
