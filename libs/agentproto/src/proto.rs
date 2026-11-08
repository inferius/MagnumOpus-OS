//! Protokoly `agentd`: brana (init) -> agentd a klient <-> agentd.
//!
//! Zprava = kod `u8` + pole (`libs/wire`, little-endian). Cteni neplatne
//! zpravy vrati `INVALID_ARGS`, nikdy paniku. Limit zpravy kanalu je
//! 64 KiB; vypisy (audit, rozdil) `agentd` zkracuje.

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::action::{Action, Outcome};
use crate::msg::{Stop, Usage};
use crate::policy::{Mode, ReadScope};
use crate::price::{Limits, Spend};
use crate::{Error, Result};

pub const VERSION: u32 = 1;

/// Nejvic vzoru "neopusti pocitac" navic v relaci.
pub const MAX_NEVER_LEAVE: usize = 64;

/// Nastaveni nove relace (posila brana s handly relace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSpec {
    /// Uzivatel (identita relaci FS, kterou brana vydala).
    pub user: u64,
    pub mode: Mode,
    /// Pracovni prostor (dataset), prazdny = bez prostoru.
    pub workspace: String,
    pub read_scope: ReadScope,
    /// `claude`, `fake`, pozdeji `local`.
    pub provider: String,
    /// Model (prazdny = vychozi poskytovatele).
    pub model: String,
    pub limits: Limits,
    pub never_leave: Vec<String>,
    /// Skript falesneho modelu (jen poskytovatel `fake`).
    pub script: String,
    /// Klient umi potvrzeni (ma uzivatelske rozhrani).
    pub can_confirm: bool,
    /// Handly za kanalem klienta a FS cteni: FS prostoru, systemove nastroje.
    pub has_ws: bool,
    pub has_sys: bool,
}

impl OpenSpec {
    pub fn new(user: u64, mode: Mode, provider: &str) -> Self {
        Self {
            user,
            mode,
            workspace: String::new(),
            read_scope: ReadScope::User,
            provider: String::from(provider),
            model: String::new(),
            limits: Limits::default(),
            never_leave: Vec::new(),
            script: String::new(),
            can_confirm: true,
            has_ws: false,
            has_sys: false,
        }
    }

    pub fn put(&self, w: &mut Writer) {
        w.u64(self.user).u8(self.mode.code()).str(&self.workspace);
        w.u8(match self.read_scope {
            ReadScope::User => 0,
            ReadScope::Workspace => 1,
        });
        w.str(&self.provider).str(&self.model);
        self.limits.put(w);
        w.u32(self.never_leave.len() as u32);
        for p in &self.never_leave {
            w.str(p);
        }
        w.u32(self.script.len() as u32).bytes(self.script.as_bytes());
        w.u8(self.can_confirm as u8).u8(self.has_ws as u8).u8(self.has_sys as u8);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let user = r.u64()?;
        let mode = Mode::from_code(r.u8()?)?;
        let workspace = r.str()?;
        let read_scope = match r.u8()? {
            0 => ReadScope::User,
            1 => ReadScope::Workspace,
            _ => return Err(Error::InvalidArgs),
        };
        let provider = r.str()?;
        let model = r.str()?;
        let limits = Limits::get(r)?;
        let n = r.u32()? as usize;
        if n > MAX_NEVER_LEAVE {
            return Err(Error::InvalidArgs);
        }
        let never_leave = (0..n).map(|_| r.str()).collect::<Result<_>>()?;
        let len = r.u32()? as usize;
        let script = core::str::from_utf8(r.take(len)?).map_err(|_| Error::InvalidArgs)?.into();
        Ok(Self {
            user,
            mode,
            workspace,
            read_scope,
            provider,
            model,
            limits,
            never_leave,
            script,
            can_confirm: r.u8()? != 0,
            has_ws: r.u8()? != 0,
            has_sys: r.u8()? != 0,
        })
    }
}

/// Zpravy brany (init) na bootstrap kanalu `agentd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateMsg {
    /// Nova relace; handly: kanal klienta, FS cteni, [FS prostoru], [system].
    Open(OpenSpec),
    /// Poskytovatel nastroju (handle = jeho kanal, protokol `registry`).
    Provider { name: String },
}

impl GateMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            GateMsg::Open(s) => {
                w.u8(1);
                s.put(&mut w);
            }
            GateMsg::Provider { name } => {
                w.u8(2).str(name);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        Ok(match r.u8()? {
            1 => GateMsg::Open(OpenSpec::get(&mut r)?),
            2 => GateMsg::Provider { name: r.str()? },
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Zpravy klienta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Ask { text: String },
    Confirmed { id: u32, outcome: Outcome },
    Steps,
    StepDiff { step: u32 },
    /// Vratit prostor na krok `to` (0 = zacatek relace).
    Revert { to: u32 },
    Audit { from: u64, max: u32 },
    Spend,
    /// API klic (agentd ho zasifruje a ulozi; zpet ho klient nedostane).
    SetKey { provider: String, key: Vec<u8> },
}

impl ClientMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            ClientMsg::Ask { text } => {
                w.u8(1).u32(text.len() as u32).bytes(text.as_bytes());
            }
            ClientMsg::Confirmed { id, outcome } => {
                w.u8(2).u32(*id);
                outcome.put(&mut w);
            }
            ClientMsg::Steps => {
                w.u8(3);
            }
            ClientMsg::StepDiff { step } => {
                w.u8(4).u32(*step);
            }
            ClientMsg::Revert { to } => {
                w.u8(5).u32(*to);
            }
            ClientMsg::Audit { from, max } => {
                w.u8(6).u64(*from).u32(*max);
            }
            ClientMsg::Spend => {
                w.u8(7);
            }
            ClientMsg::SetKey { provider, key } => {
                w.u8(8).str(provider).u32(key.len() as u32).bytes(key);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        Ok(match r.u8()? {
            1 => {
                let n = r.u32()? as usize;
                ClientMsg::Ask { text: core::str::from_utf8(r.take(n)?).map_err(|_| Error::InvalidArgs)?.into() }
            }
            2 => ClientMsg::Confirmed { id: r.u32()?, outcome: Outcome::get(&mut r)? },
            3 => ClientMsg::Steps,
            4 => ClientMsg::StepDiff { step: r.u32()? },
            5 => ClientMsg::Revert { to: r.u32()? },
            6 => ClientMsg::Audit { from: r.u64()?, max: r.u32()? },
            7 => ClientMsg::Spend,
            8 => {
                let provider = r.str()?;
                let n = r.u32()? as usize;
                if n > 4096 {
                    return Err(Error::InvalidArgs);
                }
                ClientMsg::SetKey { provider, key: r.take(n)?.to_vec() }
            }
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Krok relace v prostoru (commit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepInfo {
    pub step: u32,
    /// Snimek commitu (pro rozdil a vraceni).
    pub snapshot: String,
    pub message: String,
    pub time: i64,
}

/// Zmena v prostoru (druhy jako `fsproto::hist::Change`: 1 pridano,
/// 2 smazano, 3 zmeneno, 4 presunuto, 5 presunuto a zmeneno).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsChange {
    pub kind: u8,
    pub path: String,
    pub old_path: String,
    pub is_dir: bool,
}

/// Zpravy agentd klientovi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMsg {
    Text { text: String },
    Tool { name: String, decision: String, summary: String },
    Confirm { id: u32, action: Action, preview: String },
    Proposal { command: String, style: String, other: String, risky: bool, explanation: String },
    TurnEnd { stop: Stop, usage: Usage, cost: u64, step: u32 },
    Error { code: abi::Error, message: String },
    Steps(Vec<StepInfo>),
    Diff(Vec<WsChange>),
    Reverted { files: u32, step: u32 },
    AuditPage { lines: Vec<String>, next: u64 },
    Spend { spend: Spend, limits: Limits },
    KeyStored,
}

fn put_long(w: &mut Writer, s: &str) {
    w.u32(s.len() as u32).bytes(s.as_bytes());
}

fn get_long(r: &mut Reader) -> Result<String> {
    let n = r.u32()? as usize;
    Ok(core::str::from_utf8(r.take(n)?).map_err(|_| Error::InvalidArgs)?.into())
}

impl AgentMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            AgentMsg::Text { text } => {
                w.u8(1);
                put_long(&mut w, text);
            }
            AgentMsg::Tool { name, decision, summary } => {
                w.u8(2).str(name).str(decision).str(summary);
            }
            AgentMsg::Confirm { id, action, preview } => {
                w.u8(3).u32(*id);
                action.put(&mut w);
                put_long(&mut w, preview);
            }
            AgentMsg::Proposal { command, style, other, risky, explanation } => {
                w.u8(4).str(command).str(style).str(other).u8(*risky as u8);
                put_long(&mut w, explanation);
            }
            AgentMsg::TurnEnd { stop, usage, cost, step } => {
                w.u8(5).str(stop.as_str());
                usage.put(&mut w);
                w.u64(*cost).u32(*step);
            }
            AgentMsg::Error { code, message } => {
                w.u8(6).i32(code.code()).str(message);
            }
            AgentMsg::Steps(steps) => {
                w.u8(7).u32(steps.len() as u32);
                for s in steps {
                    w.u32(s.step).str(&s.snapshot).str(&s.message).u64(s.time as u64);
                }
            }
            AgentMsg::Diff(changes) => {
                w.u8(8).u32(changes.len() as u32);
                for c in changes {
                    w.u8(c.kind).str(&c.path).str(&c.old_path).u8(c.is_dir as u8);
                }
            }
            AgentMsg::Reverted { files, step } => {
                w.u8(9).u32(*files).u32(*step);
            }
            AgentMsg::AuditPage { lines, next } => {
                w.u8(10).u64(*next).u32(lines.len() as u32);
                for l in lines {
                    w.str(l);
                }
            }
            AgentMsg::Spend { spend, limits } => {
                w.u8(11).u64(spend.session).u64(spend.day).u64(spend.day_index);
                limits.put(&mut w);
            }
            AgentMsg::KeyStored => {
                w.u8(12);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let max = |n: u32| if n > 8192 { Err(Error::InvalidArgs) } else { Ok(n as usize) };
        Ok(match r.u8()? {
            1 => AgentMsg::Text { text: get_long(&mut r)? },
            2 => AgentMsg::Tool { name: r.str()?, decision: r.str()?, summary: r.str()? },
            3 => AgentMsg::Confirm { id: r.u32()?, action: Action::get(&mut r)?, preview: get_long(&mut r)? },
            4 => AgentMsg::Proposal {
                command: r.str()?,
                style: r.str()?,
                other: r.str()?,
                risky: r.u8()? != 0,
                explanation: get_long(&mut r)?,
            },
            5 => AgentMsg::TurnEnd { stop: Stop::parse(&r.str()?), usage: Usage::get(&mut r)?, cost: r.u64()?, step: r.u32()? },
            6 => {
                let code = abi::Error::from_code(i64::from(r.i32()?)).unwrap_or(abi::Error::IoError);
                AgentMsg::Error { code, message: r.str()? }
            }
            7 => {
                let n = max(r.u32()?)?;
                let steps = (0..n)
                    .map(|_| Ok(StepInfo { step: r.u32()?, snapshot: r.str()?, message: r.str()?, time: r.u64()? as i64 }))
                    .collect::<Result<_>>()?;
                AgentMsg::Steps(steps)
            }
            8 => {
                let n = max(r.u32()?)?;
                let c = (0..n)
                    .map(|_| Ok(WsChange { kind: r.u8()?, path: r.str()?, old_path: r.str()?, is_dir: r.u8()? != 0 }))
                    .collect::<Result<_>>()?;
                AgentMsg::Diff(c)
            }
            9 => AgentMsg::Reverted { files: r.u32()?, step: r.u32()? },
            10 => {
                let next = r.u64()?;
                let n = max(r.u32()?)?;
                AgentMsg::AuditPage { next, lines: (0..n).map(|_| r.str()).collect::<Result<_>>()? }
            }
            11 => {
                let spend = Spend { session: r.u64()?, day: r.u64()?, day_index: r.u64()? };
                AgentMsg::Spend { spend, limits: Limits::get(&mut r)? }
            }
            12 => AgentMsg::KeyStored,
            _ => return Err(Error::InvalidArgs),
        })
    }

    /// Konci tato zprava odpoved na dotaz?
    pub fn ends_turn(&self) -> bool {
        matches!(self, AgentMsg::TurnEnd { .. } | AgentMsg::Error { .. })
    }
}
