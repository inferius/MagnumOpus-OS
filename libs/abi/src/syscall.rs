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
/// Vytvori objekt pameti. `(size, process, flags, capacity) -> handle`;
/// stranky se strhnou z rozpoctu jobu procesu `process` (handle s pravem
/// `WRITE`, 0 = volajici). `flags`: `vm::VMO_PINNED` = stranky hned
/// a pripnute (pro DMA, navic rozpocet `DMA`), `vm::VMO_RESIZABLE` =
/// zvetsitelny s kapacitou `capacity` (jinak se `capacity` ignoruje), jinak 0
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
/// Okno na fyzicky rozsah mimo RAM (registry, BAR).
/// `(resource, phys, size, cache) -> handle`; `cache` z [`crate::vm::cache`]
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
/// zapise [`crate::job::JobInfo`] (80 B)
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

/// Odvola fyzicky objekt pameti: zrusi jeho mapovani ve vsech procesech
/// a dalsi `VM_MAP` selze (`ACCESS_DENIED`). Pristup pres stare mapovani
/// = vyjimka stranky. `(resource, vmo)`; rozsah objektu musi prostredek
/// povolovat.
pub const VMO_REVOKE: u64 = 47;

/// Procesy jobu a jeho potomku. `(job, out_ptr, max) -> pocet`; job
/// potrebuje `READ`, na `out_ptr` zapise nejvys `max` zaznamu
/// [`crate::job::ProcEntry`] (64 B), vrati celkovy pocet (i nad `max`)
pub const JOB_PROCESSES: u64 = 48;

/// Vypnuti nebo restart pocitace pres firmware (Resource s pravem `POWER`).
/// `(resource, op)`; `op` z [`power_op`]. aarch64: PSCI `SYSTEM_OFF`
/// a `SYSTEM_RESET`. x86_64: jen `RESET` (registr 0xCF9, pak radic
/// klavesnice); vypnuti na x86 dela sluzba ACPI zapisem do registru spanku
/// (S5), jadro AML nezna = `NOT_SUPPORTED`. Pri uspechu se nevraci.
pub const SYSTEM_POWER: u64 = 49;

/// Planovani: tridy QoS, strop jobu, Realtime rozpocet, udaje o CPU.
/// `(op, ...)`; operace a jejich argumenty v [`crate::sched::op`]
pub const SCHED_CONTROL: u64 = 50;


/// Cteni logu jadra (Resource s pravem `DIAG`). `(resource, cursor,
/// buf_ptr, buf_len, out_start_ptr) -> bajty`. Log je proud bajtu (radky
/// UTF-8); `cursor` = pozice v proudu (pocet kdy zapsanych bajtu). Kdyz
/// starsi data uz kruhovy buffer prepsal, cte se od nejstarsich dostupnych;
/// skutecny zacatek zapise na `out_start_ptr` (u64), takze ctenar pozna
/// ztratu. Nova pozice = zacatek + vracena delka. 0 = nic noveho.
pub const LOG_READ: u64 = 51;

/// Zaznam padu jadra z minuleho behu (pstore, `libs/crashrec`; Resource
/// s pravem `DIAG`). `(resource, op, buf_ptr, buf_len) -> bajty`; `op`
/// z [`crash_op`].
pub const CRASH_RECORD: u64 = 52;

/// Rizeni logu jadra za behu (Resource s pravem `DIAG`). `(resource, op,
/// buf_ptr, buf_len) -> bajty`; `op` z [`log_op`]. Text v bufferu je ASCII.
pub const LOG_CONTROL: u64 = 53;

/// Ukonci vsechny procesy jobu a jeho potomku a job uzavre: novy proces
/// ani podjob v nem uz nevznikne (`BAD_STATE`). `(job, code)`; job
/// potrebuje `MANAGE`, `code` = navratovy kod ukoncenych procesu. Volajici
/// uvnitr zabijeneho jobu skonci take (pri navratu ze syscallu).
pub const JOB_KILL: u64 = 54;

/// Futex: `(op, addr, value, deadline) -> vysledek`, `op` z [`futex_op`].
/// Fronta vlaken cekajicich na zmenu u32 v pameti vlastniho procesu (klic =
/// virtualni adresa, zarovnana na 4 B). Kontrola hodnoty a zarazeni do
/// fronty probehnou pod zamkem fronty, ktery bere i probuzeni.
pub const FUTEX: u64 = 55;

/// Operace [`FUTEX`].
pub mod futex_op {
    /// `value` = ocekavana hodnota: kdyz u32 na `addr` je rovne `value`,
    /// vlakno usne, dokud ho neprobudi `WAKE` na stejne adrese nebo
    /// nevyprsi `deadline` (ns od startu, [`super::DEADLINE_INFINITE`] = bez
    /// limitu; vyprseni = `TIMED_OUT`). Jina hodnota = hned 0. Planne
    /// probuzeni je mozne (volajici overi podminku znovu). Vraci 0.
    pub const WAIT: u64 = 0;
    /// `value` = nejvic probuzenych vlaken (`u32::MAX` = vsechna); vraci
    /// pocet probuzenych. `deadline` se nepouziva.
    pub const WAKE: u64 = 1;
}

/// Ukazatel vlakna: `(value, which) -> 0`. `which` = [`thread_pointer::TLS`]
/// (0): registr pro TLS (x86_64 `FS_BASE`, aarch64 `TPIDR_EL0`);
/// [`thread_pointer::SECOND`] (1): druhy ukazatel vlakna (x86_64 baze GS,
/// napr. TEB Windows vrstvy; aarch64 `NOT_SUPPORTED`). Jadro hodnotu
/// uklada a obnovuje pri kazdem prepnuti; nove vlakno zacina s 0. Musi
/// byt adresa userspace.
pub const THREAD_SET_POINTER: u64 = 56;

/// Rezervace adres: `(process, addr, len, flags) -> addr`. Zabere rozsah
/// adresniho prostoru bez pameti (`vm::MAP_FIXED` = presne na `addr`,
/// `vm::RESERVE_GUARD` = ochranna oblast). Mapovat do nej jde jen
/// s `vm::MAP_IN_RESERVATION`. Proces `WRITE`.
pub const VM_RESERVE: u64 = 57;

/// Zmena prav: `(process, addr, len, prot) -> 0`. Prava mapovani v rozsahu
/// (i casti); `prot` podmnozina `MAP_READ | MAP_WRITE | MAP_EXECUTE`, 0 =
/// zadny pristup, nikdy W+X, nikdy nad prava pri `VM_MAP`. Proces `WRITE`.
pub const VM_PROTECT: u64 = 58;

/// Dotaz na oblast: `(process, addr, out_ptr) -> 0`. Zapise
/// `vm::RegionInfo` (40 B) oblasti nebo volneho mista na `addr`. Proces
/// `READ`.
pub const VM_QUERY: u64 = 59;

/// Klon s kopii pri zapisu: `(vmo, offset, len, process) -> handle`. Novy
/// objekt `len` bajtu sdili stranky `vmo` od `offset`; prvni zapis
/// kterehokoli z nich si udela kopii. Strzeno z rozpoctu jobu `process`
/// (handle s `WRITE`; 0 = volajici). Zdroj `READ`; jen bezne objekty.
pub const VMO_CLONE: u64 = 60;

/// Pecet objektu pameti: `(vmo) -> 0`. Obsah se uz nikdy nezmeni
/// (`VMO_WRITE`, mapovani pro zapis a `VMO_PHYS_ADDR` selzou). Bez
/// opravneni JIT jde spoustet jen zapecetena pamet. Nevratne. Pravo
/// `WRITE`; objekt nesmi mit mapovani, ktere smi zapisovat.
pub const VMO_SEAL: u64 = 61;

/// Pozastaveni vlakna: `(thread) -> token`. Vlakno se zastavi pred
/// navratem do userspace (vlakno spici v syscallu se pocita jako
/// pozastavene hned); signal `SUSPENDED`. Zavreni tokenu vlakno obnovi,
/// vic tokenu = stoji, dokud existuje aspon jeden. Pravo `MANAGE`.
pub const THREAD_SUSPEND: u64 = 62;

/// Kontext vlakna: `(thread, op, kind, ptr, len) -> 0`. `op` =
/// `thread::op::READ` / `WRITE`, `kind` = `thread::kind::GENERAL` / `FP`,
/// struktura podle architektury (`abi::thread`). Jen u pozastaveneho
/// vlakna. Cteni pravo `READ`, zapis `WRITE`; zapis jadro overi a vlakno
/// ho pouzije pred navratem do userspace.
pub const THREAD_STATE: u64 = 63;

/// Kanal vyjimek: `(task, flags) -> channel`. `task` = proces nebo vlakno
/// (pravo `MANAGE`); `flags` = `exception::channel::DEBUGGER` (jen proces)
/// nebo 0; `HANDLER_THREAD` na vlakne ho jen oznaci jako vlakno obsluhy
/// (vraci 0). Na kazde misto nejvys jeden zivy kanal.
pub const EXCEPTION_CHANNEL: u64 = 64;

/// Rozhodnuti o vyjimce: `(token, action) -> 0`, `action` z
/// `exception::action`. Vlakno pokracuje (RESUME), jde k dalsi obsluze
/// (NEXT) nebo proces skonci (KILL).
pub const EXCEPTION_RESOLVE: u64 = 65;

/// Vlakna procesu: `(process, out_ptr, max) -> pocet`. Zapise az `max`
/// handlu (u32) zivych vlaken; prava podle handlu procesu (`READ` = cist
/// kontext, s `WRITE` i zapsat a pozastavit). Vraci celkovy pocet.
pub const PROCESS_THREADS: u64 = 66;

/// Doruceni vyjimky primo ve vlaknu: `(op, a0, a1) -> ...`, operace
/// `exception::handler` (SET_ENTRY s handlem procesu a pravem `WRITE`,
/// SET_STACK, RETURN). Obsluha dostane ramec `exception::frame` na
/// zasobniku; poradi: debugger, obsluha ve vlaknu, kanal vlakna, kanal
/// procesu, debugger.
pub const EXCEPTION_HANDLER: u64 = 67;

/// Novy plot: `(initial, flags) -> handle`; `flags` = 0. Handle ma prava
/// `BASIC | SIGNAL`, spotrebiteli se predava duplikat jen s `WAIT`
/// (`abi::fence`).
pub const FENCE_CREATE: u64 = 68;

/// Posun plotu: `(fence, value, flags) -> 0`, pravo `SIGNAL`; `flags` =
/// `fence::signal::FAILED` = trvala chyba od bodu `value`.
pub const FENCE_SIGNAL: u64 = 69;

/// Cekani na hodnoty plotu: `(items_ptr, count, flags, deadline,
/// out_index_ptr) -> index`. Polozky `fence::WaitItem` (pravo `WAIT`), nejvys
/// `WAIT_MANY_MAX`; `flags` = `fence::wait::ALL` (jinak staci jedna).
/// Chyba plotu (`FENCE_FAILED`, `FENCE_ABANDONED`) ukonci cekani; index
/// rozhodujici polozky se zapise na `out_index_ptr` (u32, 0 = nezapisovat).
pub const FENCE_WAIT: u64 = 70;

/// Jednorazova registrace do portu: `(fence, value, port, key) -> 0`. Az
/// hodnota nastane (nebo chyba), port dostane paket s `key`; vysledek zjisti
/// `FENCE_QUERY`. Plot pravo `WAIT`, port `WRITE`.
pub const FENCE_WAIT_ASYNC: u64 = 71;

/// Stav plotu: `(fence, out_ptr) -> 0`, zapise `fence::Info` (24 B); pravo
/// `WAIT`.
pub const FENCE_QUERY: u64 = 72;

/// Zpristupni cast pripnuteho objektu pameti zarizeni: `(resource, vmo,
/// offset, len, flags, iova_hint) -> handle` mapovani (`abi::dma`). Prostredek
/// s pravem `MANAGE` a identitou zarizeni, objekt pameti s pravem `DMA` (a
/// `READ`/`WRITE` podle smeru). S IOMMU jeden souvisly rozsah IOVA (vybere
/// jadro, nebo `iova_hint` overi na prekryv), bez IOMMU fyzicke useky.
/// Zavreni handlu = odmapovani se synchronnim vyprazdnenim IOTLB.
pub const DMA_MAP: u64 = 73;

/// Useky mapovani pro zarizeni: `(map, out_ptr, max) -> pocet`. Zapise
/// nejvys `max` polozek `dma::Segment` (16 B), vrati pocet vsech; pravo
/// `READ`.
pub const DMA_SEGMENTS: u64 = 74;

/// Udrzba cache mapovani: `(map, offset, len, direction) -> 0`
/// (`dma::sync`); na koherentnich platformach nedela nic. Pravo `READ`.
pub const DMA_SYNC: u64 = 75;

/// Preruseni vlakna: `(thread, code) -> 0`. Nastavi cekajici preruseni
/// `code` (0..63, `INTERRUPT_CODES`; stejny kod se slouci) a probudi vlakno
/// z blokujiciho cekani, ktere vrati `INTERRUPTED`. Pri navratu do
/// userspace dostane vlakno doruceni na obsluhu ve vlakne
/// (`EXCEPTION_HANDLER`) s druhem `exception::kind::INTERRUPT` a kodem
/// v `Report::code` (nejnizsi cekajici kod prvni). Obecny zaklad pro
/// `kill`/`pthread_kill` mezi procesy, Ctrl+C, `SIGCHLD` a pozastaveni
/// vlaken Windows vrstvy. Pravo `MANAGE`; proces vlakna bez obsluhy ve
/// vlakne = `BAD_STATE`.
pub const THREAD_INTERRUPT: u64 = 76;

/// Pocet kodu preruseni (`THREAD_INTERRUPT`).
pub const INTERRUPT_CODES: u64 = 64;

/// Registr pro `THREAD_SET_POINTER`.
pub mod thread_pointer {
    /// Ukazatel pro TLS (x86_64 `FS_BASE`, aarch64 `TPIDR_EL0`).
    pub const TLS: u64 = 0;
    /// Druhy ukazatel vlakna: x86_64 baze GS (`gs:` v programu), aarch64
    /// nema (`NOT_SUPPORTED`; Windows ARM64 pouziva `x18`, ten je v kontextu).
    pub const SECOND: u64 = 1;
}

/// Cteni a zapis pameti jineho procesu kopii pres jadro:
/// `(process, op, addr, buf, len) -> prenesene bajty` (`process::memory`).
/// `READ` kopiruje z `addr` v cilovem procesu do `buf` volajiciho (pravo
/// `READ_MEMORY`), `WRITE` z `buf` do `addr` (pravo `WRITE_MEMORY`, jen do
/// mapovani se zapisem - kod ani zapecetenou pamet nezmeni). Chyba
/// v pameti cile po prvnim bloku = mene bajtu (castecna kopie), chyba
/// hned na zacatku nebo v bufferu volajiciho = chyba. Prava ma jen tvurce
/// procesu (vychozi handle z `PROCESS_CREATE`), nikdy vycet procesu
/// (riziko: `WRITE_MEMORY` = uplna vlada nad cilem, docs/cs/windows-vrstva.md).
pub const PROCESS_MEMORY: u64 = 77;

/// Zmeni velikost zvetsitelneho objektu pameti: `(vmo, size) -> 0`
/// (`vm::VMO_RESIZABLE`, nejvys kapacita; pravo `RESIZE`). Zvetseni slibi
/// nove stranky hned (`NO_MEMORY`, kdyz nejsou); zmenseni je u vsech
/// mapovani odmapuje a uvolni (dalsi pristup = vyjimka `BEYOND_END`).
/// Pripnuta stranka za novym koncem = `BAD_STATE`, zapeceteny objekt =
/// `ACCESS_DENIED`, objekt bez `VMO_RESIZABLE` = `NOT_SUPPORTED`.
pub const VMO_SET_SIZE: u64 = 78;
/// Aktualni velikost objektu pameti v bajtech: `(vmo) -> size` (jakekoli
/// pravo; u bezneho objektu pevna).
pub const VMO_SIZE: u64 = 79;

/// Pocet definovanych syscallu (pro tabulky a testy).
pub const COUNT: u64 = 80;

/// Operace `LOG_CONTROL`.
pub mod log_op {
    /// Nastavi uroven: buffer `modul=uroven` (modul jadra jako `sched`,
    /// `arch::x86_64::apic`, nebo jmeno procesu pro jeho `DEBUG_LOG`;
    /// uroven `error`, `warn`, `info`, `debug`, `trace`).
    pub const SET: u64 = 0;
    /// Zrusi prepsani: buffer = modul.
    pub const CLEAR: u64 = 1;
    /// Vypis prepsani do bufferu (`modul=uroven` po radcich, posledni radek
    /// `potlaceno=N` = opakovane hlasky); vraci delku.
    pub const LIST: u64 = 2;
}

/// Operace `CRASH_RECORD`.
pub mod crash_op {
    /// Zkopiruje zaznam z minuleho behu (cely format `crashrec`, hlavicka
    /// a text) do bufferu; vraci delku, 0 = zadny zaznam. Maly buffer =
    /// `BUFFER_TOO_SMALL`.
    pub const READ: u64 = 0;
    /// Smaze zaznam (sluzba logu ho ulozila na disk).
    pub const CLEAR: u64 = 1;
    /// Jen testovaci jadro (`cargo xtask test`): vyvola paniku jadra; jadro
    /// zapise zaznam a hned restartuje (test, ze zaznam prezije restart).
    /// Jinde `NOT_SUPPORTED`.
    pub const TEST_PANIC: u64 = 2;
    /// Velikost oblasti zaznamu (horni mez bufferu pro `READ`); 0 = oblast
    /// neni (loader ji nedostal).
    pub const CAPACITY: u64 = 3;
}

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
/// Maximalni delka zpravy DEBUG_LOG (delsi jadro odmitne s chybou; libos
/// ji proto deli, `debug_log_chunks`).
pub const DEBUG_LOG_MAX: usize = 1024;

/// Kusy zpravy pro DEBUG_LOG: kazdy nejvys `max` bajtu, deleni na hranici
/// znaku, prednostne za koncem radku. Spojene daji puvodni text (dlouha
/// zprava se tak neztrati cela, napr. chyba testu s vypisem stromu).
pub fn debug_log_chunks(text: &str, max: usize) -> impl Iterator<Item = &str> {
    let mut rest = text;
    core::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        if rest.len() <= max {
            return Some(core::mem::take(&mut rest));
        }
        let mut cut = max.max(1);
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut == 0 {
            // `max` mensi nez prvni znak: cely znak (jinak by se necyklilo).
            cut = rest.chars().next().map_or(rest.len(), char::len_utf8);
        } else if let Some(nl) = rest[..cut].rfind('\n') {
            cut = nl + 1;
        }
        let (head, tail) = rest.split_at(cut);
        rest = tail;
        Some(head)
    })
}

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
