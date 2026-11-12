# Rust std pro aplikace

Aplikace pro MagnumOpus půjdou psát v běžném Rustu se `std` (`fn main()`,
`std::fs`, `std::thread`, `std::net`, crates z crates.io). Systémové
služby a ovladače zůstávají `no_std` nad [libos](libos.md). Tahle stránka
je **návrh ke schválení (fáze 0)**: přístup, sestavení, mapování částí
`std` na libos a služby, co chybí v jádře a dopady na budoucí plány.

> **Stav:** celé **[navrženo]**, kód zatím není. Otevřená rozhodnutí jsou
> v kapitole [Rozhodnutí ke schválení](#rozhodnutí-ke-schválení).

## Cíl a rozsah

- Cíle `x86_64-unknown-magnumopus` a `aarch64-unknown-magnumopus`
  (`target_os = "magnumopus"`) **[navrženo]**.
- `std` nad libos: alokace, vlákna, synchronizace, TLS, čas, soubory přes
  `fsd`, síť přes `netd`, procesy, argumenty, proměnné prostředí, stdio
  **[navrženo]**.
- `cargo xtask` sestaví std (sysroot) i aplikaci a dá ji do initfs
  **[navrženo]**.
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

**Doporučení: A.** Je to cesta, kterou upstream sám používá (Motor OS
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

- **`libs/osrt`** **[navrženo]**: runtime pro `std` (obdoba `moto-rt`).
  `#![no_std]`, feature `rustc-dep-of-std` (místo `core`/`alloc` závisí
  na `rustc-std-workspace-core`/`-alloc`, jako všechny závislosti std).
  Obsahuje vstupní bod, heap, vlákna, futex, TLS, klienta FS a sítě
  a stdio. Testy logiky na hostiteli jako u ostatních `libs/*`.
- **libos** dostane feature `rt` (výchozí zapnutá) **[navrženo]**:
  `entry!`, `#[global_allocator]` a `#[panic_handler]` jsou jen s ní.
  `std` si je dodá samo, takže `osrt` používá libos bez `rt`. Heap se
  přesune do modulu, který jde použít bez atributu `#[global_allocator]`
  (stejný alokátor pro služby i std aplikace).
- **fsproto, netproto, svcclient, wire, abi** dostanou feature
  `rustc-dep-of-std`. Závislost `spin` ve fsproto se nahradí vlastním
  zámkem (crate `spin` tuhle feature nemá). Protokoly tak zůstanou
  **v jednom zdroji** pro služby i pro `std`.

### Co se mění v std (lepidlo)

V připnutém nightly (`nightly-2026-11-05`) má `target_os = "motor"`
větev ve 32 souborech `library/std/src`, Hermit ve 34. Odhad pro nás
**~35 souborů**, většinou jeden řádek v `cfg_select!` a modul do ~200
řádků (soubory `sys/fs/magnumopus.rs`, `sys/thread/magnumopus.rs`,
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
komponenta `rust-src` už tam je) **[navrženo]**. Druhý toolchain jen pro
std by znamenal dvě kopie `core` a dvě sady chyb překladače. `rustc`
musí být přesně ta verze, ke které patří zdroje std; xtask to hlídá
(verze z `rustc -vV` proti verzi v hlavičce patchů).

### Jak xtask std sestaví

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sestaveni sysrootu std">
<defs><marker id="rs2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="230" height="70" rx="6"/><text class="t b" x="125" y="45" text-anchor="middle">rust-src (rustup)</text><text class="m" x="125" y="64" text-anchor="middle">library/ připnutého nightly</text>
<rect class="box" x="10" y="150" width="230" height="70" rx="6"/><text class="t b" x="125" y="175" text-anchor="middle">third_party/rust-std</text><text class="m" x="125" y="194" text-anchor="middle">patche + nové soubory sys/*</text>
<path class="ln" d="M240 55 L288 100" marker-end="url(#rs2)"/>
<path class="ln" d="M240 185 L288 140" marker-end="url(#rs2)"/>
<rect class="acc" x="290" y="85" width="210" height="70" rx="6"/><text class="t b" x="395" y="110" text-anchor="middle">target/std-src</text><text class="m" x="395" y="129" text-anchor="middle">kopie + git apply</text>
<path class="ln" d="M500 120 L548 120" marker-end="url(#rs2)"/>
<rect class="acc" x="550" y="20" width="220" height="70" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">target/sysroot</text><text class="m" x="660" y="64" text-anchor="middle">rlib core, alloc, std, osrt</text>
<rect class="ok" x="550" y="150" width="220" height="70" rx="6"/><text class="t b" x="660" y="175" text-anchor="middle">aplikace</text><text class="m" x="660" y="194" text-anchor="middle">cargo build --sysroot, initfs</text>
<path class="ln" d="M660 90 L660 148" marker-end="url(#rs2)"/>
</svg>
<figcaption>xtask připraví zdroje std s naším lepidlem, sestaví z nich sysroot a aplikace se pak překládají proti němu.</figcaption>
</figure>

1. `cargo xtask std` **[navrženo]** zkopíruje `library/` z `rust-src`
   připnutého nightly do `target/std-src/<nightly>/` a aplikuje patche
   z `third_party/rust-std/` (`git apply`, chyba = jasná hláška, které
   soubory nesedí). Do `library/std/Cargo.toml` doplní cestu k `libs/osrt`.
2. Sestaví sysroot: `cargo build -p sysroot` v kopii `library/`
   s `RUSTC_BOOTSTRAP=1`, cílem z JSON a příznaky jako bootstrap
   (`-Zforce-unstable-if-unmarked`, `compiler-builtins-mem`, `panic_abort`).
   Výsledné `.rlib` dá do `target/sysroot/<nightly>/lib/rustlib/<cíl>/lib`.
   Znovu se sestavuje jen při změně patchů, `osrt`, libos nebo nightly
   (otisk v souboru vedle sysrootu).
3. Aplikace se překládá běžným `cargo build --target <json>`
   s `--sysroot target/sysroot/<nightly>`; linkuje rust-lld se skriptem
   `user/user.ld`. `cargo xtask build` pak aplikaci přidá do initfs
   (seznam std aplikací vedle `USER_PROGRAMS`).

Proč ne přímo `-Zbuild-std`: cargo bere zdroje std vždy z `rust-src`
v sysrootu toolchainu a podstrčit mu patchnutou kopii jde jen přes
interní proměnné nebo vlastní toolchain přes `rustup toolchain link`.
Vlastní sysroot (postup, který dříve dělal Xargo) je deterministický a bez
interních přepínačů cargo. **Riziko:** závislosti std (hashbrown,
compiler_builtins...) se při prvním sestavení stahují z crates.io
(pak z cache cargo); offline první build selže. Ověří se ve fázi 1
jako první krok (spike); kdyby vlastní sysroot narazil, záloha je
`-Zbuild-std` s toolchainem přes `rustup toolchain link`.

### Popis cílů (JSON)

| Pole | x86_64 | aarch64 | Proč |
|---|---|---|---|
| `llvm-target` | `x86_64-unknown-none-elf` | `aarch64-unknown-none` | ELF bez OS v LLVM |
| `os` / `target-family` | `magnumopus` / žádná | totéž | ne `unix`, viz varianty |
| plovoucí čárka | SSE2 (hard-float) | NEON, `fp-armv8` (hard-float) | výkon (FFmpeg, LLVM, prohlížeč); jádro stav FPU/SIMD vláken už ukládá |
| `panic-strategy` | `abort` | `abort` | unwind později (viz rustc) |
| `relocation-model`, linker | `static`, rust-lld, `user/user.ld` | totéž | zavaděč umí jen `ET_EXEC` |
| `has-thread-local`, `tls-model` | ano, `local-exec` | ano, `local-exec` | nativní TLS (vyžaduje změnu jádra) |
| `max-atomic-width` | 64 | 128 nebo 64 (dle `lse`) | |

**Pozor:** služby se dál překládají pro `*-none` se soft-float (jádro
a služby bez SSE/NEON). Std aplikace budou **hard-float**. Je to jiný cíl,
takže se `.rlib` nemíchají; jen sdílené crates (libos, fsproto...) se
překládají dvakrát. AVX zatím nejde (jádro ukládá stav přes FXSAVE,
instrukce AVX skončí `#UD`); pro FFmpeg bude potřeba XSAVE v jádře.

## Mapování std na libos a služby

Legenda: **F1** = fáze 1 (hned), **F2** = další krok, **Unsupported** =
vrátí `io::ErrorKind::Unsupported`, **jádro** = chybí v jádře, **libos**
= chybí v libos/protokolech.

| Část std | Nad čím | Kdy | Co chybí |
|---|---|---|---|
| start (`main`, `exit`, `abort`) | `_start` v osrt čte startovní zprávu, volá `main` přes `lang_start`; `process_exit` | F1 | nic |
| alokátor (`System`) | heap libos (`sizeclass` + first-fit, růst přes VMO) | F1 | velké alokace (>= 1 MiB) jako vlastní VMO, aby se paměť vracela; F2 |
| `env::args` | blok parametrů procesu | F1 | **libos**: protokol parametrů (níže) |
| `env::var`, `set_var` | blok parametrů, změny v procesu | F1 | totéž |
| `env::current_dir` / `set_current_dir` | cesta v osrt, relativní cesty se skládají v procesu | F1 | totéž (počáteční adresář) |
| `io::stdout/stderr` | kanál bajtového proudu z bloku parametrů; bez něj log jádra (`debug_log`) | F1 | **libos**: protokol proudu (níže) |
| `io::stdin` | kanál proudu | F2 | totéž + napojení na termd |
| `time::Instant` | `clock_monotonic` | F1 | nic |
| `time::SystemTime` | `clock_realtime` (`abi::time::Reading`) | F1 | nic |
| `thread::spawn`, `join`, `sleep`, `yield_now` | `libos::thread`, `time::sleep`, `thread_yield` | F1 | **libos**: stráž zásobníku (níže) |
| `thread::park`, `Mutex`, `Condvar`, `RwLock`, `Once` | futex (`sys/sync/*/futex.rs` z upstream beze změny) | F1 | **jádro**: futex (rozhodnutí 2) |
| `thread_local!` | nativní ELF TLS (`#[thread_local]`), blok TLS na vlákno | F1 | **jádro**: registr vlákna (rozhodnutí 3) |
| `thread::available_parallelism` | počet CPU | F1 | **libos**: dotaz na počet CPU (dnes není, jinak Unsupported) |
| `random` (`HashMap` `RandomState`) | `random_get` | F1 | nic |
| `fs::File` open/read/write/seek/`set_len`/`sync_all` | fsproto v2 (handly relace, soubor přežije přejmenování) | F1 | nic (relace FS z bloku parametrů) |
| `fs::read_dir`, `metadata`, `create_dir`, `remove_*`, `rename` | fsproto (`LIST_PAGE`, `STAT`, `CREATE`, `REMOVE`, `RENAME`) | F1 | nic |
| `fs::Permissions` | jen `readonly` z práv FS | F2 | mapování práv FS |
| symlinky, `hard_link`, `canonicalize` | `canonicalize` = normalizace cesty; odkazy Unsupported | F2 | FS v3 rozhodne o odkazech |
| `net::TcpStream`, `TcpListener`, `UdpSocket` | netproto (`tcp_connect`, `tcp_listen`, `udp_bind`), relace sítě z bloku parametrů | F2 | nic |
| `ToSocketAddrs` (DNS) | `Net::resolve` | F2 | nic |
| `process::Command`, `Child`, `exit status` | čtení ELF z FS, `libos::spawn_in` ve vlastním jobu, blok parametrů dítěti | F2 | **libos**: spouštění s manifestem (rozhodnutí 4) |
| `io::pipe`, `Stdio::piped` | dvojice kanálů s protokolem proudu | F2 | totéž co stdio |
| `os_str`, `path` | bajty v UTF-8 (jako unix `os_str/bytes`), oddělovač `/` | F1 | nic |
| `backtrace` | Unsupported (panika vypíše zprávu) | později | unwinder + symboly |
| `fork`, signály, `os::unix` | nebude | - | záměrně |

### Blok parametrů procesu (libos) **[navrženo]**

Dnes rodič předává handly bez dohodnutého významu (`kind::USER`, štítky
podle programu) a program nemá argumenty, prostředí ani stdio. Std
potřebuje jedno společné místo:

- nový druh handlu ve startovní zprávě `kind::PARAMS`: objekt paměti jen
  pro čtení s **blokem parametrů** (verze, `argv`, `envp`, počáteční
  adresář, tabulka „jméno -> index handlu“),
- handly, na které blok ukazuje, jsou v téže startovní zprávě
  (`kind::USER` se štítkem): `stdin`, `stdout`, `stderr`, relace FS,
  relace sítě, kanál k rodiči,
- libos dostane `spawn::Options::params(...)` pro rodiče a čtení bloku pro
  děti; používat ho můžou i programy `no_std` (shell, init).

Objekt paměti místo dat ve zprávě: argumenty a prostředí nejsou omezené
velikostí zprávy kanálu.

### Protokol proudu bajtů **[navrženo]**

`stdout`/`stderr`/`stdin` a `io::pipe` jsou kanály s nejjednodušším
protokolem: každá zpráva = kus bajtů (do limitu zprávy), zavření kanálu =
konec proudu. Zpětný tlak zajistí limit fronty kanálu (zápis čeká, když je
fronta plná). Terminál (termd) a shell si proud převedou na svůj nativní
protokol; to je práce pro vlákno terminálu a koordinuje se s ním.

### Stráž zásobníku **[navrženo]**

libos dnes mapuje zásobník vlákna tam, kam ho dá jádro; hned pod ním může
ležet jiné mapování a přetečení ho tiše přepíše. Std vlákna mají výchozí
zásobník 2 MiB a hlubokou rekurzi (parsery, serde). Potřeba: rezervace
rozsahu adres bez paměti (`vm_reserve`) nebo mapování s nenamapovanou
stránkou pod zásobníkem, aby přetečení skončilo chybou stránky. Patří do
libos (a případně jádra).

## Co chybí v jádře

1. **Futex** (rozhodnutí 2) **[navrženo]**: `futex_wait(adresa,
   očekávaná hodnota, deadline)` a `futex_wake(adresa, počet)` pro vlákna
   jednoho procesu (klíč = proces + virtuální adresa). Std pak používá
   upstream implementace `Mutex`, `Condvar`, `RwLock`, `Once` a `park`
   beze změny.
2. **Registr vlákna pro TLS** (rozhodnutí 3) **[navrženo]**: jádro musí
   ukládat a obnovovat při přepnutí vlákna `FS_BASE` (x86_64) a
   `TPIDR_EL0` (aarch64) a dát ho nastavit (při `thread_start` nebo
   syscallem). **Riziko už dnes:** jádro `TPIDR_EL0` při přepnutí neukládá
   ani nenuluje, přitom ho program na aarch64 smí zapsat sám. Hodnota
   zapsaná jedním procesem je tak vidět v jiném (skrytý kanál mezi
   procesy, a kód v C s TLS by se rozbil). Opravit nezávisle na std.
3. **Počet CPU** pro `available_parallelism` (může být součástí dotazů na
   schopnosti systému).
4. Později: XSAVE (AVX) pro FFmpeg, alokace stránek na požádání (viz
   [Správa paměti](#správa-paměti)), zavádění `ET_DYN` (viz rustc).

## Fáze 1 (po schválení)

1. Spike sestavení: `third_party/rust-std` s prázdnou platformou (vše
   Unsupported), `cargo xtask std` vyrobí sysroot pro oba cíle.
2. libos: feature `rt`, blok parametrů, protokol proudu; `osrt`: start,
   heap, stdout/stderr, args/env, čas, náhoda.
3. Jádro: futex a registr vlákna (podle rozhodnutí; případně jiné vlákno).
4. `osrt`: vlákna, TLS, futex; std `Mutex`, `thread::spawn`.
5. `osrt`: `fs` (open, read, write, read_dir, metadata, create_dir).
6. `user/hello-std`: vypíše argumenty a proměnné, spustí 4 vlákna se
   sdíleným `Mutex`, zapíše a přečte soubor, vypíše adresář. Test
   v `cargo xtask test` na x86_64 i aarch64 (veřejné API je hotové až
   s ověřeným chováním na obou architekturách).
7. Docs: tahle stránka se značkami podle stavu, [libos](libos.md)
   (blok parametrů, proud), [Syscally](syscally.md) (futex, registr
   vlákna).

## Dopady na budoucnost

### libc/POSIX vrstva (cstd) pro FFmpeg

- **Jeden runtime na proces.** Program v Rustu s přilinkovaným C (FFmpeg
  v přehrávači) musí mít jeden alokátor (`malloc` = heap libos, už dnes),
  jedno TLS (`errno` a `_Thread_local` v C potřebují nativní ELF TLS, ne
  klíče), jedny vlákna (pthread nad vlákny `osrt` a futexem) a jedno
  stdio. Proto `osrt` navrhujeme jako společný základ i pro `cstd`: cstd
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

## Rozhodnutí ke schválení

1. **Přístup**: A vlastní PAL (doporučeno) / B rodina unix nad libc /
   C fasáda bez forku.
2. **Synchronizace**: A futex v jádře (doporučeno; potřebuje ho i pthread
   v cstd a sdílená paměť mezi procesy) / B emulace futexu v procesu nad
   objekty `Event` (bez změny jádra, jen v rámci procesu; jako dočasná
   záloha, pokud jádro teď nikdo nemůže měnit).
3. **TLS**: A registr vlákna v jádře + nativní ELF TLS (doporučeno;
   potřebuje ho i C) / B klíče TLS jako Motor OS (pořád potřebuje registr
   vlákna, jen pomalejší a C s `_Thread_local` nepůjde).
4. **Spouštění procesů z std** (`Command`): A přímo `spawn_in` ve vlastním
   jobu, dítě dostane jen schopnosti rodiče / B přes spouštěcí službu
   (init), která použije manifest dítěte (doporučeno pro aplikace
   s vlastním manifestem; A jen pro pomocné programy bez manifestu).
   Potřebuje souhlas, protože se dotýká modelu oprávnění.
5. **Plovoucí čárka**: hard-float pro std aplikace (doporučeno) / soft-float
   jako služby.
6. **Toolchain**: stejný nightly jako repo (doporučeno) / samostatný pro std.
7. **Protokol stdio**: prostý proud bajtů přes kanál (doporučeno) /
   nativní protokol termd.
