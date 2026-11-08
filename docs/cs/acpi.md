# ACPI a napájení

Služba **`acpid`** (`user/acpid`) spouští v userspace **ACPICA** – referenční
implementaci ACPI od Intelu – a dává systému napájení: vypnutí (S5),
restart, událost tlačítka napájení, teploty termálních zón (`_TZ`), stav
baterií a zjištění, co firmware umí pro řízení frekvence CPU (`_PSS`,
CPPC). Funguje na x86_64 (QEMU q35, běžné PC) i aarch64 (QEMU `virt`
s ACPI z UEFI).

> **Stav:** hotové vypnutí, restart, tlačítko napájení (x86 pevná událost,
> ARM Generic Event Device), čtení teplot a baterií, příkazy shellu
> `shutdown` a `reboot`. Řízení frekvence CPU je jen **připravené**
> (služba zjistí `_PSS`/`_CPC`, nic nemění). Uspání (S3) není.

## Proč ACPICA a proč v userspace

Tabulky ACPI obsahují kromě statických údajů (MADT, MCFG, HPET – ty čte
jádro samo, viz [Jádro](jadro.md)) i **AML**: bajtkód firmwaru, který se
musí interpretovat (metody `_PTS`, `_S5`, `_TMP`, `_BST`, `_EVT`, ...).
Psát vlastní interpret AML nemá smysl – ACPICA je ověřená na tisících
desek a používají ji Linux, FreeBSD, Haiku i Zephyr. Převzali jsme ji beze
změn (jediný zásah: výběr platformy v `acenv.h`).

AML je kód firmwaru s přístupem ke všemu, co si sám nadeklaruje. Chyba
v něm nebo v interpretu by v jádře znamenala pád systému; v userspace
spadne jen služba. Jádro navíc hlídá, že AML nesáhne na RAM jádra ani
procesů (viz [Bezpečnost](#bezpečnost)).

<figure class="diagram">
<svg viewBox="0 0 780 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Architektura sluzby ACPI">
<defs><marker id="ac1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="180" height="70" rx="6"/><text class="t b" x="100" y="45" text-anchor="middle">shell (shsess)</text><text class="m" x="100" y="63" text-anchor="middle">shutdown, reboot</text><text class="m" x="100" y="79" text-anchor="middle">kanál od termd</text>
<path class="ln" d="M190 55 L268 55" marker-end="url(#ac1)"/>
<rect class="acc" x="270" y="20" width="220" height="70" rx="6"/><text class="t b" x="380" y="45" text-anchor="middle">init (supervisor)</text><text class="m" x="380" y="63" text-anchor="middle">ukončí služby (FS před disky)</text><text class="m" x="380" y="79" text-anchor="middle">pak Request::Shutdown</text>
<path class="ln" d="M380 90 L380 128" marker-end="url(#ac1)"/>
<path class="ln" d="M560 150 Q520 70 492 60" marker-end="url(#ac1)"/>
<text class="m" x="600" y="95" text-anchor="middle">Event::PowerButton</text>
<rect class="ok" x="200" y="130" width="360" height="120" rx="6"/><text class="t b" x="380" y="152" text-anchor="middle">acpid (userspace)</text>
<rect class="box" x="215" y="162" width="150" height="74" rx="4"/><text class="t" x="290" y="182" text-anchor="middle">ACPICA (C)</text><text class="m" x="290" y="200" text-anchor="middle">AML, tabulky,</text><text class="m" x="290" y="216" text-anchor="middle">události, S5</text>
<rect class="box" x="395" y="162" width="150" height="74" rx="4"/><text class="t" x="470" y="182" text-anchor="middle">vrstva OS (Rust)</text><text class="m" x="470" y="200" text-anchor="middle">AcpiOs* nad libos</text><text class="m" x="470" y="216" text-anchor="middle">cstd (libc)</text>
<path class="ln" d="M365 199 L393 199" marker-end="url(#ac1)"/>
<path class="ln" d="M470 250 L470 278" marker-end="url(#ac1)"/>
<rect class="acc" x="200" y="280" width="360" height="44" rx="6"/><text class="t b" x="380" y="298" text-anchor="middle">jádro</text><text class="m" x="380" y="314" text-anchor="middle">mapování (registry, paměť ACPI), porty, linky, SYSTEM_POWER</text>
<rect class="box" x="600" y="160" width="170" height="90" rx="6"/><text class="t b" x="685" y="183" text-anchor="middle">firmware</text><text class="m" x="685" y="201" text-anchor="middle">tabulky v paměti ACPI</text><text class="m" x="685" y="217" text-anchor="middle">PM1, GPE, GED, reset</text><text class="m" x="685" y="233" text-anchor="middle">PSCI (ARM)</text>
<path class="ln" d="M560 199 L598 199" marker-end="url(#ac1)"/>
</svg>
<figcaption>Shell žádá init, init čistě ukončí služby a teprve pak požádá acpid; acpid přes ACPICA a vrstvu OS sahá na firmware a hardware jen přes jádro.</figcaption>
</figure>

## Části

| Část | Kde | Co dělá |
|---|---|---|
| ACPICA | `third_party/acpica` | převzaté zdroje, verze **20250404** (tag `R2025_04_04`), licence BSD-3 (soubor `THIRD_PARTY_LICENSES.md` v kořeni repozitáře) |
| konfigurace platformy | `libs/acpica/include/acmagnum.h` | 64 bitů, knihovna C = cstd, vlastní cache objektů, globální zámek |
| překlad a rozhraní | `libs/acpica` | `build.rs` (přes [cbuild](preklad-c.md)), `ffi.rs` (deklarace), `ns.rs` (jmenný prostor) |
| lepidlo v C | `libs/acpica/c/glue.c` | `AcpiOsPrintf`, globální zámek FACS, čtení FADT a `_CRS` |
| vrstva OS | `user/acpid/src/osl` | všech 37 funkcí `AcpiOs*`, které ACPICA volá |
| služba | `user/acpid/src` | start ACPICA, události, napájení, čidla |
| protokol | `libs/powerproto` | požadavky, odpovědi, události |
| init | `user/init/src/power.rs` | spuštění acpid, čisté vypnutí, požadavky shellu |

## Start služby

Init spustí `acpid` hned po službě času (inicializace ACPICA běží souběžně
se startem ostatních služeb) a předá mu tři handly:

1. **Resource s rozsahy pro ACPI** (`MANAGE`): celý fyzický prostor (jádro
   z něj povolí jen registry zařízení a paměť ACPI, viz níže), na x86 všechny
   I/O porty a linky ISA 0-15, na aarch64 linky SPI 32-1023; nejvýš 16
   objektů přerušení, žádná paměť pro DMA.
2. **Resource jen s právem `POWER`** (bez `MANAGE`) pro syscall
   `SYSTEM_POWER` (vypnutí a restart přes firmware).
3. **Údaje o ACPI od jádra** (`abi::acpi::Info`, bootstrap `kind::ACPI`):
   adresa a kopie RSDP a seznam oblastí paměti firmwaru s tabulkami.

Pak `acpid` inicializuje ACPICA v pořadí podle referenční příručky:

```text
AcpiInitializeSubsystem   -> globální stav, cache objektů
AcpiInitializeTables      -> RSDP (z kopie od jádra), XSDT, seznam tabulek
(MCFG -> okna ECAM pro konfiguraci PCI)
AcpiLoadTables            -> DSDT a SSDT do jmenného prostoru
AcpiEnableSubsystem       -> režim ACPI (SMI_CMD), obsluha SCI (x86)
AcpiInitializeObjects     -> _REG, _INI
AcpiUpdateAllGpes         -> GPE s metodami _Lxx/_Exx
```

a vypíše např.:

```text
acpid: SCI na lince 9
acpid: MCFG: 1 oken ECAM
acpid: ACPI pripraveno (FADT rev 3, OEM BOCHS, S5, tlacitko, zony 0, baterie 0, CPU 4)
```

Když inicializace selže, služba běží dál a na požadavky odpovídá
`NOT_SUPPORTED` (init pak nemůže vypnout přes ACPI; zůstává restart
z jádra).

## Vrstva OS (`AcpiOs*`)

ACPICA volá systém jen přes funkce `AcpiOs*` (`acpiosxf.h`). Jsou v Rustu
s ABI C (`user/acpid/src/osl`), kromě dvou v C (`glue.c`).

| Skupina | Funkce | Jak |
|---|---|---|
| paměť | `AcpiOsAllocate`, `AcpiOsFree` | `malloc`/`free` z cstd nad heapem programu |
| mapování | `AcpiOsMapMemory`, `AcpiOsUnmapMemory` | po stránkách přes `VMO_CREATE_PHYSICAL` + `VM_MAP`; RSDP z kopie od jádra |
| registry v paměti | `AcpiOsReadMemory`, `AcpiOsWriteMemory` | dočasné mapování, přístup `volatile` šířky 8-64 bitů |
| porty | `AcpiOsReadPort`, `AcpiOsWritePort` | `IOPORT_READ`/`IOPORT_WRITE` (na aarch64 chyba) |
| PCI | `AcpiOsReadPciConfiguration`, `...Write...` | okna ECAM z MCFG (stránka na funkci, mapovaná jednou); x86 bez MCFG porty 0xCF8/0xCFC |
| přerušení | `AcpiOsInstallInterruptHandler` | `INTERRUPT_CREATE_LINE` + vlákno: čeká, zavolá obsluhu ACPICA, pak potvrdí (úroveň se odmaskuje, až obsluha smaže stav) |
| zámky | `AcpiOsCreateLock`, `Acquire`, `Release` | atomický příznak; místo točení s vypnutými přerušeními přenechá CPU |
| semafory | `AcpiOsCreateSemaphore`, `Wait`, `Signal` | počítadlo pod zámkem + event jádra signalizovaný, dokud je počítadlo nenulové; ACPICA z nich skládá i mutexy |
| odložená práce | `AcpiOsExecute`, `AcpiOsWaitEventsComplete` | dvě fronty s vlastním vláknem: oznámení `Notify` zvlášť od GPE (jako Linux, jinak hrozí uváznutí) |
| čas | `AcpiOsSleep`, `AcpiOsStall`, `AcpiOsGetTimer` | `time::sleep`, krátké aktivní čekání, monotonní čas v 100 ns |
| vlákna | `AcpiOsGetThreadId` | podle zásobníku: každé vlákno, které volá ACPICA, se zaregistruje s vrcholem zásobníku (libos nemá TLS) |
| výpis | `AcpiOsPrintf`, `AcpiOsVprintf` | v C, `vprintf` z cstd -> log po řádcích s předponou `acpid:` |
| globální zámek | `ACPI_ACQUIRE_GLOBAL_LOCK` | v C, atomický algoritmus ze specifikace nad polem FACS |
| ostatní | přepisy tabulek a jmen, `AcpiOsSignal`, `AcpiOsEnterSleep` | nepoužíváme (firmware se bere, jak je), AML `Fatal` jen do logu |

## Události tlačítek

| Platforma | Zdroj | Jak se pozná |
|---|---|---|
| x86 (q35, PC) | pevná událost PM1 (`PWRBTN_STS`) | SCI -> ACPICA -> `AcpiInstallFixedEventHandler` |
| zařízení `PNP0C0C`/`PNP0C0E` | `Notify(tlačítko, 0x80)` z AML | obsluha oznámení na kořeni jmenného prostoru |
| aarch64 (HW-reduced) | Generic Event Device `ACPI0013` | přerušení z `_CRS` (SPI) -> `_EVT(číslo)` -> `Notify` |

Obsluhy běží ve vláknech (SCI, fronta oznámení, vlákno GED); hlavnímu
vláknu služby předají událost počítadlem a eventem. Služba pošle initu
`Reply::Event(PowerButton)` a init systém **čistě vypne**.

## Vypnutí a restart

**Čisté vypnutí** (init, `power::shutdown`):

1. ukončit termd a záložní konzoli (relace shellu skončí samy),
2. ukončit služby v opačném pořadí registrace (klienti před službami),
   pak `fsd`, nakonec správce zařízení (ovladače disků) a službu času
   (`acpid` běží dál - ten počítač vypne);
   na každou čeká nejvýš 1,5 s; restarty se už nedělají,
3. požádat `acpid` o `Shutdown` nebo `Reboot`.

**Vypnutí v acpid:** `AcpiEnterSleepStatePrep(5)` (metody `_PTS`, `_SST`),
pak `AcpiEnterSleepState(5)` – zápis typu spánku S5 a `SLP_EN` do PM1
(x86) nebo do registru Sleep Control (HW-reduced s registry). Platforma
HW-reduced bez registrů spánku (ARM) vypne přes jádro: PSCI `SYSTEM_OFF`.

**Restart:** registr reset z FADT (`AcpiReset`; q35: port 0xCF9 = 6), jinak
přes jádro (PSCI `SYSTEM_RESET`, na x86 0xCF9 a řadič klávesnice).

```text
init: tlacitko napajeni, vypinam system
init: vypnuti: ukoncuji sluzby
init: sluzba fsd ukoncena
init: vypnuti: sluzby ukonceny, zadam acpid
acpid: vstupuji do S5 (vypnuti)
```

### Příkazy shellu

```text
shutdown        # zeptá se "Vypnout pocitac?" a čistě vypne
shutdown -y     # bez dotazu
reboot -y       # restart
```

Relace shellu (`shsess`) má od termd kanál napájení k initu (`SessMsg::Power`;
termd ho vytvoří pro každou relaci a druhý konec pošle initu). Úlohy
na pozadí (`async`) napájení nemají. Odmítnutí dotazu = chyba `SH_REJECTED`,
nic se nevypne.

## Protokol (`libs/powerproto`)

Zpráva = kód `u32` + pole (`libs/wire`, little-endian); neplatná zpráva =
`INVALID_ARGS`.

| Požadavek | Odpověď | Poznámka |
|---|---|---|
| `Status` | `Status { features, fadt_revision, oem, thermal_zones, batteries, cpus }` | `features`: `SHUTDOWN`, `REBOOT`, `POWER_BUTTON`, `HW_REDUCED` |
| `Shutdown`, `Reboot` | žádná (počítač se vypne) / `Failed(kód)` | init je posílá až po ukončení služeb |
| `Thermal` | seznam zón: cesta, `_TMP`, `_CRT`, `_PSV` v desetinách K | 2982 = 25,0 °C |
| `Battery` | baterie `PNP0C0A`: přítomnost, stav (`_BST`), zbývá, plná a návrhová kapacita (`_BIX`/`_BIF`) | jednotky mWh/mW nebo mAh/mA |
| `CpuPerf` | procesory: počet stavů `_PSS`, nejvyšší frekvence, `_CPC` | jen zjištění, řízení není |
| – | `Event(PowerButton | SleepButton)` | kdykoli, bez požadavku |

Příklad (init):

```rust
let st = power::wait_ready(Duration::from_secs(60))?;
if st.features & powerproto::feature::POWER_BUTTON != 0 { /* ... */ }
match power::request(&Request::Thermal, Duration::from_secs(10))? {
    Reply::Thermal(zones) => for z in zones {
        println!("{}: {}", z.name, z.temp_dk.map(powerproto::celsius_text).unwrap_or_default());
    },
    _ => {}
}
```

## Bezpečnost

AML smí sahat na libovolné porty, registry a konfiguraci PCI, které si
nadeklaruje – proto má `acpid` **široký** prostředek. To je stejný model
jako v Linuxu, kde ACPICA běží přímo v jádře; tady je navíc oddělená do
procesu.

| Co | Ochrana |
|---|---|
| RAM jádra a procesů | `VMO_CREATE_PHYSICAL` povolí jen registry zařízení (mimo RAM) a paměť ACPI (`AcpiReclaimable`, `AcpiNvs`) celou uvnitř oblasti; rozsah zasahující do RAM = `ACCESS_DENIED` (test `acpi: pamet ACPI jde mapovat, RAM ne`) |
| DMA | `acpid` nemá žádnou paměť pro DMA (`dma_max = 0`) |
| vypnutí | `SYSTEM_POWER` jen s právem `POWER`: má ho kořenový prostředek, init ho předá jen acpid (omezené prostředky ho nemají nikdy) |
| shell | vypnout smí jen relace s kanálem napájení od initu (terminál); úlohy na pozadí ne |
| kód v C | zásobníkový kanárek, nulování proměnných, žádné `sprintf`/`%n` (viz [Překlad C](preklad-c.md)) |

> ⚠️ **Riziko:** proces `acpid` smí zapsat do **libovolného registru
> zařízení a portu** (i cizího ovladače) a do paměti firmwaru ACPI. Chyba
> nebo zneužití služby = ovládnutý hardware (ne RAM). Zmírnění: služba
> nečte nic ze sítě ani od uživatele kromě 6 pevných požadavků od initu.
> **Doporučené zúžení** (další krok): po načtení tabulek projít operační
> oblasti AML (`OperationRegion`) a vytvořit užší prostředek jen z nich;
> dynamicky načtené tabulky (`Load`) by pak musely žádat init.

## Omezení

- **S3/S4 (uspání)** není: před zápisem typu spánku se musí vyprázdnit
  cache CPU (`wbinvd` je privilegovaná) a uložit stav – patří do jádra
  (syscall). `ACPI_FLUSH_CPU_CACHE` je proto prázdné; pro S5 a restart to
  nevadí.
- **SCI nad linkou 15** (některé desky mají SCI na GSI 20+): jádro zatím
  směruje jen ISA linky 0-15, služba pak hlásí, že SCI nelze získat.
- **Embedded Controller** (`PNP0C09`, notebooky: baterie, teploty, víko)
  nemá ovladač: AML, které čte EC, selže a baterie se neukáže.
- **Řízení frekvence CPU**: jen zjištění `_PSS`/`_CPC`. Pro řízení je
  potřeba zápis do MSR (x86) nebo registrů CPPC – syscall jádra a politika.
- **Čisté ukončení služeb:** služby se ukončují (`PROCESS_KILL`); `fsd`
  zapisuje každou změnu commitem (CoW), takže disk je konzistentní, ale
  protokol "dopiš a skonči" zatím nemá.
- Odinstalování obsluhy SCI (`AcpiOsRemoveInterruptHandler`) nic nedělá –
  služba ACPICA nikdy neukončuje.

## Testy

- **Na hostiteli:** `cargo test -p powerproto` (kódování zpráv),
  `-p cstd` (libc), `-p cbuild` (příznaky), `-p abi` (`acpi_info_roundtrip`),
  `-p shlang` (příkazy `shutdown`/`reboot` s vlastním hostitelem).
- **V QEMU, sada `acpi`:** acpid odpovídá a hlásí vypnutí, restart
  a tlačítko (x86 bez HW-reduced, ARM s ním), čidla a CPU, mapování paměti
  ACPI ano a RAM ne, `SYSTEM_POWER` jen s právem `POWER`, linky SPI
  v prostředku.
- **Zvláštní běhy po hlavním testu** (`xtask/src/powertest.rs`; vypnout
  `--no-power`, s `--only` se vynechají):
  - `button`: init vypíše `init: test napajeni: cekam na tlacitko
    napajeni`, xtask pošle QMP `system_powerdown`, init čistě vypne a QEMU
    musí skončit **samo s kódem 0** (ne přes `isa-debug-exit`/semihosting)
    a v logu musí být celý postup (`acpid: udalost PowerButton`,
    `init: vypnuti: sluzby ukonceny`, `acpid: vstupuji do S5`),
  - `reboot`: restart (QEMU s `-no-reboot` skončí), log
    `acpid: restartuji`.

  Na aarch64 jde stisk přes Generic Event Device a vypnutí přes PSCI.

## Aktualizace ACPICA

1. stáhnout nový tag z `github.com/acpica/acpica`,
2. nahradit `third_party/acpica/source/{include,components}` (bez
   `debugger` a `disassembler`) a `LICENSE`,
3. znovu přidat dva řádky do `include/platform/acenv.h` (viz
   `third_party/acpica/README.md`),
4. upravit verzi v README a v tomto dokumentu; `_Static_assert` v
   `glue.c` ohlídá změnu rozložení struktur.
