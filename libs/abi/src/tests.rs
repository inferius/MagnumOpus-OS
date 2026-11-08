//! Testy rozhrani (bezi na hostiteli: `cargo test -p abi`).

use std::collections::HashSet;
use std::path::Path;

use crate::error::{ALL, Error, decode, encode};

#[test]
fn error_codes_are_unique_and_negative() {
    let mut seen = HashSet::new();
    for e in ALL {
        assert!(e.code() < 0, "{} ma nezaporny kod", e.name());
        assert!(seen.insert(e.code()), "kod {} je dvakrat", e.code());
        assert_eq!(Error::from_code(e.code() as i64), Some(*e));
    }
}

#[test]
fn error_codes_are_stable() {
    // Kody jsou verejne rozhrani - tento test hlida, ze se omylem nezmeni.
    assert_eq!(Error::BadHandle.code(), -1);
    assert_eq!(Error::Corrupted.code(), -10);
    assert_eq!(Error::BadAddress.code(), -11);
    assert_eq!(Error::Canceled.code(), -18);
}

#[test]
fn all_errors_listed() {
    // Kody tvori souvisly rozsah -1..=-N, takze ALL nesmi nic vynechat.
    let n = ALL.len() as i32;
    for code in 1..=n {
        assert!(
            Error::from_code(-(code as i64)).is_some(),
            "chybi kod {}",
            -code
        );
    }
    assert!(Error::from_code(-(n as i64) - 1).is_none());
}

#[test]
fn encode_decode_roundtrip() {
    assert_eq!(decode(encode(Ok(42))), Ok(42));
    for e in ALL {
        assert_eq!(decode(encode(Err(*e))), Err(*e));
    }
}

/// Kazda chyba musi byt popsana v obou jazycich dokumentace.
#[test]
fn every_error_is_documented() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    for doc in ["cs/chyby.md", "en/errors.md"] {
        let text = std::fs::read_to_string(root.join(doc)).unwrap_or_else(|e| panic!("{doc}: {e}"));
        for e in ALL {
            let heading = format!("### `{}` ({})", e.name(), e.code());
            assert!(text.contains(&heading), "{doc}: chybi sekce `{heading}`");
        }
    }
}

#[test]
fn every_syscall_is_documented() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let names = [
        "DEBUG_LOG",
        "CLOCK_MONOTONIC",
        "HANDLE_CLOSE",
        "HANDLE_DUPLICATE",
        "OBJECT_WAIT_ONE",
        "OBJECT_WAIT_MANY",
        "OBJECT_SIGNAL",
        "EVENT_CREATE",
        "CHANNEL_CREATE",
        "CHANNEL_WRITE",
        "CHANNEL_READ",
        "VMO_CREATE",
        "VMO_READ",
        "VMO_WRITE",
        "VM_MAP",
        "VM_UNMAP",
        "PROCESS_CREATE",
        "PROCESS_START",
        "PROCESS_EXIT",
        "PROCESS_KILL",
        "PROCESS_EXIT_CODE",
        "THREAD_CREATE",
        "THREAD_START",
        "THREAD_EXIT",
        "THREAD_YIELD",
        "VMO_CREATE_CONTIGUOUS",
        "VMO_CREATE_PHYSICAL",
        "VMO_PHYS_ADDR",
        "INTERRUPT_CREATE",
        "INTERRUPT_ACK",
        "RESOURCE_CREATE",
        "VM_PAGE_SIZE",
        "CLOCK_REALTIME",
        "CLOCK_SET",
        "IOPORT_READ",
        "IOPORT_WRITE",
        "INTERRUPT_CREATE_LINE",
        "RANDOM_GET",
        "RANDOM_ADD",
        "PORT_CREATE",
        "OBJECT_WAIT_ASYNC",
        "PORT_WAIT",
        "JOB_CREATE",
        "JOB_SET_LIMIT",
        "JOB_INFO",
        "CLOCK_ADJUST",
        "FB_CONTROL",
        "VMO_REVOKE",
        "JOB_PROCESSES",
        "SYSTEM_POWER",
        "",
        "LOG_READ",
        "CRASH_RECORD",
    ];
    assert_eq!(names.len() as u64, crate::syscall::COUNT);
    for doc in ["cs/syscally.md", "en/syscalls.md"] {
        let text = std::fs::read_to_string(root.join(doc)).unwrap_or_else(|e| panic!("{doc}: {e}"));
        for name in names.iter().filter(|n| !n.is_empty()) {
            assert!(
                text.contains(&format!("### `{name}`")),
                "{doc}: chybi syscall {name}"
            );
        }
    }
}

#[test]
fn job_info_roundtrip() {
    use crate::job::{JobInfo, PoolInfo, UNLIMITED};
    let info = JobInfo {
        kernel: PoolInfo {
            used: 1,
            limit: UNLIMITED,
            available: 3,
        },
        user: PoolInfo {
            used: 4,
            limit: 5,
            available: 1,
        },
        id: 77,
    };
    assert_eq!(JobInfo::decode(&info.encode()), info);
    use crate::job::ProcEntry;
    let e = ProcEntry { pid: 9, job: 77, cpu_ns: 123, threads: 2, flags: 1, name: ProcEntry::name_bytes("fsd") };
    assert_eq!(ProcEntry::decode(&e.encode()), Some(e));
    assert_eq!(e.name(), "fsd");
    // Oriznuti na hranici znaku (vicebajtove znaky).
    let long = ProcEntry::name_bytes(&"ž".repeat(20));
    assert!(core::str::from_utf8(&long[..long.iter().position(|&c| c == 0).unwrap_or(32)]).is_ok());
}

#[test]
fn acpi_info_roundtrip() {
    use crate::acpi::{Info, MAX_RANGES, alloc_free::Ranges};
    let mut rsdp = [0u8; 36];
    rsdp[..8].copy_from_slice(b"RSD PTR ");
    let mut ranges = Ranges::new();
    assert!(ranges.push((0x7F00_0000, 0x2000)));
    assert!(ranges.push((0x7FF0_0000, 0x1000)));
    let info = Info { rsdp_phys: 0x7F00_1000, rsdp, ranges };
    let mut buf = [0u8; 4096];
    let n = info.encode(&mut buf).unwrap();
    assert_eq!(n, 56 + 32);
    assert_eq!(Info::decode(&buf[..n]), Some(info.clone()));
    // Kratky buffer, spatny magic, prilis mnoho oblasti.
    assert_eq!(info.encode(&mut buf[..10]), None);
    let mut bad = buf;
    bad[0] = b'X';
    assert_eq!(Info::decode(&bad[..n]), None);
    let mut many = buf;
    many[52..56].copy_from_slice(&((MAX_RANGES + 1) as u32).to_le_bytes());
    assert_eq!(Info::decode(&many), None);
    let mut full = Ranges::new();
    for i in 0..MAX_RANGES {
        assert!(full.push((i as u64, 1)));
    }
    assert!(!full.push((0, 1)));
}
