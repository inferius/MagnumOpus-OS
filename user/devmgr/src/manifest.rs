//! Ovladace z manifestu (`<jmeno>.driver.toml` v initfs, viz
//! `driverkit::manifest`): spusteni pro zarizeni, ktera nema vestaveny
//! ovladac, a obnova po padu.
//!
//! Ovladac dostane Resource jen na BARy z `resources.bars` a stranku sve
//! konfigurace, kvoty `irq_max` a `dma_max`. Initu se posle `dev <jmeno>`
//! s kanalem ke klientovi ovladace. Kdyz ovladac skonci:
//!
//! - kod 0 (klient zavrel kanal): konec,
//! - `EXIT_RESTART` (`dk:restart`): hned znovu, nepocita se,
//! - jinak (pad, chyba): znovu, pokud manifest dovoluje (`service.restart`)
//!   a nebylo uz `max_restarts` padu. Pred restartem devmgr vypne zarizeni
//!   bus master: rozbehnute DMA spadleho ovladace se zastavi.
//!
//! Novy ovladac dostane novy Resource (kvota DMA plati za dobu zivota
//! Resource) a init novy kanal (`dev <jmeno>` znovu).

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use driverkit::config::{Bar, DriverConfig};
use driverkit::manifest::{self, Manifest};
use driverkit::proto::EXIT_RESTART;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Handle, Process, Result, Signals, Startup, println};
use pci::{Ecam, Function};

/// Ovladac z manifestu pro jedno zarizeni.
struct Instance {
    manifest: usize,
    function: Function,
    /// Okno ECAM ve startovni zprave (pro vypnuti bus master po padu).
    window: usize,
    segment: u16,
    ranges: Vec<(u64, u64)>,
    config_phys: u64,
    bars: Vec<Bar>,
    process: Option<Process>,
    crashes: u32,
}

/// Vsechny ovladace z manifestu.
pub struct Drivers {
    manifests: Vec<Manifest>,
    instances: Vec<Instance>,
}

impl Drivers {
    /// Nacte manifesty z initfs. Vadny manifest se zaloguje a preskoci.
    pub fn load(fs: &Initfs) -> Self {
        let mut manifests = Vec::new();
        for file in fs.names() {
            let Some(stem) = manifest::driver_of_file(file) else { continue };
            let text = core::str::from_utf8(fs.find(file).unwrap_or(&[])).unwrap_or("");
            match manifest::parse(text) {
                Ok(m) if m.name != stem => {
                    println!("devmgr: {file}: driver.name '{}' se lisi od jmena souboru", m.name)
                }
                Ok(m) if fs.find(&m.name).is_none() => {
                    println!("devmgr: {file}: program '{}' neni v initfs", m.name)
                }
                Ok(m) => manifests.push(m),
                Err(e) => println!("devmgr: {file}: {e}"),
            }
        }
        Self {
            manifests,
            instances: Vec::new(),
        }
    }

    /// Spusti ovladace z manifestu pro funkce okna `window`, ktere nema
    /// vestaveny ovladac (`claimed`).
    #[allow(clippy::too_many_arguments)]
    pub fn claim(
        &mut self,
        startup: &Startup,
        fs: &Initfs,
        ecam: &Ecam,
        window: usize,
        functions: &[Function],
        claimed: impl Fn(&Function) -> bool,
        parent: &Channel,
    ) {
        let (vmo, arg) = &startup.pci_ecam[window];
        for f in functions.iter().filter(|f| !claimed(f)) {
            let Some(mi) = self.manifests.iter().position(|m| {
                m.matches(f.vendor, f.device, f.class, f.subclass, f.prog_if)
            }) else {
                continue;
            };
            let m = &self.manifests[mi];
            let (ranges, config_phys, bars) = match ranges_for(ecam, vmo, startup, f, m) {
                Ok(r) => r,
                Err(e) => {
                    println!("devmgr: ovladac {} pro {}: prostredky: {e}", m.name, f.address);
                    continue;
                }
            };
            let mut inst = Instance {
                manifest: mi,
                function: *f,
                window,
                segment: (*arg >> 16) as u16,
                ranges,
                config_phys,
                bars,
                process: None,
                crashes: 0,
            };
            match spawn(&self.manifests[mi], &inst, startup, fs, parent) {
                Ok(p) => {
                    println!("devmgr: ovladac {} pro {} (manifest)", self.manifests[mi].name, f.address);
                    inst.process = Some(p);
                }
                Err(e) => println!(
                    "devmgr: ovladac {} pro {} nelze spustit: {e}",
                    self.manifests[mi].name, f.address
                ),
            }
            self.instances.push(inst);
        }
    }

    /// Zaloguje manifesty, pro ktere se nenaslo zadne zarizeni.
    pub fn report_unused(&self) {
        for (i, m) in self.manifests.iter().enumerate() {
            if !self.instances.iter().any(|x| x.manifest == i) {
                println!("devmgr: ovladac {}: zadne zarizeni podle manifestu", m.name);
            }
        }
    }

    /// Handly bezicich ovladacu pro cekani (`TERMINATED`).
    pub fn wait_items(&self) -> Vec<(&Handle, Signals)> {
        self.instances
            .iter()
            .filter_map(|i| i.process.as_ref())
            .map(|p| (p.handle(), Signals::TERMINATED))
            .collect()
    }

    /// Po cekani: ovladace, ktere skoncily, obnovi podle manifestu.
    /// `observed` jsou signaly v poradi [`wait_items`](Self::wait_items).
    pub fn reap(&mut self, observed: &[Signals], startup: &Startup, fs: &Initfs, parent: &Channel) {
        let running: Vec<usize> = (0..self.instances.len())
            .filter(|&i| self.instances[i].process.is_some())
            .collect();
        for (k, &i) in running.iter().enumerate() {
            if !observed.get(k).is_some_and(|s| s.intersects(Signals::TERMINATED)) {
                continue;
            }
            let inst = &mut self.instances[i];
            let code = inst.process.take().and_then(|p| p.wait_exit(Some(0)).ok()).unwrap_or(-1);
            let m = &self.manifests[inst.manifest];
            let a = inst.function.address;
            stop_dma(startup, inst.window, a);
            let again = match code {
                0 => {
                    println!("devmgr: ovladac {} pro {a} skoncil", m.name);
                    false
                }
                EXIT_RESTART => {
                    println!("devmgr: ovladac {} pro {a}: restart na zadost", m.name);
                    true
                }
                c => {
                    inst.crashes += 1;
                    let why = driverkit::proto::describe_exit(c);
                    if !m.restart || inst.crashes > m.max_restarts {
                        println!(
                            "devmgr: ovladac {} pro {a} spadl (kod {c}: {why}), {} padu: dal ho nespoustim",
                            m.name, inst.crashes
                        );
                        false
                    } else {
                        println!(
                            "devmgr: ovladac {} pro {a} spadl (kod {c}: {why}), obnova {}/{}",
                            m.name, inst.crashes, m.max_restarts
                        );
                        true
                    }
                }
            };
            if again {
                match spawn(m, inst, startup, fs, parent) {
                    Ok(p) => inst.process = Some(p),
                    Err(e) => println!("devmgr: ovladac {} pro {a} nelze obnovit: {e}", m.name),
                }
            }
        }
    }
}

/// Vypne bus master (a tim DMA) zarizeni po konci ovladace.
fn stop_dma(startup: &Startup, window: usize, a: pci::Address) {
    let (vmo, arg) = &startup.pci_ecam[window];
    if let Ok(ecam) = Ecam::new(vmo, *arg) {
        let cmd = ecam.read16(a, pci::COMMAND);
        ecam.write16(a, pci::COMMAND, cmd & !pci::COMMAND_BUS_MASTER);
    }
}

/// Rozsahy pro Resource (zacatek, delka), fyzicka adresa stranky
/// konfigurace a BARy pro zpravu ovladaci.
type Ranges = (Vec<(u64, u64)>, u64, Vec<Bar>);

/// Rozsahy pro Resource ovladace: BARy z manifestu a stranka konfigurace.
fn ranges_for(
    ecam: &Ecam,
    ecam_vmo: &libos::Vmo,
    startup: &Startup,
    f: &Function,
    m: &Manifest,
) -> Result<Ranges> {
    let resource = startup.root_resource.as_ref().ok_or(libos::Error::NotFound)?;
    let page = libos::vm::page_size();
    let a = f.address;
    let mut ranges = Vec::new();
    let mut bars = Vec::new();
    for &index in &m.bars {
        let (Some(bar), Some(size)) = (ecam.memory_bar(a, index), ecam.memory_bar_size(a, index)) else {
            println!("devmgr: {}: zarizeni {a} nema pametovy BAR {index}", m.name);
            continue;
        };
        if bar.phys == 0 {
            continue;
        }
        let start = bar.phys & !(page - 1);
        let end = bar.phys.saturating_add(size).next_multiple_of(page);
        ranges.push((start, end - start));
        bars.push(Bar {
            index,
            phys: bar.phys,
            size,
        });
    }
    let offset = ecam.config_offset(a).ok_or(libos::Error::InvalidArgs)?;
    let config = resource.phys_addr(ecam_vmo, offset)?;
    ranges.push((config, pci::CONFIG_SIZE));
    Ok((ranges, config, bars))
}

/// Spusti proces ovladace a initu posle `dev <jmeno>` s kanalem.
fn spawn(m: &Manifest, inst: &Instance, startup: &Startup, fs: &Initfs, parent: &Channel) -> Result<Process> {
    let resource = startup.root_resource.as_ref().ok_or(libos::Error::NotFound)?;
    let image = fs.find(&m.name).ok_or(libos::Error::NotFound)?;
    let a = inst.function.address;
    let id = (inst.segment, a.bus, a.device, a.function);
    let device = resource.restrict_device(&inst.ranges, m.irq_max, m.dma_max, id)?;
    let config = device.device_memory(inst.config_phys, pci::CONFIG_SIZE)?;
    let (client_end, driver_end) = Channel::create()?;
    let handles = vec![
        StartHandle::user(device.handle().duplicate(None)?),
        StartHandle::user(config.handle().duplicate(None)?),
        StartHandle::user(driver_end.into_handle()),
    ];
    let (process, control) = libos::spawn(&m.name, image, handles)?;
    let cfg = DriverConfig {
        bus: a.bus,
        device: a.device,
        function: a.function,
        bars: inst.bars.clone(),
    };
    control.write(driverkit::config::format(&cfg).as_bytes(), Vec::new())?;
    parent.write(format!("dev {}", m.name).as_bytes(), vec![client_end.into_handle()])?;
    Ok(process)
}
