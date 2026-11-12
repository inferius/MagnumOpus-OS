# Rust std pro aplikace

Aplikace pro MagnumOpus půjdou psát v běžném Rustu se `std` (`fn main()`,
`std::fs`, `std::thread`, `std::net`, crates z crates.io). Systémové
služby a ovladače zůstávají `no_std` nad [libos](libos.md). Tahle stránka
popisuje přístup, sestavení, mapování částí `std` na libos a služby, co
k tomu přidalo jádro a dopady na budoucí plány.

> **Stav:** návrh schválen 2026-11-12 (všechna doporučení, viz
> [Schválená rozhodnutí](#schválená-rozhodnutí)). Fáze 1 hotová:
> `hello-std` s argumenty, vlákny, TLS a soubory **[ověřeno v QEMU]** na
> x86_64 i aarch64 (sada `std`). Síť, procesy a stdin jsou další krok.

## Cíl a rozsah

- Cíle `x86_64-unknown-magnumopus` a `aarch64-unknown-magnumopus`
  (`target_os = "magnumopus"`) **[ověřeno v QEMU]**.
- `std` nad libos: alokace, vlákna, synchronizace, TLS, čas, soubory přes
  `fsd`, argumenty, proměnné prostředí, stdout/stderr **[ověřeno v QEMU]**;
  síť přes `netd`, procesy a stdin **[navrženo]** (stav po částech
  v [mapování](#mapování-std-na-libos-a-služby)).
- `cargo xtask build` sestaví std (sysroot) i aplikace a dá je do initfs
  **[ověřeno v QEMU]**.
- Mimo rozsah: dynamické linkování, `fork`, signály POSIX, port rustc
  (jen dopady, viz [Dopady na budoucnost](#dopady-na-budoucnost)).

## Přístup

### Varianty

| | A: vlastní PAL ve forku std (vzor Motor OS, Hermit) | B: rodina `unix` nad naší libc (vzor Redox) | C: bez forku, vlastní fasáda `no_std` |
|---|---|---|---|
| Jak | malé lepidlo v `library/std/src/sys/**` (větve `cfg_select!` pro `target_os = "magnumopus"`), logika v našem crate `osrt` | `target_family = "unix"`, std používá svůj unixový PAL přes crate `libc`, POSIX dodá `cstd` | crate `mostd`, který reexportuje `core`/`alloc` a přidá vlastní `fs`, `thread`... |
| Crates z crates.io | fungují, které jen používají `std`; kód s `cfg(unix)` potřebuje větev pro náš OS | fungují i ty s `cfg(unix)` a `libc` | nefungují (chtějí skutečné `std`) |
| Práce před prvním `hello` | malá: ~30 souborů lepidla + `osrt` | velká: std unix PAL potřebuje stovky funkcí POSIX (pthread, mmap, stat, poll, signály) a podporu v crate `libc` | nejmenší |
| Sémantika | přímo naše objekty (kanály, handly, relace FS) | POSIX emulace (čísla fd, `errno`, cesty, práva), nesedí na capability model | naše |
| Údržba při novém nightly | rebase lepidla (rozsah viz níže) | rebase menší, ale navíc fork crate `libc` a velká libc | žádná |
| Výkon | přímá volání libos | každé volání přes vrstvu libc | přímý |

**Schváleno: A.** Je to cesta, kterou upstream sám používá (Motor OS
a Hermit jsou v `std` přesně takhle: v `std` jen přepínače a tenké moduly,
celá logika v externím crate `moto-rt` / `hermit-abi`). B dává lepší
kompatibilitu s crates, ale nejdřív by se musela napsat skoro celá POSIX
libc a sémantika POSIX (čísla fd, `fork`, signály) jde proti návrhu
systému. C nesplní cíl (gix, prohlížeč a další crates chtějí skutečné
`std`). Kompatibilitu s `cfg(unix)` crates řešíme po jednom
(viz [gix](#gix-pro-git-vrstvu-fs)).

Do budoucna (až bude projekt veřejný a stabilní) dává smysl cíl dostat do
upstream jako tier 3, stejně jako Motor OS: odpadne údržba forku
a crates (libc, getrandom, jobserver...) můžou přidat podporu samy.

### Vrstvy

<figure class="diagram">
<svg viewBox="0 0 780 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Vrstvy std pro MagnumOpus">
<defs><marker id="rs1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="10" width="370" height="50" rx="6"/><text class="t b" x="195" y="32" text-anchor="middle">aplikace se std</text><text class="m" x="195" y="50" text-anchor="middle">hello-std, gix, prohlížeč, přehrávač</text>
<rect class="box" x="400" y="10" width="370" height="50" rx="6"/><text class="t b" x="585" y="32" text-anchor="middle">služby a ovladače no_std</text><text class="m" x="585" y="50" text-anchor="middle">fsd, netd, compd, ovladače (beze změny)</text>
<path class="ln" d="M195 60 L195 88" marker-end="url(#rs1)"/>
<rect class="acc" x="10" y="90" width="370" height="60" rx="6"/><text class="t b" x="195" y="113" text-anchor="middle">library/std (fork = lepidlo)</text><text class="m" x="195" y="131" text-anchor="middle">sys/*/magnumopus.rs: cfg_select! větve, tenké obaly</text>
<path class="ln" d="M195 150 L195 178" marker-end="url(#rs1)"/>
<rect class="acc" x="10" y="180" width="370" height="60" rx="6"/><text class="t b" x="195" y="203" text-anchor="middle">libs/osrt (rustc-dep-of-std)</text><text class="m" x="195" y="221" text-anchor="middle">start, heap, vlákna, futex, TLS, fs, síť, stdio</text>
<path class="ln" d="M195 240 L195 268" marker-end="url(#rs1)"/>
<path class="ln" d="M585 60 L585 268" marker-end="url(#rs1)"/>
<rect class="box" x="10" y="270" width="760" height="50" rx="6"/><text class="t b" x="390" y="292" text-anchor="middle">libos (feature rt vypnutá pro std) + fsproto, netproto, svcclient, abi</text><text class="m" x="390" y="310" text-anchor="middle">jeden zdroj pravdy protokolů pro std aplikace i služby</text>
</svg>
<figcaption>V std je jen lepidlo; logika je v našem crate osrt nad libos a protokoly služeb, které sdílí i služby no_std.</figcaption>
</figure>

- **`libs/osrt`** **[ověřeno v QEMU]**: runtime pro `std` (obdoba `moto-rt`).
  `#![no_std]`, feature `rustc-dep-of-std` (místo `core`/`alloc` závisí
  na `rustc-std-workspace-core`/`-alloc`, jako všechny závislosti std).
  Obsahuje vstupní bod, heap, vlákna, futex, TLS, klienta FS a sítě
  a stdio. Testy logiky na hostiteli jako u ostatních `libs/*`.
- **libos** má feature `rt` (výchozí zapnutá) **[implementováno]**:
  `entry!`, heap, `#[global_allocator]` a `#[panic_handler]` jsou jen
  s ní (`rt_own.rs`). `std` si je dodá samo, takže knihovny (`fsproto`,
  `svcclient`, `osrt`) závisí na libos s `default-features = false`.
  Std aplikace mají jiný alokátor než služby: **dlmalloc nad objekty
  paměti** (vendorovaný v rust-src, `rustc-dep-of-std`); uvolněný segment
  se odmapuje, takže se paměť vrací systému.
- **abi, elf, initfs, tz, wire, libos, svcclient, fsproto** mají feature
  `rustc-dep-of-std` **[implementováno]** (netproto přijde se sítí).
  `spin::Mutex` ve fsproto a svcclient nahradil `libos::sync::Mutex` nad
  futexem (crate `spin` tu feature nemá a v offline buildu std by chyběl).
  Protokoly tak zůstávají **v jednom zdroji** pro služby i pro `std`.

### Co se mění v std (lepidlo)

V připnutém nightly (`nightly-2026-11-05`) má `target_os = "motor"`
větev ve 32 souborech `library/std/src`, Hermit ve 34. Naše lepidlo ve
fázi 1: **35 souborů, ~1000 řádků** (`third_party/rust-std/std.patch`),
většinou jeden řádek v `cfg_select!` a modul do ~200 řádků (soubory `sys/fs/magnumopus.rs`, `sys/thread/magnumopus.rs`,
`sys/pal/magnumopus/mod.rs`...). Plus `library/std/Cargo.toml` (závislost
na `osrt`) a `std/src/os/magnumopus/` (rozšíření, např.
`os::magnumopus::io::AsRawHandle`).

**Údržba při aktualizaci toolchainu:** lepidlo je série patchů nad
`library/` z komponenty `rust-src` připnutého nightly. Nový nightly =
patche se znovu aplikují; konflikt bývá v seznamech `cfg_select!`, když
upstream přidá jinou platformu (minuty). Jednou za čas upstream přestaví
`sys/` (v roce 2025 rozdělení `pal` na `sys/{fs,thread,...}`), pak
je přepis lepidla na 1 až 2 dny. Logika v `osrt` se nemění. Aktualizace
nightly se dělá **jen záměrně a pro celé repo najednou** (jeden toolchain
pro jádro, služby i std).

## Toolchain a sestavení

### Připnutý nightly

Stejný jako zbytek repa: `rust-toolchain.toml` (`nightly-2026-11-05`,
komponenta `rust-src` už tam je) **[implementováno]**. Druhý toolchain
jen pro std by znamenal dvě kopie `core` a dvě sady chyb překladače.
`rustc` musí být přesně ta verze, ke které patří zdroje std: xtask bere
zdroje z `rust-src` téhož toolchainu, kterým překládá, a cesty
(`target/std-src/<nightly>`, `target/sysroot/<nightly>`) nesou jméno
nightly z `rust-toolchain.toml`. Patch, který na nový nightly nesedí,
skončí chybou `git apply`.

### Jak xtask std sestaví

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sestaveni sysrootu std">
<defs><marker id="rs2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="230" height="70" rx="6"/><text class="t b" x="125" y="45" text-anchor="middle">rust-src (rustup)</text><text class="m" x="125" y="64" text-anchor="middle">library/ + vendor/</text>
<rect class="box" x="10" y="150" width="230" height="70" rx="6"/><text class="t b" x="125" y="175" text-anchor="middle">third_party/rust-std</text><text class="m" x="125" y="194" text-anchor="middle">std.patch + targets/*.json</text>
<path class="ln" d="M240 55 L288 100" marker-end="url(#rs2)"/>
<path class="ln" d="M240 185 L288 140" marker-end="url(#rs2)"/>
<rect class="acc" x="290" y="85" width="210" height="70" rx="6"/><text class="t b" x="395" y="110" text-anchor="middle">target/std-src</text><text class="m" x="395" y="129" text-anchor="middle">kopie + git apply</text>
<path class="ln" d="M500 120 L548 120" marker-end="url(#rs2)"/>
<rect class="acc" x="550" y="20" width="220" height="70" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">target/sysroot</text><text class="m" x="660" y="64" text-anchor="middle">rlib + rmeta, otisk</text>
<rect class="ok" x="550" y="150" width="220" height="70" rx="6"/><text class="t b" x="660" y="175" text-anchor="middle">aplikace</text><text class="m" x="660" y="194" text-anchor="middle">--sysroot, user-std.ld, initfs</text>
<path class="ln" d="M660 90 L660 148" marker-end="url(#rs2)"/>
</svg>
<figcaption>xtask připraví zdroje std s naším lepidlem, sestaví z nich sysroot a aplikace se pak překládají proti němu.</figcaption>
</figure>

Kód: `xtask/src/stdsys.rs` **[ověřeno v QEMU]** (sadu `std` staví
`cargo xtask build`).

1. **Zdroje:** `cargo xtask std` (a `build`) zkopíruje `library/`
   z `rust-src` připnutého nightly do `target/std-src/<nightly>/`, založí
   v kopii vlastní repozitář git a aplikuje `third_party/rust-std/std.patch`
   (`git apply`; když nesedí, chyba řekne, že patch je pro jiný nightly).
   Cestu k `libs/osrt` v `std/Cargo.toml` přepíše na absolutní (funguje
   i s jiným `CARGO_TARGET_DIR`). Kopie se dělá znovu jen při změně patche
   (otisk v `patch.stamp`).
2. **Sysroot:** `cargo build --offline --release -p sysroot
   --features compiler-builtins-mem` v kopii s `RUSTC_BOOTSTRAP=1`,
   `-Zjson-target-spec` a `-Zforce-unstable-if-unmarked`. Závislosti std
   (hashbrown, dlmalloc, compiler_builtins...) jsou ve `vendor/` komponenty
   rust-src, takže build jde **offline** (riziko z návrhu odpadlo).
   Soubory knihoven (`.rlib` i `.rmeta`: tento nightly dává do rlib jen
   zkrácená metadata) vezme xtask z hlášení cargo (`compiler-artifact`)
   a zkopíruje do `target/sysroot/<nightly>/lib/rustlib/<cíl>/lib`. Cargo
   překládá inkrementálně: změna osrt nebo libos = jen tyto crates a std.
3. **Aplikace:** `cargo build --target <json> -Zjson-target-spec` se
   `--sysroot` a linker skriptem `user/user-std.ld`. Otisk obsahu sysrootu
   jde do příznaků (`--cfg=mo_sysroot="..."`): cargo změnu sysrootu sám
   nesleduje a aplikace by zůstala slinkovaná se starým std. Hotové
   binárky přidá `cargo xtask build` do initfs (seznam
   `stdsys::STD_PROGRAMS`).

Proč ne přímo `-Zbuild-std`: cargo bere zdroje std vždy z `rust-src`
v sysrootu toolchainu a podstrčit mu patchnutou kopii jde jen přes
interní proměnné. Vlastní sysroot je deterministický a bez interních
přepínačů.

### Linker skript a TLS

`user/user-std.ld` má rozložení jako `user/user.ld` (statický `ET_EXEC`
od 4 MiB, skupiny R-X, R--, RW- na stránkách 64 KiB) a navíc segment
`PT_TLS`. `.tdata` i `.tbss` jsou v jedné výstupní sekci (vynucená
`PROGBITS`), takže celá šablona TLS je v souboru; symboly
`__tls_template_start`, `__tls_template_size` a `__tls_align` ji předají
runtime. Blok TLS vlákna (`osrt::tls`, model local-exec):

```text
x86_64 (varianta II):  [ data TLS | TCB: ukazatel sám na sebe ]
                                    ^ TP = FS_BASE
aarch64 (varianta I):  [ TCB 16 B | data TLS ]
                       ^ TP = TPIDR_EL0
```

Posuny se počítají se zarovnáním `p_align` segmentu (stejně jako lld),
blok jako celek se zarovná aspoň na 16 B. TP nastaví nové vlákno jako
první věc, ještě před kódem `std`.

### Popis cílů (JSON)

| Pole | x86_64 | aarch64 | Proč |
|---|---|---|---|
| `llvm-target` | `x86_64-unknown-none-elf` | `aarch64-unknown-none` | ELF bez OS v LLVM |
| `os` / `target-family` | `magnumopus` / žádná | totéž | ne `unix`, viz varianty |
| plovoucí čárka | SSE2 (hard-float) | NEON, `fp-armv8` (hard-float) | výkon (FFmpeg, LLVM, prohlížeč); jádro stav FPU/SIMD vláken už ukládá |
| `panic-strategy` | `abort` | `abort` | unwind později (viz rustc) |
| `relocation-model`, linker | `static`, rust-lld, `user/user.ld` | totéž | zavaděč umí jen `ET_EXEC` |
| `has-thread-local`, `tls-model` | ano, `local-exec` | ano, `local-exec` | nativní TLS (vyžaduje změnu jádra) |
| `max-atomic-width` | 64 | 128 | |

**Pozor:** služby se dál překládají pro `*-none` se soft-float (jádro
a služby bez SSE/NEON). Std aplikace budou **hard-float**. Je to jiný cíl,
takže se `.rlib` nemíchají; jen sdílené crates (libos, fsproto...) se
překládají dvakrát. AVX zatím nejde (jádro ukládá stav přes FXSAVE,
instrukce AVX skončí `#UD`); pro FFmpeg bude potřeba XSAVE v jádře.

## Mapování std na libos a služby

Legenda sloupce Stav: značky stavu z [Psaní dokumentace](dokumentace.md);
**Unsupported** = vrátí `io::ErrorKind::Unsupported`. Testy sady `std`
v `user/init/src/tests/std_app.rs`; „hello-std“ = test `std: hello-std:
argumenty, vlakna, TLS, soubory` (x86_64 i aarch64).

| Část std | Nad čím | Stav |
|---|---|---|
| start (`main`, `exit`, `abort`) | `_start` v `osrt::start` čte startovní zprávu (`libos::rt::startup`), připraví TLS a zavolá `main` od rustc; `process_exit` | **[ověřeno v QEMU]** hello-std |
| alokátor (`System`) | dlmalloc nad objekty paměti (`osrt::heap`), zámek nad futexem | **[ověřeno v QEMU]** hello-std |
| `env::args` | blok parametrů procesu | **[ověřeno v QEMU]** hello-std |
| `env::var`, `set_var`, `vars` | blok parametrů, změny jen v procesu | **[ověřeno v QEMU]** hello-std |
| `env::current_dir` / `set_current_dir` | cesta v `osrt::env`, relativní cesty se skládají v procesu (`.` a `..`) | **[ověřeno v QEMU]** hello-std |
| `io::stdout/stderr` | kanál proudu bajtů; bez něj log jádra po řádcích | **[ověřeno v QEMU]** hello-std |
| `io::stdin` | kanál proudu (`osrt::stdio::read`) | **[implementováno]**, napojení na termd **[navrženo]** |
| `time::Instant` | `clock_monotonic` | **[ověřeno v QEMU]** hello-std |
| `time::SystemTime` | `clock_realtime` (před rokem 1970 = epocha) | **[ověřeno v QEMU]** hello-std |
| `thread::spawn`, `join`, `sleep`, `yield_now` | `libos::thread` + blok TLS (`osrt::thread`) | **[ověřeno v QEMU]** hello-std |
| `Mutex`, `Condvar`, `RwLock`, `Once`, `park` | upstream implementace nad futexem (`osrt::futex`) | **[ověřeno v QEMU]** hello-std (`Mutex`, `Condvar`), ostatní **[implementováno]** |
| `thread_local!` | nativní ELF TLS (local-exec), destruktory spouští std na konci vlákna | **[ověřeno v QEMU]** hello-std |
| `thread::available_parallelism` | `SCHED_CONTROL` `CPU_INFO` (počet CPU) | **[ověřeno v QEMU]** hello-std |
| `random` (`HashMap` `RandomState`) | `random_get` | **[ověřeno v QEMU]** hello-std |
| `fs::File` open/read/write/seek/`set_len`/`sync_all`, append | fsproto v2 (handly relace; pozice v `osrt::fs::File`) | **[ověřeno v QEMU]** hello-std |
| `fs::read_dir`, `metadata`, `create_dir`, `remove_*`, `rename`, `canonicalize` | fsproto (`STAT`, `LIST`, `CREATE`, `REMOVE`, `RENAME`); `canonicalize` = normalizace + kontrola existence | **[ověřeno v QEMU]** hello-std (bez `canonicalize`) |
| `fs::Permissions` | jen `readonly` z účinných práv FS; nastavení Unsupported | **[implementováno]** |
| symlinky, `hard_link`, zámky souborů, časy souborů | Unsupported | FS v3 rozhodne **[navrženo]** |
| `net::TcpStream`, `TcpListener`, `UdpSocket`, DNS | netproto (`tcp_connect`, `tcp_listen`, `udp_bind`, `resolve`), relace sítě se štítkem `NET` | **[navrženo]** (fáze 2) |
| `process::Command`, `io::pipe` | spouštěcí služba s manifestem dítěte (rozhodnutí 4), roury = dvojice kanálů | **[navrženo]** (fáze 2); dnes Unsupported |
| `os_str`, `path` | bajty (jako unix), oddělovač `/`; `std::os::magnumopus::ffi::{OsStrExt, OsStringExt}` | **[ověřeno v QEMU]** hello-std |
| `backtrace` | Unsupported (panika vypíše zprávu na stderr) | později |
| `fork`, signály, `os::unix` | nebude | záměrně |

### Blok parametrů procesu (libos)

Formát `abi::params`, čtení a vytvoření `libos::params::Params`, předání
ve startovní zprávě jako `kind::PARAMS` **[ověřeno v QEMU]** (hello-std;
testy formátu na hostiteli `cargo test -p abi params`). Standardní
handly mají štítky `abi::params::tag` (`STDIN`, `STDOUT`, `STDERR`, `FS`,
`NET`). Podrobnosti a příklad: [libos](libos.md#blok-parametrů-procesu).

### Protokol proudu bajtů

`stdout`/`stderr`/`stdin` jsou kanály: každá zpráva = kus bajtů (nejvýš
64 KiB), zavření kanálu = konec proudu, prázdná zpráva konec není
**[ověřeno v QEMU]** (hello-std, test čte stdout až do zavření).
Zpětný tlak: kanál nemá signál „lze zapisovat“, proto při plné frontě
(`SHOULD_WAIT`) zápis počká milisekundu a zkusí to znovu. Napojení na termd
a shell je práce vlákna terminálu **[navrženo]**.

## Co přidalo jádro

1. **Futex** **[ověřeno v QEMU]** (test `std: futex: wait, wake,
   vyprseni`): `FUTEX_WAIT(adresa, očekávaná hodnota, deadline)`
   a `FUTEX_WAKE(adresa, počet)`, fronta per proces (klíč = virtuální
   adresa), viz [Syscally](syscally.md#futex).
2. **Registr vlákna pro TLS** **[ověřeno v QEMU]** (test `std: registr
   vlakna: vlastni hodnota kazdeho vlakna`): `THREAD_SET_POINTER`, jádro
   ukládá `FS_BASE` / `TPIDR_EL0` ve vlákně a nahrává ho při každém
   přepnutí. Tím je opravený i únik `TPIDR_EL0` mezi procesy na aarch64
   (program si ho mohl zapsat a jádro ho při přepnutí nenulovalo).
3. **Počet CPU** už jádro dávalo (`SCHED_CONTROL` `CPU_INFO`).
4. Později: XSAVE (AVX) pro FFmpeg, alokace stránek na požádání (viz
   [Správa paměti](#správa-paměti)), zavádění `ET_DYN` (viz rustc).

## Známá omezení

- **Zásobník vlákna bez strážní stránky** **[navrženo]**: přetečení
  může přepsat sousední mapování. Potřebuje rezervaci adres bez paměti
  (`vm_reserve`) v jádře.
- **Odpojené vlákno** (zahozený `JoinHandle`): zásobník a blok TLS zůstanou
  do konce procesu (vlákno je může ještě používat).
- **Destruktory TLS hlavního vlákna** se nespouští (proces po `main`
  končí, jako u Hermitu).
- **Plná fronta stdout** = čekání po milisekundách (viz protokol proudu).
- **`current_exe`** je Unsupported (proces nezná svou cestu, program
  může být i z initfs).
- **Odmapování paměti** vrací dlmalloc jen po celých segmentech; menší
  volné kusy zůstávají procesu.

## Fáze 1 (hotovo)

1. Spike sestavení: JSON cíle, sysroot z neupraveného std (předstupeň bodu 4).
2. Jádro: futex a registr vlákna **[ověřeno v QEMU]**.
3. libos: feature `rt`, blok parametrů, `sync::Mutex` **[ověřeno v QEMU]**.
4. `osrt` a lepidlo std (35 souborů) **[ověřeno v QEMU]**.
5. `user/hello-std` v initfs, sada `std` na x86_64 i aarch64
   **[ověřeno v QEMU]**.
6. `cargo xtask std` a `cargo xtask build` sestaví std i aplikace
   **[ověřeno v QEMU]**.

Další krok (fáze 2) **[navrženo]**: síť (`std::net` nad netproto),
`std::process::Command` přes spouštěcí službu s manifestem, stdin a stdio
v terminálu, strážní stránka zásobníku, pak průzkum `cargo check -p gix`.

## Dopady na budoucnost

### libc/POSIX vrstva (cstd) pro FFmpeg

- **Jeden runtime na proces.** Program v Rustu s přilinkovaným C (FFmpeg
  v přehrávači) musí mít jeden alokátor (`malloc` = heap libos, už dnes),
  jedno TLS (`errno` a `_Thread_local` v C potřebují nativní ELF TLS, ne
  klíče), jedny vlákna (pthread nad vlákny `osrt` a futexem) a jedno
  stdio. Proto `osrt` je navržený jako společný základ i pro `cstd`: cstd
  pak bude tenká vrstva C ABI nad `osrt`.
- `cstd` potřebuje tabulku čísel fd (int -> soubor, proud, socket); patří
  do `osrt`, aby `std` a C viděly totéž (`os::magnumopus::io::AsRawFd`).
- FFmpeg: `pthread_*`, `mmap` anonymní (jde přes VMO), soubory, sockety
  volitelně, bez `fork`. Asembler pro SIMD: na x86_64 jen SSE (AVX čeká na
  XSAVE v jádře), na aarch64 NEON; std cíl musí být hard-float (je).

### Port rustc + LLVM + lld v systému

Uživatel zvolil plný port (varianta A, bez spěchu). Co z toho plyne pro
návrh už teď:

- **Lepidlo std jako patche nad `library/`**, ne kopie: stejné patche
  půjdou aplikovat na úplný checkout `rust-lang/rust` (x.py s naším cílem
  jako tier 3), ze kterého se rustc pro náš systém křížově přeloží.
- **Procedurální makra** jsou dynamické knihovny, které rustc nahrává za
  běhu (`dlopen`). Bez dynamického zavaděče rustc nepřeloží crates se
  `serde_derive` apod. Potřeba: zavaděč `ET_DYN`, `dlopen` v `osrt`,
  `dynamic-linking` v cíli. Je to největší kus práce pro rustc v systému.
- **Unwinding**: rustc chytá paniky (`catch_unwind`) kvůli hlášení ICE;
  s `panic=abort` funguje, ale hůř. Unwinder (libunwind) přijde s C++
  (LLVM je C++ a potřebuje libc++, libc++abi, libunwind nad cstd).
  `user.ld` už `.eh_frame` zachovává.
- **Procesy a paměť**: rustc spouští linker jako proces (`Command`),
  překládá paralelně (vlákna, jobserver přes rouru) a potřebuje GB paměti;
  `memmap2` pro metadata jde nahradit čtením souboru.
- **Crates s `cfg(unix)`/`cfg(windows)`** (libc, jobserver, memmap2,
  tempfile, filetime, getrandom...) potřebují větev pro náš OS. Fork každé
  z nich je drahý; dlouhodobě je lepší upstream tier 3 a podpora v crates.

### gix pro git vrstvu FS

gix stojí na `std::fs`, `std::path`, `std::process` (jen pro externí
helpery) a čistém Rustu (SHA-1, zlib přes `miniz_oxide`). Rizika: některé
crates gix mají jen větve `unix`/`windows` (vlastník souboru v `gix-sec`,
bit spustitelnosti a symlinky v `gix-fs`/`gix-index`, `gix-tempfile`
se signály). Po fázi 1 uděláme průzkum `cargo check -p gix` pro náš cíl
s minimem features a sepíšeme, co chybí (větve `cfg`, nebo práva
a symlinky v `std::os::magnumopus`). Vrstva verzí FS v3 pak s gix mluví
přes běžné `std::fs`.

### Správa paměti

- Dnes: objekt paměti alokuje všechny stránky hned, není rezervace bez
  paměti, `mmap` souborů ani copy-on-write.
- Pro std fáze 1 to nevadí (alokátor a zásobníky jdou přes VMO).
- Pro velké aplikace (prohlížeč, rustc): **stránky na požádání** jsou
  potřeba (2 MiB zásobník x desítky vláken, rezervace velkých heapů,
  arény JIT/LLVM). Bez nich se paměť rezervuje fyzicky hned.
- `mmap` souborů: zatím emulace čtením do anonymní paměti (jen pro čtení
  nebo soukromé kopie). Sdílené mapování souborů a CoW přijdou s podporou
  v jádře a fsd; `fork` nebude (procesy jen přes `spawn`).

## Schválená rozhodnutí

Schváleno 2026-11-12, všechna doporučení:

1. **Přístup**: A vlastní PAL ve forku std (lepidlo) s logikou v `osrt`.
2. **Synchronizace**: futex v jádře.
3. **TLS**: registr vlákna v jádře + nativní ELF TLS.
4. **Spouštění procesů z std** (`Command`): aplikace s vlastním
   manifestem přes spouštěcí službu (init), která použije manifest
   dítěte; přímé `spawn_in` jen pro pomocné programy bez manifestu.
5. **Plovoucí čárka**: hard-float pro std aplikace.
6. **Toolchain**: stejný nightly jako repo.
7. **Protokol stdio**: prostý proud bajtů přes kanál.
