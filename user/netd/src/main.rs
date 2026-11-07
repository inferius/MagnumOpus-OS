//! netd: sluzba site (TCP/IP).
//!
//! Start (kind USER, v tomto poradi):
//! 1. kanal k ovladaci sitove karty (`netproto::link`),
//! 2. kanal korenove relace (vsechna prava) - drzi ho init a z nej
//!    vytvari uzsi relace pro programy (`op::RESTRICT`).
//!
//! Sluzba pocka na `INFO` od ovladace, odpovi rodici na bootstrap kanalu
//! `ok` (nebo `err:<popis>`) a spusti DHCP. Pak obsluhuje kanaly relaci
//! a socketu (protokol `netproto`, popis v `docs/cs/sit.md`).
//!
//! Stack: smoltcp (ARP, IPv4, ICMP, UDP, TCP, DHCP) za modulem `stack`;
//! resolver DNS je vlastni (`resolve`).
//!
//! Omezeni: sluzba ceka na vsechny kanaly jednim `OBJECT_WAIT_MANY`, ktery
//! bere nejvys 64 objektu. Relaci a socketu muze byt najednou nejvys
//! [`MAX_OBJECTS`]; dalsi dostanou `NO_RESOURCES`. TODO: objekt jadra pro
//! cekani na libovolne mnozstvi kanalu (port).

#![no_std]
#![no_main]

extern crate alloc;

mod link;
mod obj;
mod ping;
mod rand;
mod resolve;
mod session;
mod stack;
mod tcp;
mod udp;

use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::time::Duration;

use libos::abi::syscall::WAIT_MANY_MAX;
use libos::{Channel, Error, Handle, Signals, Startup, println, wait_many};
use netproto::Auth;

use crate::obj::{Kind, Obj, Step};
use crate::stack::{Stack, now_ns};

/// Nejvic relaci a socketu najednou (jedno misto v `wait_many` je kanal
/// ovladace).
pub const MAX_OBJECTS: usize = WAIT_MANY_MAX - 1;

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let bootstrap = startup.bootstrap;
    let mut handles = startup.handles.into_iter();
    let (Some(link_h), Some(root_h)) = (handles.next(), handles.next()) else {
        println!("netd: chybi handly ve startovni zprave");
        let _ = bootstrap.write(b"err:chybi handly", Vec::new());
        return 1;
    };
    let link_ch = Channel::from_handle(link_h);
    let info = match link_ch
        .read(Some(libos::time::deadline_after(Duration::from_secs(5))))
        .and_then(|m| netproto::link::Info::decode(&m.data))
    {
        Ok(i) => i,
        Err(e) => {
            println!("netd: ovladac sitove karty neposlal INFO: {e}");
            let _ = bootstrap.write(b"err:ovladac neodpovida", Vec::new());
            return 2;
        }
    };
    let mut stack = Stack::new(link::Link::new(link_ch, info));
    let _ = bootstrap.write(b"ok", Vec::new());
    println!("netd: bezim, cekam na DHCP");

    let mut objs: Vec<Option<Obj>> = vec![Some(Obj {
        ch: Channel::from_handle(root_h),
        kind: Kind::Session(Rc::new(RefCell::new(Auth::root()))),
        pending: None,
    })];
    run(&mut stack, &mut objs);
    println!(
        "netd: ovladac skoncil (zahozeno ramcu: prijem {}, odesilani {})",
        stack.dev.rx_dropped, stack.dev.tx_dropped
    );
    3
}

/// Hlavni smycka; vrati se, kdyz skonci ovladac.
fn run(stack: &mut Stack, objs: &mut Vec<Option<Obj>>) {
    loop {
        stack.poll();
        let mut created = Vec::new();
        let live = objs.iter().flatten().count();
        for o in objs.iter_mut().flatten() {
            if let Kind::Stream(s) = &mut o.kind {
                tcp::update_fin(stack, s);
            }
            if let Some(p) = o.pending.take() {
                let step = obj::progress(stack, &mut o.kind, p);
                finish(stack, o, step, &mut created, live);
            }
        }
        add_objects(objs, created);
        // Odeslat, co pozadavky zaradily, a uklidit zavrena spojeni.
        stack.poll();
        tcp::reap_closing(stack);

        let now = now_ns();
        let mut wake = stack
            .poll_delay_ns()
            .map_or(u64::MAX, |d| now.saturating_add(d));
        for o in objs.iter().flatten() {
            if let Some(p) = &o.pending {
                wake = wake.min(p.wake_at());
            }
        }
        if stack.dev.has_rx() {
            wake = now;
        }

        // Cekani: kanal ovladace a vsechny kanaly klientu. Kanal
        // s rozpracovanym pozadavkem se necte (jen se hlida jeho zavreni).
        let mut slots = Vec::new();
        let mut items: Vec<(&Handle, Signals)> = vec![(
            stack.dev.channel().handle(),
            Signals::READABLE | Signals::PEER_CLOSED,
        )];
        for (i, o) in objs.iter().enumerate() {
            if let Some(o) = o {
                let mask = if o.pending.is_some() {
                    Signals::PEER_CLOSED
                } else {
                    Signals::READABLE | Signals::PEER_CLOSED
                };
                items.push((o.ch.handle(), mask));
                slots.push(i);
            }
        }
        let deadline = if wake == u64::MAX { None } else { Some(wake) };
        let observed = match wait_many(&items, deadline) {
            Ok((_, s)) => Some(s),
            Err(_) => None,
        };
        drop(items);

        if let Err(Error::PeerClosed) = stack.dev.pump() {
            return;
        }
        let Some(observed) = observed else {
            continue;
        };
        let mut created = Vec::new();
        let live = objs.iter().flatten().count();
        for (k, &i) in slots.iter().enumerate() {
            let sig = observed[k + 1];
            let Some(o) = objs[i].as_mut() else {
                continue;
            };
            if sig.contains(Signals::PEER_CLOSED) {
                let o = objs[i].take().unwrap();
                obj::close(stack, o.kind, o.pending);
                continue;
            }
            if o.pending.is_some() || !sig.contains(Signals::READABLE) {
                continue;
            }
            match o.ch.try_read() {
                Ok(msg) => {
                    let step = obj::request(stack, &mut o.kind, &msg.data);
                    finish(stack, o, step, &mut created, live);
                }
                Err(Error::ShouldWait) => {}
                Err(_) => {
                    let o = objs[i].take().unwrap();
                    obj::close(stack, o.kind, o.pending);
                }
            }
        }
        add_objects(objs, created);
    }
}

/// Dokonci krok pozadavku: ulozi rozpracovany, nebo posle odpoved
/// (s kanalem noveho objektu).
fn finish(stack: &mut Stack, o: &mut Obj, step: Step, created: &mut Vec<Obj>, live: usize) {
    let (mut result, new) = match step {
        Step::Wait(p) => {
            o.pending = Some(p);
            return;
        }
        Step::Done(r, n) => (r, n),
    };
    let mut handles = Vec::new();
    if let Some(kind) = new {
        if result.is_err() {
            obj::close(stack, kind, None);
        } else if live + created.len() >= MAX_OBJECTS {
            obj::close(stack, kind, None);
            result = Err(Error::NoResources);
        } else {
            match Channel::create() {
                Ok((client, server)) => {
                    handles.push(client.into_handle());
                    created.push(Obj {
                        ch: server,
                        kind,
                        pending: None,
                    });
                }
                Err(e) => {
                    obj::close(stack, kind, None);
                    result = Err(e);
                }
            }
        }
    }
    // Klient, ktery mezitim kanal zavrel, se uklidi pri dalsim cekani.
    let _ = o.ch.write(&netproto::reply(result), handles);
}

fn add_objects(objs: &mut Vec<Option<Obj>>, created: Vec<Obj>) {
    for n in created {
        match objs.iter_mut().find(|o| o.is_none()) {
            Some(slot) => *slot = Some(n),
            None => objs.push(Some(n)),
        }
    }
}
