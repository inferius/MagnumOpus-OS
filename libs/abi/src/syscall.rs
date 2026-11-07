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
/// Vytvori objekt pameti. `(size) -> handle`
pub const VMO_CREATE: u64 = 11;
/// Cte z objektu pameti. `(handle, offset, buf_ptr, len)`
pub const VMO_READ: u64 = 12;
/// Zapisuje do objektu pameti. `(handle, offset, buf_ptr, len)`
pub const VMO_WRITE: u64 = 13;
/// Namapuje objekt pameti. `(process, vmo, vmo_offset, len, flags, addr) -> addr`
pub const VM_MAP: u64 = 14;
/// Zrusi mapovani. `(process, addr, len)`
pub const VM_UNMAP: u64 = 15;
/// Vytvori prazdny proces. `(name_ptr, name_len) -> handle`
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

/// Nahodne bajty z generatoru jadra. `(buf_ptr, len, flags) -> 1 = nasazeny`;
/// `len` nejvys [`RANDOM_MAX`], `flags` z [`random_flags`]
pub const RANDOM_GET: u64 = 34;
/// Prida entropii do zasobniku jadra (Resource s pravem `MANAGE`).
/// `(resource, ptr, len, credit_bits)`
pub const RANDOM_ADD: u64 = 35;
/// Vytvori port (fronta udalosti). `() -> handle`
pub const PORT_CREATE: u64 = 36;
/// Jednorazova registrace: az signaly objektu protnou masku, port dostane
/// paket s klicem. `(handle, port, key, mask)`
pub const OBJECT_WAIT_ASYNC: u64 = 37;
/// Vyzvedne paket z portu (ceka do terminu). `(port, deadline_ns, out_ptr)`;
/// na `out_ptr` zapise [`crate::object::PortPacket`] (16 B)
pub const PORT_WAIT: u64 = 38;

/// Pocet definovanych syscallu (pro tabulky a testy).
pub const COUNT: u64 = 39;

/// Nejvic bajtu v jednom `RANDOM_GET` / `RANDOM_ADD`.
pub const RANDOM_MAX: usize = 4096;
/// Nejvic registraci a cekajicich paketu jednoho portu.
pub const PORT_MAX_PENDING: usize = 65_536;

/// Priznaky `RANDOM_GET`.
pub mod random_flags {
    /// Nenasazeny generator = `SHOULD_WAIT` misto slabych dat (pro klice).
    pub const REQUIRE_SEEDED: u64 = 1;
    pub const ALL: u64 = REQUIRE_SEEDED;
}

/// Deadline "cekat neomezene".
pub const DEADLINE_INFINITE: u64 = u64::MAX;

/// Limity zprav kanalu.
pub const CHANNEL_MAX_DATA: usize = 64 * 1024;
pub const CHANNEL_MAX_HANDLES: usize = 64;
/// Maximalni pocet zprav cekajicich ve fronte jednoho konce kanalu.
pub const CHANNEL_MAX_QUEUED: usize = 1024;
/// Maximalni pocet objektu v jednom `OBJECT_WAIT_MANY`.
pub const WAIT_MANY_MAX: usize = 64;
/// Maximalni delka jmena procesu nebo vlakna (bajty UTF-8).
pub const NAME_MAX: usize = 64;
/// Maximalni delka zpravy DEBUG_LOG.
pub const DEBUG_LOG_MAX: usize = 1024;

/// Hodnota argumentu `device` syscallu `RESOURCE_CREATE` pro zarizeni PCI.
pub const fn pci_device(segment: u16, bus: u8, device: u8, function: u8) -> u64 {
    1 << 32
        | (segment as u64) << 16
        | (bus as u64) << 8
        | ((device as u64 & 0x1F) << 3)
        | (function as u64 & 7)
}
