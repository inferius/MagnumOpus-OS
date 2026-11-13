//! audiod: zvukova sluzba. Mixuje streamy klientu do jednoho vystupu
//! zarizeni, rozvadi nahravany zvuk, prevadi formaty a vzorkovaci
//! frekvence, ridi hlavni hlasitost a ztlumeni.
//!
//! Start: handly `kind::USER` = kanaly k ovladacum zvukovych karet
//! (`audioproto::device`). Bootstrap kanal od initu je prvni klient
//! (protokol `audioproto::client`); jen pres nej smi init za behu pridat
//! dalsi ovladac (zprava `device` s kanalem - restart ovladace) a v testu
//! obnovy ukoncit ovladac (`crash-driver`). Dalsi klienty vytvari
//! `CONNECT`.
//!
//! Zarizeni: aktivni je jedno (prvni s analogovym vystupem, nebo vybrane
//! `SELECT`); ostatni jsou v seznamu (`DEVICES`). Ovladac, ktery skonci,
//! se ze seznamu vyradi a sluzba prejde na dalsi.
//!
//! Smycka: ceka na kanaly klientu a na udalosti period od ovladace (vystup
//! i vstup). Po kazde periode smicha streamy predstih (adaptivni, viz `device.rs`)
//! period pred pozici DMA (rozpocet latence viz `device.rs`) a rozda
//! nahrana data. Kdyz zadny stream nebezi dele nez [`IDLE_STOP`], zastavi
//! DMA (uspora energie; zarizeni neprehrava ticho do nekonecna).

#![no_std]
#![no_main]

extern crate alloc;

mod device;
mod input;
mod service;
mod streams;

use alloc::vec;
use alloc::vec::Vec;

use audiomix::mix::Mixer;
use libos::{Channel, Startup, println};

use service::{Client, Service, Slot};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let mut slots = Vec::new();
    for h in startup.handles {
        let chan = Channel::from_handle(h);
        match device::query(&chan) {
            Ok(info) => slots.push(Slot { chan: Some(chan), info }),
            Err(e) => println!("audiod: ovladac neodpovida: {e}"),
        }
    }
    let mut svc = Service {
        slots,
        active: None,
        device: None,
        rec: None,
        lost: None,
        clients: vec![Client { chan: startup.bootstrap, open: true, privileged: true }],
        streams: Vec::new(),
        next_id: 1,
        master: 100,
        muted: false,
        mixer: Mixer::new(256),
        out: Vec::new(),
        idle_since: None,
    };
    svc.choose_default();
    if svc.device.is_none() {
        println!("audiod: zadne zvukove zarizeni, streamy budou odmitnuty");
    }
    let _ = svc.clients[0].chan.write(b"ok", Vec::new());
    svc.run();
    0
}
