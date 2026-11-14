//! Testy ovladace edu v simulaci (bez QEMU): `cargo test -p edu`.

use driverkit::sim::Sim;
use driverkit::{DeviceInfo, Exit, run};

use crate::Edu;
use crate::model::EduModel;

fn sim() -> Sim {
    let info = DeviceInfo { bus: 0, device: 4, function: 0, vendor: crate::VENDOR, device_id: crate::DEVICE };
    let manifest = driverkit::manifest::parse(include_str!("../edu.driver.toml")).unwrap();
    Sim::new("edu", info, EduModel::new()).with_manifest(&manifest)
}

#[test]
fn factorial_with_interrupt() {
    let sim = sim();
    sim.send(b"fact 5");
    sim.send(b"fact 12");
    sim.send(b"dk:ping");
    sim.close();
    // Zavreny kanal ovladac zpracuje az po zpravach ve fronte; na
    // preruseni uz ale neceka. Proto se vysledky ctou po krocich.
    let p = sim.platform();
    let mut r = driverkit::Runner::<Edu<_>>::new(&p).ok().unwrap();
    assert_eq!(r.step(&p), None); // fact 5
    assert_eq!(r.step(&p), None); // fact 12 (zaneprazdneno)
    assert_eq!(r.step(&p), None); // dk:ping
    assert_eq!(sim.sent_text(), ["chyba zarizeni je zaneprazdnene", "dk:pong edu"]);
    let exit = r.step(&p).unwrap();
    assert_eq!(exit, Exit::ClientClosed);
    r.finish(&p, &exit);
    assert!(sim.logs().iter().any(|l| l == "edu 00:04.0: karta edu 1.0 zije"));
}

#[test]
fn factorial_result() {
    let sim = sim();
    let p = sim.platform();
    let mut r = driverkit::Runner::<Edu<_>>::new(&p).ok().unwrap();
    sim.send(b"fact 10");
    assert_eq!(r.step(&p), None); // zprava
    assert_eq!(r.step(&p), None); // preruseni
    assert_eq!(sim.sent_text(), ["fact 10 3628800"]);
}

#[test]
fn dma_roundtrip_stays_in_iommu() {
    let sim = sim();
    let p = sim.platform();
    let mut r = driverkit::Runner::<Edu<_>>::new(&p).ok().unwrap();
    sim.send(b"dma ahoj z DMA");
    for _ in 0..3 {
        assert_eq!(r.step(&p), None); // zprava, DMA tam, DMA zpet
    }
    assert_eq!(sim.sent_text(), ["dma ahoj z DMA"]);
    assert!(sim.now() >= 200_000_000, "dva prenosy po 100 ms");
    // Data prosla vnitrnim bufferem zarizeni.
    sim.with_model(|m: &mut EduModel, _| assert_eq!(&m.buffer()[..10], b"ahoj z DMA"));
    assert!(sim.faults().is_empty(), "{:?}", sim.faults());
}

#[test]
fn sg_roundtrip_across_pages() {
    let sim = sim();
    let p = sim.platform();
    let mut r = driverkit::Runner::<Edu<_>>::new(&p).ok().unwrap();
    let text: String = (0..1000).map(|i| (b'a' + (i % 26) as u8) as char).collect();
    sim.send(format!("sg {text}").as_bytes());
    // Zprava, cesta tam, cesta zpet (v simulaci je buffer souvisly: jeden kus).
    for _ in 0..3 {
        assert_eq!(r.step(&p), None);
    }
    assert_eq!(sim.sent_text(), [format!("sg {text}")]);
    assert!(sim.faults().is_empty(), "{:?}", sim.faults());
}

#[test]
fn stale_address_after_unmap_faults() {
    let sim = sim();
    let p = sim.platform();
    let mut r = driverkit::Runner::<Edu<_>>::new(&p).ok().unwrap();
    sim.send(b"stale");
    assert_eq!(r.step(&p), None); // zprava
    assert_eq!(r.step(&p), None); // preruseni po (zablokovanem) prenosu
    let sent = sim.sent_text();
    let iova = u64::from_str_radix(sent[0].strip_prefix("stale 0x").unwrap(), 16).unwrap();
    // Buffer zanikl pred prenosem: zapis zarizeni zablokovan.
    assert_eq!(sim.faults(), [driverkit::DriverError::DmaFault { iova, len: 64, write: true }]);
}

#[test]
fn bad_commands() {
    let sim = sim();
    sim.send(b"fact x");
    sim.send(b"nic");
    sim.close();
    assert_eq!(run::<_, Edu<_>>(&sim.platform()), Exit::ClientClosed);
    assert_eq!(sim.sent_text(), ["chyba fact ocekava cislo", "chyba neznamy prikaz"]);
}

#[test]
fn manifest_quota_is_enough() {
    // Buffery 2x 4 KiB a 12 KiB po strankach (plus 4 KiB pro `stale`) se
    // vejdou do dma_max = 32K z manifestu; s mensi kvotou probe selze
    // citelne.
    let sim = sim().dma_max(4096);
    let Exit::Failed { probe: true, err } = run::<_, Edu<_>>(&sim.platform()) else { panic!() };
    assert!(format!("{err}").contains("edu-zpet"), "{err}");
}

/// Kod bez komentaru a s jednou mezerou misto libovolnych mezer (priklady
/// v prirucce maji komentare prelozene do anglictiny).
fn normalize(code: &str) -> String {
    let mut out = String::new();
    for line in code.lines() {
        let line = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        for w in line.split_whitespace() {
            out.push_str(w);
            out.push(' ');
        }
    }
    out
}

/// Priklady kodu v prirucce (`docs/cs/ovladace.md`, `docs/en/drivers.md`)
/// musi byt doslova z prekladaneho kodu ovladace edu: zmena kodu bez
/// zmeny prirucky (nebo naopak) je chyba testu.
#[test]
fn guide_examples_are_compiled_code() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let read = |p: &str| std::fs::read_to_string(format!("{dir}/{p}")).unwrap();
    let sources = normalize(
        &["src/lib.rs", "src/main.rs", "src/model.rs", "src/tests.rs", "edu.driver.toml"]
            .iter()
            .map(|p| read(p))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    for guide in ["../../docs/cs/ovladace.md", "../../docs/en/drivers.md"] {
        let text = read(guide);
        let mut checked = 0;
        let mut block: Option<String> = None;
        for line in text.lines() {
            match (&mut block, line.trim_start()) {
                (None, l) if l == "```rust" || l == "```toml" => block = Some(String::new()),
                (Some(b), "```") => {
                    let code = normalize(b);
                    assert!(
                        sources.contains(&code),
                        "{guide}: priklad neni v kodu user/edu (zmenil se kod nebo prirucka?):\n{b}"
                    );
                    checked += 1;
                    block = None;
                }
                (Some(b), _) => {
                    b.push_str(line);
                    b.push('\n');
                }
                _ => {}
            }
        }
        assert!(checked >= 10, "{guide}: malo prikladu ({checked})");
    }
}
