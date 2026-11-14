//! Napajeni: sluzba ACPI (`acpid`) a ciste vypnuti systemu.
//!
//! Init spusti `acpid` s omezenym prostredkem (pamet mimo RAM a pamet
//! firmwaru ACPI, I/O porty, pevne linky preruseni), s pravem `POWER`
//! (vypnuti a restart pres firmware) a s udaji o ACPI od jadra. Kanal
//! k acpid hlida supervisor (udalosti tlacitek, `on_events`).
//!
//! Ciste vypnuti ([`shutdown`]): ukoncit sluzby (klienti pred FS, FS pred
//! disky), pak pozadat acpid o S5 nebo restart. Sluzba souboroveho systemu
//! zapisuje kazdou zmenu commitem (CoW), takze po ukonceni je disk
//! konzistentni; protokol "dopis a skonci" zatim nema (viz `docs/cs/acpi.md`).

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::hw::Resource;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Rights, Startup, println, time};
use powerproto::{Event, Reply, Request};

use crate::supervisor::Supervisor;

/// Bezici sluzba napajeni.
pub struct Power {
    pub process: Process,
    channel: Channel,
    /// Udalosti, ktere prisly behem cekani na odpoved.
    events: VecDeque<Event>,
}

/// Sluzba napajeni (supervisor ceka na jeji kanal).
pub static POWER: spin::Mutex<Option<Power>> = spin::Mutex::new(None);

/// Rozsahy prostredku pro acpid: AML smi sahat na registry, ktere si sam
/// nadeklaruje, proto cely fyzicky prostor (jadro z nej povoli jen registry
/// zarizeni a pamet ACPI, ne RAM), vsechny I/O porty (x86) a pevne linky.
fn acpi_ranges() -> Vec<(u64, u64)> {
    let mut r = vec![(0, 1u64 << 52)];
    if cfg!(target_arch = "x86_64") {
        r.push((abi::syscall::RANGE_IO_PORTS, 0x1_0000));
        r.push(libos::hw::irq_lines(0, 16));
    } else {
        // SPI GIC (Generic Event Device a podobne).
        r.push(libos::hw::irq_lines(32, abi::syscall::IRQ_LINES_MAX - 32));
    }
    r
}

/// Nejvic preruseni sluzby (SCI, GED).
const ACPI_IRQ_MAX: u32 = 16;

impl Power {
    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    /// Posle pozadavek a pocka na odpoved (udalosti mezitim schova).
    pub fn request(&mut self, r: &Request, timeout: Duration) -> Result<Reply> {
        self.channel.write(&r.encode(), Vec::new())?;
        let deadline = time::deadline_after(timeout);
        loop {
            let m = self.channel.read(Some(deadline))?;
            match Reply::decode(&m.data)? {
                Reply::Event(e) => self.events.push_back(e),
                reply => return Ok(reply),
            }
        }
    }

    /// Udalosti, ktere uz prisly (bez cekani).
    pub fn take_events(&mut self) -> Vec<Event> {
        while let Ok(m) = self.channel.try_read() {
            if let Ok(Reply::Event(e)) = Reply::decode(&m.data) {
                self.events.push_back(e);
            }
        }
        self.events.drain(..).collect()
    }
}

/// Spusti acpid. Bez udaju ACPI od jadra (`NOT_FOUND`) se nespousti.
pub fn start(startup: &Startup) -> Result<()> {
    let info = startup.acpi.as_ref().ok_or(Error::NotFound)?;
    let root = startup.root_resource.as_ref().ok_or(Error::NotFound)?;
    let fs = Initfs::open(startup)?;
    let image = fs.find("acpid").ok_or(Error::NotFound)?;
    let res: Resource = root.restrict(&acpi_ranges(), ACPI_IRQ_MAX, 0)?;
    let power = root.handle().duplicate(Some(Rights::BASIC | Rights::POWER))?;
    let info = info.handle().duplicate(Some(Rights::BASIC | Rights::READ | Rights::MAP))?;
    let handles = vec![StartHandle::user(res.into_handle()), StartHandle::user(power), StartHandle::user(info)];
    let (process, channel) = libos::spawn("acpid", image, handles)?;
    *POWER.lock() = Some(Power { process, channel, events: VecDeque::new() });
    Ok(())
}

/// Jeden pozadavek na acpid (testy, shell). Bez sluzby `NOT_FOUND`.
pub fn request(r: &Request, timeout: Duration) -> Result<Reply> {
    POWER.lock().as_mut().ok_or(Error::NotFound)?.request(r, timeout)
}

/// Udalosti od acpid (vola supervisor, kdyz je kanal citelny).
pub fn on_events(sup: &mut Supervisor) {
    let events = match POWER.lock().as_mut() {
        Some(p) => p.take_events(),
        None => return,
    };
    for e in events {
        match e {
            Event::PowerButton => {
                println!("init: tlacitko napajeni, vypinam system");
                shutdown(sup, false);
            }
            Event::SleepButton => println!("init: tlacitko spanku (uspani zatim neni)"),
        }
    }
}

/// Pozadavek shellu na vypnuti nebo restart, ktery supervisor provede po
/// kole (bool = restart; kanal pro odpoved pri chybe).
static PENDING: spin::Mutex<Option<(bool, Channel)>> = spin::Mutex::new(None);

/// Zprava od relace shellu na kanalu napajeni. Vypnuti/restart se jen
/// zaznamena (provede ho supervisor, [`run_pending`]) a kanal si init
/// nechá; jinak odpovi hned a kanal vrati.
pub fn request_from_shell(data: &[u8], chan: Channel, session: Option<u64>) -> Option<Channel> {
    let reply = match Request::decode(data) {
        Ok(Request::Shutdown) | Ok(Request::Reboot) => {
            let reboot = Request::decode(data) == Ok(Request::Reboot);
            println!("init: shell zada {}", if reboot { "restart" } else { "vypnuti" });
            *PENDING.lock() = Some((reboot, chan));
            return None;
        }
        // Balicek pro vyvojare sestavi sluzba logu.
        // Instalator: kanal k installd jako handle odpovedi.
        Ok(Request::OpenInstaller) => {
            let r = crate::install::open();
            let _ = match r {
                Ok(ch) => chan.write(&Reply::Ok.encode(), alloc::vec![ch.into_handle()]),
                Err(e) => chan.write(&Reply::Failed(e).encode(), Vec::new()),
            };
            return Some(chan);
        }
        Ok(Request::Report) => match crate::logd::report() {
            Ok((path, preview)) => Reply::Text(alloc::format!("{path}
{preview}")),
            Err(e) => Reply::Failed(e),
        },
        Ok(Request::LogLevel { target, level }) => match crate::logd::log_level(&target, &level) {
            Ok(text) => Reply::Text(text),
            Err(e) => Reply::Failed(e),
        },
        // Povoleni aplikaci uzivatele relace (prikaz `povoleni`).
        Ok(Request::Permissions { app, perm }) => match crate::perms::shell(session, &app, &perm) {
            Ok(text) => Reply::Text(text),
            Err(e) => Reply::Failed(e),
        },
        // Diagnosticky snimek: kompozitor (blok v logu, obrazek pres logd).
        Ok(Request::Snapshot) => match crate::gui::snapshot() {
            Ok(()) => Reply::Text(alloc::string::String::from("snimek: blok stavu v logu, obrazek v /system/log/snimek-*.bmp")),
            Err(e) => Reply::Failed(e),
        },
        // Dotazy (stav, cidla) preposlat acpid.
        Ok(r) => request(&r, Duration::from_secs(10)).unwrap_or_else(Reply::Failed),
        Err(e) => Reply::Failed(e),
    };
    let _ = chan.write(&reply.encode(), Vec::new());
    Some(chan)
}

/// Provede cekajici pozadavek shellu (vola supervisor po kole).
pub fn run_pending(sup: &mut Supervisor) {
    let Some((reboot, chan)) = PENDING.lock().take() else { return };
    let e = shutdown(sup, reboot);
    let _ = chan.write(&Reply::Failed(e).encode(), Vec::new());
}

/// Jak dlouho ceka init na konec jedne sluzby pri vypinani.
const STOP_WAIT: Duration = Duration::from_millis(1500);

/// Probehl uz pokus o vypnuti (test napajeni pak skonci hned).
pub static ATTEMPTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Ciste vypne (`reboot` = restartuje) system. Vraci jen pri chybe.
pub fn shutdown(sup: &mut Supervisor, reboot: bool) -> Error {
    ATTEMPTED.store(true, core::sync::atomic::Ordering::Release);
    let what = if reboot { "restart" } else { "vypnuti" };
    println!("init: {what}: ukoncuji sluzby");
    crate::terminal::TERMINALS.lock().stop_all();
    sup.stop_all(STOP_WAIT);
    println!("init: {what}: sluzby ukonceny, zadam acpid");
    let req = if reboot { Request::Reboot } else { Request::Shutdown };
    match request(&req, Duration::from_secs(10)) {
        Ok(Reply::Failed(e)) => {
            println!("init: {what} selhal: {e}");
            e
        }
        Ok(_) => Error::BadState,
        Err(e) => {
            println!("init: {what}: acpid neodpovida ({e})");
            e
        }
    }
}

/// Ukonceni acpid (testy).
#[allow(dead_code)]
pub fn stop() {
    if let Some(p) = POWER.lock().take() {
        let _ = p.process.kill();
    }
}

/// Je acpid spusteny?
#[allow(dead_code)]
pub fn running() -> bool {
    POWER.lock().is_some()
}

/// Pocka, az acpid odpovi na `Status` (po inicializaci ACPICA).
pub fn wait_ready(timeout: Duration) -> Result<powerproto::Status> {
    match request(&Request::Status, timeout)? {
        Reply::Status(s) => Ok(s),
        Reply::Failed(e) => Err(e),
        _ => Err(Error::InvalidArgs),
    }
}
