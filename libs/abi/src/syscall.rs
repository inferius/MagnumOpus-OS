//! Cisla a konvence syscallu (x86_64).
//!
//! Volani: instrukce `syscall`, cislo v `rax`, argumenty v `rdi`, `rsi`,
//! `rdx`, `r10`, `r8`, `r9` (`rcx` a `r11` instrukce prepise). Vysledek
//! v `rax`: nezaporne cislo = uspech, zaporne = kod [`crate::Error`].
//!
//! Cisla nejsou stabilni rozhrani pro aplikace - to je knihovna libos.
//! Popis kazdeho volani: `docs/cs/syscally.md`, `docs/en/syscalls.md`.

/// Zapise text (UTF-8) do logu jadra. `(ptr, len)`
pub const DEBUG_LOG: u64 = 0;
/// Monotonicky cas v ns od startu. `()`
pub const CLOCK_MONOTONIC: u64 = 1;
/// Zavre handle. `(handle)`
pub const HANDLE_CLOSE: u64 = 2;
/// Novy handle na stejny objekt s podmnozinou prav. `(handle, rights) -> handle`
pub const HANDLE_DUPLICATE: u64 = 3;
/// Ceka na signaly jednoho objektu. `(handle, mask, deadline_ns, out_signals_ptr)`
pub const OBJECT_WAIT_ONE: u64 = 4;
/// Ceka na vice objektu. `(items_ptr, count, deadline_ns) -> index`
pub const OBJECT_WAIT_MANY: u64 = 5;
/// Zmeni signaly eventu. `(handle, clear, set)`
pub const OBJECT_SIGNAL: u64 = 6;
/// Vytvori event. `() -> handle`
pub const EVENT_CREATE: u64 = 7;
/// Vytvori par koncu kanalu. `(out_handles_ptr)` (dva u32)
pub const CHANNEL_CREATE: u64 = 8;
/// Posle zpravu. `(handle, data_ptr, data_len, handles_ptr, handles_count)`
pub const CHANNEL_WRITE: u64 = 9;
/// Precte zpravu. `(handle, data_ptr, data_cap, handles_ptr, handles_cap, out_actual_ptr)`
pub const CHANNEL_READ: u64 = 10;
/// Vytvori objekt pameti. `(size, process) -> handle`; stranky se strhnou
/// z rozpoctu jobu procesu `process` (handle s pravem `WRITE`, 0 = volajici)
pub const VMO_CREATE: u64 = 11;
/// Cte z objektu pameti. `(handle, offset, buf_ptr, len)`
pub const VMO_READ: u64 = 12;
/// Zapisuje do objektu pameti. `(handle, offset, buf_ptr, len)`
pub const VMO_WRITE: u64 = 13;
/// Namapuje objekt pameti. `(process, vmo, vmo_offset, len, flags, addr) -> addr`
pub const VM_MAP: u64 = 14;
/// Zrusi mapovani. `(process, addr, len)`
pub const VM_UNMAP: u64 = 15;
/// Vytvori prazdny proces v jobu. `(job, name_ptr, name_len) -> handle`;
/// job potrebuje pravo `MANAGE`
pub const PROCESS_CREATE: u64 = 16;
/// Spusti proces prvnim vlaknem. `(process, thread, entry, stack, bootstrap_handle)`
pub const PROCESS_START: u64 = 17;
/// Ukonci aktualni proces. `(exit_code)` - nevraci se
pub const PROCESS_EXIT: u64 = 18;
/// Ukonci jiny proces. `(process)`
pub const PROCESS_KILL: u64 = 19;
/// Navratovy kod skonceneho procesu. `(process, out_code_ptr)`
pub const PROCESS_EXIT_CODE: u64 = 20;
/// Vytvori vlakno v procesu. `(process, name_ptr, name_len) -> handle`
pub const THREAD_CREATE: u64 = 21;
/// Spusti vlakno v jiz bezicim procesu. `(thread, entry, stack, arg0, arg1)`
pub const THREAD_START: u64 = 22;
/// Ukonci aktualni vlakno. `()` - nevraci se
pub const THREAD_EXIT: u64 = 23;
/// Prenecha CPU. `()`
pub const THREAD_YIELD: u64 = 24;

/// Fyzicky souvisla pamet pro DMA. `(resource, size) -> handle`
pub const VMO_CREATE_CONTIGUOUS: u64 = 25;
/// Okno na registry zarizeni. `(resource, phys, size) -> handle`
pub const VMO_CREATE_PHYSICAL: u64 = 26;
/// Fyzicka adresa stranky objektu pameti. `(resource, vmo, offset) -> phys`
pub const VMO_PHYS_ADDR: u64 = 27;
/// Objekt preruseni s MSI zpravou. `(resource, out_msi_ptr) -> handle`
pub const INTERRUPT_CREATE: u64 = 28;
/// Potvrzeni preruseni (smaze SIGNALED). `(interrupt)`
pub const INTERRUPT_ACK: u64 = 29;
/// Omezeny prostredek pro ovladac.
/// `(resource, ranges_ptr, ranges_count, irq_max, dma_max, device) -> handle`
pub const RESOURCE_CREATE: u64 = 30;
/// Velikost stranky adresniho prostoru procesu. `(process) -> bajty`
pub const VM_PAGE_SIZE: u64 = 31;
/// Realny cas. `(out_ptr)` - zapise [`crate::time::Reading`] (16 B)
pub const CLOCK_REALTIME: u64 = 32;
/// Nastavi realny cas (Resource s pravem `SET_CLOCK`).
/// `(resource, in_ptr, flags) -> 1 = zapsano i do hardwarovych hodin`;
/// na `in_ptr` je [`crate::time::SetRequest`] (16 B), `flags` z [`crate::time::set_flags`]
pub const CLOCK_SET: u64 = 33;
/// Precte I/O port (x86). `(resource, port, width) -> hodnota`; width 1, 2, 4
pub const IOPORT_READ: u64 = 34;
/// Zapise I/O port (x86). `(resource, port, width, value)`
pub const IOPORT_WRITE: u64 = 35;
/// Objekt preruseni na pevne lince: x86_64 ISA IRQ 0-15 pres IOAPIC
/// (spousteni a polarita z MADT), aarch64 SPI 32-1019 pres GIC.
/// `(resource, line, flags) -> handle`; `flags` z [`line_flags`] (x86_64
/// je ignoruje, plati MADT)
pub const INTERRUPT_CREATE_LINE: u64 = 36;

/// Nahodne bajty z generatoru jadra. `(buf_ptr, len, flags) -> 1 = nasazeny`;
/// `len` nejvys [`RANDOM_MAX`], `flags` z [`random_flags`]
pub const RANDOM_GET: u64 = 37;
/// Prida entropii do zasobniku jadra (Resource s pravem `MANAGE`).
/// `(resource, ptr, len, credit_bits)`
pub const RANDOM_ADD: u64 = 38;
/// Vytvori port (fronta udalosti). `() -> handle`
pub const PORT_CREATE: u64 = 39;
/// Jednorazova registrace: az signaly objektu protnou masku, port dostane
/// paket s klicem. `(handle, port, key, mask)`
pub const OBJECT_WAIT_ASYNC: u64 = 40;
/// Vyzvedne paket z portu (ceka do terminu). `(port, deadline_ns, out_ptr)`;
/// na `out_ptr` zapise [`crate::object::PortPacket`] (16 B)
pub const PORT_WAIT: u64 = 41;

/// Vytvori podrizeny job. `(parent_job) -> handle`; rodic potrebuje `MANAGE`
pub const JOB_CREATE: u64 = 42;
/// Nastavi limit rozpoctu jobu. `(job, pool, bytes)`; job potrebuje `WRITE`,
/// `pool` z [`crate::job::pool`], `bytes` = [`crate::job::UNLIMITED`] = bez limitu
pub const JOB_SET_LIMIT: u64 = 43;
/// Stav rozpoctu jobu. `(job, out_ptr)`; job potrebuje `READ`, na `out_ptr`
/// zapise [`crate::job::JobInfo`] (48 B)
pub const JOB_INFO: u64 = 44;
/// Plynula korekce realneho casu (Resource s pravem `SET_CLOCK`).
/// `(resource, offset_ns i64, freq_ppb i64, flags, out_ptr)`; `flags`
/// z [`crate::time::adjust`], na `out_ptr` (0 = nic) zapise
/// [`crate::time::AdjustStatus`] po zmene
pub const CLOCK_ADJUST: u64 = 45;

/// Framebuffer z firmware (objekt z bootstrapu `FRAMEBUFFER`).
/// `(vmo, op, out_ptr)`; `op` z [`fb_op`]. `INFO` potrebuje `READ` a na
/// `out_ptr` zapise [`FbInfo`] (16 B); `CLAIM` a `RELEASE` potrebuji
/// `WRITE`. Jiny objekt pameti = `WRONG_TYPE`.
pub const FB_CONTROL: u64 = 46;

// 47 je vyhrazeno pro `VMO_REVOKE` (vetev Venus), 48 pro `JOB_PROCESSES`
// (vetev 6b).

/// Vypnuti nebo restart pocitace pres firmware (Resource s pravem `POWER`).
/// `(resource, op)`; `op` z [`power_op`]. aarch64: PSCI `SYSTEM_OFF`
/// a `SYSTEM_RESET`. x86_64: jen `RESET` (registr 0xCF9, pak radic
/// klavesnice); vypnuti na x86 dela sluzba ACPI zapisem do registru spanku
/// (S5), jadro AML nezna = `NOT_SUPPORTED`. Pri uspechu se nevraci.
pub const SYSTEM_POWER: u64 = 49;

/// Pocet definovanych syscallu (pro tabulky a testy).
pub const COUNT: u64 = 50;

/// Operace `SYSTEM_POWER`.
pub mod power_op {
    /// Vypnout (PSCI SYSTEM_OFF).
    pub const OFF: u64 = 0;
    /// Restartovat (PSCI SYSTEM_RESET, x86 reset CPU).
    pub const RESET: u64 = 1;
}

/// Nejvyssi cislo linky preruseni v `RESOURCE_CREATE` a
/// `INTERRUPT_CREATE_LINE` (vyhradne): x86_64 ISA IRQ 0-15, aarch64 cisla
/// preruseni GIC (SPI 32-1019).
pub const IRQ_LINES_MAX: u32 = 1024;

/// Operace `FB_CONTROL`.
pub mod fb_op {
    /// Geometrie a format pixelu.
    pub const INFO: u64 = 0;
    /// Vyslovne prevzeti: jadro prestane kreslit log do framebufferu (log
    /// dal jde na seriovou linku).
    /// Kdyz obrazovku drzi jiny zivy proces, `BAD_STATE` (predani: stary
    /// vlastnik `RELEASE` nebo konec procesu, pak novy `CLAIM`).
    pub const CLAIM: u64 = 1;
    /// Vraceni: jadro obrazovku smaze a kresli log dal. Jen vlastnik
    /// (proces, ktery naposledy zavolal `CLAIM`); jinak `BAD_STATE`.
    pub const RELEASE: u64 = 2;
}

/// Geometrie framebufferu (`FB_CONTROL`, `fb_op::INFO`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbInfo {
    pub width: u32,
    pub height: u32,
    /// Pixelu na radek (muze byt vic nez `width`).
    pub stride: u32,
    /// 1 = R, G, B v pameti; 2 = B, G, R (32 bitu na pixel).
    pub format: u32,
}

impl FbInfo {
    pub const SIZE: usize = 16;

    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        for (i, v) in [self.width, self.height, self.stride, self.format].iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn decode(b: &[u8; 16]) -> Self {
        let v = |i: usize| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        Self { width: v(0), height: v(1), stride: v(2), format: v(3) }
    }
}

/// Priznaky `INTERRUPT_CREATE_LINE` (spousteni linky; na x86_64 rozhoduje
/// MADT a priznaky se ignoruji).
pub mod line_flags {
    /// Spousteni hranou (bez priznaku urovni).
    pub const EDGE: u64 = 1;
    pub const ALL: u64 = EDGE;
}

/// Nejvic bajtu v jednom `RANDOM_GET` / `RANDOM_ADD` (jedno volani = jedna
/// kopie pres buffer jadra; vic dat = vic volani).
pub const RANDOM_MAX: usize = 4096;

/// Priznaky `RANDOM_GET`.
pub mod random_flags {
    /// Nenasazeny generator = `SHOULD_WAIT` misto slabych dat (pro klice).
    pub const REQUIRE_SEEDED: u64 = 1;
    pub const ALL: u64 = REQUIRE_SEEDED;
}

/// Deadline "cekat neomezene".
pub const DEADLINE_INFINITE: u64 = u64::MAX;

/// Limity jedne zpravy kanalu. Zprava se kopiruje jednim syscallem pres
/// buffer jadra; vetsi data patri do objektu pameti (handle ve zprave).
pub const CHANNEL_MAX_DATA: usize = 64 * 1024;
pub const CHANNEL_MAX_HANDLES: usize = 64;
/// Rizeni toku: nejvic zprav cekajicich ve fronte jednoho konce kanalu.
/// Pamet front hlida rozpocet jobu odesilatele; tento limit jen brani tomu,
/// aby odesilatel predbehl prijemce neomezene daleko (`SHOULD_WAIT`).
pub const CHANNEL_MAX_QUEUED: usize = 1024;
/// Nejvic objektu v jednom `OBJECT_WAIT_MANY` (pole lezi na zasobniku
/// jadra). Na vic objektu je port (`OBJECT_WAIT_ASYNC`), bez limitu poctu.
pub const WAIT_MANY_MAX: usize = 64;
/// Maximalni delka jmena procesu nebo vlakna (bajty UTF-8).
pub const NAME_MAX: usize = 64;
/// Maximalni delka zpravy DEBUG_LOG.
pub const DEBUG_LOG_MAX: usize = 1024;

/// Druh rozsahu v `RESOURCE_CREATE` (horni dva bity zacatku rozsahu):
/// 0 = fyzicka pamet (zarovnana na stranky), [`RANGE_IO_PORTS`] = I/O porty,
/// [`RANGE_IRQ_LINES`] = stare linky preruseni (ISA IRQ).
pub const RANGE_KIND_MASK: u64 = 3 << 62;
/// Rozsah I/O portu: zacatek = `RANGE_IO_PORTS | port`, delka = pocet portu.
pub const RANGE_IO_PORTS: u64 = 1 << 62;
/// Rozsah linek preruseni: zacatek = `RANGE_IRQ_LINES | linka`, delka = pocet.
pub const RANGE_IRQ_LINES: u64 = 2 << 62;

/// Hodnota argumentu `device` syscallu `RESOURCE_CREATE` pro zarizeni PCI.
pub const fn pci_device(segment: u16, bus: u8, device: u8, function: u8) -> u64 {
    1 << 32
        | (segment as u64) << 16
        | (bus as u64) << 8
        | ((device as u64 & 0x1F) << 3)
        | (function as u64 & 7)
}
