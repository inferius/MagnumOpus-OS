//! Vulkan pres Venus: render uzel ovladace virtio-gpu a testovaci program
//! `vk-probe` (knihovna `venus`).
//!
//! Bezi pred sadou `gui`, dokud displej drzi init (pak ho prevezme
//! kompozitor). Zarizeni bez Venusu (vychozi QEMU s `--gpu 2d`, Windows
//! QEMU bez render serveru) = test se preskoci. Venus je jen v
//! `cargo xtask test --qemu-wsl --gpu venus`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::display::{Reply, Request, caps};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Startup, println, time};

use super::{Test, TestResult, fail};

pub const TESTS: &[Test] = &[
    ("venus: jmeno fyzickeho zarizeni Vulkan od hostitele", device_name),
    ("venus: obrazek vyplneny barvou a precteny zpet (pamet, command buffer)", clear_image),
    ("venus: skladani vrstev na GPU shodne s gfx (vkcomp, shader SPIR-V)", compose),
];

/// Spusteny `vk-probe`: kanal s vysledky (radek na test), `None` = test
/// se preskakuje (bez displeje nebo bez Venusu).
struct Probe {
    _proc: libos::Process,
    ch: Channel,
}

static PROBE: spin::Once<Result<Option<Probe>, String>> = spin::Once::new();
/// vk-probe hlasil `skip` (zarizeni nema Venus): dalsi radky neposle.
static SKIPPED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn probe(s: &Startup) -> Result<Option<&'static Probe>, String> {
    PROBE.call_once(|| start_probe(s)).as_ref().map(|p| p.as_ref()).map_err(|e| e.clone())
}

/// Dalsi radek od vk-probe.
fn next_line(p: &Probe) -> Result<String, String> {
    let m = p.ch.read(Some(time::deadline_after(TIMEOUT))).map_err(|e| fail("vysledek vk-probe", e))?;
    String::from_utf8(m.data).map_err(|_| String::from("vysledek neni UTF-8"))
}

/// Prvni volani Vulkanu muze trvat (lavapipe preklada shadery LLVM).
const TIMEOUT: Duration = Duration::from_secs(90);

fn ask(display: &Channel, r: Request) -> Result<(Reply, Vec<libos::Handle>), String> {
    display.write(&r.encode(), Vec::new()).map_err(|e| fail("displej: zapis", e))?;
    let m = display
        .read(Some(time::deadline_after(Duration::from_secs(10))))
        .map_err(|e| fail("displej: odpoved", e))?;
    let reply = Reply::decode(&m.data).map_err(|e| fail("displej: zprava", e))?;
    Ok((reply, m.handles))
}

fn start_probe(s: &Startup) -> Result<Option<Probe>, String> {
    let guard = crate::gui::DISPLAY.lock();
    let Some(display) = guard.as_ref() else {
        println!("init: test vulkan: bez displeje, preskoceno");
        return Ok(None);
    };
    let (Reply::Modes { caps: c, .. }, _) = ask(display, Request::Info)? else {
        return Err("displej: cekal jsem Modes".into());
    };
    if c & caps::VENUS == 0 {
        println!("init: test vulkan: zarizeni nema Venus, preskoceno");
        return Ok(None);
    }
    let (reply, mut handles) = ask(display, Request::OpenRender)?;
    if reply != Reply::Render || handles.len() != 1 {
        return Err(alloc::format!("OPEN_RENDER: {reply:?}"));
    }
    let render = handles.remove(0);
    drop(guard);
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("vk-probe").ok_or("vk-probe neni v initfs")?;
    let (proc, ch) =
        libos::spawn("vk-probe", image, vec![StartHandle::user(render)]).map_err(|e| fail("spusteni vk-probe", e))?;
    Ok(Some(Probe { _proc: proc, ch }))
}

fn device_name(s: &Startup) -> TestResult {
    let Some(p) = probe(s)? else { return Ok(()) };
    let line = next_line(p)?;
    if let Some(why) = line.strip_prefix("skip ") {
        SKIPPED.store(true, core::sync::atomic::Ordering::Relaxed);
        println!("init: test vulkan: {why}, preskoceno");
        return Ok(());
    }
    let rest = line.strip_prefix("ok ").ok_or_else(|| alloc::format!("vk-probe: {line}"))?;
    let mut parts = rest.split('|');
    let count: u32 = parts.next().and_then(|n| n.parse().ok()).ok_or("vk-probe: pocet zarizeni")?;
    let name = parts.next().unwrap_or("");
    crate::check!(count >= 1, "aspon jedno fyzicke zarizeni");
    crate::check!(!name.is_empty() && name != "?", "jmeno zarizeni");
    println!("init: test vulkan: fyzicke zarizeni Vulkan: {name} ({rest})");
    Ok(())
}

fn clear_image(s: &Startup) -> TestResult {
    let Some(p) = probe(s)? else { return Ok(()) };
    // Po `skip` vk-probe nic dalsiho neposle a skonci.
    if SKIPPED.load(core::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }
    let line = next_line(p)?;
    let rest = line.strip_prefix("ok ").ok_or_else(|| alloc::format!("vk-probe: obrazek: {line}"))?;
    println!("init: test vulkan: obrazek: {rest}");
    Ok(())
}

fn compose(s: &Startup) -> TestResult {
    let Some(p) = probe(s)? else { return Ok(()) };
    if SKIPPED.load(core::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }
    let line = next_line(p)?;
    let rest = line.strip_prefix("ok ").ok_or_else(|| alloc::format!("vk-probe: skladani: {line}"))?;
    println!("init: test vulkan: skladani: {rest}");
    Ok(())
}
