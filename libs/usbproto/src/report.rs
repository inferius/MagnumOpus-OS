//! Parser deskriptoru reportu HID (HID 1.11, kapitola 6.2.2).
//!
//! Deskriptor je posloupnost polozek: prefix (velikost dat 0/1/2/4 B,
//! typ, znacka) a data. Typy:
//! - **main**: Input, Output, Feature (pole reportu), Collection, End
//!   Collection,
//! - **global** (plati do zmeny): Usage Page, Logical Minimum/Maximum,
//!   Report Size, Report ID, Report Count, Push, Pop,
//! - **local** (plati do pristi polozky main): Usage, Usage Minimum/Maximum.
//!
//! Vysledek je seznam vstupnich poli ([`Field`]); co znamenaji, resi
//! [`crate::hid_report`]. Parser je obranny: deskriptor ze zarizeni muze byt
//! nesmyslny, proto ma limity na pocet poli, usage a hloubku.

use alloc::vec::Vec;

/// Nejvic poli v deskriptoru.
pub const MAX_FIELDS: usize = 256;
/// Nejvic usage v jednom poli.
pub const MAX_USAGES: usize = 1024;
/// Nejvic bitu jednoho reportu (bez bajtu id).
pub const MAX_REPORT_BITS: u32 = 8 * 1024;

/// Priznaky polozky Input (bity dat).
pub mod flags {
    /// Konstantni (vypln).
    pub const CONSTANT: u32 = 1 << 0;
    /// Promenna (kazdy prvek ma vlastni usage), jinak pole (prvky = indexy
    /// usage, napr. stisknute klavesy).
    pub const VARIABLE: u32 = 1 << 1;
    /// Relativni hodnota (pohyb), jinak absolutni.
    pub const RELATIVE: u32 = 1 << 2;
}

/// Usage: strana (horni 16 bitu) a id (dolni 16 bitu).
pub type Usage = u32;

pub const fn usage(page: u16, id: u16) -> Usage {
    (page as u32) << 16 | id as u32
}

/// Vstupni pole reportu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Id reportu (0 = deskriptor id nepouziva).
    pub report_id: u8,
    /// Bitova pozice prvniho prvku v reportu (bez bajtu id).
    pub bit_offset: u32,
    /// Bitu na prvek.
    pub size: u8,
    pub count: u16,
    pub flags: u32,
    pub logical_min: i32,
    pub logical_max: i32,
    /// Usage prvku (promenna) nebo seznam / rozsah usage pro indexy (pole).
    pub usages: Vec<Usage>,
    /// Rozsah usage (Usage Minimum/Maximum), pokud byl.
    pub usage_range: Option<(Usage, Usage)>,
}

impl Field {
    pub fn is_variable(&self) -> bool {
        self.flags & flags::VARIABLE != 0
    }

    pub fn is_relative(&self) -> bool {
        self.flags & flags::RELATIVE != 0
    }

    /// Usage prvku `i` promenneho pole: z rozsahu, ze seznamu, nebo posledni
    /// usage seznamu (opakuje se pro zbytek prvku).
    pub fn usage_of(&self, i: usize) -> Option<Usage> {
        if let Some((min, max)) = self.usage_range {
            let u = min.checked_add(i as u32)?;
            return (u <= max).then_some(u);
        }
        self.usages.get(i).or(self.usages.last()).copied()
    }

    /// Usage pro hodnotu prvku pole (index od `logical_min`).
    pub fn usage_for_index(&self, value: i32) -> Option<Usage> {
        if value < self.logical_min || value > self.logical_max {
            return None;
        }
        let idx = (value - self.logical_min) as u32;
        if let Some((min, max)) = self.usage_range {
            let u = min.checked_add(idx)?;
            return (u <= max).then_some(u);
        }
        self.usages.get(idx as usize).copied()
    }

    /// Hodnota prvku `i` z reportu (bez bajtu id); se znamenkem, kdyz
    /// logicke minimum je zaporne.
    pub fn value(&self, report: &[u8], i: usize) -> Option<i32> {
        let start = self.bit_offset + i as u32 * self.size as u32;
        let raw = extract(report, start, self.size)?;
        Some(if self.logical_min < 0 {
            sign_extend(raw, self.size)
        } else {
            raw as i32
        })
    }
}

/// Bity `start..start + size` (little-endian, size <= 32).
fn extract(data: &[u8], start: u32, size: u8) -> Option<u32> {
    if size == 0 || size > 32 {
        return None;
    }
    let mut v: u64 = 0;
    let first = (start / 8) as usize;
    let last = ((start + size as u32 - 1) / 8) as usize;
    for (n, b) in data.get(first..=last)?.iter().enumerate() {
        v |= (*b as u64) << (8 * n);
    }
    v >>= start % 8;
    Some((v & ((1u64 << size) - 1)) as u32)
}

fn sign_extend(v: u32, size: u8) -> i32 {
    let shift = 32 - size as u32;
    ((v << shift) as i32) >> shift
}

/// Rozebrany deskriptor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReportDescriptor {
    pub fields: Vec<Field>,
    /// Reporty zacinaji bajtem id.
    pub uses_ids: bool,
}

impl ReportDescriptor {
    /// Nejdelsi vstupni report v bajtech (vcetne bajtu id).
    pub fn max_input_len(&self) -> usize {
        let bits = self
            .fields
            .iter()
            .map(|f| f.bit_offset + f.size as u32 * f.count as u32)
            .max()
            .unwrap_or(0);
        bits.div_ceil(8) as usize + self.uses_ids as usize
    }
}

#[derive(Clone, Copy, Default)]
struct Globals {
    usage_page: u16,
    logical_min: i32,
    logical_max: i32,
    report_size: u32,
    report_id: u8,
    report_count: u32,
}

/// Rozebere deskriptor; `None` = poruseny (nekonecna delka, nesmyslne
/// rozmery, prilis mnoho poli nebo usage).
pub fn parse(d: &[u8]) -> Option<ReportDescriptor> {
    let mut out = ReportDescriptor::default();
    let mut g = Globals::default();
    let mut stack: Vec<Globals> = Vec::new();
    let mut usages: Vec<Usage> = Vec::new();
    let mut umin: Option<Usage> = None;
    let mut umax: Option<Usage> = None;
    // Bitova pozice dalsiho vstupniho pole pro kazde id reportu.
    let mut offsets = [0u32; 256];
    let mut depth = 0usize;
    let mut i = 0;
    while i < d.len() {
        let prefix = d[i];
        if prefix == 0xFE {
            // Dlouha polozka: velikost, znacka, data - nepouzivaji se.
            let size = *d.get(i + 1)? as usize;
            i += 3 + size;
            continue;
        }
        let size = match prefix & 3 {
            3 => 4,
            n => n as usize,
        };
        let data = d.get(i + 1..i + 1 + size)?;
        i += 1 + size;
        let unsigned = data
            .iter()
            .enumerate()
            .fold(0u32, |v, (n, b)| v | (*b as u32) << (8 * n));
        let signed = if size == 0 {
            0
        } else {
            sign_extend(unsigned, (size * 8) as u8)
        };
        let kind = (prefix >> 2) & 3;
        let tag = prefix >> 4;
        // Usage s 4 B dat obsahuje i stranu, jinak plati Usage Page.
        let full = |v: u32| -> Usage {
            if size == 4 {
                v
            } else {
                usage(g.usage_page, v as u16)
            }
        };
        match (kind, tag) {
            // Main.
            (0, 0x8) => {
                // Input.
                let bits = g.report_size.checked_mul(g.report_count)?;
                if g.report_size > 32 || bits > MAX_REPORT_BITS {
                    return None;
                }
                let off = &mut offsets[g.report_id as usize];
                if *off + bits > MAX_REPORT_BITS {
                    return None;
                }
                let flags = unsigned;
                if flags & flags::CONSTANT == 0 && g.report_count > 0 && g.report_size > 0 {
                    if out.fields.len() >= MAX_FIELDS || usages.len() > MAX_USAGES {
                        return None;
                    }
                    let usage_range = match (umin, umax) {
                        (Some(a), Some(b)) if a <= b => Some((a, b)),
                        _ => None,
                    };
                    out.fields.push(Field {
                        report_id: g.report_id,
                        bit_offset: *off,
                        size: g.report_size as u8,
                        count: g.report_count as u16,
                        flags,
                        logical_min: g.logical_min,
                        logical_max: g.logical_max,
                        usages: core::mem::take(&mut usages),
                        usage_range,
                    });
                }
                *off += bits;
                usages.clear();
                umin = None;
                umax = None;
            }
            (0, 0x9) | (0, 0xB) => {
                // Output, Feature: zabiraji jine reporty, jen vymazat lokalni.
                usages.clear();
                umin = None;
                umax = None;
            }
            (0, 0xA) => {
                depth += 1;
                if depth > 32 {
                    return None;
                }
                usages.clear();
                umin = None;
                umax = None;
            }
            (0, 0xC) => depth = depth.saturating_sub(1),
            // Global.
            (1, 0x0) => g.usage_page = unsigned as u16,
            (1, 0x1) => g.logical_min = signed,
            (1, 0x2) => {
                // Maximum je se znamenkem jen tehdy, kdyz je minimum zaporne.
                g.logical_max = if g.logical_min < 0 { signed } else { unsigned as i32 };
            }
            (1, 0x7) => g.report_size = unsigned,
            (1, 0x8) => {
                if unsigned == 0 || unsigned > 255 {
                    return None;
                }
                g.report_id = unsigned as u8;
                out.uses_ids = true;
            }
            (1, 0x9) => g.report_count = unsigned,
            (1, 0xA) => {
                if stack.len() >= 16 {
                    return None;
                }
                stack.push(g);
            }
            (1, 0xB) => g = stack.pop()?,
            // Local.
            (2, 0x0) => {
                if usages.len() >= MAX_USAGES {
                    return None;
                }
                usages.push(full(unsigned));
            }
            (2, 0x1) => umin = Some(full(unsigned)),
            (2, 0x2) => umax = Some(full(unsigned)),
            _ => {}
        }
    }
    Some(out)
}
