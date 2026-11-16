//! Brana testovaciho behu (priznak `TEST_RUN` od jadra s `qemu-test`).
//!
//! Testy userspace (`src/tests/`) jsou v initu jen s feature `tests`, kterou
//! xtask zapina jen pro `cargo xtask test`. Produkcni init (obraz, zive USB,
//! instalator, aktualizace) je nema; xtask to overuje tvrdou kontrolou
//! (`xtask/src/testcode.rs`). Kdyby jadro presto poslalo `TEST_RUN`
//! initu bez testu, beh skonci chybou (ne tichym "proslo").

use libos::Startup;

/// Zvlastni beh testu zaznamu padu (`crash-test` v initfs)?
pub fn crash_mode(startup: &Startup) -> bool {
    #[cfg(feature = "tests")]
    return crate::tests::diag::crash_mode(startup);
    #[cfg(not(feature = "tests"))]
    {
        let _ = startup;
        false
    }
}

/// Rezim testu napajeni (`power-test` v initfs).
pub fn power_mode(startup: &Startup) -> Option<alloc::string::String> {
    #[cfg(feature = "tests")]
    return crate::tests::power::mode(startup);
    #[cfg(not(feature = "tests"))]
    {
        let _ = startup;
        None
    }
}

/// Zalozni resolver testu na pomocnem serveru hostitele.
pub fn local_dns(startup: &Startup, root: &netproto::Net) {
    #[cfg(feature = "tests")]
    crate::tests::net::local_dns(startup, root);
    #[cfg(not(feature = "tests"))]
    let _ = (startup, root);
}

/// Testovaci beh: zvlastni scenar nebo vsechny sady; navratovy kod initu.
pub fn run(startup: &Startup, power: Option<alloc::string::String>, crash: bool) -> i64 {
    #[cfg(feature = "tests")]
    {
        if let Some(mode) = power {
            // Vypnuti nebo restart (QEMU skonci samo).
            return crate::tests::power::scenario(startup, &mode);
        }
        if crash {
            // Zaznam padu jadra prezije restart.
            return crate::tests::diag::scenario(startup);
        }
        let failed = crate::tests::run_all(startup);
        if failed == 0 {
            libos::println!("init: vsechny testy userspace prosly");
            0
        } else {
            libos::println!("init: {failed} testu userspace selhalo");
            1
        }
    }
    #[cfg(not(feature = "tests"))]
    {
        let _ = (startup, power, crash);
        libos::println!("init: TEST_RUN, ale init je sestaven bez testu (feature tests); konec s chybou");
        1
    }
}
