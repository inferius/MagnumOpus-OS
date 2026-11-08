//! Paleta prikazu fyzickou klavesnici (QMP): xtask po znacce stiskne
//! Ctrl+K, napise hledani a Enter. Nejdriv prepnuti na okno Ukazky, pak
//! akce z jejiho menu ("Pricist 1", `MENU_UPDATE`) pres API akci.

use alloc::format;
use alloc::string::String;
use core::time::Duration;

use guiproto::a11y::Role;
use libos::{Startup, println, time};

use super::TestResult;
use super::gui::{tree, windows, with};

/// Znacky pro xtask (musi sedet s `xtask/src/qmp.rs`, modul `gui`).
pub const QEMU_SWITCH: &str = "init: test gui: paleta prepne na Ukazku";
pub const QEMU_ACTION: &str = "init: test gui: paleta pricte v Ukazce";

const DEMO: &str = "org.magnumopus.demo";

/// Hodnota pocitadla Ukazky ze stromu pristupnosti.
fn counter(ch: &libos::Channel, window: u32) -> Result<Option<i64>, String> {
    Ok(tree(ch, window)?.iter().find_map(|n| (n.role == Role::Label).then(|| n.name.strip_prefix("Počítadlo: ")?.parse().ok()).flatten()))
}

pub fn palette_keys(_: &Startup) -> TestResult {
    with(|s| {
        let demo = windows(&s.inspect)?.into_iter().find(|w| w.app == DEMO).ok_or("okno Ukazky chybi")?;
        // 1. Ctrl+K, "uka", Enter: prepnuti na okno Ukazky.
        println!("{QEMU_SWITCH}");
        let end = time::deadline_after(Duration::from_secs(30));
        loop {
            if windows(&s.inspect)?.iter().any(|w| w.app == DEMO && w.focused) {
                break;
            }
            if time::now() > end {
                return Err(String::from("paleta neprepnula na okno Ukazky"));
            }
            time::sleep(Duration::from_millis(100));
        }
        // 2. Ctrl+K, "pricist", Enter: akce menu Ukazky (Pricist 1).
        let before = counter(&s.inspect, demo.id)?.ok_or("pocitadlo Ukazky chybi")?;
        println!("{QEMU_ACTION}");
        let end = time::deadline_after(Duration::from_secs(30));
        loop {
            let now = counter(&s.inspect, demo.id)?;
            if now == Some(before + 1) {
                return Ok(());
            }
            if time::now() > end {
                return Err(format!("akce z palety neprobehla: pocitadlo {now:?}, ocekavano {}", before + 1));
            }
            time::sleep(Duration::from_millis(100));
        }
    })
}
