//! Ovladace z manifestu: hledani `user/*/<jmeno>.driver.toml`, zarizeni pro
//! test v QEMU a generator kostry noveho ovladace (`cargo xtask
//! new-driver`).
//!
//! Ovladac s manifestem nepotrebuje zadny zapis v xtask ani v devmgr:
//! xtask ho najde, prelozi, prida do initfs (program i manifest) a do testu
//! na hostiteli. Viz `docs/cs/ovladace.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use driverkit::manifest::{self, Manifest};

use crate::Result;

/// Ovladac nalezeny v `user/`.
pub struct Found {
    pub manifest: Manifest,
    /// Cesta k manifestu (pro initfs a hlasky).
    pub path: PathBuf,
}

/// Najde a zkontroluje manifesty ovladacu. Chyba manifestu je chyba buildu
/// (lepsi nez ovladac, ktery v systemu tise nebezi).
pub fn discover(root: &Path) -> Result<Vec<Found>> {
    let mut out = Vec::new();
    let mut dirs: Vec<PathBuf> = fs::read_dir(root.join("user"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let path = dir.join(Manifest::file_name(&name));
        if !path.exists() {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        let m = manifest::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if m.name != name {
            return Err(format!(
                "{}: driver.name '{}' se musi shodovat se jmenem adresare '{name}'",
                path.display(),
                m.name
            )
            .into());
        }
        out.push(Found { manifest: m, path });
    }
    Ok(out)
}

/// Ma QEMU zarizeni `device` (`-device help`)? QEMU ve WSL se nezkouma
/// (vraci `false`).
pub fn qemu_has_device(qemu: &Path, device: &str) -> bool {
    let Ok(out) = Command::new(qemu).args(["-device", "help"]).output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.contains(&format!("name \"{device}\""))
}

/// Argumenty QEMU pro testovaci zarizeni ovladacu z manifestu. `edu` je
/// vyukove zarizeni QEMU pro ukazkovy ovladac; DMA maska 64 bitu, at IOVA
/// z IOMMU nebo fyzicka adresa nad 256 MiB projde.
pub const EDU_DEVICE: &str = "edu,dma_mask=0xffffffffffffffff";

/// Obsah `driver-test` v initfs (viz `user/init/src/tests/driverkit.rs`):
/// ocekavane ovladace, jediny ovladac (`--driver`) a emulovana IOMMU
/// (`--iommu`; `none` = testy ochrany IOMMU se preskoci).
pub fn test_plan(expect: &[&str], only: Option<&str>, iommu: &str) -> String {
    format!("expect = {}\nonly = {}\niommu = {iommu}\n", expect.join(","), only.unwrap_or(""))
}

// --- Generator kostry -----------------------------------------------------

/// Parametry `new-driver`.
pub struct NewDriver {
    pub name: String,
    pub pci: (u16, u16),
}

/// Rozebere argumenty `new-driver <jmeno> --pci vvvv:dddd`.
pub fn parse_new_driver(args: &[String]) -> Result<NewDriver> {
    let usage = "pouziti: cargo xtask new-driver <jmeno> --pci <vendor:device>";
    let mut name = None;
    let mut pci = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pci" => {
                let v = it.next().ok_or(usage)?;
                pci = Some(match manifest::parse_pci(v) {
                    Some(manifest::Match::Pci { vendor, device }) => (vendor, device),
                    _ => return Err(format!("--pci '{v}': ocekavano vvvv:dddd (hex)").into()),
                });
            }
            n if !n.starts_with('-') && name.is_none() => name = Some(n.to_string()),
            other => return Err(format!("neznamy argument {other:?}\n{usage}").into()),
        }
    }
    let name = name.ok_or(usage)?;
    if !manifest::valid_name(&name) {
        return Err(format!("jmeno '{name}': mala pismena, cislice a '-', nejvys 32 znaku").into());
    }
    Ok(NewDriver {
        name,
        pci: pci.ok_or(usage)?,
    })
}

/// Soubory kostry: (cesta v repozitari, obsah).
pub fn skeleton(d: &NewDriver) -> Vec<(String, String)> {
    let ident = d.name.replace('-', "_");
    let ty: String = d
        .name
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect();
    let ty = if ty.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("Drv{ty}")
    } else {
        ty
    };
    let fill = |t: &str| {
        t.replace("{{name}}", &d.name)
            .replace("{{ident}}", &ident)
            .replace("{{Type}}", &ty)
            .replace("{{vendor}}", &format!("{:04x}", d.pci.0))
            .replace("{{device}}", &format!("{:04x}", d.pci.1))
    };
    let n = &d.name;
    vec![
        (format!("user/{n}/Cargo.toml"), fill(T_CARGO)),
        (format!("user/{n}/{n}.driver.toml"), fill(T_MANIFEST)),
        (format!("user/{n}/src/lib.rs"), fill(T_LIB)),
        (format!("user/{n}/src/main.rs"), fill(T_MAIN)),
        (format!("user/{n}/src/model.rs"), fill(T_MODEL)),
        (format!("user/{n}/src/tests.rs"), fill(T_TESTS)),
        (format!("docs/cs/ovladac-{n}.md"), fill(T_DOC_CS)),
        (format!("docs/en/driver-{n}.md"), fill(T_DOC_EN)),
    ]
}

/// Vytvori kostru v repozitari `root`: soubory, clen workspace, radek
/// v `docs/README.md`. Existujici adresar ovladace je chyba (nic se
/// neprepise).
pub fn new_driver(root: &Path, d: &NewDriver) -> Result<()> {
    let dir = root.join("user").join(&d.name);
    if dir.exists() {
        return Err(format!("{} uz existuje", dir.display()).into());
    }
    for (path, content) in skeleton(d) {
        let p = root.join(&path);
        fs::create_dir_all(p.parent().unwrap())?;
        fs::write(&p, content)?;
        println!("xtask: vytvoren {path}");
    }
    add_workspace_member(&root.join("Cargo.toml"), &format!("user/{}", d.name))?;
    add_docs_row(&root.join("docs/README.md"), &d.name)?;
    println!(
        "xtask: hotovo. Dalsi kroky:\n  cargo test -p {n}            # test v simulaci\n  cargo xtask test --driver {n}  # build, QEMU, test jen tohoto ovladace\n  prirucka: docs/cs/ovladace.md",
        n = d.name
    );
    Ok(())
}

/// Prida clena do `[workspace] members` (pred `"xtask"`).
fn add_workspace_member(cargo_toml: &Path, member: &str) -> Result<()> {
    let text = fs::read_to_string(cargo_toml)?;
    let line = format!("    \"{member}\",\n");
    if text.contains(&line) {
        return Ok(());
    }
    let anchor = "    \"xtask\",\n]";
    let pos = text
        .find(anchor)
        .ok_or("Cargo.toml: nenalezen konec seznamu members (\"xtask\")")?;
    let mut out = text.clone();
    out.insert_str(pos, &line);
    fs::write(cargo_toml, out)?;
    println!("xtask: Cargo.toml: pridan clen {member}");
    Ok(())
}

/// Prida stranku ovladace do tabulky skupiny "Ovladače / Drivers".
fn add_docs_row(readme: &Path, name: &str) -> Result<()> {
    let text = fs::read_to_string(readme)?;
    let row = format!(
        "| [Ovladač {name}](cs/ovladac-{name}.md) | [Driver {name}](en/driver-{name}.md) | zařízení, protokol, testy / device, protocol, tests |\n"
    );
    let group = text
        .find("### Ovladače / Drivers")
        .ok_or("docs/README.md: chybi skupina '### Ovladače / Drivers'")?;
    // Konec tabulky skupiny = prvni prazdny radek po zacatku tabulky.
    let table = text[group..].find("\n|").ok_or("docs/README.md: skupina ovladacu bez tabulky")? + group + 1;
    let end = text[table..].find("\n\n").map_or(text.len(), |e| table + e + 1);
    let mut out = text.clone();
    out.insert_str(end, &row);
    fs::write(readme, out)?;
    println!("xtask: docs/README.md: pridana stranka ovladace");
    Ok(())
}

const T_CARGO: &str = r#"[package]
name = "{{name}}"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false
description = "Ovladac zarizeni PCI {{vendor}}:{{device}} (driverkit)"

[[bin]]
name = "{{name}}"
test = false
bench = false

[dependencies]
driverkit = { path = "../../libs/driverkit" }

# Program pro jadro OS: backend driverkitu nad jadrem.
[target.'cfg(target_os = "magnumopus")'.dependencies]
driverkit = { path = "../../libs/driverkit", features = ["os"] }
libos = { path = "../../libs/libos" }

# Testy na hostiteli: simulace zarizeni.
[dev-dependencies]
driverkit = { path = "../../libs/driverkit", features = ["sim"] }
"#;

const T_MANIFEST: &str = r#"# Manifest ovladace (docs/cs/ovladace.md, "Manifest").
[driver]
name = "{{name}}"
description = "TODO: co je to za zarizeni"
license = "MIT OR Apache-2.0"
# TODO: "vlastni", nebo odkud je kod prevzaty (projekt, verze, licence).
# Kod pod GPL neprekladat: z nej jen fakta o hardwaru.
origin = "vlastni"

[match]
pci = ["{{vendor}}:{{device}}"]

[resources]
# Jen to, co ovladac opravdu potrebuje (nejmensi prava).
bars = [0]
irq_max = 1
dma_max = "64K"

[service]
restart = true
max_restarts = 5
"#;

const T_LIB: &str = r#"//! {{name}}: ovladac zarizeni PCI {{vendor}}:{{device}}.
//!
//! TODO: popis zarizeni, odkaz na datasheet, puvod kodu a licence
//! (pravidla zdroju: docs/cs/ovladace.md).
//!
//! Protokol pro klienta:
//!
//! | zprava | odpoved                     |
//! |--------|-----------------------------|
//! | `id`   | `id <hodnota registru ID>`  |

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[cfg(test)]
mod model;
#[cfg(test)]
mod tests;

use alloc::format;

use driverkit::{DmaBuffer, Driver, IrqId, Platform, Result};

driverkit::registers! {
    /// TODO: registry podle datasheetu.
    pub ID @ 0x00: u32;
    pub STATUS @ 0x04: u32 {
        /// Zarizeni dokoncilo praci (zapis 1 maze).
        DONE: 0,
    }
}

/// Ovladac (genericky: v systemu nad jadrem, v testech nad simulaci).
pub struct {{Type}}<P: Platform> {
    regs: P::Mmio,
    #[allow(dead_code)]
    irq: IrqId,
    #[allow(dead_code)]
    dma: DmaBuffer<P::Dma>,
}

impl<P: Platform> Driver<P> for {{Type}}<P> {
    fn probe(p: &P) -> Result<Self> {
        let regs = p.map_bar(0)?;
        // TODO: reset zarizeni a kontrola identifikace.
        let irq = p.enable_irq()?;
        let dma = p.alloc_dma("data", 4096)?;
        driverkit::log!(p, "zarizeni nalezeno, ID {:#x}", ID.read(&regs));
        Ok(Self { regs, irq, dma })
    }

    fn message(&mut self, p: &P, msg: &[u8]) -> Result<()> {
        let reply = match msg {
            b"id" => format!("id {:#x}", ID.read(&self.regs)),
            _ => "chyba neznamy prikaz".into(),
        };
        p.send(reply.as_bytes())?;
        Ok(())
    }

    fn interrupt(&mut self, _p: &P, _irq: IrqId) -> Result<()> {
        if STATUS::DONE.is_set(&self.regs) {
            // Potvrdit (W1C) a zpracovat dokoncenou praci.
            STATUS.write(&self.regs, STATUS::DONE.value(1)?);
        }
        Ok(())
    }

    fn stop(&mut self, _p: &P) {
        // TODO: zastavit zarizeni (zadne DMA ani preruseni po navratu).
    }
}
"#;

const T_MAIN: &str = r#"//! Program ovladace {{name}}: start nad jadrem. Logika je v knihovne
//! (`src/lib.rs`), aby sla testovat v simulaci na hostiteli.

#![cfg_attr(target_os = "magnumopus", no_std, no_main)]

#[cfg(target_os = "magnumopus")]
libos::entry!(main);

#[cfg(target_os = "magnumopus")]
fn main(startup: libos::Startup) -> i64 {
    driverkit::os::main::<{{ident}}::{{Type}}<_>>(startup, "{{name}}")
}

/// Na hostiteli program nic nedela (testy jsou v knihovne).
#[cfg(not(target_os = "magnumopus"))]
fn main() {}
"#;

const T_MODEL: &str = r#"//! Model zarizeni {{vendor}}:{{device}} pro testy na hostiteli.
//! TODO: doplnit chovani podle datasheetu (jen to, co ovladac pouziva).

use driverkit::sim::{Bus, DeviceModel, RegFile};

pub const ID_VALUE: u64 = 0x{{device}}{{vendor}};

pub struct Model {
    regs: RegFile,
}

impl Model {
    pub fn new() -> Self {
        Self {
            regs: RegFile::new().ro(0x00, ID_VALUE).reg(0x04, 0, 0, 1),
        }
    }
}

impl DeviceModel for Model {
    fn bar_size(&self, index: u8) -> Option<u64> {
        (index == 0).then_some(0x1000)
    }

    fn read(&mut self, _: &mut Bus, _: u8, off: u64, _: u8) -> u64 {
        self.regs.read(off)
    }

    fn write(&mut self, _: &mut Bus, _: u8, off: u64, _: u8, v: u64) {
        self.regs.write(off, v);
    }

    fn as_any(&mut self) -> &mut dyn core::any::Any {
        self
    }
}
"#;

const T_TESTS: &str = r#"//! Testy ovladace {{name}} v simulaci: `cargo test -p {{name}}`.

use driverkit::sim::Sim;
use driverkit::{DeviceInfo, Exit, run};

use crate::{{Type}};
use crate::model::{ID_VALUE, Model};

fn sim() -> Sim {
    let info = DeviceInfo { bus: 0, device: 5, function: 0, vendor: 0x{{vendor}}, device_id: 0x{{device}} };
    let manifest = driverkit::manifest::parse(include_str!("../{{name}}.driver.toml")).unwrap();
    Sim::new("{{name}}", info, Model::new()).with_manifest(&manifest)
}

#[test]
fn probe_and_reply() {
    let sim = sim();
    sim.send(b"id");
    sim.send(b"dk:ping");
    sim.close();
    assert_eq!(run::<_, {{Type}}<_>>(&sim.platform()), Exit::ClientClosed);
    assert_eq!(sim.sent_text(), [format!("id {ID_VALUE:#x}"), "dk:pong {{name}}".into()]);
    assert!(sim.faults().is_empty(), "DMA mimo pridelenou pamet: {:?}", sim.faults());
}
"#;

const T_DOC_CS: &str = r#"# Ovladač {{name}}

Ovladač zařízení PCI `{{vendor}}:{{device}}` na sadě driverkit (viz
[Ovladače](ovladace.md)).

TODO: co zařízení dělá, odkud jsou fakta (datasheet), původ kódu a licence.

## Protokol

| Zpráva | Odpověď |
|---|---|
| `id` | `id <hodnota registru ID>` |
| `dk:ping` | `dk:pong {{name}}` (společné pro všechny ovladače) |

## Testy

- v simulaci: `cargo test -p {{name}}`,
- v QEMU: `cargo xtask test --driver {{name}}` (když QEMU zařízení nemá,
  test v QEMU ovladač jen přeskočí).
"#;

const T_DOC_EN: &str = r#"# Driver {{name}}

Driver for the PCI device `{{vendor}}:{{device}}` on the driverkit (see
[Drivers](drivers.md)).

TODO: what the device does, where the facts come from (datasheet), code
origin and licence.

## Protocol

| Message | Reply |
|---|---|
| `id` | `id <value of the ID register>` |
| `dk:ping` | `dk:pong {{name}}` (common to all drivers) |

## Tests

- in the simulation: `cargo test -p {{name}}`,
- in QEMU: `cargo xtask test --driver {{name}}` (when QEMU lacks the
  device, the QEMU test just skips the driver).
"#;
