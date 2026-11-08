//! Nativni ovladace displeje (`bochs-display`, `intel-display`): vyber,
//! hlidani a bezpecny navrat na obraz firmwaru.
//!
//! Spravce zarizeni spusti prvni instanci a posle zpravu
//! `display-native <program> <b> <d> <f>` s handly [konec klienta, konec
//! ovladace, prostredek zarizeni, konfigurace PCI, zaznam firmwaru,
//! proces]. Init drzi **konec ovladace** ([`Kept`]) a zaznam firmwaru:
//!
//! - Vyber (`gui::display`): virtio-gpu ma prednost, pak nativni ovladac,
//!   ktery odpovi na `SYNC` a hlasi pripojeny monitor; jinak `fbdisplay`.
//!   Ovladac neznamou generaci nebo bez monitoru skonci / nic nehlasi
//!   a pouzije se framebuffer firmwaru.
//! - Pad: supervisor spusti novou instanci s `restore` nad stejnym koncem
//!   kanalu. Ta vrati registry ze zaznamu (obraz firmwaru), posle
//!   kompozitoru `RESET` a dal slouzi jen rezim firmwaru. Obrazovka tak
//!   nezustane cerna (`docs/cs/displej.md`).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::display::{Reply, Request};
use guiproto::modes;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Handle, Process, Result, println, time};

use crate::supervisor::{Kept, Service, Spec, StartCtx, Started, Supervisor};

/// Jmeno ve supervisoru.
pub const NAME: &str = "display-native";
/// Jak dlouho cekat na odpoved ovladace pri vyberu.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Vse k (znovu)spusteni ovladace.
pub struct Kit {
    pub program: String,
    pub pci: String,
    /// Konec klienta (dostane kompozitor nebo test).
    pub client: Option<Channel>,
    driver_end: Kept,
    device: Handle,
    config: Handle,
    snapshot: Handle,
    /// Bezici instance (prvni spustil spravce zarizeni).
    pub process: Option<Process>,
    /// Ovladac kresli GUI: dostava framebuffer firmwaru a prevezme ho
    /// (`FB_CONTROL CLAIM`), aby do nej jadro dal nekreslilo log.
    pub gui: bool,
}

/// Ovladace nahlasene spravcem zarizeni (vyber si je bere).
pub static NATIVES: spin::Mutex<Vec<Kit>> = spin::Mutex::new(Vec::new());
/// Vybrany ovladac pro GUI (hlida ho supervisor).
pub static CHOSEN: spin::Mutex<Option<Kit>> = spin::Mutex::new(None);

/// Zprava od spravce zarizeni.
pub fn add(text: &[u8], handles: Vec<Handle>) {
    let text = String::from_utf8_lossy(text);
    let mut w = text.split_whitespace().skip(1);
    let program = String::from(w.next().unwrap_or("?"));
    let pci: Vec<&str> = w.collect();
    let mut h = handles.into_iter();
    let (Some(client), Some(driver_end), Some(device), Some(config), Some(snapshot), Some(process)) =
        (h.next(), h.next(), h.next(), h.next(), h.next(), h.next())
    else {
        println!("init: neplatna zprava display-native od devmgr");
        return;
    };
    NATIVES.lock().push(Kit {
        program,
        pci: pci.join(" "),
        client: Some(Channel::from_handle(client)),
        driver_end: Kept::new(Channel::from_handle(driver_end)),
        device,
        config,
        snapshot,
        process: Some(Process::from_handle(process)),
        gui: false,
    });
}

/// Odpovi ovladac a hlasi pripojeny monitor s rezimem? (Precte
/// i zbytky odpovedi; kanal pak zustava cisty pro kompozitor, ktery se
/// stejne synchronizuje.)
pub fn probe(ch: &Channel) -> Result<modes::Connector> {
    let end = time::deadline_after(PROBE_TIMEOUT);
    let token = 0x6469_7370_6c61_7921u64;
    ch.write(&Request::Sync { token }.encode(), Vec::new())?;
    loop {
        if Reply::decode(&ch.read(Some(end))?.data).is_ok_and(|r| r.ends_sync(token)) {
            break;
        }
    }
    ch.write(&Request::Connectors.encode(), Vec::new())?;
    loop {
        match Reply::decode(&ch.read(Some(end))?.data)? {
            r if r.is_event() => continue,
            Reply::Connectors { connectors } => return modes::primary(&connectors).cloned().ok_or(Error::NotFound),
            Reply::Error { code, .. } => return Err(code),
            _ => return Err(Error::Corrupted),
        }
    }
}

/// Vybere nativni ovladac pro GUI: prvni, ktery projde [`probe`]
/// (poradi PCI: na x86 je karta firmwaru prvni). Vraci konec klienta.
///
/// Instanci od spravce zarizeni nahradi vlastni s framebufferem firmwaru:
/// ovladac ho pak prevezme jako `fbdisplay` a jadro prestane kreslit log
/// do pameti, kterou ukazuje GUI. Prvni instance do te doby registry
/// obrazu nezmenila (jen zaznam firmwaru, ktery nova zapise znovu).
pub fn choose(startup: &libos::Startup) -> Option<Channel> {
    let mut list = NATIVES.lock();
    let mut i = 0;
    while i < list.len() {
        let ok = list[i].client.as_ref().map(probe);
        match ok {
            Some(Ok(c)) => {
                let mut kit = list.remove(i);
                kit.gui = true;
                if let Some(old) = kit.process.take() {
                    let _ = old.kill();
                    let _ = old.wait_exit(Some(time::deadline_after(PROBE_TIMEOUT)));
                }
                match kit.spawn(startup, false) {
                    Ok(p) => kit.process = Some(p),
                    Err(e) => {
                        println!("init: nativni displej {} nelze spustit znovu: {e}", kit.program);
                        continue;
                    }
                }
                println!("init: displej je nativni ovladac {} ({}, {} {})", kit.program, kit.pci, modes::kind::name(c.kind), c.monitor);
                let ch = kit.client.take();
                *CHOSEN.lock() = Some(kit);
                return ch;
            }
            Some(Err(e)) => println!("init: nativni displej {} ({}) nepouzit: {e}", list[i].program, list[i].pci),
            None => {}
        }
        i += 1;
    }
    None
}

impl Kit {
    /// Spusti novou instanci (po padu: `restore`).
    pub fn spawn(&self, startup: &libos::Startup, restore: bool) -> Result<Process> {
        let fs = Initfs::open(startup)?;
        let image = fs.find(&self.program).ok_or(Error::NotFound)?;
        let handles = vec![
            StartHandle::user(self.device.duplicate(None)?),
            StartHandle::user(self.config.duplicate(None)?),
            StartHandle::user(self.driver_end.give()?.into_handle()),
            StartHandle::user(self.snapshot.duplicate(None)?),
        ];
        let mut handles = handles;
        if self.gui && let Some(fb) = startup.framebuffer.as_ref() {
            handles.push(StartHandle::user(fb.handle().duplicate(None)?));
        }
        let (p, control) = libos::spawn(&self.program, image, handles)?;
        let flag = if restore { " restore" } else { "" };
        control.write(format!("pci {}{flag}", self.pci).as_bytes(), Vec::new())?;
        Ok(p)
    }
}

/// Adapter pro supervisor: restart = instance v rezimu obnovy.
pub struct NativeService {
    kit: Kit,
    _process: Option<Process>,
}

impl NativeService {
    pub fn new(kit: Kit) -> Self {
        Self { kit, _process: None }
    }
}

impl Service for NativeService {
    fn start(&mut self, ctx: &StartCtx) -> Result<Started> {
        // Supervisor startuje jen po padu (prvni instanci spustil devmgr).
        let p = self.kit.spawn(ctx.startup, true)?;
        println!("init: {} spadl, nova instance vraci obraz firmwaru (generace {})", self.kit.program, ctx.generation);
        let started = Started {
            process: p.handle().duplicate(None)?,
            ready: None,
        };
        self._process = Some(p);
        Ok(started)
    }
}

/// Zaradi vybrany ovladac do supervisoru (bezny beh).
pub fn supervise(sup: &mut Supervisor) {
    let Some(mut kit) = CHOSEN.lock().take() else { return };
    let Some(p) = kit.process.take() else { return };
    sup.register(
        Spec {
            name: NAME,
            deps: &[],
            service: Some(Box::new(NativeService::new(kit))),
        },
        Some(&p),
    );
}
