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
        assert!(Error::from_code(-(code as i64)).is_some(), "kod {}", -code);
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
        "SCHED_CONTROL",
        "LOG_READ",
        "CRASH_RECORD",
        "LOG_CONTROL",
        "JOB_KILL",
        "FUTEX",
        "THREAD_SET_POINTER",
        "VM_RESERVE",
        "VM_PROTECT",
        "VM_QUERY",
        "VMO_CLONE",
        "VMO_SEAL",
        "THREAD_SUSPEND",
        "THREAD_STATE",
        "EXCEPTION_CHANNEL",
        "EXCEPTION_RESOLVE",
        "PROCESS_THREADS",
        "EXCEPTION_HANDLER",
        "FENCE_CREATE",
        "FENCE_SIGNAL",
        "FENCE_WAIT",
        "FENCE_WAIT_ASYNC",
        "FENCE_QUERY",
        "DMA_MAP",
        "DMA_SEGMENTS",
        "DMA_SYNC",
        "THREAD_INTERRUPT",
        "PROCESS_MEMORY",
        "VMO_SET_SIZE",
        "VMO_SIZE",
        "VMO_DIRTY",
        "THREAD_SELF",
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
        dma: PoolInfo {
            used: 8,
            limit: 9,
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

#[test]
fn sched_info_roundtrip() {
    use crate::sched::{CpuInfo, QosInfo};
    let q = QosInfo { base: 2, effective: 1, job_max: 0, budget_us: 1000, period_us: 5000, ..Default::default() };
    assert_eq!(QosInfo::decode(&q.encode()), q);
    let c = CpuInfo { kind: 1, capacity: 512, cluster: 3, hw_id: 7, run_ns: [1, 2, 3, 4], idle_ns: 9, ..Default::default() };
    assert_eq!(CpuInfo::decode(&c.encode()), c);
}

#[test]
fn thread_context_roundtrip() {
    use crate::thread::{A64General, X64General};
    let mut x = X64General::default();
    x.rax = 1;
    x.r15 = 0xF0F0;
    x.rip = 0x40_1000;
    x.gs_base = 7;
    assert_eq!(X64General::decode(&x.encode()), x);
    let mut a = A64General::default();
    a.x[0] = 5;
    a.x[30] = 0x30;
    a.pc = 0x40_2000;
    a.tpidr = 9;
    assert_eq!(A64General::decode(&a.encode()), a);
    assert_eq!(X64General::SIZE, 20 * 8);
    assert_eq!(A64General::SIZE, 35 * 8);
}

#[test]
fn exception_report_roundtrip() {
    use crate::exception::Report;
    let r = Report { kind: 14, stage: 3, access: 1, reason: crate::exception::reason::BEYOND_END, pid: 7, tid: 9, pc: 0x40_1000, addr: 0x1234, code: 6 };
    assert_eq!(Report::decode(&r.encode()), Some(r));
    assert_eq!(Report::decode(&[0u8; 10]), None);
}

#[test]
fn exception_frame_layout() {
    use crate::exception::frame::*;
    use crate::thread::{A64_FP_SIZE, A64General, X64_FP_SIZE, X64General};
    assert!(GENERAL >= crate::exception::Report::SIZE);
    assert!(X64_FP >= GENERAL + X64General::SIZE && X64_FP % 64 == 0);
    assert!(A64_FP >= GENERAL + A64General::SIZE && A64_FP % 64 == 0);
    // Puvodni pevne velikosti (FXSAVE, NEON) a XSAVE s AVX-512 (2688 B).
    assert_eq!(size(X64_FP, X64_FP_SIZE), 832);
    assert_eq!(size(A64_FP, A64_FP_SIZE), 1024);
    assert_eq!(size(X64_FP, 2688) % 64, 0);
}

#[test]
fn fp_header_roundtrip() {
    use crate::thread::fp::{HEADER, Header, format};
    let h = Header { format: format::X64_XSAVE, len: 832, features: 0x7 };
    let b = h.encode();
    assert_eq!(Header::decode(&b), Some(h));
    assert_eq!(h.total(), HEADER + 832);
    let mut bad = b;
    bad[40] = 1;
    assert_eq!(Header::decode(&bad), None, "nenulova rezerva");
    assert_eq!(Header::decode(&b[..10]), None);
}

/// Dlouha zprava DEBUG_LOG se deli (jadro by ji odmitla celou): kusy
/// nejvys `max` bajtu, hranice znaku, prednostne za koncem radku.
#[test]
fn debug_log_chunks_split_long_text() {
    use crate::syscall::debug_log_chunks;
    fn parts(t: &str, m: usize) -> std::vec::Vec<&str> {
        debug_log_chunks(t, m).collect()
    }
    assert_eq!(parts("", 8), std::vec::Vec::<&str>::new());
    assert_eq!(parts("kratke", 8), ["kratke"]);
    assert_eq!(parts("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    assert_eq!(parts("ab\ncdefgh", 6), ["ab\n", "cdefgh"]);
    // Viceajtove znaky se nerozpuli.
    let t = "čččč";
    for p in parts(t, 3) {
        assert!(p.len() <= 3 && !p.is_empty());
    }
    assert_eq!(parts(t, 3).concat(), t);
    // Mez mensi nez znak: aspon cely znak.
    assert_eq!(parts("čč", 1), ["č", "č"]);
    let long: std::string::String = (0..3000).map(|i| if i % 70 == 69 { '\n' } else { 'x' }).collect();
    let ps = parts(&long, crate::syscall::DEBUG_LOG_MAX);
    assert!(ps.len() >= 3 && ps.iter().all(|p| p.len() <= crate::syscall::DEBUG_LOG_MAX));
    assert_eq!(ps.concat(), long);
}

#[test]
fn loader_table_offsets() {
    // Posuny pouziva i crt v C (user/dltest/crt.c): nova pole jen na konec.
    use crate::loader::Interface;
    use core::mem::offset_of;
    assert_eq!(offset_of!(Interface, run_fini), 24);
    assert_eq!(offset_of!(Interface, libd_session), 80);
    assert_eq!(offset_of!(Interface, run_init), 88);
    assert_eq!(core::mem::size_of::<Interface>(), 96);
    assert!(crate::loader::has_field(96, 96) && !crate::loader::has_field(88, 96));
}
