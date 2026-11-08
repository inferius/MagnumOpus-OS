//! ps2: ovladac radice i8042 (PS/2 klavesnice a mys).
//!
//! Spousti ho spravce zarizeni (na x86, bez PCI). Startovni zprava (kind
//! USER, v tomto poradi):
//! 1. omezeny Resource: porty 0x60 a 0x64, linky preruseni 1 a 12,
//! 2. kanal ke sluzbe vstupu pro klavesnici,
//! 3. kanal ke sluzbe vstupu pro mys.
//!
//! Po inicializaci posle na bootstrap kanal vysledek: `ps2` a slova
//! `klavesnice` / `mys` podle toho, co nasel (napr. `ps2 klavesnice mys`),
//! nebo `ps2 nic`, kdyz radic chybi - pak skonci. Spravce preda sluzbe
//! vstupu jen kanaly nalezenych zarizeni.
//!
//! Na synchronizaci nove instance sluzby vstupu (`ISYN`) odpovi a predstavi
//! se znovu; zavreny kanal ke sluzbe ho neukonci (`docs/cs/obnova-sluzeb.md`).
//!
//! Scan kody (sada 1 s prekladem radice) a pakety mysi dekoduje knihovna
//! `i8042`, sem patri jen pristup k radici.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

use i8042::controller::{COMMAND, DATA, IRQ_KEYBOARD, IRQ_MOUSE, cmd, config, dev, status};
use i8042::{mouse, scancode};
use inputproto::code::{EV_KEY, EV_MSC, MSC_SCAN};
use inputproto::device::{self, RawEvent, caps};
use libos::hw::{Interrupt, Resource};
use libos::{Channel, Error, Result, Signals, Startup, println, time};

libos::entry!(main);

/// Jak dlouho cekat na radic pri jednom zapisu nebo cteni.
const IO_TIMEOUT: Duration = Duration::from_millis(50);
/// Reset zarizeni (self-test klavesnice nebo mysi) trva az stovky ms.
const RESET_TIMEOUT: Duration = Duration::from_millis(1000);
/// Zaloha za ztracenou hranu preruseni: radic se zkontroluje aspon takhle
/// casto.
const POLL: Duration = Duration::from_millis(500);

struct Ctl<'a> {
    res: &'a Resource,
}

impl Ctl<'_> {
    fn status(&self) -> Result<u8> {
        self.res.port_read8(COMMAND)
    }

    /// Pocka, az radic prijme dalsi zapis.
    fn wait_write(&self) -> Result<()> {
        let deadline = time::deadline_after(IO_TIMEOUT);
        while self.status()? & status::INPUT_FULL != 0 {
            if time::now() > deadline {
                return Err(Error::TimedOut);
            }
            libos::time::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    /// Precte bajt z vystupniho bufferu (ceka nejvys `timeout`).
    fn read(&self, timeout: Duration) -> Result<u8> {
        let deadline = time::deadline_after(timeout);
        while self.status()? & status::OUTPUT_FULL == 0 {
            if time::now() > deadline {
                return Err(Error::TimedOut);
            }
            libos::time::sleep(Duration::from_millis(1));
        }
        self.res.port_read8(DATA)
    }

    fn command(&self, c: u8) -> Result<()> {
        self.wait_write()?;
        self.res.port_write8(COMMAND, c)
    }

    fn command_reply(&self, c: u8) -> Result<u8> {
        self.command(c)?;
        self.read(IO_TIMEOUT)
    }

    fn write_config(&self, v: u8) -> Result<()> {
        self.command(cmd::WRITE_CONFIG)?;
        self.wait_write()?;
        self.res.port_write8(DATA, v)
    }

    /// Zahodi bajty, ktere v radici zustaly (od firmwaru, pred resetem).
    fn flush(&self) -> Result<()> {
        for _ in 0..32 {
            if self.status()? & status::OUTPUT_FULL == 0 {
                break;
            }
            self.res.port_read8(DATA)?;
        }
        Ok(())
    }

    /// Posle bajt zarizeni (klavesnici, nebo mysi pres `WRITE_AUX`)
    /// a pocka na ACK; `RESEND` zopakuje (nejvys 3x).
    fn send(&self, aux: bool, b: u8) -> Result<()> {
        for _ in 0..3 {
            if aux {
                self.command(cmd::WRITE_AUX)?;
            }
            self.wait_write()?;
            self.res.port_write8(DATA, b)?;
            match self.read(IO_TIMEOUT * 4)? {
                dev::ACK => return Ok(()),
                dev::RESEND => continue,
                _ => return Err(Error::IoError),
            }
        }
        Err(Error::IoError)
    }

    /// Reset zarizeni: ACK, pak self-test 0xAA (mys navic posle ID).
    fn reset(&self, aux: bool) -> Result<()> {
        self.send(aux, dev::RESET)?;
        let deadline = time::deadline_after(RESET_TIMEOUT);
        loop {
            match self.read(RESET_TIMEOUT) {
                Ok(dev::SELF_TEST_OK) => break,
                Ok(_) if time::now() < deadline => continue,
                Ok(_) => return Err(Error::IoError),
                Err(e) => return Err(e),
            }
        }
        if aux {
            // ID mysi po resetu (0); nekde chybi, proto jen kratce.
            let _ = self.read(IO_TIMEOUT);
        }
        Ok(())
    }

    /// ID mysi.
    fn mouse_id(&self) -> Result<u8> {
        self.send(true, dev::GET_ID)?;
        self.read(IO_TIMEOUT * 4)
    }

    fn sample_rates(&self, rates: &[u8]) -> Result<()> {
        for &r in rates {
            self.send(true, dev::SET_SAMPLE_RATE)?;
            self.send(true, r)?;
        }
        Ok(())
    }
}

/// Co inicializace nasla.
struct Found {
    keyboard: bool,
    mouse: Option<mouse::Kind>,
}

/// Inicializace radice a zarizeni (preruseni radice vypnuta, cte se
/// dotazovanim). Postup podle OSDev / Linux i8042.
fn init(c: &Ctl) -> Result<Found> {
    // Bez radice cte port 0x64 vsechny bity v jednicce.
    if c.status()? == 0xFF {
        return Err(Error::NotFound);
    }
    c.command(cmd::DISABLE_KEYBOARD)?;
    c.command(cmd::DISABLE_AUX)?;
    c.flush()?;
    let cfg = c.command_reply(cmd::READ_CONFIG)?;
    let cfg = (cfg & !(config::KEYBOARD_IRQ | config::AUX_IRQ)) | config::TRANSLATE;
    c.write_config(cfg)?;
    if c.command_reply(cmd::SELF_TEST)? != 0x55 {
        return Err(Error::IoError);
    }
    // Self-test muze konfiguraci vratit na vychozi.
    c.write_config(cfg)?;

    // Druhy port: po zapnuti zmizi priznak "hodiny vypnute".
    c.command(cmd::ENABLE_AUX)?;
    let has_aux = c.command_reply(cmd::READ_CONFIG)? & config::AUX_DISABLED == 0;
    c.command(cmd::DISABLE_AUX)?;

    let kbd_port = c.command_reply(cmd::TEST_KEYBOARD)? == 0;
    let aux_port = has_aux && c.command_reply(cmd::TEST_AUX)? == 0;

    let mut found = Found {
        keyboard: false,
        mouse: None,
    };
    if kbd_port {
        c.command(cmd::ENABLE_KEYBOARD)?;
        found.keyboard = c.reset(false).and_then(|_| c.send(false, dev::ENABLE)).is_ok();
        if !found.keyboard {
            println!("ps2: klavesnice neodpovida na reset");
        }
    }
    if aux_port {
        c.command(cmd::ENABLE_AUX)?;
        let kind = c.reset(true).and_then(|_| {
            // IntelliMouse: kolecko, pak 5 tlacitek (mys, ktera je neumi,
            // posloupnost ignoruje a hlasi dal ID 0).
            c.sample_rates(&i8042::controller::WHEEL_MAGIC)?;
            let mut id = c.mouse_id()?;
            if id == 3 {
                c.sample_rates(&i8042::controller::FIVE_BUTTON_MAGIC)?;
                id = c.mouse_id()?;
            }
            c.sample_rates(&[100])?;
            c.send(true, dev::ENABLE)?;
            Ok(mouse::Kind::from_id(id))
        });
        match kind {
            Ok(k) => found.mouse = Some(k),
            Err(e) => println!("ps2: mys neodpovida ({e})"),
        }
    }
    c.flush()?;
    let mut cfg = cfg & !(config::KEYBOARD_DISABLED | config::AUX_DISABLED);
    if found.keyboard {
        cfg |= config::KEYBOARD_IRQ;
    }
    if found.mouse.is_some() {
        cfg |= config::AUX_IRQ;
    }
    c.write_config(cfg)?;
    Ok(found)
}

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(kbd_ch), Some(mouse_ch)) = (handles.next(), handles.next(), handles.next())
    else {
        println!("ps2: chybi handly ve startovni zprave");
        return 1;
    };
    let res = Resource::from_handle(res);
    let kbd_ch = Channel::from_handle(kbd_ch);
    let mouse_ch = Channel::from_handle(mouse_ch);
    let ctl = Ctl { res: &res };

    let found = match init(&ctl) {
        Ok(f) => f,
        Err(e) => {
            if e != Error::NotFound {
                println!("ps2: radic neodpovida spravne: {e}");
            }
            let _ = startup.bootstrap.write(b"ps2 nic", Vec::new());
            return 0;
        }
    };
    let mut report = alloc::string::String::from("ps2");
    if found.keyboard {
        report.push_str(" klavesnice");
    }
    if let Some(k) = found.mouse {
        report.push_str(" mys");
        println!("ps2: {}", k.name());
    }
    if found.keyboard {
        println!("ps2: klavesnice");
    }
    if !found.keyboard && found.mouse.is_none() {
        report.push_str(" nic");
    }
    let _ = startup.bootstrap.write(report.as_bytes(), Vec::new());
    if !found.keyboard && found.mouse.is_none() {
        return 0;
    }

    let irqs = (
        found
            .keyboard
            .then(|| res.interrupt_line(IRQ_KEYBOARD))
            .transpose(),
        found
            .mouse
            .is_some()
            .then(|| res.interrupt_line(IRQ_MOUSE))
            .transpose(),
    );
    let (Ok(kbd_irq), Ok(mouse_irq)) = irqs else {
        println!("ps2: preruseni nelze nastavit");
        return 2;
    };
    let kbd = found
        .keyboard
        .then(|| Service::new(kbd_ch, device::encode_hello(caps::KEYBOARD, "PS/2 klavesnice")));
    let mouse_svc = found
        .mouse
        .map(|k| Service::new(mouse_ch, device::encode_hello(caps::POINTER, k.name())));
    let mut dev = Devices {
        keys: scancode::Decoder::new(),
        mouse: found.mouse.map(mouse::Decoder::new),
        kbd_ch: kbd,
        mouse_ch: mouse_svc,
    };
    match dev.serve(&ctl, kbd_irq.as_ref(), mouse_irq.as_ref()) {
        Ok(()) => 0,
        Err(e) => {
            println!("ps2: {e}");
            3
        }
    }
}

/// Kanal ke sluzbe vstupu pro jedno zarizeni a jeho predstaveni
/// (posila se znovu na synchronizaci nove instance sluzby, `ISYN`).
struct Service {
    ch: Channel,
    hello: Vec<u8>,
    /// Sluzba kanal zavrela: ovladac nekonci (obnova sluzeb), jen ho
    /// prestane hlidat.
    closed: bool,
}

impl Service {
    fn new(ch: Channel, hello: Vec<u8>) -> Self {
        let _ = ch.write(&hello, Vec::new());
        Self {
            ch,
            hello,
            closed: false,
        }
    }

    /// Zpravy od sluzby (synchronizace) nebo jeji zavreni.
    fn signaled(&mut self, sig: Signals) {
        if sig.intersects(Signals::READABLE) {
            while let Ok(m) = self.ch.try_read() {
                device::answer_sync(&m.data, &self.hello, None, |b| {
                    self.ch.write(b, Vec::new()).is_ok()
                });
            }
        } else if sig.intersects(Signals::PEER_CLOSED) {
            println!("ps2: sluzba vstupu zavrela kanal, cekam");
            self.closed = true;
        }
    }
}

struct Devices {
    keys: scancode::Decoder,
    mouse: Option<mouse::Decoder>,
    kbd_ch: Option<Service>,
    mouse_ch: Option<Service>,
}

impl Devices {
    /// Obsluha preruseni a synchronizace sluzby vstupu (konci jen chybou
    /// radice; ukonceni ovladace je na spravci zarizeni).
    fn serve(&mut self, c: &Ctl, kbd: Option<&Interrupt>, mouse: Option<&Interrupt>) -> Result<()> {
        loop {
            let mut items = Vec::new();
            for irq in [kbd, mouse].into_iter().flatten() {
                items.push((irq.handle(), Signals::SIGNALED));
            }
            let irq_count = items.len();
            for s in [&self.kbd_ch, &self.mouse_ch].into_iter().flatten() {
                if !s.closed {
                    items.push((s.ch.handle(), Signals::READABLE | Signals::PEER_CLOSED));
                }
            }
            let observed = match libos::wait_many(&items, Some(time::deadline_after(POLL))) {
                Ok((_, o)) => o,
                Err(Error::TimedOut) => Vec::new(),
                Err(e) => return Err(e),
            };
            drop(items);
            let mut k = irq_count;
            for s in [&mut self.kbd_ch, &mut self.mouse_ch].into_iter().flatten() {
                if !s.closed {
                    if let Some(sig) = observed.get(k) {
                        s.signaled(*sig);
                    }
                    k += 1;
                }
            }
            // Potvrdit driv nez cteni: bajt, ktery prijde behem cteni,
            // preruseni znovu nastavi.
            for irq in [kbd, mouse].into_iter().flatten() {
                irq.ack()?;
            }
            self.drain(c)?;
        }
    }

    /// Precte vse, co radic ma, a posle udalosti.
    fn drain(&mut self, c: &Ctl) -> Result<()> {
        let mut keys = Vec::new();
        let mut moves = Vec::new();
        for _ in 0..256 {
            let st = c.status()?;
            if st & status::OUTPUT_FULL == 0 {
                break;
            }
            let b = c.res.port_read8(DATA)?;
            if st & (status::TIMEOUT | status::PARITY) != 0 {
                continue;
            }
            if st & status::AUX_DATA != 0 {
                if let Some(m) = &mut self.mouse
                    && let Some(p) = m.feed(b)
                {
                    m.events(p, &mut moves);
                }
            } else if let Some(k) = self.keys.feed(b) {
                if k.scancode != k.code as u32 {
                    keys.push(RawEvent::new(EV_MSC, MSC_SCAN, k.scancode as i32));
                }
                keys.push(RawEvent::new(EV_KEY, k.code, k.pressed as i32));
                keys.push(RawEvent::syn());
            }
        }
        send(&self.kbd_ch, &keys)?;
        send(&self.mouse_ch, &moves)
    }
}

/// Posle udalosti sluzbe; plna fronta = zahodit (neblokovat radic).
fn send(ch: &Option<Service>, events: &[RawEvent]) -> Result<()> {
    let Some(Service { ch, closed: false, .. }) = ch else {
        return Ok(());
    };
    for chunk in events.chunks(device::MAX_EVENTS) {
        match ch.write(&device::encode_events(chunk), Vec::new()) {
            Ok(()) | Err(Error::ShouldWait) => {}
            Err(Error::PeerClosed) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
