//! Falesny poskytovatel modelu: deterministicke odpovedi ze skriptu.
//!
//! Pro testy v QEMU a na hostiteli (zadna sit, zadna cena). Skript je
//! posloupnost odpovedi oddelenych radkem `---`; n-te volani modelu dostane
//! n-tou odpoved (pres vsechny dotazy relace). Radky odpovedi:
//!
//! ```text
//! say <text>            textovy blok (\n = novy radek)
//! tool <jmeno> <json>   volani nastroje (vstup = JSON objekt)
//! stop <duvod>          end_turn / tool_use / max_tokens / refusal
//! usage <vstup> <vystup> spotreba tokenu (vychozi odhad z velikosti)
//! # komentar
//! ```
//!
//! V textu `say` se nahradi `{result}` obsahem vysledku nastroju
//! z posledni zpravy uzivatele (oddelene `|`) a `{user}` poslednim textem
//! uzivatele - test tak overi, ze vysledek nastroje dorazil k modelu.

use alloc::string::String;
use alloc::vec::Vec;

use crate::json::{self, Value};
use crate::msg::{Block, Message, Role, Stop, Usage};
use crate::price::Price;
use crate::provider::{Provider, ProviderError, Request, Response};
use crate::stream::{Event, events_of};

#[derive(Debug, Clone, PartialEq)]
enum Line {
    Say(String),
    Tool(String, Value),
    Stop(Stop),
    Usage(u64, u64),
}

/// Chyba skriptu: cislo radku a popis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub line: usize,
    pub msg: String,
}

impl core::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "skript falesneho modelu, radek {}: {}", self.line, self.msg)
    }
}

pub struct Fake {
    turns: Vec<Vec<Line>>,
    next: usize,
    /// Pocet volani (pro testy: kolikrat se model volal).
    pub calls: usize,
    /// Posledni pozadavek (pro testy: co dostal model).
    pub last_request: Option<Request>,
}

impl Fake {
    pub fn parse(script: &str) -> Result<Self, ScriptError> {
        let mut turns = Vec::new();
        let mut cur = Vec::new();
        for (i, raw) in script.lines().enumerate() {
            let line = raw.trim();
            let err = |msg: &str| ScriptError { line: i + 1, msg: String::from(msg) };
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "---" {
                turns.push(core::mem::take(&mut cur));
                continue;
            }
            let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
            match cmd {
                "say" => cur.push(Line::Say(rest.replace("\\n", "\n"))),
                "tool" => {
                    let (name, input) = rest.split_once(' ').unwrap_or((rest, "{}"));
                    let v = json::parse(input).map_err(|_| err("vstup nastroje neni JSON"))?;
                    if !matches!(v, Value::Object(_)) {
                        return Err(err("vstup nastroje neni objekt"));
                    }
                    cur.push(Line::Tool(String::from(name), v));
                }
                "stop" => cur.push(Line::Stop(Stop::parse(rest))),
                "usage" => {
                    let (a, b) = rest.split_once(' ').ok_or_else(|| err("usage <vstup> <vystup>"))?;
                    let n = |s: &str| s.trim().parse::<u64>().map_err(|_| err("cislo"));
                    cur.push(Line::Usage(n(a)?, n(b)?));
                }
                _ => return Err(err("neznamy prikaz (say, tool, stop, usage)")),
            }
        }
        if !cur.is_empty() {
            turns.push(cur);
        }
        Ok(Self { turns, next: 0, calls: 0, last_request: None })
    }

    /// Zbyva jeste nejaka odpoved?
    pub fn remaining(&self) -> usize {
        self.turns.len() - self.next
    }
}

fn last_user(req: &Request) -> (String, String) {
    let Some(m) = req.messages.iter().rev().find(|m| m.role == Role::User) else {
        return (String::new(), String::new());
    };
    let mut results = Vec::new();
    let mut text = String::new();
    for b in &m.content {
        match b {
            Block::ToolResult { content, .. } => results.push(content.as_str()),
            Block::Text(t) => text.push_str(t),
            _ => {}
        }
    }
    (results.join("|"), text)
}

impl Provider for Fake {
    fn name(&self) -> &str {
        "fake"
    }

    fn model(&self) -> &str {
        "fake-1"
    }

    fn price(&self) -> Price {
        // Nenulova cena: testy limitu utraty pocitaji s realnymi cisly
        // (1 USD za milion tokenu vstupu i vystupu).
        Price { input: 1_000_000, output: 1_000_000, cache_write: 0, cache_read: 0 }
    }

    fn complete(&mut self, req: &Request, sink: &mut dyn FnMut(&Event)) -> Result<Response, ProviderError> {
        self.calls += 1;
        self.last_request = Some(req.clone());
        let Some(turn) = self.turns.get(self.next) else {
            return Err(ProviderError::ScriptEnd);
        };
        self.next += 1;
        let (results, user) = last_user(req);
        let mut content = Vec::new();
        let mut stop = None;
        let mut usage = Usage { input: (req.approx_bytes() as u64).div_ceil(4), output: 10, ..Default::default() };
        let mut tools = 0;
        for l in turn {
            match l {
                Line::Say(t) => content.push(Block::Text(t.replace("{result}", &results).replace("{user}", &user))),
                Line::Tool(name, input) => {
                    tools += 1;
                    content.push(Block::ToolUse {
                        id: alloc::format!("fake-{}-{tools}", self.calls),
                        name: name.clone(),
                        input: input.clone(),
                    });
                }
                Line::Stop(s) => stop = Some(s.clone()),
                Line::Usage(i, o) => usage = Usage { input: *i, output: *o, ..Default::default() },
            }
        }
        let stop = stop.unwrap_or(if tools > 0 { Stop::ToolUse } else { Stop::EndTurn });
        let message = Message { role: Role::Assistant, content };
        for ev in events_of(&message, &stop, &usage) {
            sink(&ev);
        }
        Ok(Response { message, stop, usage, bad_inputs: Vec::new(), sent_bytes: req.approx_bytes() as u64 })
    }
}
