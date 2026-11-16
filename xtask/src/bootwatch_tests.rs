use super::*;

const FW: &str = "UEFI firmware (version edk2)\nConnect: PciRoot(0x0)/Pci(0x1,0x0): Not Found\n";

#[test]
fn first_start_confirmed_by_loader() {
    let t0 = Instant::now();
    let mut w = Watch::new(false, t0);
    assert_eq!(w.step(FW, t0 + BOOT_GRACE), None, "na hranici jeste ne");
    let ok = format!("{FW}[ INFO]: loader: os loader 0.1.0\n");
    assert_eq!(w.step(&ok, t0 + BOOT_GRACE * 2), None);
    assert!(w.done, "bez restartu se dal necte");
}

#[test]
fn first_start_stuck_in_firmware() {
    let t0 = Instant::now();
    let mut w = Watch::new(false, t0);
    let e = w.step(FW, t0 + BOOT_GRACE + Duration::from_secs(1)).expect("zaseknuti");
    assert!(e.contains("po startu QEMU"), "{e}");
    assert!(e.contains("Not Found"), "posledni radek v hlaseni: {e}");
}

#[test]
fn reboot_after_panic_must_reach_loader_again() {
    let t0 = Instant::now();
    let mut w = Watch::new(true, t0);
    let first = "[ INFO]: loader: os loader 0.1.0\ninit: test padu\nKERNEL PANIC na cpu3\n";
    // Panika videna v t0+10 s: lhuta se pocita od ni, ne od startu behu.
    assert_eq!(w.step("[ INFO]: loader: os loader 0.1.0\n", t0 + Duration::from_secs(5)), None);
    assert_eq!(w.step(first, t0 + Duration::from_secs(10)), None);
    assert_eq!(w.step(&format!("{first}{FW}"), t0 + BOOT_GRACE), None);
    let e = w.step(&format!("{first}{FW}"), t0 + BOOT_GRACE + Duration::from_secs(11)).expect("zaseknuti");
    assert!(e.contains("po restartu"), "{e}");
    // Druhy start s loaderem = v poradku.
    let mut w = Watch::new(true, t0);
    let both = format!("{first}{FW}[ INFO]: loader: os loader 0.1.0\n");
    assert_eq!(w.step(&both, t0 + BOOT_GRACE * 3), None);
    assert!(!w.done, "test padu hlida dal");
}

#[test]
fn starts_counts_resets_only_when_asked() {
    let log = "loader: os loader\nKERNEL PANIC\nloader: os loader\n";
    assert_eq!(starts(log, true), (2, 2));
    assert_eq!(starts(log, false), (1, 2));
}
