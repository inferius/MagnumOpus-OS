//! Restart spadlych ovladacu (zatim zvukove karty `hda`).
//!
//! Vlastni vlakno ceka na konec procesu hlidanych ovladacu. Kdyz ovladac
//! skonci, spusti ho znovu nad stejnou funkci PCI a initu posle novy kanal
//! (zpravou jako pri startu, napr. `audio`); init ho preda sluzbe.
//! Nejvic [`MAX_RESTARTS`] za [`WINDOW`], pak to vzda (ovladac, ktery
//! pada hned po startu, nesmi zahltit system). Pred restartem vypne
//! zarizeni bus master: DMA spadleho ovladace se zastavi.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::hw::Resource;
use libos::{Channel, Process, Signals, Vmo, println, time};
use pci::{Ecam, Function};

use crate::Driver;

pub const MAX_RESTARTS: usize = 5;
pub const WINDOW: Duration = Duration::from_secs(60);

/// Hlidany ovladac: proces a co je potreba k novemu startu.
pub struct Watched {
    pub driver: &'static Driver,
    pub image: &'static [u8],
    pub function: Function,
    /// Index okna ECAM (`Startup::pci_ecam`).
    pub window: usize,
    pub process: Process,
}

/// Prostredky vlakna (duplikaty handlu z hlavniho vlakna).
pub struct Ctx {
    pub resource: Resource,
    /// Okna ECAM: (VMO, argument ze startovni zpravy).
    pub windows: Vec<(Vmo, u32)>,
    pub parent: Channel,
}

/// Spusti vlakno, ktere hlida `list` (prazdny seznam = nic).
pub fn spawn(list: Vec<Watched>, ctx: Ctx) {
    if list.is_empty() {
        return;
    }
    if let Err(e) = libos::thread::spawn("devmgr-restart", move || run(list, ctx)) {
        println!("devmgr: vlakno restartu ovladacu nejde spustit: {e}");
    }
}

fn run(mut list: Vec<Watched>, ctx: Ctx) {
    let ecams: Vec<Option<Ecam>> = ctx.windows.iter().map(|(v, a)| Ecam::new(v, *a).ok()).collect();
    let mut starts: Vec<Vec<u64>> = vec![Vec::new(); list.len()];
    while !list.is_empty() {
        let items: Vec<_> = list.iter().map(|w| (w.process.handle(), Signals::TERMINATED)).collect();
        let observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(e) => {
                println!("devmgr: cekani na ovladace: {e}");
                return;
            }
        };
        drop(items);
        let mut i = list.len();
        while i > 0 {
            i -= 1;
            if !observed.get(i).is_some_and(|s| s.intersects(Signals::TERMINATED)) {
                continue;
            }
            let w = &list[i];
            let code = libos::sys::process_exit_code(w.process.handle().raw()).unwrap_or(i64::MIN);
            println!(
                "devmgr: ovladac {} pro {} skoncil: {}",
                w.driver.program,
                w.function.address,
                libos::abi::process::describe_exit(code)
            );
            // Rozbehnute DMA spadleho ovladace zastavit (bus master vypnout):
            // radic jinak dal zapisuje do pameti, kterou ovladac uz nema
            // (bez IOMMU poskozeni pameti). Novy ovladac ho zapne znovu.
            if let Some(ecam) = ecams[w.window].as_ref() {
                let a = w.function.address;
                let cmd = ecam.read16(a, pci::COMMAND);
                ecam.write16(a, pci::COMMAND, cmd & !pci::COMMAND_BUS_MASTER);
            }
            let now = time::now();
            starts[i].retain(|&t| now.saturating_sub(t) < WINDOW.as_nanos() as u64);
            let restarted = if starts[i].len() >= MAX_RESTARTS {
                println!("devmgr: ovladac {} pada porad dokola, vzdavam to", w.driver.program);
                None
            } else {
                let window = &ctx.windows[w.window];
                ecams[w.window].as_ref().and_then(|ecam| {
                    crate::start_driver_image(
                        w.driver,
                        w.image,
                        &ctx.resource,
                        &window.0,
                        ecam,
                        (window.1 >> 16) as u16,
                        &w.function,
                        &ctx.parent,
                    )
                    .map_err(|e| println!("devmgr: ovladac {} nejde spustit znovu: {e}", w.driver.program))
                    .ok()
                })
            };
            match restarted {
                Some(p) => {
                    starts[i].push(now);
                    list[i].process = p;
                    println!("devmgr: ovladac {} pro {} spusten znovu", list[i].driver.program, list[i].function.address);
                }
                None => {
                    list.remove(i);
                    starts.remove(i);
                }
            }
        }
    }
}
