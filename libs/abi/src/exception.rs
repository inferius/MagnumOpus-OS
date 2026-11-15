//! Doruceni vyjimek: kanaly vyjimek (`EXCEPTION_CHANNEL`), zprava
//! o vyjimce a rozhodnuti obsluhy (`EXCEPTION_RESOLVE`).
//!
//! Poradi obsluh: debugger procesu (prvni sance), kanal vlakna, kanal
//! procesu, debugger (druha sance); kdyz nikdo nevyresi, proces skonci
//! (kod `process::EXIT_FAULT_BASE - kind`). Vlakno ve vyjimce stoji a jeho
//! kontext jde cist a zapsat (`THREAD_STATE`).

/// Druh vyjimky = cislo chyby jako u ukonceni procesu (vektory x86;
/// aarch64 na ne mapuje sve vyjimky).
pub mod kind {
    /// Deleni nulou (jen x86_64; aarch64 deleni nulou nehlasi).
    pub const DIVIDE: u32 = 0;
    /// Breakpoint (`int3` - PC uz za instrukci; `brk` - PC na instrukci,
    /// obsluha ho posune o 4).
    pub const BREAKPOINT: u32 = 3;
    /// Neplatna instrukce.
    pub const INVALID_OPCODE: u32 = 6;
    /// Obecna ochrana (x86_64) / jina chyba (aarch64).
    pub const GENERAL: u32 = 13;
    /// Vypadek stranky, ktery jadro nevyresilo (adresa v `addr`).
    pub const PAGE_FAULT: u32 = 14;
    /// Nezarovnany PC nebo SP (aarch64).
    pub const ALIGNMENT: u32 = 17;
    /// Preruseni vlakna (`THREAD_INTERRUPT`), ne vyjimka CPU: kod
    /// preruseni v `Report::code`, jen doruceni ve vlakne.
    pub const INTERRUPT: u32 = 256;
}

/// Ktera obsluha zpravu dostala (`Report::stage`).
pub mod stage {
    pub const DEBUGGER_FIRST: u32 = 1;
    pub const THREAD: u32 = 2;
    pub const PROCESS: u32 = 3;
    pub const DEBUGGER_SECOND: u32 = 4;
    /// Obsluha primo ve vlaknu (`EXCEPTION_HANDLER`).
    pub const IN_THREAD: u32 = 5;
}

/// Druh pristupu u vypadku stranky (`Report::access`).
pub mod access {
    pub const READ: u32 = 0;
    pub const WRITE: u32 = 1;
    pub const EXEC: u32 = 2;
}

/// Rozhodnuti obsluhy (`EXCEPTION_RESOLVE`).
pub mod action {
    /// Pokracovat (s kontextem zmenenym pres `THREAD_STATE`).
    pub const RESUME: u32 = 1;
    /// Predat dalsi obsluze (zavreni tokenu bez rozhodnuti = totez).
    pub const NEXT: u32 = 2;
    /// Ukoncit proces (jako neosetrena vyjimka).
    pub const KILL: u32 = 3;
}

/// Priznaky `EXCEPTION_CHANNEL`.
pub mod channel {
    /// Kanal debuggeru procesu (prvni a druha sance).
    pub const DEBUGGER: u32 = 1 << 0;
    /// Na vlakne: oznaci ho jako vlakno obsluhy - jeho vyjimky preskoci
    /// kanaly vlakna a procesu (necekalo by samo na sebe). Kanal nevznikne,
    /// syscall vrati 0.
    pub const HANDLER_THREAD: u32 = 1 << 1;
}

/// Doruceni primo ve vlaknu (`EXCEPTION_HANDLER`): operace.
pub mod handler {
    /// `(process, entry)`: vstupni adresa obsluhy procesu (0 = zrusit);
    /// musi lezet ve spustitelnem mapovani (W^X: zapecetene nebo s JIT).
    pub const SET_ENTRY: u32 = 0;
    /// `(base, size)`: alternativni zasobnik aktualniho vlakna (0 = zadny).
    pub const SET_STACK: u32 = 1;
    /// `(frame, disposition)`: navrat z obsluhy; `disposition` =
    /// `action::RESUME` (obnovit kontext z ramce) nebo `action::NEXT`
    /// (dalsi obsluha v poradi).
    pub const RETURN: u32 = 2;
    /// Nejvetsi hloubka vnoreni obsluh ve vlaknu; dalsi vyjimka jde rovnou
    /// na kanaly vyjimek nebo ke konci procesu.
    pub const MAX_DEPTH: u32 = 8;
}

/// Ramec vyjimky na uzivatelskem zasobniku (zarovnany na 64 B), ktery
/// dostane obsluha ve vlaknu (x86_64 `rdi`, aarch64 `x0`): `Report`, pak
/// obecne registry a stav FPU/SIMD (formaty `abi::thread`) v okamziku
/// vyjimky. `RETURN` z neho kontext obnovi (s kontrolou jako
/// `THREAD_STATE`).
pub mod frame {
    /// Offset `Report`.
    pub const REPORT: usize = 0;
    /// Offset obecnych registru.
    pub const GENERAL: usize = 64;
    /// x86_64: offset FP casti (hlavicka `thread::fp::Header`, data o 64 B
    /// dal).
    pub const X64_FP: usize = 256;
    /// aarch64: offset FP casti.
    pub const A64_FP: usize = 384;

    /// Velikost ramce s FP casti na `fp_off` a daty delky `fp_len`
    /// (zaokrouhleno na 64 B). Delka dat zavisi na CPU: ramec ma
    /// promennou velikost.
    pub const fn size(fp_off: usize, fp_len: usize) -> usize {
        (fp_off + crate::thread::fp::HEADER + fp_len).next_multiple_of(64)
    }
}

/// Zprava o vyjimce (56 B, little endian). Ke zprave patri dva handly:
/// vlakno (READ, WRITE, MANAGE) a token vyjimky.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// [`kind`].
    pub kind: u32,
    /// [`stage`].
    pub stage: u32,
    /// [`access`] (jen vypadek stranky).
    pub access: u32,
    pub _reserved: u32,
    pub pid: u64,
    /// Cislo vlakna.
    pub tid: u64,
    /// Adresa instrukce.
    pub pc: u64,
    /// Adresa pristupu (vypadek stranky), jinak 0.
    pub addr: u64,
    /// Surovy kod architektury (x86_64 chybovy kod, aarch64 ISS).
    pub code: u64,
}

impl Report {
    pub const SIZE: usize = 56;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.kind.to_le_bytes());
        b[4..8].copy_from_slice(&self.stage.to_le_bytes());
        b[8..12].copy_from_slice(&self.access.to_le_bytes());
        b[16..24].copy_from_slice(&self.pid.to_le_bytes());
        b[24..32].copy_from_slice(&self.tid.to_le_bytes());
        b[32..40].copy_from_slice(&self.pc.to_le_bytes());
        b[40..48].copy_from_slice(&self.addr.to_le_bytes());
        b[48..56].copy_from_slice(&self.code.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != Self::SIZE {
            return None;
        }
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        Some(Self {
            kind: u32_at(0),
            stage: u32_at(4),
            access: u32_at(8),
            _reserved: 0,
            pid: u64_at(16),
            tid: u64_at(24),
            pc: u64_at(32),
            addr: u64_at(40),
            code: u64_at(48),
        })
    }
}
