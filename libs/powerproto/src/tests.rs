//! Testy kodovani protokolu napajeni.

use super::*;

fn roundtrip(r: Reply) {
    assert_eq!(Reply::decode(&r.encode()), Ok(r));
}

#[test]
fn pozadavky_tam_a_zpet() {
    for q in [Request::Status, Request::Shutdown, Request::Reboot, Request::Thermal, Request::Battery, Request::CpuPerf] {
        assert_eq!(Request::decode(&q.encode()), Ok(q));
    }
}

#[test]
fn shell_pozadavky_tam_a_zpet() {
    for q in [
        Request::Appearance { system: false, change: String::new() },
        Request::Appearance { system: true, change: String::from("schema tmavy") },
        Request::Permissions { app: String::from("org.example.rec"), perm: String::from("microphone") },
        Request::Accounts,
        Request::Ssh { op: String::from("key-add"), arg: String::from("ssh-ed25519 AAAA x"), password: String::from("heslo") },
    ] {
        assert_eq!(Request::decode(&q.encode()), Ok(q.clone()));
    }
}

#[test]
fn systemovy_soubor_tam_a_zpet() {
    for q in [
        Request::SystemFile { op: 1, path: String::from("/system/config/x.toml"), data: alloc::vec![1, 2, 3] },
        Request::SystemFile { op: 2, path: String::from("/system/x"), data: alloc::vec![] },
    ] {
        assert_eq!(Request::decode(&q.encode()), Ok(q.clone()));
    }
    let big = Request::SystemFile { op: 1, path: String::from("/system/x"), data: alloc::vec![0; SYSTEM_FILE_MAX + 1] };
    assert_eq!(Request::decode(&big.encode()), Err(Error::InvalidArgs));
}

#[test]
fn odpovedi_tam_a_zpet() {
    roundtrip(Reply::Ok);
    roundtrip(Reply::Failed(Error::NotSupported));
    roundtrip(Reply::Event(Event::PowerButton));
    roundtrip(Reply::Event(Event::SleepButton));
    roundtrip(Reply::Status(Status {
        features: feature::SHUTDOWN | feature::POWER_BUTTON,
        fadt_revision: 6,
        oem: "BOCHS ".into(),
        thermal_zones: 1,
        batteries: 0,
        cpus: 4,
    }));
    roundtrip(Reply::Thermal(alloc::vec![
        ThermalZone { name: "\\_TZ.TZ00".into(), temp_dk: Some(3001), critical_dk: Some(3732), passive_dk: None },
        ThermalZone::default(),
    ]));
    roundtrip(Reply::Battery(alloc::vec![Battery {
        name: "\\_SB.BAT0".into(),
        present: true,
        unit_mw: true,
        state: battery_state::CHARGING,
        rate: Some(1500),
        remaining: Some(40000),
        full: Some(50000),
        design: Some(52000),
    }]));
    roundtrip(Reply::CpuPerf(alloc::vec![CpuPerf { name: "\\_SB.C000".into(), pss_states: 3, max_mhz: 2400, cppc: false }]));
}

#[test]
fn neplatne_zpravy() {
    assert_eq!(Request::decode(&[]), Err(Error::InvalidArgs));
    assert_eq!(Request::decode(&[99, 0, 0, 0]), Err(Error::InvalidArgs));
    // Prebytecna data za pozadavkem.
    let mut q = Request::Status.encode();
    q.push(0);
    assert_eq!(Request::decode(&q), Err(Error::InvalidArgs));
    // Prilis mnoho polozek.
    let mut w = Writer::op(op::R_THERMAL);
    w.u32(MAX_ITEMS as u32 + 1);
    assert_eq!(Reply::decode(&w.finish()), Err(Error::InvalidArgs));
    // Neplatna hodnota bool.
    let mut w = Writer::op(op::R_CPU_PERF);
    w.u32(1).str("x").u32(0).u32(0).u8(7);
    assert_eq!(Reply::decode(&w.finish()), Err(Error::InvalidArgs));
    // Oriznuta zprava.
    let full = Reply::Event(Event::PowerButton).encode();
    assert_eq!(Reply::decode(&full[..5]), Err(Error::InvalidArgs));
}

#[test]
fn procenta_a_stupne() {
    let b = Battery { remaining: Some(25), full: Some(100), ..Default::default() };
    assert_eq!(b.percent(), Some(25));
    assert_eq!(Battery { full: Some(0), remaining: Some(1), ..Default::default() }.percent(), None);
    assert_eq!(celsius_text(2982), "25.0 C");
    assert_eq!(celsius_text(2722), "-1.0 C");
}
