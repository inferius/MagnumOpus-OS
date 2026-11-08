//! Manifest ovladace: `user/<jmeno>/<jmeno>.driver.toml`.
//!
//! Manifest rika spravci zarizeni, pro jaka zarizeni ovladac spustit a co
//! mu smi dat (nejmensi prava). xtask ho pribali do initfs, devmgr ho cte
//! pri startu; novy ovladac tak nepotrebuje zadnou zmenu v devmgr.
//!
//! ```toml
//! [driver]
//! name = "edu"                       # jmeno programu v initfs
//! description = "QEMU edu (vyukove zarizeni)"
//! license = "MIT OR Apache-2.0"
//! origin = "vlastni"                 # odkud kod je (prevzato z ... + licence)
//!
//! [match]
//! pci = ["1234:11e8"]                # vendor:device (hex)
//! class = ["ff.00.00"]               # trida.podtrida.rozhrani (hex)
//!
//! [resources]
//! bars = [0]                         # BARy, ktere ovladac dostane
//! irq_max = 1                        # kolik preruseni smi zapnout
//! dma_max = "64K"                    # kolik DMA pameti smi alokovat
//!
//! [service]
//! restart = true                     # po padu spustit znovu
//! max_restarts = 5                   # nejvys tolikrat za beh systemu
//! ```
//!
//! Chyby nesou cestu klice a radu (`match.pci[0]: ocekavano "vvvv:dddd"`).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Pravidlo, pro ktere zarizeni ovladac je.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    Pci { vendor: u16, device: u16 },
    Class { class: u8, subclass: u8, prog_if: u8 },
}

/// Nacteny manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub description: String,
    pub license: String,
    pub origin: String,
    pub matches: Vec<Match>,
    pub bars: Vec<u8>,
    pub irq_max: u32,
    pub dma_max: u64,
    pub restart: bool,
    pub max_restarts: u32,
}

/// Nejvys DMA pameti pro jeden ovladac (ochrana pred preklepem v manifestu).
pub const DMA_LIMIT: u64 = 256 << 20;
/// Nejvys preruseni pro jeden ovladac.
pub const IRQ_LIMIT: u32 = 64;

impl Manifest {
    /// Hodi se ovladac na funkci PCI?
    pub fn matches(&self, vendor: u16, device: u16, class: u8, subclass: u8, prog_if: u8) -> bool {
        self.matches.iter().any(|m| match *m {
            Match::Pci { vendor: v, device: d } => v == vendor && d == device,
            Match::Class {
                class: c,
                subclass: s,
                prog_if: p,
            } => c == class && s == subclass && p == prog_if,
        })
    }

    /// Jmeno souboru manifestu v initfs.
    pub fn file_name(name: &str) -> String {
        format!("{name}.driver.toml")
    }
}

/// Je to jmeno souboru manifestu ovladace? Vraci jmeno ovladace.
pub fn driver_of_file(file: &str) -> Option<&str> {
    file.strip_suffix(".driver.toml").filter(|n| !n.is_empty())
}

/// Jmeno ovladace: mala pismena, cislice, `-`; 1-32 znaku.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
}

/// Precte manifest z textu TOML.
pub fn parse(text: &str) -> Result<Manifest, String> {
    let t = conf::parse(text).map_err(|e| format!("TOML: {e}"))?;
    let name = t
        .get_str("driver.name")
        .ok_or("chybi driver.name (jmeno programu v initfs)")?;
    if !valid_name(name) {
        return Err(format!(
            "driver.name '{name}': jen mala pismena, cislice a '-', nejvys 32 znaku"
        ));
    }
    let mut matches = Vec::new();
    if let Some(v) = t.get("match.pci") {
        let arr = v.as_array().ok_or("match.pci: ocekavano pole retezcu")?;
        for (i, s) in arr.iter().enumerate() {
            let s = s
                .as_str()
                .ok_or_else(|| format!("match.pci[{i}]: ocekavan retezec"))?;
            matches.push(parse_pci(s).ok_or_else(|| {
                format!("match.pci[{i}] '{s}': ocekavano \"vvvv:dddd\" (hex)")
            })?);
        }
    }
    if let Some(v) = t.get("match.class") {
        let arr = v.as_array().ok_or("match.class: ocekavano pole retezcu")?;
        for (i, s) in arr.iter().enumerate() {
            let s = s
                .as_str()
                .ok_or_else(|| format!("match.class[{i}]: ocekavan retezec"))?;
            matches.push(parse_class(s).ok_or_else(|| {
                format!("match.class[{i}] '{s}': ocekavano \"tt.pp.rr\" (hex)")
            })?);
        }
    }
    if matches.is_empty() {
        return Err("prazdny [match]: ovladac by nikdy nebezel (match.pci nebo match.class)".into());
    }
    let mut bars = Vec::new();
    if let Some(v) = t.get("resources.bars") {
        let arr = v.as_array().ok_or("resources.bars: ocekavano pole cisel")?;
        for (i, b) in arr.iter().enumerate() {
            match b.as_int() {
                Some(n @ 0..=5) => bars.push(n as u8),
                _ => return Err(format!("resources.bars[{i}]: BAR je cislo 0-5")),
            }
        }
    }
    let irq_max = match t.get_int("resources.irq_max") {
        None => 1,
        Some(n) if (0..=IRQ_LIMIT as i64).contains(&n) => n as u32,
        Some(n) => return Err(format!("resources.irq_max = {n}: povoleno 0-{IRQ_LIMIT}")),
    };
    let dma_max = match t.get("resources.dma_max") {
        None => 0,
        Some(v) => match conf::parse_size(v) {
            Some(n) if n <= DMA_LIMIT => n,
            _ => {
                return Err(format!(
                    "resources.dma_max: velikost (napr. 65536 nebo \"64K\") nejvys {} MiB",
                    DMA_LIMIT >> 20
                ));
            }
        },
    };
    let max_restarts = match t.get_int("service.max_restarts") {
        None => 5,
        Some(n) if (0..=1000).contains(&n) => n as u32,
        Some(n) => return Err(format!("service.max_restarts = {n}: povoleno 0-1000")),
    };
    let text = |k: &str| t.get_str(k).unwrap_or("").to_string();
    let m = Manifest {
        name: name.to_string(),
        description: text("driver.description"),
        license: text("driver.license"),
        origin: text("driver.origin"),
        matches,
        bars,
        irq_max,
        dma_max,
        restart: t.get_bool("service.restart").unwrap_or(true),
        max_restarts,
    };
    if m.license.is_empty() || m.origin.is_empty() {
        return Err("driver.license a driver.origin jsou povinne (pravidla zdroju ovladacu)".into());
    }
    Ok(m)
}

fn hex16(s: &str) -> Option<u16> {
    (s.len() == 4).then(|| u16::from_str_radix(s, 16).ok())?
}

fn hex8(s: &str) -> Option<u8> {
    (s.len() == 2).then(|| u8::from_str_radix(s, 16).ok())?
}

/// `vvvv:dddd` -> pravidlo PCI ID.
pub fn parse_pci(s: &str) -> Option<Match> {
    let (v, d) = s.split_once(':')?;
    Some(Match::Pci {
        vendor: hex16(v)?,
        device: hex16(d)?,
    })
}

fn parse_class(s: &str) -> Option<Match> {
    let mut p = s.split('.');
    let m = Match::Class {
        class: hex8(p.next()?)?,
        subclass: hex8(p.next()?)?,
        prog_if: hex8(p.next()?)?,
    };
    p.next().is_none().then_some(m)
}
