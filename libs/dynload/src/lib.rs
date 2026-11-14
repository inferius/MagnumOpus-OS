//! Logika zavadece `ld.so` bez systemu (testy na hostiteli,
//! `cargo test -p dynload`). Zavadec (`user/ld`) ji pouziva pro:
//!
//! - [`tls_layout`]: statickou cast TLS vsech modulu nahranych pri startu
//!   (varianta II na x86_64, varianta I na aarch64 - stejne jako `osrt::tls`
//!   a jak to predpoklada linker pro local-exec v programu),
//! - [`compute`]: hodnotu, kterou relokace zapise (z `elf::reloc::Kind`,
//!   baze modulu, addendu a definice symbolu),
//! - [`initfs_path`]: kde lezi knihovna (`/system/lib` = `lib/` v initfs).
//!
//! Popis: `docs/cs/dynamicke-linkovani.md`.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use elf::Machine;
use elf::reloc::Kind;

#[cfg(test)]
mod tests;

/// Velikost TCB (x86_64: ukazatel sam na sebe + rezerva; aarch64: 16 B
/// vyhrazenych ABI) - stejne jako `osrt::tls`.
pub const TCB_SIZE: u64 = 16;

/// Rezerva statickeho TLS pro moduly nahrane pozdeji pres `dlopen`, ktere
/// pouzivaji model initial-exec (`DF_STATIC_TLS`) - typicky ovladace
/// Vulkan/GL z Mesy (ICD, NVK). Bez rezervy by jejich `dlopen` selhal: TP
/// relativni pristup potrebuje misto ve statickem bloku kazdeho vlakna.
/// glibc: puvodni pevna rezerva 1664 B, dnes `rtld.optional_static_tls`
/// (512 B) + 144 B na jmenny prostor (vychozi 4 -> 1520 B). Bereme 2 KiB:
/// pokryje obe hodnoty glibc a je to nasobek zarovnani 64 B.
pub const STATIC_TLS_SURPLUS: u64 = 2048;

/// Sablona TLS modulu (`PT_TLS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsModule {
    /// `p_memsz`.
    pub size: u64,
    /// `p_align` (aspon 1).
    pub align: u64,
}

/// Rozlozeni statickeho TLS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsLayout {
    /// Posun dat modulu od ukazatele vlakna (TP); `None` = modul TLS nema.
    pub offsets: Vec<Option<i64>>,
    /// Velikost bloku (data a TCB).
    pub size: u64,
    /// Zarovnani bloku.
    pub align: u64,
    /// Kde v bloku lezi TP (x86_64 za daty, aarch64 na zacatku).
    pub tp: u64,
    /// Rezerva pro moduly z `dlopen`: (posun zacatku od TP, velikost).
    pub surplus: (i64, u64),
}

/// Rozlozi TLS modulu v poradi `mods` (prvni = program: linker pro nej
/// pocital local-exec s presne timto posunem) a za ne rezervu `surplus`
/// bajtu ([`STATIC_TLS_SURPLUS`]).
///
/// - x86_64 (varianta II): data pod TP, modul k ma posun
///   `-zarovnane(soucet velikosti 0..=k, align_k)`; za TP je TCB.
/// - aarch64 (varianta I): na TP 16 B TCB, data od `zarovnane(16, align)`
///   nahoru.
pub fn tls_layout(machine: Machine, mods: &[Option<TlsModule>], surplus: u64) -> TlsLayout {
    let align = mods.iter().flatten().map(|m| m.align.max(1)).max().unwrap_or(1).max(TCB_SIZE);
    let mut offsets = Vec::with_capacity(mods.len());
    match machine {
        Machine::X86_64 => {
            let mut cum = 0u64;
            for m in mods {
                offsets.push(m.map(|m| {
                    cum = (cum + m.size).next_multiple_of(m.align.max(1));
                    -(cum as i64)
                }));
            }
            // Rezerva pod posledni modul (zarovnana na 64 B).
            cum = (cum + surplus).next_multiple_of(64);
            let data = cum.next_multiple_of(align);
            TlsLayout { offsets, size: data + TCB_SIZE, align, tp: data, surplus: (-(cum as i64), surplus) }
        }
        Machine::Aarch64 => {
            let mut cum = TCB_SIZE;
            for m in mods {
                offsets.push(m.map(|m| {
                    let off = cum.next_multiple_of(m.align.max(1));
                    cum = off + m.size;
                    off as i64
                }));
            }
            let start = cum.next_multiple_of(64);
            cum = start + surplus;
            TlsLayout { offsets, size: cum.next_multiple_of(align), align, tp: 0, surplus: (start as i64, surplus) }
        }
    }
}

/// Definice symbolu, na ktery relokace odkazuje.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Def {
    /// Baze modulu, ktery symbol definuje.
    pub base: u64,
    /// `st_value` (u TLS posun v sablone modulu).
    pub value: u64,
    /// TLS modulu: (cislo modulu od 1, posun od TP).
    pub tls: Option<(u64, i64)>,
}

/// Co zapsat na misto relokace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apply {
    /// Jedno slovo.
    Word(u64),
    /// Deskriptor TLS: funkce a argument (2 slova).
    Desc(u64, u64),
    /// Zavolat IFUNC na teto adrese, vysledek zapsat.
    CallIfunc(u64),
}

/// Hodnota relokace `kind` v modulu s bazi `base`. `def` = definice
/// symbolu (`None` u relokace bez symbolu nebo u slabeho nedefinovaneho
/// symbolu = 0); `own_tls` = TLS modulu s relokaci (TLS relokace se
/// symbolem 0 odkazuji na vlastni modul); `tlsdesc` = adresa resitele
/// statickeho deskriptoru TLS v zavadeci.
pub fn compute(kind: Kind, base: u64, addend: i64, def: Option<Def>, own_tls: Option<(u64, i64)>, tlsdesc: u64) -> Result<Apply, &'static str> {
    let s = def.map_or(0, |d| d.base.wrapping_add(d.value));
    let tls = || -> Result<(u64, i64, u64), &'static str> {
        match def {
            Some(d) => d.tls.map(|(m, o)| (m, o, d.value)).ok_or("TLS relokace na symbol modulu bez TLS"),
            None => own_tls.map(|(m, o)| (m, o, 0)).ok_or("TLS relokace v modulu bez TLS"),
        }
    };
    Ok(match kind {
        Kind::None => return Err("relokace NONE se nezapisuje"),
        Kind::Relative => Apply::Word(base.wrapping_add_signed(addend)),
        Kind::Absolute | Kind::GlobDat | Kind::JumpSlot => Apply::Word(s.wrapping_add_signed(addend)),
        Kind::IRelative => Apply::CallIfunc(base.wrapping_add_signed(addend)),
        Kind::TlsModule => Apply::Word(tls()?.0),
        Kind::TlsOffset => Apply::Word(tls()?.2.wrapping_add_signed(addend)),
        Kind::TlsTpOffset => {
            let (_, off, v) = tls()?;
            Apply::Word(v.wrapping_add_signed(off).wrapping_add_signed(addend))
        }
        Kind::TlsDesc => {
            let (_, off, v) = tls()?;
            Apply::Desc(tlsdesc, v.wrapping_add_signed(off).wrapping_add_signed(addend))
        }
        Kind::Copy => return Err("relokace COPY (program prelozte jako PIE s -fPIE)"),
        Kind::Unknown(_) => return Err("neznamy typ relokace"),
    })
}

/// Jmeno knihovny z `DT_NEEDED` -> soubor v initfs: `/system/lib/x` nebo
/// holé `x` = `lib/x`. Jine absolutni cesty a `..` = `None` (knihovny jen
/// ze systemu; disk a `$ORIGIN` aplikaci prijdou se sluzbou `libd`).
pub fn initfs_path(needed: &str) -> Option<String> {
    let name = needed.strip_prefix("/system/lib/").unwrap_or(needed);
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return None;
    }
    let mut p = String::from("lib/");
    p.push_str(name);
    Some(p)
}
