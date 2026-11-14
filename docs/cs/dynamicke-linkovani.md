# Dynamické linkování

Sdílené knihovny ELF (`ET_DYN`) a programy PIE: zavaděč `ld.so`,
relokace, TLS sdílených knihoven, `dlopen`, sdílení kódu knihoven mezi
procesy a pečeť RELRO. Je to krok 4 v pořadí převzetí cizího softwaru
([architektura.md](architektura.md#bezpečnost)): potřebuje ho unixová část
Wine a FFmpeg (LGPL, dynamicky). Programy v Rustu zůstávají statické
(`ET_EXEC`, [rust-std.md](rust-std.md)).

## Stav

| Část | Co | Stav |
|---|---|---|
| DL-1 | parser `ET_DYN` v `libs/elf`: programové hlavičky (`PT_INTERP`, `PT_TLS`, `PT_GNU_RELRO`, `PT_DYNAMIC`), dynamická sekce, symboly s GNU a SysV hashem, relokace RELA a RELR obou architektur, odmítnutí TEXTREL | **[implementováno]** (host testy `cargo test -p elf` proti souborům z clang a ld.lld) |
| DL-2 | zavaděč `user/ld` + `libs/dynload`: `PT_INTERP` ve `spawn`, ASLR, `DT_NEEDED`, `BIND_NOW`, relokace, statické TLS hlavního vlákna, RELRO jen pro čtení; PIC a link přes `ld.lld` v `cbuild`, knihovny v initfs (`lib/`) | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `procesy`, testy `dynamicke linkovani: ...`) |
| DL-3 | tabulka zavaděče (`abi::loader`, `dl*` v ABI už teď), TLS nových vláken (libos, osrt), DTV a generace, statická rezerva TLS, `fini`, `dl_iterate_phdr`, `dladdr`, `r_debug` | **[ověřeno v QEMU]** x86_64 i aarch64 (test `dynamicke linkovani: vlakna libos ...`) |
| DL-4a | služba knihoven `libd` (zapečetěné šablony, sdílení kódu knihoven mezi procesy), spojení s libd pro každý proces (program se zavaděčem spustí kdokoli), pečeť oblastí (`PROT_SEAL`, jako `mseal`) pro kód a RELRO | **[ověřeno v QEMU]** x86_64 i aarch64 (sady `pamet`, `procesy`) |
| DL-4b | `dlopen`/`dlsym`/`dlclose` přes libd (dynamické TLS: líná alokace přes DTV, dynamický TLSDESC), rezerva statického TLS z manifestu (`[app] static_tls_reserve`) a měření | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `procesy`) |
| DL-4c | knihovny z disku a `$ORIGIN` (až budou aplikace z disku `/apps/<id>/`) | **[navrženo]** |

## Návrh (schválené volby)

<figure>
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Spuštění programu PIE">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="dl1" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="5" y="30" width="130" height="60" rx="6"/><text class="t b" x="70" y="55" text-anchor="middle">spawn (rodič)</text><text class="m" x="70" y="73" text-anchor="middle">program + ld.so</text>
<rect class="acc" x="165" y="30" width="130" height="60" rx="6"/><text class="t b" x="230" y="55" text-anchor="middle">ld.so (dítě)</text><text class="m" x="230" y="73" text-anchor="middle">PT_INTERP, ASLR</text>
<rect class="acc" x="325" y="30" width="130" height="60" rx="6"/><text class="t b" x="390" y="55" text-anchor="middle">DT_NEEDED</text><text class="m" x="390" y="73" text-anchor="middle">/system/lib, $ORIGIN</text>
<rect class="acc" x="485" y="30" width="130" height="60" rx="6"/><text class="t b" x="550" y="55" text-anchor="middle">relokace</text><text class="m" x="550" y="73" text-anchor="middle">BIND_NOW, TLS</text>
<rect class="box" x="645" y="30" width="110" height="60" rx="6"/><text class="t b" x="700" y="55" text-anchor="middle">RELRO R--</text><text class="m" x="700" y="73" text-anchor="middle">pak vstup programu</text>
<line class="ln" x1="135" y1="60" x2="161" y2="60" marker-end="url(#dl1)"/>
<line class="ln" x1="295" y1="60" x2="321" y2="60" marker-end="url(#dl1)"/>
<line class="ln" x1="455" y1="60" x2="481" y2="60" marker-end="url(#dl1)"/>
<line class="ln" x1="615" y1="60" x2="641" y2="60" marker-end="url(#dl1)"/>
<text class="m" x="5" y="130">Kód knihoven ze zapečetěných šablon (libd, sdílené stránky), data a GOT jako kopie při zápisu.</text>
<text class="m" x="5" y="150">DL-1 (tato část): parser libs/elf, ze kterého ld.so čte hlavičky, symboly a relokace.</text>
</svg>
<figcaption>Spuštění programu PIE se sdílenými knihovnami (DL-2 až DL-4b; knihovny z disku a <code>$ORIGIN</code> v DL-4c).</figcaption>
</figure>

- **Zavaděč:** samostatný `ld.so` (zároveň `libc.so` ve stylu musl),
  program má `PT_INTERP`; zavaděč psaný v Rustu (`libs/dynload`,
  `user/ld`), sám staticky relokovaný.
- **Formát:** ELF `ET_DYN` podle SysV gABI: `DT_GNU_HASH` i `DT_HASH`,
  RELA a RELR, `DT_RUNPATH`, `SONAME`. Jen `BIND_NOW` a plné RELRO (žádné
  líné PLT). **`DT_TEXTREL` se odmítá** (zápis do spustitelné paměti
  odporuje W^X). TLS: local-exec, initial-exec, general/local-dynamic přes
  `__tls_get_addr`, TLSDESC (na aarch64 výchozí). Verze symbolů
  (`DT_VERSYM`) se čtou, ale nevyhodnocují; skrytá verze jen vyřadí symbol
  z vyhledávání.
- **Kde leží knihovny:** `/system/lib` (neměnná součást obrazu) a soukromé
  knihovny aplikace přes `$ORIGIN`; `LD_LIBRARY_PATH` jen pro vývoj.
- **Verze:** soubor se jmenuje přesně jako `SONAME` s hlavní verzí
  (`libavcodec.so.61`), bez řetězů symlinků.
- **ASLR:** náhodná báze každého `ET_DYN` v každém procesu, zarovnání
  64 KiB.
- **Rust zůstává statický**; dynamické jsou C/C++ (`cstd` jako `libc.so`)
  a pluginy `cdylib`.
- **RELRO:** po relokaci `VM_PROTECT` na R-- a snížení `max_flags`
  (jako `mseal`).

## Zavaděč `ld.so` (DL-2)

**[ověřeno v QEMU]** x86_64 i aarch64.

**Start programu PIE.** `spawn` (libos) pozná `ET_DYN` s `PT_INTERP` =
`/system/lib/ld.so` (`abi::auxv::INTERP`; jiný zavaděč = `NOT_SUPPORTED`,
knihovna bez zavaděče se nespustí):

1. Program i zavaděč (`lib/ld.so` z initfs volajícího) namapuje na
   **náhodné báze** (`spawn::random_base`: rezervace celého rozsahu modulu
   v `0x2000_0000_0000..0x5000_0000_0000`, zarovnání 64 KiB, ~29 bitů);
   kód programu z initfs se dál sdílí přes šablony.
2. Na vrchol zásobníku zapíše blok SysV (`abi::auxv`): `argc = 0`, prázdné
   `argv` a `envp`, `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`, `AT_PAGESZ`,
   `AT_BASE`, `AT_ENTRY`, `AT_RANDOM` (16 B náhody). Čísla jako SysV, takže
   převzatý `getauxval` funguje.
3. Do bootstrap kanálu **před** startovní zprávu vloží **zprávu
   zavaděče** (`abi::bootstrap::LOADER_MAGIC`, "OSLD"): handle procesu
   a initfs jen pro čtení (`READ | MAP`). Startovní zpráva i kanál zůstanou
   programu (dál přes něj mluví s rodičem).
4. Vlákno začne na vstupu zavaděče (`rdi`/`x0` = bootstrap, SP = blok).

**Zavaděč** (`user/ld`, Rust, static-PIE: `relocation-model=pie`,
`-pie --no-dynamic-linker`, jen relativní relokace):

1. Vstup v asembleru spočítá bázi (`__ehdr_start`) a `_DYNAMIC` relativně
   k PC a **sám se zrelokuje** (RELA `R_*_RELATIVE` a RELR) dřív, než sáhne
   na data s ukazateli.
2. Přečte zprávu zavaděče a auxv; program vidí jako načtený modul
   (`ElfFile::parse_loaded`).
3. Knihovny z `DT_NEEDED` **do šířky**, každou jednou (jméno nebo
   `SONAME`), z initfs `lib/<jméno>` (`dynload::initfs_path`;
   `/system/lib/x` = `lib/x`, jiné cesty se odmítnou). Každá dostane vlastní
   náhodnou bázi; segmenty bez zápisu se **zapečetí** (W^X: spustit jde
   jen zapečetěná paměť).
4. **Statické TLS** všech modulů (`dynload::tls_layout`: x86_64 varianta II
   s TCB za TP, aarch64 varianta I se 16 B TCB) - program je první modul,
   takže jeho local-exec sedí s linkerem. Nastaví ukazatel vlákna.
5. **Relokace**, závislosti napřed, program poslední: `RELATIVE`, `ABS64`,
   `GLOB_DAT`, `JUMP_SLOT` (vše hned), `IRELATIVE`, TLS
   (`DTPMOD`/`DTPOFF`/`TPOFF`, TLSDESC se statickým resolverem, vestavěný
   `__tls_get_addr`), RELR. Symboly v pořadí program, knihovny (lokální
   a skryté symboly patří modulu samotnému); nedefinovaný slabý = 0,
   nedefinovaný silný = chyba.
6. RELRO přepne na jen pro čtení (`VM_PROTECT`), zavolá `DT_INIT`
   a `DT_INIT_ARRAY` (závislosti napřed, program poslední).
7. Odmapuje initfs, zavře své handly a skočí na vstup programu se stejným
   bootstrapem a SP (program může číst auxv).

Chyba (chybí knihovna, symbol, vadný soubor) = `ld.so: ...` v logu a konec
procesu s kódem 127 (`abi::auxv::EXIT_LOAD_FAILED`).

**Programy a knihovny v C** se překládají přes `cbuild::Build::link`
([preklad-c.md](preklad-c.md)): programy `-fPIE` (TLS programu
local-exec, cizí initial-exec), knihovny `-fPIC` (TLS general-dynamic,
na aarch64 TLSDESC). Program PIE linkuje s `--allow-shlib-undefined`
(funkce zavaděče jako `__tls_get_addr` při linku nikde nejsou). Testovací
moduly jsou v `user/dltest` (minimální `crt.c`: `_start` -> `program_main`
-> konec syscallem; `cstd` jako `libc.so` zatím není).

⚠️ **Omezení DL-2 (vědomá, řeší DL-3 a DL-4):**

- Sdílení kódu knihoven a spouštění kýmkoli řeší DL-4a (`libd`, výše); knihovny
  z disku a `$ORIGIN` DL-4c.
- TLS nových vláken a fini řeší DL-3a (výše).
- RELRO je od DL-4a zapečetěné (`PROT_SEAL`).
- Initfs je pro zavaděč dostupný celý (jen pro čtení) po dobu zavádění;
  program ho nedostane (zavaděč ho před skokem odmapuje a handly zavře).

## Služba knihoven a pečeť (DL-4a)

**[ověřeno v QEMU]** x86_64 i aarch64.

- **`libd`** (`user/libd`, protokol `libs/libdproto`) drží šablony
  sdílených knihoven z `/system/lib` (initfs `lib/<jméno>`): segmenty bez
  zápisu jako **zapečetěné VMO** (všechny procesy mapují tytéž stránky
  kódu), datové jako šablonu pro klon s kopií při zápisu (`VMO_CLONE`,
  i nuly do `mem_size`). Šablona se hledá podle jména a obsahu (BLAKE3).
  Klient dostane handly kódu `READ | MAP | EXECUTE` a dat `READ | MAP`,
  nikdy zápis.
- **Spojení:** init spustí libd hned po službě času (jen initfs) a jeho
  bootstrap kanál je první spojení. `spawn` (libos) vytvoří každému
  potomkovi vlastní spojení (`Connect`, `bootstrap::kind::LIBD`), takže
  program se zavaděčem může spustit **kdokoli** se spojením, ne jen init.
  Zavaděč dostane ve zprávě zavaděče další spojení (místo initfs) a nechá
  si ho (`dlopen` v DL-4b). Sestavení libos pro Rust std spojení zatím
  nemá (zavře ho): aplikace se std programy se zavaděčem nespouštějí.
- **`ld.so`** samotný dává `spawn` také od libd (sdílený kód zavaděče).
- **Pečeť** (`PROT_SEAL` ve `VM_PROTECT`, jako Linux `mseal`,
  [syscally.md](syscally.md#vm_protect)): zapečetěnou oblast nejde
  odmapovat, uvolnit, změnit její práva ani přes ni mapovat, do konce
  procesu. `spawn` zapečetí kód programu a zavaděče, zavaděč kód knihoven,
  RELRO programu, knihoven i své vlastní po relokaci (GOT tak nejde vrátit
  na RW ani nahradit novým anonymním mapováním).
- **Nedůvěryhodný vstup:** jména jen `A-Z a-z 0-9 . _ + -` bez tečky na
  začátku, nejvýš 255 B (`libdproto::valid_name`, fuzz test dekodéru);
  soubor nejvýš 256 MiB, nejvýš 16 segmentů, ELF přes `libs/elf` (TEXTREL,
  REL odmítnuty, hlavičky v prvním segmentu). Spojení: nejvýš 256 živých
  vytvořených z jednoho spojení (na proces) a 4096 celkem.
- ⚠️ **Restart libd:** procesy spuštěné dřív mají spojení se starou
  instancí (zavřené) a program se zavaděčem už nespustí; init dostane nové
  spojení. Obnova spojení lístkem (`svcclient`) zatím není.

## dlopen a dynamické TLS (DL-4b)

**[ověřeno v QEMU]** x86_64 i aarch64.

- **`dlopen(cesta, příznaky)`**: cesta jen `/system/lib/<jméno>` nebo
  `<jméno>` (knihovny od `libd`; disk a `$ORIGIN` až DL-4c), null = program
  (globální rozsah). Už nahraný modul (jméno nebo `SONAME`) vrátí tentýž
  handle a zvýší počítadlo. Nový modul a jeho chybějící závislosti se
  nahrají od libd, **relokují hned** (`LAZY` = `NOW`) v rozsahu globální
  rozsah (program, knihovny ze startu, moduly s `RTLD_GLOBAL`) + lokální
  skupina knihovny (ona a závislosti do šířky), kód a RELRO se zapečetí
  a zavolají se inicializátory (závislosti napřed). `RTLD_NOLOAD` jen
  vrátí handle nahraného modulu. **Chyba kdekoli** (chybějící knihovna,
  nedefinovaný symbol, málo rezervy TLS) = nic se nezmění: nové moduly se
  zahodí (ještě nejsou zapečetěné), rezerva TLS se vrátí, důvod jde do
  `dlerror`.
- **`dlsym(handle, jméno)`**: hledá v lokální skupině modulu,
  `RTLD_DEFAULT` (0) a handle programu v globálním rozsahu; proměnná TLS
  = adresa ve volajícím vlákně (líný blok se případně založí), IFUNC =
  výsledek resolveru. `RTLD_NEXT` zatím není (chyba).
- **`dlclose`** sníží počítadlo; modul zůstává nahraný (kód je zapečetěný,
  čísla modulů TLS se neopakují) - jako `RTLD_NODELETE` u všech.
- **`dlerror`** je pro každé vlákno zvlášť (podle TP); řetězec platí do
  dalšího `dl*` ve vlákně, čtení chybu vymaže. Volání `dl*` jsou
  serializovaná **rekurzivním** zámkem: inicializátor smí volat `dlopen`.
- **TLS modulu z `dlopen`:**
  - **initial-exec** (`DF_STATIC_TLS` nebo relokace posunu od TP): kus
    **rezervy statického TLS** (`dynload::Surplus`) - stejný posun od TP
    ve všech vláknech. Zavaděč šablonu zkopíruje do bloků všech žijících
    vláken (seznam bloků) a doplní jejich DTV; nová vlákna ji dostanou
    v `thread_tls_new`. Zarovnání víc než zarovnání TP nejde.
  - **ostatní** (general-dynamic, TLSDESC): **líný blok** přes DTV. Položka
    je prázdná, dokud vlákno k TLS modulu nesáhne: pomalá cesta
    `__tls_get_addr` (x86_64 GD) nebo **dynamického resolveru TLSDESC**
    (aarch64 vždy, x86_64 `-mtls-dialect=gnu2`) alokuje blok (šablona,
    nuly do konce), zapíše ho do DTV a při konci vlákna ho uvolní.
    Resolver na rychlé cestě jen čte DTV; na pomalé uloží všechny registry,
    které smí měnit volaná funkce v Rustu - x86_64 `rcx rdx rsi rdi r8-r11`
    a `xmm0-15` (zavaděč je přeložený bez AVX, horní poloviny `ymm`/`zmm`
    nemění), aarch64 `x1-x18` a `q0-q31`. ABI deskriptoru mění jen `rax`/`x0`
    (a příznaky).
  - DTV má pevnou velikost (256 modulů s TLS) a čísla se neopakují, takže
    prázdná položka vždy znamená „blok ještě není“ (glibc navíc porovnává
    generaci, protože čísla po `dlclose` recykluje).
  - Přístup k TLS bez možnosti vrátit chybu (málo paměti pro líný blok,
    neznámý modul) ukončí proces, jako glibc.
- **Rezerva statického TLS** pro proces: manifest `[app] static_tls_reserve`
  (např. `"8K"`, nejvýš 64 KiB, viz [manifest](firewall.md#manifest-aplikace))
  -> init `spawn::Options::static_tls_reserve` -> auxv
  `abi::loader::AT_MO_TLS_RESERVE` -> zavaděč (`dynload::tls_reserve`; bez
  hodnoty výchozí 2 KiB). Rezerva je v bloku **každého** vlákna, proto
  horní mez.
- **Měření rezervy:** zavaděč zapíše do logu spotřebu při každém `dlopen`
  modulu, který rezervu použil, a při konci procesu (jen když se použila):

  ```text
  ld.so: dlopen libdlstatic.so.1 (statické TLS 4 B): rezerva statickeho TLS pouzito 4 z 2048 B
  ld.so: konec procesu: rezerva statickeho TLS pouzito 4 z 2048 B
  ```

- **Vyčerpání rezervy** = `dlopen` selže a `dlerror` řekne, co dělat:

  ```text
  dlopen: libdlbig.so.1: staticke TLS (initial-exec) potrebuje 4096 B (zarovnani 1), v rezerve zbyva 2048 z 2048 B; zvyste [app] static_tls_reserve v manifestu aplikace (nejvys 64 KiB)
  ```

- **libos:** `libos::loader::{dlopen, dlsym, dlclose}` (chyba jako text
  z `dlerror`), `Library` = handle.
- **Alokátor zavaděče** (`user/ld/src/heap.rs`) uvolňuje
  (`linked_list_allocator` nad rezervací adres, kterou vybere jádro).
- `r_debug` se při `dlopen` doplní (`RT_ADD`, `_dl_debug_state`,
  `RT_CONSISTENT`, `_dl_debug_state`); `dl_iterate_phdr` vidí nové moduly
  (`adds`).
- ⚠️ **Omezení:** `RTLD_NEXT` není; moduly se neodstraňují (paměť
  knihovny zůstane do konce procesu); finalizátory modulů z `dlopen`
  běží až na konci procesu a v pořadí nahrání (ne podle závislostí na
  starších modulech); kód v C, který volá `__tls_get_addr` se zásobníkem
  nezarovnaným na 16 B (starý GCC), by na pomalé cestě mohl spadnout
  (clang je v pořádku).

## Vlákna, fini a debugger (DL-3)

**[ověřeno v QEMU]** x86_64 i aarch64.

- **Tabulka zavaděče** `abi::loader::Interface` (verze, velikost;
  `thread_tls_new`, `thread_tls_free`, `run_fini`, `dl_iterate_phdr`).
  Adresu zapíše zavaděč do auxv (`AT_MO_LOADER`, místo připraví `spawn`)
  a při skoku do programu předá navíc SP (auxv) a `ENTRY_MAGIC`
  (x86_64 `rsi`/`rdx`, aarch64 `x1`/`x2`). libos (`rt_own` -> `loader::init`)
  a crt ji tak najdou; program bez zavaděče má `loader::interface() = None`
  a chová se jako dřív.
- **TLS nových vláken:** `libos::thread::spawn` pod zavaděčem vezme blok
  (`thread_tls_new`: kopie šablon všech modulů, rezerva, na x86_64 TCB)
  a ukazatel vlákna nastaví v trampolíně dřív než kód vlákna; `join` blok
  vrátí. `osrt` (Rust std) pod zavaděčem vlastní blok nevytváří.
- **Statická rezerva TLS** (výchozí `dynload::STATIC_TLS_SURPLUS` = 2 KiB,
  od DL-4b nastavitelná, viz níže) za moduly ze startu: pro pozdější
  `dlopen` modulů s initial-exec TLS (ovladače Vulkan/GL z Mesy). glibc má
  pevnou rezervu 1664 B, dnes 512 B + 144 B na jmenný prostor (1520 B);
  2 KiB pokryje obojí.
- **fini:** normální konec programu (`rt_own` po `main`, crt po
  `program_main`) zavolá `run_fini`: program první, závislosti poslední,
  `DT_FINI_ARRAY` pozpátku, pak `DT_FINI`; podruhé nic. Kill a pád fini
  nevolají; přímý `PROCESS_EXIT` také ne.
- **`dl_iterate_phdr`** (rozložení `struct dl_phdr_info` jako glibc, pro
  unwinder a převzatý kód): v tabulce a jako vestavěný symbol pro C;
  bez alokace, callback běží bez zámku (smí volat `dlopen`). Moduly:
  program, knihovny ze startu a z `dlopen` (zavaděč sám ne).
- **`r_debug`** (SVR4, `link_map` program a knihovny, `r_brk` =
  `_dl_debug_state`): adresa v `DT_DEBUG` programu (zapsaná před RELRO).
- **DTV a generace** (model glibc, příprava na `dlopen`): každé vlákno má
  v bloku TLS vektor `[generace, blok modulu 1, 2, ...]`, ukazatel na něj
  v TCB (x86_64 `TP + 8`, aarch64 `TP + 0`). `__tls_get_addr` čte blok
  z DTV; moduly ze startu ho mají ve statickém TLS. Líná alokace bloku
  modulu z `dlopen` viz DL-4b.
- **`dlopen`, `dlsym`, `dlclose`, `dlerror`, `dladdr`** jsou v tabulce
  i jako vestavěné symboly pro C (ABI se v DL-4b nezměnilo). `dladdr`
  najde modul podle adresy (symbol zatím ne).
- Zavaděč si **ponechá handle vlastního procesu** (bloky TLS vláken,
  `dlopen`); program má handle na tentýž proces stejně, práva navíc
  nedostane.

## Parser `libs/elf` (DL-1)

**[implementováno]** Soubor je nedůvěryhodný: každé čtení má kontrolu mezí,
vadný vstup vrátí `ElfError`, nikdy nepanikaří (test
`damaged_input_does_not_panic`: zkrácení a tisíce náhodně změněných bajtů).

| API | Co |
|---|---|
| `ElfFile::parse(data)` | statický program (`ET_EXEC`) pro tuto architekturu - loader, jádro, `spawn` (beze změny) |
| `ElfFile::parse_object(data, Machine)` | `ET_EXEC` i `ET_DYN` pro zadanou architekturu (host testy umí obě) |
| `program_headers`, `load_segments` (s `align`), `interp`, `tls` + `tls_data`, `relro` | programové hlavičky |
| `dynamic()` -> `Dynamic` | `DT_NEEDED` (nejvýš 32), `SONAME`, `RUNPATH` (jinak `RPATH`), tabulky, `init`/`fini` a pole, příznaky (`bind_now`, `pie`, `static_tls`), verze; **odmítne `DT_TEXTREL`/`DF_TEXTREL` (`ElfError::TextRelocations`) a `DT_REL`** |
| `symbols(&Dynamic)` -> `SymbolTable` | `get(i)`, `lookup(jméno)` přes GNU hash (Bloomův filtr), jinak SysV; vrací jen exportované symboly (globální/slabé, viditelnost default/protected, verze ne skrytá) |
| `relas(&Dynamic)` | RELA a PLT jako `Rela { offset, kind, sym, addend }`; `Kind` společný pro obě architektury (`Relative`, `Absolute`, `GlobDat`, `JumpSlot`, `TlsModule`, `TlsOffset`, `TlsTpOffset`, `TlsDesc`, `IRelative`, `Copy`, `Unknown`) |
| `relrs(&Dynamic)` | adresy relativních relokací z RELR (dekodér `Relr`) |
| `at_vaddr(addr, len)` | data souboru na virtuální adrese (musí ležet v datech jednoho `PT_LOAD`) |

Testovací soubory (`libs/elf/tests/fixtures/`, zdroj v C a `gen.sh`):
knihovna s RELA a oběma hashi, táž s RELR a jen GNU hashem, program PIE
s `PT_INTERP`, knihovna s TEXTREL (jen x86_64 - na aarch64 ji lld
nevytvoří). Překládají se clangem a ld.lld s `max-page-size=65536`
a commitují se, testy clang nepotřebují.

## Testy

| Test (`cargo test -p elf`) | Ověřuje |
|---|---|
| `shared_library_headers` | `PT_TLS` a jeho data, RELRO, `DT_NEEDED`, `SONAME`, `RUNPATH $ORIGIN`, `BIND_NOW`, zarovnání 64 KiB (x86_64 i aarch64) |
| `symbol_lookup_gnu_and_sysv` | vyhledání přes GNU i jen SysV hash, druh a hodnota symbolu, skrytý a nedefinovaný symbol se nenajde |
| `relocations_x86_64`, `relocations_aarch64` | druhy relokací a jejich symboly (TLS GD s `__tls_get_addr`, TLSDESC na aarch64), `Rela::value` |
| `relr_matches_relative_relocations`, `relr_decoder` | RELR ze souboru a syntetické bitmapy (i prázdná bitmapa posune o 63 slov) |
| `pie_program` | `PT_INTERP`, `DF_1_PIE`, `ElfFile::parse` PIE odmítne |
| `rejects_textrel_and_wrong_machine` | TEXTREL, jiná architektura, krátký soubor |
| `hash_functions_match_reference` | GNU a SysV hash proti známým hodnotám |
| `damaged_input_does_not_panic` | zkrácení a náhodné změny bez paniky |
| `loaded_module_view` | modul načtený v paměti (`parse_loaded`) dává stejné údaje jako soubor |
| `cargo test -p dynload` | rozložení TLS proti posunům linkeru (obě varianty), hodnoty všech druhů relokací, cesty knihoven |
| `cargo test -p cbuild` (`link_skutecne_moduly`) | link knihovny a PIE přes `ld.lld` na obou architekturách |
| QEMU `procesy`: `dynamicke linkovani: PIE s knihovnami` | `dltest`: volání přes dvě knihovny, ukazatel na funkci (ABS64), inicializátor po relokaci závislostí, TLS knihoven (GD/TLSDESC), TLS knihovny z programu (initial-exec), TLS programu (local-exec) |
| QEMU `procesy`: `ASLR - dva behy, ruzne baze` | dva běhy `dltest` mají různou bázi programu |
| QEMU `procesy`: `chybejici knihovna -> kod 127` | `dltest-missing`: `ld.so: knihovna ... nenalezena`, kód 127 |
| QEMU `procesy`: `vlakna libos s TLS knihoven, dl_iterate_phdr, r_debug, fini` | `dltest-rs` (Rust + libos jako PIE, jen test): 4 vlákna mají vlastní čerstvou kopii TLS knihovny (přes DTV), hlavní vlákno beze změny; tabulka zavaděče; `dl_iterate_phdr` (3 moduly, jména, TLS); `r_debug` přes `DT_DEBUG` (verze 1, 3 uzly); finalizátor knihovny; `dladdr` funkce knihovny; `dl*`: vadné jméno a cesta mimo `/system/lib` = chyba, nahraná knihovna = handle a `dlsym`, `RTLD_NOLOAD`; `dlopen` knihoven s TLS (TLSDESC i `__tls_get_addr`, zarovnaný blok, initial-exec v rezervě, inicializátor, `dlsym` proměnné TLS) v hlavním vlákně, ve vlákně založeném před `dlopen` i po něm; 4 KiB initial-exec = chyba s radou `static_tls_reserve` |
| `cargo test -p dynload` (`tls_surplus_after_modules`) | rezerva TLS za moduly nemění jejich posuny |
| QEMU `procesy`: `rezerva statickeho TLS z voleb spusteni (manifest)` | `dltest-dl`: `dlopen` knihovny se 4 KiB initial-exec s výchozí rezervou selže s radou v `dlerror`, s `static_tls_reserve` 8 KiB uspěje a TLS sedí |
| `cargo test -p dynload` (`surplus_*`, `tls_reserve_bounds`) | místo v rezervě pod moduly (x86_64) a nad nimi (aarch64), zarovnání, vyčerpání; mez rezervy |
| `cargo test -p manifest` (`static_tls_reserve`) | `[app] static_tls_reserve`: velikost, nejvýš 64K |
| `cargo test -p libdproto` | jména knihoven, zprávy a meze; fuzz dekodéru (20 000 poškozených zpráv bez paniky) |
| QEMU `pamet`: `pecet: ...` | zapečetěné mapování a rezervace: `VM_PROTECT`, `VM_UNMAP`, decommit, mapování přes ni a obsazení rezervace selžou, sousední oblasti fungují |
| QEMU `procesy`: `libd sdili sablony knihoven mezi procesy` | druhý běh `dltest` nevytvoří novou šablonu, jen zásahy mezipaměti libd |
| QEMU `procesy`: `vlakna libos ...` (bit 256) | kód a RELRO knihovny jsou zapečetěné (`ATTR_SEALED`, změna práv i odmapování selže) |
