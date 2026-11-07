//! Operace na kanalu relace.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use libos::{Error, Result, println};
use netproto::{Policy, Reader, Writer, op, rights};

use crate::obj::{AuthRef, Kind, Pending, Step};
use crate::stack::{Stack, deadline};
use crate::{ping, resolve, tcp, udp};

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
            let mask = r.u32()?;
            let policy = Policy::get(r)?;
            auth.borrow().require(rights::RESTRICT)?;
            let child = auth.borrow().restrict(mask, policy)?;
            println!(
                "netd: nova relace s pravy {}",
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
        Pending::Ping(png) => ping::progress(stack, png, now),
        _ => Step::err(Error::BadState),
    }
}
