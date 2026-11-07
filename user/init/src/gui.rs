//! GUI (milnik 6c): start kompozitoru `compd`, predani vstupu a spousteni
//! aplikaci s oknem.
//!
//! Identitu aplikace (id a jmeno z manifestu, co smi) posila kompozitoru
//! init, ne aplikace: odznak duvery tak nejde podvrhnout. Aplikace dostane
//! jen kanal sveho okna (startovni zprava, kind USER).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::control::{self, ConnectInfo, Status};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Handle, Process, Result, Startup, Vmo, println, time};

use crate::apps::{self, App};
use crate::input::Input;

/// Kanal ovladace grafiky (od spravce zarizeni, nebo `fbdisplay`), dokud
/// ho nevezme kompozitor.
pub static DISPLAY: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);

/// Bezici GUI (bezny beh).
pub static GUI: spin::Mutex<Option<Gui>> = spin::Mutex::new(None);

/// Ovladac `fbdisplay` (framebuffer z firmwaru), bezi-li.
pub static FBDISPLAY: spin::Mutex<Option<Process>> = spin::Mutex::new(None);

const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Gui {
    pub process: Process,
    pub control: Channel,
    pub apps: Vec<App>,
    /// Velikost obrazovky.
    pub size: (u32, u32),
}

/// Displej pro kompozitor - jedine misto, kde init rozhoduje, kdo
/// kresli na obrazovku: kanal virtio-gpu od spravce zarizeni (ma
/// prednost), jinak ovladac `fbdisplay` nad framebufferem z firmwaru
/// (bootstrap `FRAMEBUFFER`, init preda duplikat handlu, takze
/// `Startup::framebuffer` zustava pro zachrannou konzoli).
pub fn display(startup: &Startup) -> Result<Channel> {
    if let Some(ch) = DISPLAY.lock().take() {
        return Ok(ch);
    }
    let fb = startup.framebuffer.as_ref().ok_or(Error::NotFound)?;
    let fb = Vmo::from_handle(fb.handle().duplicate(None)?);
    let fs = Initfs::open(startup)?;
    let image = fs.find("fbdisplay").ok_or(Error::NotFound)?;
    let (mine, theirs) = Channel::create()?;
    let (process, _boot) = libos::spawn(
        "fbdisplay",
        image,
        vec![StartHandle::user(fb.into_handle()), StartHandle::user(theirs.into_handle())],
    )?;
    println!("init: displej je framebuffer z firmwaru (fbdisplay)");
    *FBDISPLAY.lock() = Some(process);
    Ok(mine)
}

/// Bezi GUI nad framebufferem (`fbdisplay`)? Kdyz skonci, jadro
/// framebuffer uvolni a init muze spustit zachrannou konzoli.
// Pouzije start zachranne konzole (vlakno terminalu 6b).
#[allow(dead_code)]
pub fn uses_framebuffer() -> bool {
    FBDISPLAY.lock().is_some()
}

/// Spusti kompozitor nad displejem z [`display`].
pub fn start(startup: &Startup) -> Result<Gui> {
    let display = display(startup)?;
    let fs = Initfs::open(startup)?;
    let image = fs.find("compd").ok_or(Error::NotFound)?;
    let (process, control) = libos::spawn("compd", image, Vec::new())?;
    let mut gui = Gui {
        process,
        control,
        apps: Vec::new(),
        size: (0, 0),
    };
    let (text, _) = gui.command(control::DISPLAY.as_bytes(), vec![display.into_handle()])?;
    let mut it = text.split_whitespace().map(|v| v.parse::<u32>().unwrap_or(0));
    gui.size = (it.next().unwrap_or(0), it.next().unwrap_or(0));
    println!("init: kompozitor bezi ({}x{})", gui.size.0, gui.size.1);
    Ok(gui)
}

impl Gui {
    /// Ridici prikaz; odpoved `ok ...` -> text a handly.
    pub fn command(&self, data: &[u8], handles: Vec<Handle>) -> Result<(String, Vec<Handle>)> {
        crate::input::drain(&self.control);
        self.control.write(data, handles)?;
        let msg = self.control.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
        let reply = core::str::from_utf8(&msg.data).map_err(|_| Error::Corrupted)?;
        if let Some(code) = reply.strip_prefix("err ") {
            return Err(libos::abi::error::ALL.iter().copied().find(|e| e.name() == code).unwrap_or(Error::Corrupted));
        }
        let rest = reply.strip_prefix("ok").ok_or(Error::Corrupted)?;
        Ok((String::from(rest.trim()), msg.handles))
    }

    /// Prikaz, ktery vraci kanal (`dialog`, `inspect`, `actions <jmeno>`,
    /// `connect`).
    pub fn channel(&self, data: &[u8]) -> Result<Channel> {
        let (_, mut h) = self.command(data, Vec::new())?;
        if h.len() != 1 {
            return Err(Error::Corrupted);
        }
        Ok(Channel::from_handle(h.remove(0)))
    }

    pub fn status(&self) -> Result<Status> {
        let (t, _) = self.command(control::STATUS.as_bytes(), Vec::new())?;
        Status::parse(&t).ok_or(Error::Corrupted)
    }

    /// Preda kompozitoru fokus vstupu a kanal bezpecne zkratky od `inputd`
    /// (predchozi vlastnik, init, o ne prijde).
    pub fn take_input(&self, input: &Input) -> Result<()> {
        let (mine, theirs) = Channel::create()?;
        input.command(inputproto::control::FOCUS, vec![theirs.into_handle()])?;
        self.command(control::INPUT.as_bytes(), vec![mine.into_handle()])?;
        let (mine, theirs) = Channel::create()?;
        input.command(inputproto::control::SECURE, vec![theirs.into_handle()])?;
        self.command(control::SECURE.as_bytes(), vec![mine.into_handle()])?;
        Ok(())
    }

    /// Spusti aplikaci z initfs s oknem: identita z manifestu, kanal okna
    /// jako prvni handle startovni zpravy.
    pub fn launch(&mut self, startup: &Startup, name: &str) -> Result<usize> {
        let fs = Initfs::open(startup)?;
        let text = fs.find(&format!("{name}.manifest.toml")).and_then(|t| core::str::from_utf8(t).ok()).unwrap_or("");
        let m = apps::manifest(&fs, name)?;
        let actions = match guiproto::action::parse_manifest(text) {
            Ok(a) => a,
            Err(e) => {
                println!("init: akce v manifestu {name}: {}", e.0);
                return Err(Error::InvalidArgs);
            }
        };
        let mut perms = Vec::new();
        if m.as_ref().is_some_and(|m| m.network.is_some()) {
            perms.push(String::from("síť"));
        }
        let info = ConnectInfo {
            id: m.as_ref().map_or_else(|| format!("initfs.{name}"), |m| m.id.clone()),
            name: m.as_ref().map_or_else(|| String::from(name), |m| m.name.clone()),
            perms,
            actions,
        };
        let ch = self.channel(&info.encode())?;
        let app = apps::spawn(startup, name, vec![StartHandle::user(ch.into_handle())])?;
        self.apps.push(app);
        Ok(self.apps.len() - 1)
    }
}

/// Bezny beh: kompozitor, vstup a ukazkove aplikace.
pub fn start_desktop(startup: &Startup, input: Option<&Input>) {
    let mut gui = match start(startup) {
        Ok(g) => g,
        Err(Error::NotFound) => {
            println!("init: grafika (virtio-gpu ani framebuffer) neni, GUI nespoustim");
            return;
        }
        Err(e) => {
            println!("init: kompozitor nelze spustit: {e}");
            return;
        }
    };
    if let Some(i) = input
        && let Err(e) = gui.take_input(i)
    {
        println!("init: vstup pro kompozitor: {e}");
    }
    for app in ["gui-term", "gui-demo"] {
        if let Err(e) = gui.launch(startup, app) {
            println!("init: {app} nelze spustit: {e}");
        }
    }
    *GUI.lock() = Some(gui);
}
