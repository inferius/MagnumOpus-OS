//! Zvlastni behy testu napajeni (po hlavnim behu `cargo xtask test`).
//!
//! Konec QEMU je tu vysledek testu, proto vlastni behy: init dostane
//! v initfs `power-test` s rezimem a misto testu userspace udela scenar
//! z `user/init/src/tests/power.rs`:
//!
//! - `button`: init vypise [`MARKER_BUTTON`], xtask posle pres QMP
//!   `system_powerdown` (stisk tlacitka napajeni; x86 pevna udalost ACPI,
//!   aarch64 Generic Event Device), init cisto ukonci sluzby a acpid vypne
//!   pocitac (x86 S5, aarch64 PSCI SYSTEM_OFF),
//! - `reboot`: init rovnou pozada o restart (QEMU s `-no-reboot` skonci).
//!
//! Uspech = QEMU skoncilo samo s kodem 0 (ne pres isa-debug-exit nebo
//! semihosting jadra) a v logu jsou vsechny znacky [`markers`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use crate::{Options, Result, build, image_for, qemu};

/// Musi sedet s `user/init/src/tests/power.rs` (`scenario`).
pub const MARKER_BUTTON: &str = "init: test napajeni: cekam na tlacitko napajeni";

/// Znacky, ktere musi byt v logu po uspesnem behu rezimu `mode`.
pub fn markers(mode: &str) -> &'static [&'static str] {
    match mode {
        "button" => &[
            "acpid: ACPI pripraveno",
            "acpid: udalost PowerButton",
            "init: tlacitko napajeni, vypinam system",
            "init: vypnuti: sluzby ukonceny",
            "acpid: vstupuji do S5",
        ],
        _ => &["acpid: ACPI pripraveno", "init: restart: sluzby ukonceny", "acpid: restartuji"],
    }
}

/// Oba rezimy za sebou.
pub fn run_all(opts: &mut Options) -> Result<()> {
    for mode in ["button", "reboot"] {
        println!("xtask: test napajeni ({mode}) na {}", opts.arch.name);
        opts.extra_initfs.retain(|(n, _)| n != "power-test");
        opts.extra_initfs.push((String::from("power-test"), mode.as_bytes().to_vec()));
        opts.power_test = Some(mode);
        let artifacts = build(opts)?;
        let image = image_for(&artifacts, opts)?;
        let r = qemu::run(&image, &artifacts, opts);
        opts.power_test = None;
        opts.extra_initfs.retain(|(n, _)| n != "power-test");
        r?;
        println!("xtask: test napajeni ({mode}) prosel");
    }
    Ok(())
}

/// Vlakno, ktere po znacce v logu posle QMP `system_powerdown`.
pub fn spawn_button(log: std::path::PathBuf, port: Arc<AtomicU16>) {
    std::thread::spawn(move || {
        let start = Instant::now();
        loop {
            if start.elapsed() > Duration::from_secs(600) {
                return;
            }
            let seen = std::fs::read(&log).map(|b| String::from_utf8_lossy(&b).contains(MARKER_BUTTON)).unwrap_or(false);
            if seen {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        match crate::qmp::command(port.load(Ordering::Relaxed), r#"{"execute":"system_powerdown"}"#) {
            Ok(()) => println!("xtask: QMP: system_powerdown poslano (tlacitko napajeni)"),
            Err(e) => eprintln!("xtask: QMP: system_powerdown selhalo: {e}"),
        }
    });
}

/// Overi znacky v logu po konci QEMU s kodem 0.
pub fn check_log(log: &std::path::Path, mode: &str) -> std::result::Result<(), String> {
    let text = std::fs::read(log).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| e.to_string())?;
    let missing: Vec<&str> = markers(mode).iter().copied().filter(|m| !text.contains(m)).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("v logu chybi: {}", missing.join("; ")))
    }
}
