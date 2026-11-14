//! launchd: spousteci sluzba (protokol `libs/launchproto`, popis
//! `docs/cs/rust-std.md`, kapitola Procesy).
//!
//! Spusti ji init a preda ji obraz initfs (`kind::INITFS`). Kanalova
//! prava nema zadna: vsechny schopnosti potomka (job, relace FS a site,
//! stdio) dodava volajici v pozadavku, takze potomek nikdy nedostane vic
//! nez rodic. launchd pridava jen to, co rodic sam neumi: najde program
//! (registr prikazu, `/system/bin`, initfs), uplatni manifest (limity
//! jobu, zuzeni site) a da potomkovi vlastni kanal k launchd.
//!
//! Bootstrap kanal = ridici kanal od initu: `client` -> `ok` + kanal
//! klienta (init ho preda aplikaci se stitkem `LAUNCH`).

#![no_std]
#![no_main]

extern crate alloc;

mod launch;

use alloc::vec::Vec;

use launchproto::{Reader, Registry, op, reply, request_head};
use libos::initfs::Initfs;
use libos::{Channel, Error, Handle, Port, Signals, Startup, println};

libos::entry!(main);

/// Klic ridiciho kanalu v portu (klienti maji klic = index + 1).
const CONTROL: u64 = 0;

pub struct Launchd {
    pub initfs: Initfs,
    pub registry: Registry,
    port: Port,
    /// Klientske kanaly (index + 1 = klic v portu; `None` = volne misto).
    clients: Vec<Option<Channel>>,
}

impl Launchd {
    /// Novy klientsky kanal: nas konec se hlida v portu, druhy se vrati.
    pub fn new_client(&mut self) -> Result<Channel, Error> {
        let (mine, theirs) = Channel::create()?;
        let slot = match self.clients.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                self.clients.push(None);
                self.clients.len() - 1
            }
        };
        self.port.wait_async(mine.handle(), slot as u64 + 1, Signals::READABLE | Signals::PEER_CLOSED)?;
        self.clients[slot] = Some(mine);
        Ok(theirs)
    }

    fn serve_client(&mut self, slot: usize) {
        loop {
            let Some(ch) = self.clients.get(slot).and_then(Option::as_ref) else { return };
            let msg = match ch.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => break,
                Err(_) => {
                    // Klient skoncil.
                    self.clients[slot] = None;
                    return;
                }
            };
            let (data, handles) = self.request(&msg.data, msg.handles);
            if let Some(ch) = self.clients[slot].as_ref() {
                let _ = ch.write(&data, handles);
            }
        }
        if let Some(ch) = self.clients[slot].as_ref() {
            let _ = self.port.wait_async(ch.handle(), slot as u64 + 1, Signals::READABLE | Signals::PEER_CLOSED);
        }
    }

    /// Obslouzi jeden pozadavek; vraci odpoved a jeji handly.
    fn request(&mut self, data: &[u8], handles: Vec<Handle>) -> (Vec<u8>, Vec<Handle>) {
        let mut r = Reader::new(data);
        let Ok((code, id)) = request_head(&mut r) else {
            return (reply(0, Err((Error::Corrupted, "neplatna zprava"))), Vec::new());
        };
        match code {
            op::LAUNCH => match launch::launch(self, &mut r, handles) {
                Ok(hs) => (reply(id, Ok(&[])), hs),
                Err((e, msg)) => (reply(id, Err((e, &msg))), Vec::new()),
            },
            op::LOOKUP => match r.str().map_err(|e| (e, alloc::string::String::from("neplatna zprava"))).and_then(|p| launch::lookup(self, &p)) {
                Ok(data) => (reply(id, Ok(&data)), Vec::new()),
                Err((e, msg)) => (reply(id, Err((e, &msg))), Vec::new()),
            },
            op::CONNECT => match self.new_client() {
                Ok(ch) => (reply(id, Ok(&[])), alloc::vec![ch.into_handle()]),
                Err(e) => (reply(id, Err((e, "novy kanal"))), Vec::new()),
            },
            _ => (reply(id, Err((Error::NotSupported, "neznama operace"))), Vec::new()),
        }
    }
}

fn main(startup: Startup) -> i64 {
    let initfs = match Initfs::open(&startup) {
        Ok(f) => f,
        Err(e) => {
            println!("launchd: chybi initfs: {e}");
            return 1;
        }
    };
    let registry = launch::registry(&initfs);
    let Ok(port) = Port::create() else { return 2 };
    let control = startup.bootstrap;
    if port.wait_async(control.handle(), CONTROL, Signals::READABLE | Signals::PEER_CLOSED).is_err() {
        return 3;
    }
    let mut d = Launchd { initfs, registry, port, clients: Vec::new() };
    println!("launchd: bezim, {} prikazu v registru", d.registry.commands().len());
    loop {
        let Ok(p) = d.port.wait(None) else { continue };
        if p.key == CONTROL {
            loop {
                match control.try_read() {
                    Ok(m) => {
                        let text = core::str::from_utf8(&m.data).unwrap_or("");
                        let (reply, hs) = match text {
                            "client" => match d.new_client() {
                                Ok(ch) => ("ok", alloc::vec![ch.into_handle()]),
                                Err(_) => ("err NO_RESOURCES", Vec::new()),
                            },
                            _ => ("err NOT_SUPPORTED", Vec::new()),
                        };
                        let _ = control.write(reply.as_bytes(), hs);
                    }
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        println!("launchd: init zavrel ridici kanal, koncim");
                        return 0;
                    }
                }
            }
            let _ = d.port.wait_async(control.handle(), CONTROL, Signals::READABLE | Signals::PEER_CLOSED);
        } else {
            d.serve_client(p.key as usize - 1);
        }
    }
}
