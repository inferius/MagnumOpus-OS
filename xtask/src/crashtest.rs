//! Zvlastni beh testu zaznamu padu (po hlavnim behu `cargo xtask test`).
//!
//! QEMU bezi bez `-no-reboot`: init (initfs `crash-test`) pri prvnim startu
//! vyvola paniku jadra (`CRASH_RECORD` `TEST_PANIC`), jadro zapise zaznam do
//! vyhrazene RAM a restartuje. Pri druhem startu ho logd ulozi do
//! `/system/pady`, init ho overi, sestavi balicek a skonci uspechem
//! (`user/init/src/tests/diag.rs`). Restart v QEMU RAM nemaze - jako teply
//! restart skutecneho PC.

use crate::{Options, Result, build, image_for, qemu};

/// Musi sedet s `user/init/src/tests/diag.rs`.
pub const MARKERS: &[&str] = &[
    "init: test padu: vyvolavam paniku jadra",
    "KERNEL PANIC",
    "zaznam padu z minuleho behu: KERNEL_PANIC",
    "logd: zaznam padu jadra #MO-",
    "init: test padu: zaznam prezil restart",
];

pub fn run(opts: &mut Options) -> Result<()> {
    println!("xtask: test zaznamu padu na {}", opts.arch.name);
    opts.extra_initfs.push((String::from("crash-test"), b"1".to_vec()));
    opts.crash_test = true;
    let r = build(opts).and_then(|a| {
        let image = image_for(&a, opts)?;
        qemu::run(&image, &a, opts)
    });
    opts.crash_test = false;
    opts.extra_initfs.retain(|(n, _)| n != "crash-test");
    r?;
    println!("xtask: test zaznamu padu prosel");
    Ok(())
}

/// Overi znacky v logu.
pub fn check_log(log: &std::path::Path) -> std::result::Result<(), String> {
    let text = std::fs::read(log).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| e.to_string())?;
    let missing: Vec<&str> = MARKERS.iter().copied().filter(|m| !text.contains(m)).collect();
    if missing.is_empty() { Ok(()) } else { Err(format!("v logu chybi: {}", missing.join("; "))) }
}
