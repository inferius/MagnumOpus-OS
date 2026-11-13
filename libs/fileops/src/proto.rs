//! Protokol aplikace (spravce souboru) <-> `files-copy` (verze 1).
//!
//! Aplikace posle [`ClientMsg::Start`] a dostane [`ServerMsg::Started`] s
//! cislem ulohy; stav uloh prichazi jako [`ServerMsg::Status`] (po
//! `Subscribe` vsechny ulohy relace, pak pri zmene). Konflikt je stav
//! ulohy (`conflict`); odpoved [`ClientMsg::Decide`] smi poslat kterekoli
//! okno relace (uloha nepatri oknu, prezije jeho zavreni).
//!
//! Zprava = kod `u32` + pole (`libs/wire`). Neplatna zprava =
//! `INVALID_ARGS`, nikdy panika.

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::job::{Conflict, Job, Op, Phase, Resolution, get_res, put_res};
use crate::{Error, Result};

pub const VERSION: u32 = 1;
/// Nejvic zdroju v jedne uloze (a jedne zprave; zprava kanalu ma 64 KiB).
pub const MAX_SOURCES: usize = 512;

mod code {
    pub const HELLO: u32 = 1;
    pub const START: u32 = 2;
    pub const DECIDE: u32 = 3;
    pub const CANCEL: u32 = 4;
    pub const PAUSE: u32 = 5;
    pub const RESUME: u32 = 6;
    pub const SUBSCRIBE: u32 = 7;
    pub const STARTED: u32 = 101;
    pub const REJECTED: u32 = 102;
    pub const STATUS: u32 = 103;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Hello { version: u32 },
    Start { req: u32, op: Op, sources: Vec<String>, dest: String },
    Decide { job: u64, resolution: Resolution, all: bool },
    Cancel { job: u64 },
    Pause { job: u64 },
    Resume { job: u64 },
    Subscribe,
}

/// Stav ulohy pro rozhrani.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub job: u64,
    pub op: Op,
    pub phase: Phase,
    pub dest: String,
    pub total_items: u32,
    pub done_items: u32,
    pub skipped: u32,
    pub errors: u32,
    pub done_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percent: u8,
    pub conflict: Option<Conflict>,
    /// Posledni chyba polozky (zdroj, chyba).
    pub last_error: Option<(String, Error)>,
}

impl Status {
    pub fn of(j: &Job) -> Self {
        Self {
            job: j.id,
            op: j.op,
            phase: j.phase,
            dest: j.dest.clone(),
            total_items: j.total_items(),
            done_items: j.done_items,
            skipped: j.skipped,
            errors: j.error_count,
            done_bytes: j.done_bytes,
            total_bytes: j.total_bytes,
            percent: j.percent(),
            conflict: j.conflict().cloned(),
            last_error: j.errors.last().cloned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    Started { req: u32, job: u64 },
    Rejected { req: u32, error: Error },
    Status(Status),
}

fn put_op(w: &mut Writer, op: Op) {
    w.u8(match op {
        Op::Copy => 0,
        Op::Move => 1,
    });
}

fn get_op(r: &mut Reader) -> Result<Op> {
    match r.u8()? {
        0 => Ok(Op::Copy),
        1 => Ok(Op::Move),
        _ => Err(Error::InvalidArgs),
    }
}

fn put_phase(w: &mut Writer, p: Phase) {
    w.u8(match p {
        Phase::Running => 0,
        Phase::Paused => 1,
        Phase::Conflict => 2,
        Phase::Done => 3,
        Phase::Canceled => 4,
    });
}

fn get_phase(r: &mut Reader) -> Result<Phase> {
    Ok(match r.u8()? {
        0 => Phase::Running,
        1 => Phase::Paused,
        2 => Phase::Conflict,
        3 => Phase::Done,
        4 => Phase::Canceled,
        _ => return Err(Error::InvalidArgs),
    })
}

fn put_err(w: &mut Writer, e: Error) {
    w.u64(e.code() as i64 as u64);
}

fn get_err(r: &mut Reader) -> Result<Error> {
    Error::from_code(r.u64()? as i64).ok_or(Error::InvalidArgs)
}

fn done(r: &Reader) -> Result<()> {
    if r.is_empty() { Ok(()) } else { Err(Error::InvalidArgs) }
}

impl ClientMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            ClientMsg::Hello { version } => {
                w.u32(code::HELLO).u32(*version);
            }
            ClientMsg::Start { req, op, sources, dest } => {
                w.u32(code::START).u32(*req);
                put_op(&mut w, *op);
                w.str(dest).u32(sources.len() as u32);
                for s in sources {
                    w.str(s);
                }
            }
            ClientMsg::Decide { job, resolution, all } => {
                w.u32(code::DECIDE).u64(*job);
                put_res(&mut w, Some(*resolution));
                w.u8(*all as u8);
            }
            ClientMsg::Cancel { job } => {
                w.u32(code::CANCEL).u64(*job);
            }
            ClientMsg::Pause { job } => {
                w.u32(code::PAUSE).u64(*job);
            }
            ClientMsg::Resume { job } => {
                w.u32(code::RESUME).u64(*job);
            }
            ClientMsg::Subscribe => {
                w.u32(code::SUBSCRIBE);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let m = match r.u32()? {
            code::HELLO => ClientMsg::Hello { version: r.u32()? },
            code::START => {
                let req = r.u32()?;
                let op = get_op(&mut r)?;
                let dest = r.str()?;
                let n = r.u32()? as usize;
                if n == 0 || n > MAX_SOURCES {
                    return Err(Error::InvalidArgs);
                }
                let mut sources = Vec::with_capacity(n);
                for _ in 0..n {
                    sources.push(r.str()?);
                }
                ClientMsg::Start { req, op, sources, dest }
            }
            code::DECIDE => {
                let job = r.u64()?;
                let resolution = get_res(&mut r)?.ok_or(Error::InvalidArgs)?;
                ClientMsg::Decide { job, resolution, all: r.u8()? != 0 }
            }
            code::CANCEL => ClientMsg::Cancel { job: r.u64()? },
            code::PAUSE => ClientMsg::Pause { job: r.u64()? },
            code::RESUME => ClientMsg::Resume { job: r.u64()? },
            code::SUBSCRIBE => ClientMsg::Subscribe,
            _ => return Err(Error::InvalidArgs),
        };
        done(&r)?;
        Ok(m)
    }
}

impl ServerMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            ServerMsg::Started { req, job } => {
                w.u32(code::STARTED).u32(*req).u64(*job);
            }
            ServerMsg::Rejected { req, error } => {
                w.u32(code::REJECTED).u32(*req);
                put_err(&mut w, *error);
            }
            ServerMsg::Status(s) => {
                w.u32(code::STATUS).u64(s.job);
                put_op(&mut w, s.op);
                put_phase(&mut w, s.phase);
                w.str(&s.dest).u32(s.total_items).u32(s.done_items).u32(s.skipped).u32(s.errors).u64(s.done_bytes);
                w.u8(s.total_bytes.is_some() as u8).u64(s.total_bytes.unwrap_or(0)).u8(s.percent);
                match &s.conflict {
                    Some(c) => {
                        w.u8(1).str(&c.source).str(&c.target);
                    }
                    None => {
                        w.u8(0);
                    }
                }
                match &s.last_error {
                    Some((p, e)) => {
                        w.u8(1).str(p);
                        put_err(&mut w, *e);
                    }
                    None => {
                        w.u8(0);
                    }
                }
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let m = match r.u32()? {
            code::STARTED => ServerMsg::Started { req: r.u32()?, job: r.u64()? },
            code::REJECTED => ServerMsg::Rejected { req: r.u32()?, error: get_err(&mut r)? },
            code::STATUS => {
                let job = r.u64()?;
                let op = get_op(&mut r)?;
                let phase = get_phase(&mut r)?;
                let dest = r.str()?;
                let (total_items, done_items, skipped, errors, done_bytes) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u64()?);
                let has_total = r.u8()? != 0;
                let total = r.u64()?;
                let percent = r.u8()?;
                if percent > 100 {
                    return Err(Error::InvalidArgs);
                }
                let conflict = match r.u8()? {
                    0 => None,
                    1 => Some(Conflict { source: r.str()?, target: r.str()? }),
                    _ => return Err(Error::InvalidArgs),
                };
                let last_error = match r.u8()? {
                    0 => None,
                    1 => Some((r.str()?, get_err(&mut r)?)),
                    _ => return Err(Error::InvalidArgs),
                };
                ServerMsg::Status(Status {
                    job,
                    op,
                    phase,
                    dest,
                    total_items,
                    done_items,
                    skipped,
                    errors,
                    done_bytes,
                    total_bytes: has_total.then_some(total),
                    percent,
                    conflict,
                    last_error,
                })
            }
            _ => return Err(Error::InvalidArgs),
        };
        done(&r)?;
        Ok(m)
    }
}
