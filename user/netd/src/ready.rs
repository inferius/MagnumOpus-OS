//! Pripravenost socketu (`op::READINESS`): event s uzivatelskymi bity
//! (`netproto::ready`), ktery klient sleduje pres port (backend mio,
//! neblokujici smycky). Sluzba drzi event s pravem `SIGNAL`, klient dostane
//! duplikat jen s `WAIT`.
//!
//! Stav je urovnovy ("level"): po kazdem kole sluzby se u sledovanych
//! socketu prepocita a do eventu se zapise jen zmena. Klient po udalosti
//! zkusi operaci s terminem `TRY`; `SHOULD_WAIT` = uz neni pripraveny,
//! pocka na dalsi udalost (jako epoll s ONESHOT/kqueue).

use libos::{Event, Result, Rights, Signals};
use netproto::ready;
use smoltcp::socket::tcp::State;

use crate::obj::{Kind, Obj};
use crate::stack::Stack;

/// Event pripravenosti objektu a posledni zapsane bity.
pub struct Readiness {
    event: Event,
    bits: Signals,
}

impl Readiness {
    pub fn new() -> Result<Self> {
        Ok(Self {
            event: Event::create()?,
            bits: Signals::NONE,
        })
    }

    /// Duplikat pro klienta: jen cekani (a predani dal).
    pub fn client_handle(&self) -> Result<libos::Handle> {
        self.event
            .handle()
            .duplicate(Some(Rights::WAIT | Rights::DUPLICATE | Rights::TRANSFER))
    }
}

/// Aktualni bity pripravenosti objektu.
fn compute(stack: &mut Stack, o: &mut Obj) -> Signals {
    let mut bits = Signals::NONE;
    match &mut o.kind {
        Kind::Stream(s) => {
            crate::tcp::update_fin(stack, s);
            let fin = s.fin_seen;
            let sock = stack.tcp(s.sock);
            let state = sock.state();
            // Cteni neblokuje i na konci proudu a po preruseni (vrati 0
            // nebo chybu).
            if sock.can_recv() || !sock.may_recv() {
                bits = bits | ready::READABLE;
            }
            if sock.can_send() || !sock.may_send() {
                bits = bits | ready::WRITABLE;
            }
            if fin || state == State::Closed {
                bits = bits | ready::HUP;
            }
            if state == State::Closed && !fin {
                bits = bits | ready::ERROR;
            }
        }
        Kind::Listener(l) => {
            if crate::tcp::listener_has_ready(stack, l) {
                bits = bits | ready::READABLE;
            }
        }
        Kind::Udp(u) => {
            let sock = stack.udp(u.sock);
            if sock.can_recv() {
                bits = bits | ready::READABLE;
            }
            if sock.can_send() {
                bits = bits | ready::WRITABLE;
            }
        }
        Kind::Session(_) => {}
    }
    bits
}

/// Prepocita pripravenost objektu a zapise zmenu do eventu.
pub fn update(stack: &mut Stack, o: &mut Obj) {
    if o.ready.is_none() {
        return;
    }
    let bits = compute(stack, o);
    let Some(r) = o.ready.as_mut() else {
        return;
    };
    if bits.0 != r.bits.0 {
        let clear = Signals(r.bits.0 & !bits.0);
        let set = Signals(bits.0 & !r.bits.0);
        if r.event.update(clear, set).is_ok() {
            r.bits = bits;
        }
    }
}
