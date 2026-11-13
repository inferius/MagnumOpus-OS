//! Mapa disku pro graf oddilu (GUI krok "Nahled zmen"): useky disku teď
//! a po instalaci v poradi na disku, vcetne volneho mista. Textovy
//! pruvodce pouziva stejna data jen jako seznam (`Plan::preview`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diskguard::gpt::{self, Table};
use diskguard::install::is_installer_owned;
use diskguard::{ESP_TYPE, OUR_TYPE};

use crate::{ALIGN_BYTES, DiskInfo, Plan, entry_name, human};

/// Druh useku (barvu podle nej voli rozhrani).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegKind {
    /// Oddil EFI jineho systemu.
    Efi,
    /// Oddil EFI MagnumOpus.
    OurEfi,
    /// Datovy oddil MagnumOpus.
    Ours,
    /// Oddil jineho systemu (Windows, Linux, data).
    Other,
    /// Volne (nerozdelene) misto.
    Free,
}

impl SegKind {
    pub const fn code(self) -> u8 {
        match self {
            SegKind::Efi => 0,
            SegKind::OurEfi => 1,
            SegKind::Ours => 2,
            SegKind::Other => 3,
            SegKind::Free => 4,
        }
    }

    pub fn from_code(c: u8) -> Option<Self> {
        [SegKind::Efi, SegKind::OurEfi, SegKind::Ours, SegKind::Other, SegKind::Free].into_iter().find(|k| k.code() == c)
    }
}

/// Usek disku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    pub kind: SegKind,
    pub bytes: u64,
    /// Popis pro graf a ctecku obrazovky ("Basic data partition 480 GB").
    pub label: String,
    /// Usek vznikne nebo se prepise instalaci (zvyrazneni v grafu).
    pub new: bool,
}

/// Oddil pro mapu: prvni a posledni sektor, druh, popis, novy.
type Part = (u64, u64, SegKind, String, bool);

fn kind_of(e: &gpt::Entry) -> SegKind {
    let ours = is_installer_owned(e);
    if e.type_guid == ESP_TYPE {
        if ours { SegKind::OurEfi } else { SegKind::Efi }
    } else if e.type_guid == OUR_TYPE {
        SegKind::Ours
    } else {
        SegKind::Other
    }
}

fn label(kind: SegKind, name: &str, bytes: u64) -> String {
    let base = match (kind, name.is_empty()) {
        (_, false) => String::from(name),
        (SegKind::Efi | SegKind::OurEfi, true) => String::from("EFI"),
        (SegKind::Ours, true) => String::from("MagnumOpus"),
        (SegKind::Free, true) => String::from("volné místo"),
        (SegKind::Other, true) => String::from("oddíl"),
    };
    format!("{base} {}", human(bytes))
}

fn parts_of(t: &Table, ss: usize) -> Vec<Part> {
    t.entries()
        .filter(|e| !e.is_empty())
        .map(|e| {
            let k = kind_of(&e);
            (e.first, e.last, k, label(k, &entry_name(&e), (e.last - e.first + 1) * ss as u64), false)
        })
        .collect()
}

/// Useky v poradi na disku; mezery aspon 1 MiB jsou volne misto.
fn segments(disk: &DiskInfo, mut parts: Vec<Part>) -> Vec<Seg> {
    let ss = disk.sector_size as u64;
    let min_gap = ALIGN_BYTES / ss;
    let first = gpt::first_usable(disk.sector_size);
    let last = gpt::last_usable(disk.sectors, disk.sector_size);
    parts.sort_by_key(|p| p.0);
    let mut out = Vec::new();
    let mut at = first;
    let free = |a: u64, b: u64, out: &mut Vec<Seg>| {
        if b >= a && b - a + 1 >= min_gap {
            let bytes = (b - a + 1) * ss;
            out.push(Seg { kind: SegKind::Free, bytes, label: label(SegKind::Free, "", bytes), new: false });
        }
    };
    for (a, b, kind, label, new) in parts {
        if a > at {
            free(at, a - 1, &mut out);
        }
        out.push(Seg { kind, bytes: (b - a + 1) * ss, label, new });
        at = at.max(b + 1);
    }
    free(at, last, &mut out);
    out
}

/// Disk teď (`table` = jeho GPT, `None` = prazdny nebo bez GPT).
pub fn now(disk: &DiskInfo, table: Option<&Table>) -> Vec<Seg> {
    segments(disk, table.map(|t| parts_of(t, disk.sector_size)).unwrap_or_default())
}

/// Disk po instalaci podle planu.
pub fn after(disk: &DiskInfo, table: Option<&Table>, plan: &Plan) -> Vec<Seg> {
    let ss = disk.sector_size;
    let size = |r: (u64, u64)| (r.1 - r.0 + 1) * ss as u64;
    let esp = (plan.esp.0, plan.esp.1, SegKind::OurEfi, label(SegKind::OurEfi, "EFI MagnumOpus", size(plan.esp)), true);
    let data_new = plan.format_data;
    let data = (plan.data.0, plan.data.1, SegKind::Ours, label(SegKind::Ours, "MagnumOpus", size(plan.data)), data_new);
    let mut parts: Vec<Part> = match plan.gpt {
        // Cely disk: z puvodni tabulky nezbude nic.
        crate::GptWrite::New(_) => Vec::new(),
        // Nase oddily nahradi stejne useky stavajici tabulky (preinstalace).
        _ => table
            .map(|t| parts_of(t, ss))
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.0 != plan.esp.0 && p.0 != plan.data.0)
            .collect(),
    };
    parts.push(esp);
    parts.push(data);
    segments(disk, parts)
}
