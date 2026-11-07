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
//! Cekani: jeden port jadra (`PORT_WAIT`). Kazdy kanal ma na portu dve
//! jednorazove registrace: "ke cteni" (obnovi se po precteni zpravy, ale
//! ne behem rozpracovaneho pozadavku) a "zavreno" (jen jednou). Pocet
//! socketu tak neomezuje cekani, jen [`MAX_OBJECTS`].

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

use libos::{Channel, Error, Port, Signals, Startup, println};
use netproto::Auth;

use crate::obj::{Kind, Obj, Step};
use crate::stack::{Stack, now_ns};

/// Nejvic relaci a socketu najednou (ochrana pameti sluzby; kazdy objekt
/// ma na portu dve registrace).
pub const MAX_OBJECTS: usize = 16_384;

/// Klic paketu portu: bit 63 = registrace "zavreno", bity 32..63 =
/// generace slotu (stary paket po znovupouziti slotu se pozna), dolnich
/// 32 bitu = slot + 1 (0 = kanal ovladace).
const KEY_CLOSE: u64 = 1 << 63;

fn key(slot: Option<usize>, generation: u32, close: bool) -> u64 {
    let s = slot.map_or(0, |i| i as u64 + 1);
    (close as u64) << 63 | ((generation as u64) & 0x7FFF_FFFF) << 32 | s
}

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
    let port = match Port::create() {
        Ok(p) => p,
        Err(e) => {
            println!("netd: port nelze vytvorit: {e}");
            let _ = bootstrap.write(b"err:port", Vec::new());
            return 2;
        }
    };
    let mut stack = Stack::new(link::Link::new(link_ch, info));
    let _ = bootstrap.write(b"ok", Vec::new());
    println!("netd: bezim, cekam na DHCP");

    let mut svc = Service::new(
        port,
        Obj::new(
            Channel::from_handle(root_h),
            Kind::Session(Rc::new(RefCell::new(Auth::root()))),
        ),
    );
    svc.run(&mut stack);
    println!(
        "netd: ovladac skoncil (zahozeno ramcu: prijem {}, odesilani {})",
        stack.dev.rx_dropped, stack.dev.tx_dropped
    );
    3
}

struct Service {
    port: Port,
    objs: Vec<Option<Obj>>,
    /// Generace slotu (zvysi se pri kazdem uvolneni).
    gens: Vec<u32>,
    /// Volne sloty k znovupouziti.
    free_slots: Vec<usize>,
    /// Pocet zivych objektu.
    live: usize,
    /// Sloty s rozpracovanym pozadavkem (posouvaji se v kazdem kole).
    pending: Vec<usize>,
    /// Sloty, ktere potrebuji obnovit registraci na portu: (slot, generace).
    to_arm: Vec<(usize, u32)>,
    link_armed: bool,
}

impl Service {
    fn new(port: Port, root: Obj) -> Self {
        let mut svc = Self {
            port,
            objs: Vec::new(),
            gens: Vec::new(),
            free_slots: Vec::new(),
            live: 0,
            pending: Vec::new(),
            to_arm: Vec::new(),
            link_armed: false,
        };
        svc.add_objects(vec![root]);
        svc
    }

    /// Hlavni smycka; vrati se, kdyz skonci ovladac. Prace v kazdem kole je
    /// umerna aktivnim objektum (rozpracovane pozadavky, prijate pakety),
    /// ne poctu vsech socketu.
    fn run(&mut self, stack: &mut Stack) {
        // Konec ovladace hlidat jednou registraci.
        if self
            .port
            .wait_async(
                stack.dev.channel().handle(),
                key(None, 0, true),
                Signals::PEER_CLOSED,
            )
            .is_err()
        {
            return;
        }
        loop {
            stack.poll();
            self.progress_pending(stack);
            // Odeslat, co pozadavky zaradily, a uklidit zavrena spojeni.
            stack.poll();
            tcp::reap_closing(stack);
            if self.arm(stack).is_err() {
                println!("netd: registrace na portu selhala, koncim");
                return;
            }

            let now = now_ns();
            let mut wake = stack
                .poll_delay_ns()
                .map_or(u64::MAX, |d| now.saturating_add(d));
            for &i in &self.pending {
                if let Some(Some(o)) = self.objs.get(i)
                    && let Some(p) = &o.pending
                {
                    wake = wake.min(p.wake_at());
                }
            }
            if stack.dev.has_rx() {
                wake = now;
            }
            // Prvni paket s terminem, dalsi uz jen bez cekani.
            let mut deadline = if wake == u64::MAX { None } else { Some(wake) };
            while let Ok(packet) = self.port.wait(deadline) {
                if !self.handle_packet(stack, packet.key, Signals(packet.signals)) {
                    return;
                }
                deadline = Some(0);
            }
        }
    }

    /// Posune rozpracovane pozadavky; dokoncene objekty pujdou znovu
    /// zaregistrovat "ke cteni".
    fn progress_pending(&mut self, stack: &mut Stack) {
        let mut created = Vec::new();
        let list = core::mem::take(&mut self.pending);
        for i in list {
            let Some(Some(o)) = self.objs.get_mut(i) else {
                continue;
            };
            let Some(p) = o.pending.take() else {
                continue;
            };
            let step = obj::progress(stack, &mut o.kind, p);
            finish(stack, o, step, &mut created, self.live);
            if o.pending.is_some() {
                self.pending.push(i);
            } else {
                self.to_arm.push((i, self.gens[i]));
            }
        }
        self.add_objects(created);
    }

    /// Obnovi registrace: kanal ovladace, "zavreno" u novych objektu
    /// a "ke cteni" u objektu bez rozpracovaneho pozadavku.
    fn arm(&mut self, stack: &Stack) -> libos::Result<()> {
        if !self.link_armed {
            self.port.wait_async(
                stack.dev.channel().handle(),
                key(None, 0, false),
                Signals::READABLE,
            )?;
            self.link_armed = true;
        }
        for (i, g) in core::mem::take(&mut self.to_arm) {
            if self.gens.get(i) != Some(&g) {
                continue;
            }
            let Some(Some(o)) = self.objs.get_mut(i) else {
                continue;
            };
            if !o.close_armed {
                self.port
                    .wait_async(o.ch.handle(), key(Some(i), g, true), Signals::PEER_CLOSED)?;
                o.close_armed = true;
            }
            if !o.read_armed && o.pending.is_none() {
                self.port
                    .wait_async(o.ch.handle(), key(Some(i), g, false), Signals::READABLE)?;
                o.read_armed = true;
            }
        }
        Ok(())
    }

    /// Zpracuje paket portu. `false` = ovladac skoncil.
    fn handle_packet(&mut self, stack: &mut Stack, k: u64, signals: Signals) -> bool {
        let close = k & KEY_CLOSE != 0;
        let slot = (k & 0xFFFF_FFFF) as usize;
        let generation = ((k >> 32) & 0x7FFF_FFFF) as u32;
        if slot == 0 {
            if close {
                return false;
            }
            self.link_armed = false;
            return !matches!(stack.dev.pump(), Err(Error::PeerClosed));
        }
        let i = slot - 1;
        // Paket pro uz uvolneny slot (stara generace) se ignoruje.
        if self.gens.get(i) != Some(&generation) {
            return true;
        }
        let Some(Some(o)) = self.objs.get_mut(i) else {
            return true;
        };
        if close || signals.contains(Signals::PEER_CLOSED) && !signals.contains(Signals::READABLE) {
            self.free(stack, i);
            return true;
        }
        o.read_armed = false;
        if o.pending.is_some() {
            // Dalsi zprava pocka; registrace se obnovi po dokonceni.
            return true;
        }
        let mut created = Vec::new();
        match o.ch.try_read() {
            Ok(msg) => {
                let step = obj::request(stack, &mut o.kind, &msg.data);
                finish(stack, o, step, &mut created, self.live);
                if o.pending.is_some() {
                    self.pending.push(i);
                } else {
                    self.to_arm.push((i, generation));
                }
            }
            Err(Error::ShouldWait) => self.to_arm.push((i, generation)),
            Err(_) => self.free(stack, i),
        }
        self.add_objects(created);
        true
    }

    /// Uvolni slot: uklid objektu a nova generace (stare pakety neplati).
    fn free(&mut self, stack: &mut Stack, i: usize) {
        if let Some(o) = self.objs[i].take() {
            obj::close(stack, o.kind, o.pending);
            self.live -= 1;
            self.free_slots.push(i);
        }
        self.gens[i] = self.gens[i].wrapping_add(1) & 0x7FFF_FFFF;
    }

    fn add_objects(&mut self, created: Vec<Obj>) {
        for n in created {
            let i = match self.free_slots.pop() {
                Some(i) => {
                    self.objs[i] = Some(n);
                    i
                }
                None => {
                    self.objs.push(Some(n));
                    self.gens.push(0);
                    self.objs.len() - 1
                }
            };
            self.live += 1;
            self.to_arm.push((i, self.gens[i]));
        }
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
                    created.push(Obj::new(server, kind));
                }
                Err(e) => {
                    obj::close(stack, kind, None);
                    result = Err(e);
                }
            }
        }
    }
    // Klient, ktery mezitim kanal zavrel, se uklidi po paketu "zavreno".
    let _ = o.ch.write(&netproto::reply(result), handles);
}
