//! Relace agenta: konverzace, smycka model -> nastroje -> model, pravidla,
//! potvrzeni, pracovni prostor, limity a audit.
//!
//! Relace bezi ve vlastnim vlakne. Kanal klienta je jediny vstup od
//! uzivatele; vysledky nastroju jdou modelu jako data (`tool_result`).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use agentproto::action::{Action, Outcome};
use agentproto::audit::{Kind, Record, summarize_input};
use agentproto::claude::{Claude, Options};
use agentproto::fake::Fake;
use agentproto::json::Value;
use agentproto::msg::{Block, Message, Role, Stop, Usage};
use agentproto::policy::{Decision, Policy};
use agentproto::price::{Limits, Spend, usd};
use agentproto::proto::{AgentMsg, ClientMsg, OpenSpec};
use agentproto::provider::{Provider, Request};
use agentproto::secret::Secret;
use agentproto::stream::Event;
use agentproto::tool::{ToolSpec, validate};
use fsproto::FsClient;
use libos::{Channel, Error, Handle, println};

use crate::ext::{self, Ext};
use crate::net::{Https, NetCtx};
use crate::state::{self, State};
use crate::tools;
use crate::workspace::Workspace;

/// Stav sdileny relacemi.
pub struct Shared {
    pub state: spin::Mutex<State>,
    pub providers: spin::Mutex<Vec<Arc<spin::Mutex<Ext>>>>,
    pub net: Option<Arc<spin::Mutex<NetCtx>>>,
}

/// Odkud nastroj je.
#[derive(Clone)]
enum Source {
    Builtin,
    /// Systemove nastroje relace (kanal od brany).
    Sys,
    /// Globalni poskytovatel (index v `Shared::providers`).
    Ext(Arc<spin::Mutex<Ext>>),
}

struct Offered {
    spec: ToolSpec,
    source: Source,
}

pub struct Session {
    id: u64,
    spec: OpenSpec,
    policy: Policy,
    limits: Limits,
    client: Channel,
    read: FsClient,
    ws: Option<Workspace>,
    sys: Option<Ext>,
    provider: Box<dyn Provider>,
    tools: Vec<Offered>,
    system: String,
    messages: Vec<Message>,
    step: u32,
    spend: Spend,
    next_confirm: u32,
    shared: Arc<Shared>,
    open: bool,
}

/// Vysledek jednoho volani nastroje.
struct Called {
    content: String,
    is_error: bool,
    wrote_ws: bool,
}

fn err_text(e: Error) -> String {
    format!("chyba {}: {}", e.name(), e.description())
}

fn make_provider(spec: &OpenSpec, shared: &Shared) -> Result<Box<dyn Provider>, String> {
    match spec.provider.as_str() {
        "fake" => Fake::parse(&spec.script).map(|f| Box::new(f) as Box<dyn Provider>).map_err(|e| e.to_string()),
        "claude" => {
            let key = shared.state.lock().load_key(spec.user, "claude").unwrap_or_else(|_| Secret::empty());
            let mut opts = Options::default();
            if !spec.model.is_empty() {
                opts.model = spec.model.clone();
            }
            Ok(Box::new(Claude::new(Https { ctx: shared.net.clone() }, key, opts)))
        }
        other => Err(format!("neznamy poskytovatel {other}")),
    }
}

/// Vlakno relace: otevre relaci z handlu brany a obsluhuje klienta.
pub fn run(shared: Arc<Shared>, id: u64, spec: OpenSpec, handles: Vec<Handle>) {
    let mut hs = handles.into_iter();
    let (Some(client), Some(read)) = (hs.next().map(Channel::from_handle), hs.next()) else {
        println!("agentd: relace {id} bez kanalu klienta nebo FS");
        return;
    };
    let fail = |client: &Channel, code: Error, msg: String| {
        let _ = client.write(&AgentMsg::Error { code, message: msg }.encode(), Vec::new());
    };
    let read = FsClient::new(Channel::from_handle(read));
    let ws_fs = if spec.has_ws { hs.next().map(|h| FsClient::new(Channel::from_handle(h))) } else { None };
    let sys_ch = if spec.has_sys { hs.next().map(Channel::from_handle) } else { None };
    let ws = match ws_fs {
        Some(fs) => match Workspace::start(fs, &spec.workspace, id) {
            Ok(w) => Some(w),
            Err(e) => {
                let why = if e == Error::NotSupported { "pracovni prostor musi byt dataset".into() } else { err_text(e) };
                fail(&client, e, format!("pracovni prostor {}: {why}", spec.workspace));
                return;
            }
        },
        None => None,
    };
    let sys = sys_ch.and_then(|c| match Ext::register("sys", c) {
        Ok(x) => Some(x),
        Err(e) => {
            println!("agentd: systemove nastroje relace {id}: {e}");
            None
        }
    });
    let provider = match make_provider(&spec, &shared) {
        Ok(p) => p,
        Err(m) => {
            fail(&client, Error::InvalidArgs, m);
            return;
        }
    };
    let policy = Policy {
        mode: spec.mode,
        workspace: ws.as_ref().map(|w| w.path.clone()),
        read_scope: spec.read_scope,
        never_leave: spec.never_leave.clone(),
        can_confirm: spec.can_confirm,
    };
    let system = agentsh::system_prompt(spec.mode.name(), policy.workspace.as_deref());
    let spend = shared.state.lock().day_spend(spec.user);
    let mut s = Session {
        id,
        limits: spec.limits.or_default(),
        policy,
        client,
        read,
        ws,
        sys,
        provider,
        tools: Vec::new(),
        system,
        messages: Vec::new(),
        step: 0,
        spend,
        next_confirm: 1,
        shared,
        open: true,
        spec,
    };
    s.collect_tools();
    s.audit(Kind::Session, "open", &format!("rezim={} prostor={} poskytovatel={}", s.spec.mode.name(), s.spec.workspace, s.provider.name()), "-", "ok");
    s.serve();
    s.audit(Kind::Session, "close", "", "-", "ok");
}

impl Session {
    fn send(&self, m: &AgentMsg) {
        let _ = self.client.write(&m.encode(), Vec::new());
    }

    fn audit(&self, kind: Kind, what: &str, input: &str, decision: &str, result: &str) {
        let r = Record {
            seq: 0,
            time: state::now().1,
            session: self.id,
            step: self.step,
            kind,
            what: String::from(what),
            input: String::from(input),
            decision: String::from(decision),
            result: String::from(result),
        };
        if let Err(e) = self.shared.state.lock().audit(self.spec.user, r) {
            // Audit nesmi zablokovat uzivatele, ale musi byt videt.
            println!("agentd: zapis auditu selhal: {e}");
        }
    }

    /// Nabidka nastroju podle rezimu: vestavene, systemove relace, globalni.
    fn collect_tools(&mut self) {
        let mut out: Vec<Offered> = Vec::new();
        let globals: Vec<Arc<spin::Mutex<Ext>>> = self.shared.providers.lock().clone();
        let push = |out: &mut Vec<Offered>, spec: ToolSpec, source: Source| {
            if self.policy.mode.offers(&spec) && !out.iter().any(|o| o.spec.name == spec.name) {
                out.push(Offered { spec, source });
            }
        };
        // Sluzby maji prednost pred pripravenymi nastroji (dev.build od prekladace).
        for g in &globals {
            let tools = g.lock().tools.clone();
            for t in tools {
                push(&mut out, t, Source::Ext(g.clone()));
            }
        }
        if let Some(sys) = &self.sys {
            for t in sys.tools.clone() {
                push(&mut out, t, Source::Sys);
            }
        }
        for t in tools::builtin() {
            if t.name.starts_with("ws.") && self.ws.is_none() {
                continue;
            }
            push(&mut out, t, Source::Builtin);
        }
        self.tools = out;
    }

    fn serve(&mut self) {
        while self.open {
            let m = match self.client.read(None) {
                Ok(m) => m,
                Err(_) => return,
            };
            let msg = match ClientMsg::decode(&m.data) {
                Ok(c) => c,
                Err(e) => {
                    self.send(&AgentMsg::Error { code: e, message: String::from("neplatna zprava") });
                    continue;
                }
            };
            match msg {
                ClientMsg::Ask { text } => self.ask(&text),
                ClientMsg::Confirmed { .. } => self.send(&AgentMsg::Error { code: Error::BadState, message: String::from("zadne potvrzeni neceka") }),
                ClientMsg::Steps => match &self.ws {
                    Some(w) => self.send(&AgentMsg::Steps(w.list())),
                    None => self.no_ws(),
                },
                ClientMsg::StepDiff { step } => match self.ws.as_ref().map(|w| w.diff(step)) {
                    Some(Ok(d)) => self.send(&AgentMsg::Diff(d.into_iter().take(1000).collect())),
                    Some(Err(e)) => self.send(&AgentMsg::Error { code: e, message: err_text(e) }),
                    None => self.no_ws(),
                },
                ClientMsg::Revert { to } => self.revert(to),
                ClientMsg::Audit { from, max } => {
                    let page = self.shared.state.lock().audit_page(self.spec.user, from, max.min(500));
                    match page {
                        Ok((lines, next)) => self.send(&AgentMsg::AuditPage { lines, next }),
                        Err(e) => self.send(&AgentMsg::Error { code: e, message: err_text(e) }),
                    }
                }
                ClientMsg::Spend => {
                    let day = self.shared.state.lock().day_spend(self.spec.user);
                    self.send(&AgentMsg::Spend { spend: Spend { session: self.spend.session, ..day }, limits: self.limits });
                }
                ClientMsg::SetKey { provider, key } => {
                    let key = Secret::new(key);
                    let r = self.shared.state.lock().store_key(self.spec.user, &provider, key.expose());
                    drop(key);
                    match r {
                        Ok(()) => {
                            // Klic nikdy do auditu, jen ze byl ulozen.
                            self.audit(Kind::Session, "set-key", &provider, "-", "ok");
                            if provider == self.spec.provider
                                && let Ok(p) = make_provider(&self.spec, &self.shared)
                            {
                                self.provider = p;
                            }
                            self.send(&AgentMsg::KeyStored);
                        }
                        Err(e) => self.send(&AgentMsg::Error { code: e, message: err_text(e) }),
                    }
                }
            }
        }
    }

    fn no_ws(&self) {
        self.send(&AgentMsg::Error { code: Error::NotFound, message: String::from("relace nema pracovni prostor") });
    }

    fn revert(&mut self, to: u32) {
        let Some(w) = self.ws.as_mut() else { return self.no_ws() };
        let r = w.revert(to);
        match r {
            Ok(files) => {
                self.audit(Kind::Session, "revert", &format!("na krok {to}"), "-", &format!("ok {files} polozek"));
                // Model se dozvi, ze se prostor zmenil mimo nej.
                self.push_user_text(&format!("[system: uzivatel vratil pracovni prostor na krok {to}]"));
                self.send(&AgentMsg::Reverted { files, step: to });
            }
            Err(e) => {
                self.audit(Kind::Session, "revert", &format!("na krok {to}"), "-", e.name());
                self.send(&AgentMsg::Error { code: e, message: err_text(e) });
            }
        }
    }

    /// Text uzivatele (k predchozi zprave uzivatele, kdyz konverzace na ni konci).
    fn push_user_text(&mut self, text: &str) {
        match self.messages.last_mut() {
            Some(m) if m.role == Role::User => m.content.push(Block::Text(String::from(text))),
            _ => self.messages.push(Message::user_text(text)),
        }
    }

    fn ask(&mut self, text: &str) {
        self.push_user_text(text);
        let mut calls = 0u32;
        let mut turn = Usage::default();
        let mut turn_cost = 0u64;
        loop {
            calls += 1;
            let req = Request {
                system: self.system.clone(),
                messages: self.messages.clone(),
                tools: self.tools.iter().map(|o| o.spec.clone()).collect(),
                max_tokens: self.limits.max_tokens,
            };
            let price = self.provider.price();
            let estimate = price.estimate(req.approx_bytes(), self.limits.max_tokens);
            let (day, _) = state::now();
            self.spend.day = self.shared.state.lock().day_spend(self.spec.user).day;
            self.spend.day_index = day;
            if let Err(over) = self.spend.check(&self.limits, estimate, calls) {
                let msg = format!("{}: odhad {} (relace {}, den {})", over.describe(), usd(estimate), usd(self.spend.session), usd(self.spend.day));
                self.audit(Kind::Model, &self.model_name(), &format!("{} B", req.approx_bytes()), "deny", over.describe());
                let code = if over == agentproto::price::Over::Calls { Error::OutOfRange } else { Error::NoResources };
                self.send(&AgentMsg::Error { code, message: msg });
                return;
            }
            let client = &self.client;
            let mut sink = |ev: &Event| {
                if let Event::TextDelta { text, .. } = ev
                    && !text.is_empty()
                {
                    let _ = client.write(&AgentMsg::Text { text: text.clone() }.encode(), Vec::new());
                }
            };
            let resp = match self.provider.complete(&req, &mut sink) {
                Ok(r) => r,
                Err(e) => {
                    self.audit(Kind::Model, &self.model_name(), &format!("{} B", req.approx_bytes()), "-", &format!("chyba {}", e.code().name()));
                    self.send(&AgentMsg::Error { code: e.code(), message: format!("{e}") });
                    return;
                }
            };
            let cost = price.cost(&resp.usage);
            self.spend.session = self.spend.session.saturating_add(cost);
            if let Err(e) = self.shared.state.lock().add_spend(self.spec.user, cost) {
                println!("agentd: zapis utraty selhal: {e}");
            }
            turn.add(&resp.usage);
            turn_cost += cost;
            self.audit(
                Kind::Model,
                &self.model_name(),
                &format!("{} zprav, {} nastroju", req.messages.len(), req.tools.len()),
                "-",
                &format!(
                    "stop={} vstup={} vystup={} cena={} odeslano={} B",
                    resp.stop.as_str(),
                    resp.usage.input,
                    resp.usage.output,
                    usd(cost),
                    resp.sent_bytes
                ),
            );
            let uses: Vec<(String, String, Value)> =
                resp.message.tool_uses().map(|(i, n, v)| (String::from(i), String::from(n), v.clone())).collect();
            self.messages.push(resp.message);
            if resp.stop == Stop::Refusal || uses.is_empty() {
                self.send(&AgentMsg::TurnEnd { stop: resp.stop, usage: turn, cost: turn_cost, step: self.step });
                return;
            }
            self.step += 1;
            let mut results = Vec::new();
            let mut wrote = Vec::new();
            for (id, name, input) in uses {
                let c = match resp.bad_inputs.iter().find(|(b, _)| *b == id) {
                    Some((_, why)) => Called { content: format!("neplatny vstup nastroje: {why}"), is_error: true, wrote_ws: false },
                    None => self.call_tool(&name, &input),
                };
                if c.wrote_ws {
                    wrote.push(name.clone());
                }
                results.push(Block::ToolResult { id, content: c.content, is_error: c.is_error });
            }
            // Vsechny vysledky kroku v jedne zprave uzivatele.
            self.messages.push(Message { role: Role::User, content: results });
            if !wrote.is_empty()
                && let Some(w) = self.ws.as_mut()
            {
                wrote.dedup();
                if let Err(e) = w.commit(self.step, &wrote.join(", ")) {
                    self.audit(Kind::Session, "commit", &format!("krok {}", self.step), "-", e.name());
                }
            }
            if !self.open {
                return;
            }
        }
    }

    fn model_name(&self) -> String {
        format!("{}/{}", self.provider.name(), self.provider.model())
    }

    /// Jedno volani nastroje: schema, pravidla, provedeni, audit.
    fn call_tool(&mut self, name: &str, input: &Value) -> Called {
        let summary = summarize_input(input);
        let Some(o) = self.tools.iter().find(|o| o.spec.name == name) else {
            self.audit(Kind::Tool, name, &summary, "deny", "neznamy nastroj");
            self.send(&AgentMsg::Tool { name: String::from(name), decision: String::from("deny"), summary });
            return Called { content: format!("nastroj {name} neexistuje"), is_error: true, wrote_ws: false };
        };
        let (spec, source) = (o.spec.clone(), o.source.clone());
        if let Err(why) = validate(&spec.schema, input) {
            self.audit(Kind::Tool, name, &summary, "deny", "neplatny vstup");
            return Called { content: format!("neplatny vstup: {why}"), is_error: true, wrote_ws: false };
        }
        // Pripravene nastroje bez implementace (preklad a testy bez prekladace).
        if matches!(source, Source::Builtin) && name.starts_with("dev.") {
            self.audit(Kind::Tool, name, &summary, "-", "NOT_SUPPORTED");
            return Called { content: String::from("preklad a testy zatim v systemu nejsou (NOT_SUPPORTED)"), is_error: true, wrote_ws: false };
        }
        let decision = self.policy.decide(&spec, input);
        self.send(&AgentMsg::Tool { name: String::from(name), decision: String::from(decision.name()), summary: summary.clone() });
        let (c, label) = match decision {
            Decision::Deny(why) => (Called { content: why, is_error: true, wrote_ws: false }, "deny"),
            Decision::Allow => (self.exec_read(name, input, &source), "allow"),
            Decision::Workspace => (self.exec_ws(name, input, &source), "allow-ws"),
            Decision::Confirm => self.confirm(name, input, &source),
        };
        let result = if c.is_error { c.content.chars().take(120).collect::<String>() } else { String::from("ok") };
        self.audit(Kind::Tool, name, &summary, label, &result);
        c
    }

    fn ok(content: String) -> Called {
        Called { content, is_error: false, wrote_ws: false }
    }

    fn error(e: Error) -> Called {
        Called { content: err_text(e), is_error: true, wrote_ws: false }
    }

    /// Nastroj sluzby (systemove relace nebo globalni) s capability relace.
    fn call_ext(&mut self, name: &str, input: &Value, source: &Source) -> Called {
        let json = input.to_json();
        let r = match source {
            Source::Sys => match self.sys.as_mut() {
                Some(s) => s.call(name, &json, None),
                None => Err(Error::NotFound),
            },
            Source::Ext(p) => {
                // Capability relace: nove spojeni FS se stejnou identitou.
                let cap = self.read.connect().ok().map(|c| c.into_channel().into_handle());
                p.lock().call(name, &json, cap)
            }
            Source::Builtin => Err(Error::NotFound),
        };
        match r {
            Ok((content, is_error)) => Called { content, is_error, wrote_ws: false },
            Err(e) => Called { content: ext::failed(name, e), is_error: true, wrote_ws: false },
        }
    }

    fn exec_read(&mut self, name: &str, input: &Value, source: &Source) -> Called {
        if !matches!(source, Source::Builtin) {
            return self.call_ext(name, input, source);
        }
        match name {
            "ws.steps" => match &self.ws {
                Some(w) => Self::ok(w.list().iter().map(|s| format!("krok {}: {}", s.step, s.message)).collect::<Vec<_>>().join("\n")),
                None => Self::error(Error::NotFound),
            },
            "ws.diff" => match self.ws.as_ref().map(|w| w.diff(input.u64_of("step") as u32)) {
                Some(Ok(d)) => Self::ok(if d.is_empty() {
                    String::from("beze zmen")
                } else {
                    d.iter().map(|c| format!("{} {}", ["?", "pridano", "smazano", "zmeneno", "presunuto", "presunuto+zmeneno"][c.kind.min(5) as usize], c.path)).collect::<Vec<_>>().join("\n")
                }),
                Some(Err(e)) => Self::error(e),
                None => Self::error(Error::NotFound),
            },
            "shell.propose" => self.propose(input),
            _ => match tools::read(&self.read, name, input) {
                // Vypis a hledani: cesty, ktere neopusti pocitac, vynechat.
                Ok(text) if name == "fs.search" => {
                    Self::ok(text.lines().filter(|l| !self.policy.never_leaves(l)).collect::<Vec<_>>().join("\n"))
                }
                Ok(text) => Self::ok(text),
                Err(e) => Self::error(e),
            },
        }
    }

    fn exec_ws(&mut self, name: &str, input: &Value, source: &Source) -> Called {
        if !matches!(source, Source::Builtin) {
            return self.call_ext(name, input, source);
        }
        let Some(w) = self.ws.as_ref() else { return Self::error(Error::NotFound) };
        match tools::write(&w.fs, name, input) {
            Ok(t) => Called { content: t, is_error: false, wrote_ws: true },
            Err(e) => Self::error(e),
        }
    }

    /// Potvrzeni u klienta. Vestavenou akci provede klient svymi pravy;
    /// u nastroje sluzby potvrzeni jen povoli volani.
    fn confirm(&mut self, name: &str, input: &Value, source: &Source) -> (Called, &'static str) {
        let (action, preview) = match source {
            Source::Builtin => match tools::action(&self.read, name, input) {
                Ok(a) => a,
                Err(e) => return (Self::error(e), "deny"),
            },
            _ => (Action::Tool { name: String::from(name), input: input.to_json() }, format!("nastroj {name}: {}", summarize_input(input))),
        };
        let id = self.next_confirm;
        self.next_confirm += 1;
        self.send(&AgentMsg::Confirm { id, action, preview });
        let outcome = loop {
            let m = match self.client.read(None) {
                Ok(m) => m,
                Err(_) => {
                    self.open = false;
                    break Outcome::Declined;
                }
            };
            match ClientMsg::decode(&m.data) {
                Ok(ClientMsg::Confirmed { id: rid, outcome }) if rid == id => break outcome,
                _ => self.send(&AgentMsg::Error { code: Error::BadState, message: String::from("ceka se na potvrzeni") }),
            }
        };
        match outcome {
            Outcome::Done(text) => match source {
                Source::Builtin => (Self::ok(format!("uzivatel potvrdil: {text}")), "confirm-yes"),
                _ => (self.call_ext(name, input, source), "confirm-yes"),
            },
            Outcome::Declined => (Called { content: String::from("uzivatel akci odmitl"), is_error: true, wrote_ws: false }, "confirm-no"),
            Outcome::Failed(e, t) => (Called { content: format!("akce selhala: {} {t}", e.name()), is_error: true, wrote_ws: false }, "confirm-yes"),
        }
    }

    /// Navrh prikazu shellu: overi parser shlang (vsechny kroky).
    fn propose(&mut self, input: &Value) -> Called {
        let style = match input.str_of("style") {
            "C" | "c" => Some(shlang_style(true)),
            "B" | "b" => Some(shlang_style(false)),
            _ => None,
        };
        let mut steps: Vec<&str> = alloc::vec![input.str_of("command")];
        if let Some(more) = input.get("steps").and_then(Value::as_array) {
            steps.extend(more.iter().filter_map(Value::as_str));
        }
        match agentsh::plan(&steps, style) {
            Err(e) => Called { content: format!("navrh neprosel parserem, oprav ho:\n{e}"), is_error: true, wrote_ws: false },
            Ok(props) => {
                let explanation = input.str_of("explanation");
                for p in &props {
                    let mut ex = String::from(explanation);
                    if p.risky {
                        ex.push_str(&format!("\nPozor: {}", p.reasons.join("; ")));
                    }
                    self.send(&AgentMsg::Proposal {
                        command: p.command.clone(),
                        style: String::from(agentsh::style_name(p.style)),
                        other: p.other.clone(),
                        risky: p.risky,
                        explanation: ex,
                    });
                }
                Self::ok(format!("navrh overen ({} kroku); uzivatel ho uvidi a spusti sam", props.len()))
            }
        }
    }
}

fn shlang_style(c: bool) -> agentsh::Style {
    if c { agentsh::Style::C } else { agentsh::Style::B }
}
