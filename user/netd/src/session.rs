//! Operace na kanalu relace.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use libos::{Error, Result, println};
use netproto::{ReadAddr, Reader, Restriction, Writer, op, rights};

use crate::obj::{AuthRef, Deferred, Kind, Pending, Step, ask_progress};
use crate::stack::{Stack, deadline};
use crate::{ping, resolve, tcp, udp};

/// Nejvic zaznamu auditu v jedne odpovedi (zprava kanalu = 64 KiB).
const AUDIT_BATCH: usize = 256;

pub fn request(stack: &mut Stack, auth: &AuthRef, code: u32, r: &mut Reader) -> Result<Step> {
    match code {
        op::INFO => {
            auth.borrow().require(rights::INFO)?;
            Ok(info_reply(stack, auth))
        }
        op::WAIT_CONFIGURED => {
            let timeout = r.u64()?;
            auth.borrow().require(rights::INFO)?;
            if stack.configured {
                return Ok(info_reply(stack, auth));
            }
            Ok(Step::Wait(Pending::WaitConfigured {
                deadline: deadline(timeout),
            }))
        }
        op::TCP_CONNECT => tcp::connect(stack, auth, r),
        op::TCP_LISTEN => tcp::listen(stack, auth, r),
        op::UDP_BIND => udp::bind(stack, auth, r),
        op::RESOLVE => resolve::start(stack, auth, r),
        op::PING => ping::start(stack, auth, r),
        op::RESTRICT => {
            let restriction = Restriction::get(r)?;
            auth.borrow().require(rights::RESTRICT)?;
            let child = auth.borrow().restrict_with(restriction)?;
            println!(
                "netd: nova relace{}{} s pravy {}",
                if child.app().is_empty() { "" } else { " " },
                child.app(),
                netproto::rights_names(child.rights)
            );
            Ok(Step::Done(
                Ok(Writer::new()),
                Some(Kind::Session(Rc::new(RefCell::new(child)))),
            ))
        }
        op::SET_DNS => {
            let n = r.u8()? as usize;
            let mut servers = Vec::with_capacity(n);
            for _ in 0..n.min(8) {
                servers.push(r.socket_addr()?);
            }
            auth.borrow().require(rights::CONFIGURE)?;
            println!("netd: servery DNS nastaveny rucne: {:?}", servers);
            stack.set_dns(servers);
            Ok(Step::ok())
        }
        op::AUDIT => {
            let from = r.u64()?;
            let max = (r.u16()? as usize).min(AUDIT_BATCH);
            auth.borrow().require(rights::AUDIT)?;
            let mut w = Writer::new();
            stack.fw.audit_read(from, max, &mut w);
            Ok(Step::reply(Ok(w)))
        }
        op::STATS => {
            auth.borrow().require(rights::AUDIT)?;
            let mut w = Writer::new();
            stack.fw.stats(&mut w);
            Ok(Step::reply(Ok(w)))
        }
        _ => Err(Error::NotSupported),
    }
}

fn info_reply(stack: &Stack, auth: &AuthRef) -> Step {
    let mut w = Writer::new();
    stack.info(auth.borrow().rights).put(&mut w);
    Step::reply(Ok(w))
}

pub fn progress(stack: &mut Stack, auth: &AuthRef, p: Pending, now: u64) -> Step {
    match p {
        Pending::WaitConfigured { deadline } => {
            if stack.configured {
                info_reply(stack, auth)
            } else if now >= deadline {
                Step::err(Error::TimedOut)
            } else {
                Step::Wait(Pending::WaitConfigured { deadline })
            }
        }
        Pending::Connect(c) => tcp::connect_progress(stack, c, now),
        Pending::Resolve(res) => resolve::progress(stack, auth, res, now),
        Pending::Forward(f) => resolve::progress_forward(stack, auth, f, now),
        Pending::Ping(png) => ping::progress(stack, png, now),
        Pending::Ask(a) => match ask_progress(stack, a, now) {
            Ok((granted, then)) => {
                let r = match then {
                    Deferred::Connect(req) => tcp::connect_start(stack, auth, req, granted),
                    Deferred::Ping(req) => ping::start_checked(stack, auth, req, granted),
                    Deferred::Resolve(req) => resolve::start_server(stack, auth, req, granted),
                    Deferred::SendTo { .. } => Err(Error::BadState),
                };
                r.unwrap_or_else(Step::err)
            }
            Err(step) => step,
        },
        _ => Step::err(Error::BadState),
    }
}
