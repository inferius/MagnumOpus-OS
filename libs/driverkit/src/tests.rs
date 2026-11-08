//! Testy sady na hostiteli: registry, DMA, kruhy, manifest, smycka
//! a simulace (`cargo test -p driverkit --features sim`).

use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::io::Io;
use crate::manifest::{self, Match};
use crate::{DriverError, Field, Reg};

/// Pametove okno registru bez zarizeni.
struct Mem(RefCell<Vec<u8>>);

impl Mem {
    fn new(len: usize) -> Self {
        Self(RefCell::new(vec![0; len]))
    }
    fn get(&self, off: u64, n: usize) -> u64 {
        let m = self.0.borrow();
        let mut b = [0u8; 8];
        b[..n].copy_from_slice(&m[off as usize..off as usize + n]);
        u64::from_le_bytes(b)
    }
    fn put(&self, off: u64, n: usize, v: u64) {
        self.0.borrow_mut()[off as usize..off as usize + n].copy_from_slice(&v.to_le_bytes()[..n]);
    }
}

impl Io for Mem {
    fn read8(&self, off: u64) -> u8 {
        self.get(off, 1) as u8
    }
    fn read16(&self, off: u64) -> u16 {
        self.get(off, 2) as u16
    }
    fn read32(&self, off: u64) -> u32 {
        self.get(off, 4) as u32
    }
    fn read64(&self, off: u64) -> u64 {
        self.get(off, 8)
    }
    fn write8(&self, off: u64, v: u8) {
        self.put(off, 1, v as u64)
    }
    fn write16(&self, off: u64, v: u16) {
        self.put(off, 2, v as u64)
    }
    fn write32(&self, off: u64, v: u32) {
        self.put(off, 4, v as u64)
    }
    fn write64(&self, off: u64, v: u64) {
        self.put(off, 8, v)
    }
    fn len(&self) -> u64 {
        self.0.borrow().len() as u64
    }
}

crate::registers! {
    /// Ridici registr.
    CTRL @ 0x04: u32 {
        /// Zapnuti.
        EN: 0,
        MODE: 4..=6,
        TOP: 28..=31,
    }
    WIDE @ 0x10: u64;
}

#[test]
fn reg_fields_read_and_write() {
    let io = Mem::new(0x20);
    CTRL::EN.set(&io);
    CTRL::MODE.write(&io, 5).unwrap();
    assert_eq!(CTRL.read(&io), 1 | 5 << 4);
    assert_eq!(CTRL::MODE.read(&io), 5);
    assert!(CTRL::EN.is_set(&io));
    CTRL::EN.clear(&io);
    assert_eq!(CTRL.read(&io), 5 << 4);
    CTRL::TOP.write(&io, 0xF).unwrap();
    assert_eq!(CTRL.read(&io), 0xF000_0050);
    WIDE.write(&io, 0x1122_3344_5566_7788);
    assert_eq!(io.read32(0x10), 0x5566_7788);
}

#[test]
fn reg_field_overflow_is_error_not_silent() {
    let io = Mem::new(0x20);
    CTRL.write(&io, 0x8000_0001);
    let e = CTRL::MODE.write(&io, 8).unwrap_err();
    assert_eq!(
        e,
        DriverError::FieldOverflow { reg: "CTRL", field: "MODE", value: 8, bits: 3 }
    );
    // Registr zustal beze zmeny.
    assert_eq!(CTRL.read(&io), 0x8000_0001);
    assert!(alloc::format!("{e}").contains("CTRL.MODE"));
}

#[test]
fn field_value_and_mask() {
    let f: Field<u32> = Field::new(Reg::new("R", 0), "F", 8, 15);
    assert_eq!(f.mask(), 0xFF00);
    assert_eq!(f.value(0xAB).unwrap(), 0xAB00);
    assert_eq!(f.get(0x12AB_CD34), 0xCD);
}

#[test]
fn manifest_full() {
    let m = manifest::parse(
        r#"
[driver]
name = "edu"
description = "QEMU edu"
license = "MIT OR Apache-2.0"
origin = "vlastni"
[match]
pci = ["1234:11e8", "1234:abcd"]
class = ["ff.00.00"]
[resources]
bars = [0, 2]
irq_max = 2
dma_max = "64K"
[service]
restart = false
max_restarts = 3
qos = "realtime"
rt_budget_us = 1000
rt_period_us = 5000
"#,
    )
    .unwrap();
    assert_eq!(m.name, "edu");
    assert_eq!(m.matches[0], Match::Pci { vendor: 0x1234, device: 0x11e8 });
    assert_eq!(m.matches[2], Match::Class { class: 0xff, subclass: 0, prog_if: 0 });
    assert_eq!(m.bars, [0, 2]);
    assert_eq!((m.irq_max, m.dma_max, m.restart, m.max_restarts), (2, 65536, false, 3));
    assert_eq!(m.qos, Some((abi::sched::class::REALTIME, 1000, 5000)));
    assert!(m.matches(0x1234, 0x11e8, 0, 0, 0));
    assert!(m.matches(1, 1, 0xff, 0, 0));
    assert!(!m.matches(0x1234, 0x0001, 1, 0, 0));
}

#[test]
fn manifest_errors_are_readable() {
    let base = "[driver]\nname = \"x\"\nlicense = \"MIT\"\norigin = \"vlastni\"\n";
    let err = |extra: &str| manifest::parse(&alloc::format!("{base}{extra}")).unwrap_err();
    assert!(err("").contains("prazdny [match]"));
    assert!(err("[match]\npci = [\"12:34\"]\n").contains("match.pci[0]"));
    assert!(err("[match]\nclass = [\"01.02\"]\n").contains("match.class[0]"));
    assert!(err("[match]\npci = [\"1234:0001\"]\n[resources]\nbars = [6]\n").contains("resources.bars[0]"));
    assert!(err("[match]\npci = [\"1234:0001\"]\n[service]\nqos = \"fast\"\n").contains("service.qos"));
    assert!(err("[match]\npci = [\"1234:0001\"]\n[service]\nqos = \"realtime\"\n").contains("rt_budget_us"));
    assert!(err("[match]\npci = [\"1234:0001\"]\n[resources]\ndma_max = \"1G\"\n").contains("dma_max"));
    let e = manifest::parse("[driver]\nname = \"Bad_Name\"\n").unwrap_err();
    assert!(e.contains("mala pismena"), "{e}");
    let e = manifest::parse("[driver]\nname = \"x\"\n[match]\npci = [\"1234:0001\"]\n").unwrap_err();
    assert!(e.contains("license"), "{e}");
}

#[test]
fn manifest_file_names() {
    assert_eq!(manifest::driver_of_file("edu.driver.toml"), Some("edu"));
    assert_eq!(manifest::driver_of_file(".driver.toml"), None);
    assert_eq!(manifest::driver_of_file("edu.manifest.toml"), None);
    assert_eq!(manifest::Manifest::file_name("edu"), "edu.driver.toml");
}

#[test]
fn error_messages_have_hints() {
    let e = DriverError::Os { op: "BAR 2", err: abi::Error::AccessDenied };
    assert!(alloc::format!("{e}").contains("resources"));
    let e = DriverError::DmaFault { iova: 0x1234, len: 8, write: true };
    let s = alloc::format!("{e}");
    assert!(s.contains("0x1234") && s.contains("IOMMU"), "{s}");
    assert_eq!(DriverError::from(abi::Error::PeerClosed), DriverError::ClientClosed);
}

#[cfg(feature = "sim")]
mod sim_tests;

#[test]
fn driver_config_message() {
    use crate::config::{self, Bar};
    let c = config::parse("pci 0 4 0 bar 0 0xfea00000 0x100000 bar 2 0x800000000 0x4000").unwrap();
    assert_eq!((c.bus, c.device, c.function), (0, 4, 0));
    assert_eq!(c.bars[1], Bar { index: 2, phys: 0x8_0000_0000, size: 0x4000 });
    assert_eq!(config::parse(&config::format(&c)), Some(c));
    // Stary tvar (virtio) bez BARu.
    assert_eq!(config::parse("pci 1 2 3").unwrap().bars, []);
    for bad in ["pci 0 32 0", "pci 0 1", "pcx 0 0 0", "pci 0 1 0 bar 6 0x0 0x10", "pci 0 1 0 bar 0 10 0x10", "pci 0 1 0 bar 0 0x0 0x0", "pci 0 1 0 navic"] {
        assert_eq!(config::parse(bad), None, "{bad}");
    }
}
