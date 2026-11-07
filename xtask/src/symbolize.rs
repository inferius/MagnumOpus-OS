//! Preklad adres z vypisu zasobniku jadra na jmena funkci.
//!
//! Jadro samo tabulku symbolu nema (zbytecne by rostlo), ale vypisuje
//! adresy. xtask je po behu prelozi pomoci `llvm-nm` (komponenta
//! `llvm-tools` z rust-toolchain.toml) nad ELF souborem jadra.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Result;

/// Najde `llvm-nm` v sysrootu aktivniho toolchainu.
fn llvm_nm() -> Result<PathBuf> {
    let out = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()?;
    let sysroot = PathBuf::from(String::from_utf8(out.stdout)?.trim());
    let host_out = Command::new("rustc")
        .args(["--print", "host-tuple"])
        .output()?;
    let host = String::from_utf8(host_out.stdout)?.trim().to_string();
    let exe = if cfg!(windows) {
        "llvm-nm.exe"
    } else {
        "llvm-nm"
    };
    let path = sysroot.join("lib/rustlib").join(host).join("bin").join(exe);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "llvm-nm nenalezen ({}), chybi komponenta llvm-tools?",
            path.display()
        )
        .into())
    }
}

/// Seznam (adresa, jmeno) serazeny podle adresy.
fn load_symbols(elf: &Path) -> Result<Vec<(u64, String)>> {
    let out = Command::new(llvm_nm()?)
        .args(["--defined-only", "--numeric-sort", "--demangle"])
        .arg(elf)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut symbols: Vec<(u64, String)> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let addr = u64::from_str_radix(parts.next()?, 16).ok()?;
            let kind = parts.next()?;
            let name = parts.next()?;
            matches!(kind, "t" | "T").then(|| (addr, name.to_string()))
        })
        .collect();
    symbols.sort_by_key(|s| s.0);
    Ok(symbols)
}

fn lookup(symbols: &[(u64, String)], addr: u64) -> Option<(&str, u64)> {
    let i = symbols.partition_point(|s| s.0 <= addr).checked_sub(1)?;
    let (start, name) = &symbols[i];
    Some((name, addr - start))
}

/// Vypise vypis zasobniku z logu s prelozenymi adresami. Nedela nic,
/// pokud log neobsahuje paniku ani fatalni vyjimku.
pub fn report(log: &Path, kernel_elf: &Path) -> Result<()> {
    let Ok(text) = std::fs::read_to_string(log) else {
        return Ok(());
    };
    if !text.contains("KERNEL PANIC") && !text.contains("neosetrena vyjimka") {
        return Ok(());
    }
    let symbols = load_symbols(kernel_elf)?;
    println!("xtask: vypis zasobniku s jmeny funkci:");
    for line in text.lines() {
        let trimmed = line.trim();
        let hex = trimmed.strip_prefix("0x").or_else(|| {
            // Adresa instrukce z vypisu fatalni vyjimky: "rip 0x..." (x86_64)
            // nebo "pc 0x..." (aarch64), viz `arch::PC_NAME` v jadru.
            trimmed
                .strip_prefix("rip 0x")
                .or_else(|| trimmed.strip_prefix("pc 0x"))
                .map(|r| r.split_whitespace().next().unwrap_or(""))
        });
        let Some(addr) = hex.and_then(|h| u64::from_str_radix(h, 16).ok()) else {
            continue;
        };
        match lookup(&symbols, addr) {
            Some((name, off)) => println!("    {addr:#018x}  {name} + {off:#x}"),
            None => println!("    {addr:#018x}  ?"),
        }
    }
    Ok(())
}
