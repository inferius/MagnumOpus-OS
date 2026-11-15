# Výjimky a kontext vlákna

Jak program, jeho rodič nebo debugger obslouží výjimku CPU (výpadek stránky,
neplatná instrukce, breakpoint, dělení nulou ...) a jak se čte a mění
kontext vlákna. Je to krok 5 v pořadí převzetí cizího softwaru
([architektura.md](architektura.md#bezpečnost)): podklad pro SEH ve Windows
vrstvě, signály (`SIGSEGV`, `SIGFPE`) v `cstd`, výpadky v JIT a GC
a budoucí debugger. Syscally: [syscally.md](syscally.md).

## Stav

| Část | Co | Stav |
|---|---|---|
| 1 | pozastavení vlákna (`THREAD_SUSPEND`), čtení a zápis kontextu (`THREAD_STATE`) | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `procesy`) |
| 2 | kanály výjimek (`EXCEPTION_CHANNEL`, `EXCEPTION_RESOLVE`), výčet vláken (`PROCESS_THREADS`), vlákno obsluhy | **[ověřeno v QEMU]** x86_64 i aarch64 |
| 3 | doručení přímo ve vlákně (`EXCEPTION_HANDLER`), alternativní zásobník, limit vnoření | **[ověřeno v QEMU]** x86_64 i aarch64 |
| 4 | typové API libos: `Frame` (registry, PC, SP, FPU, návrat), `alloc_alt_stack`, `signal_class` | **[ověřeno v QEMU]** x86_64 i aarch64 |
| - | single-step a hardwarové breakpointy, doručení výjimky Windows `PAGE_GUARD` | **[navrženo]** |

## Pořadí obsluh

<figure>
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Pořadí obsluh výjimky">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.warn{fill:#fbefe6;stroke:#a0602a;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}.warn{fill:#33261c;stroke:#d39a63}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="vy1" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="5" y="20" width="110" height="56" rx="6"/><text class="t b" x="60" y="44" text-anchor="middle">výjimka</text><text class="m" x="60" y="62" text-anchor="middle">jádro nevyřešilo</text>
<rect class="acc" x="135" y="20" width="110" height="56" rx="6"/><text class="t b" x="190" y="44" text-anchor="middle">debugger</text><text class="m" x="190" y="62" text-anchor="middle">první šance</text>
<rect class="acc" x="265" y="20" width="110" height="56" rx="6"/><text class="t b" x="320" y="44" text-anchor="middle">ve vlákně</text><text class="m" x="320" y="62" text-anchor="middle">rámec na zásobníku</text>
<rect class="acc" x="395" y="20" width="110" height="56" rx="6"/><text class="t b" x="450" y="44" text-anchor="middle">kanál vlákna</text>
<rect class="acc" x="525" y="20" width="110" height="56" rx="6"/><text class="t b" x="580" y="44" text-anchor="middle">kanál procesu</text>
<rect class="acc" x="525" y="120" width="110" height="56" rx="6"/><text class="t b" x="580" y="144" text-anchor="middle">debugger</text><text class="m" x="580" y="162" text-anchor="middle">druhá šance</text>
<rect class="warn" x="655" y="120" width="100" height="56" rx="6"/><text class="t b" x="705" y="144" text-anchor="middle">konec</text><text class="m" x="705" y="162" text-anchor="middle">procesu</text>
<line class="ln" x1="115" y1="48" x2="131" y2="48" marker-end="url(#vy1)"/>
<line class="ln" x1="245" y1="48" x2="261" y2="48" marker-end="url(#vy1)"/>
<line class="ln" x1="375" y1="48" x2="391" y2="48" marker-end="url(#vy1)"/>
<line class="ln" x1="505" y1="48" x2="521" y2="48" marker-end="url(#vy1)"/>
<line class="ln" x1="580" y1="76" x2="580" y2="116" marker-end="url(#vy1)"/>
<line class="ln" x1="635" y1="148" x2="651" y2="148" marker-end="url(#vy1)"/>
<text class="m" x="5" y="200">Každá obsluha: RESUME (pokračovat) / NEXT (další v pořadí) / KILL. Vlákno obsluhy přeskočí vlákno a proces.</text>
</svg>
<figcaption>Výjimka jde obsluhám v pořadí, dokud ji některá nevyřeší.</figcaption>
</figure>

1. **Jádro samo** zkusí výpadek stránky vyřešit (stránky na požádání,
   kopie při zápisu - [sprava-pameti.md](sprava-pameti.md)).
2. **Debugger** procesu (kanál s `DEBUGGER`), první šance.
3. **Obsluha ve vlákně** (`EXCEPTION_HANDLER`): rychlá cesta bez
   přepnutí kontextu.
4. **Kanál vlákna**, pak **kanál procesu** (`EXCEPTION_CHANNEL`).
5. **Debugger**, druhá šance.
6. Nikdo: proces skončí kódem `EXIT_FAULT_BASE - druh` (v logu adresa,
   registry a zpětná stopa).

**Vlákno obsluhy** (`HANDLER_THREAD`) přeskočí kroky 3 a 4: vlákno, které
kanál čte, by jinak čekalo samo na sebe.

## Kanály výjimek

Kanál je běžný `Channel`, druhý konec drží jádro. Zpráva
`abi::exception::Report` (druh, stupeň, druh přístupu, proces, vlákno, PC,
adresa, kód architektury) nese handly vlákna a tokenu. Vlákno mezitím
stojí a obsluha čte a mění jeho kontext (`THREAD_STATE`) a rozhodne
(`EXCEPTION_RESOLVE`): `RESUME`, `NEXT`, `KILL`. Zavření tokenu bez
rozhodnutí = `NEXT`, takže smrt obsluhy nic nezasekne.

```rust
let ch = child.exception_channel(false)?;
let e = libos::exception::receive(&ch, None)?;
if e.report.kind == abi::exception::kind::PAGE_FAULT {
    // namapovat stránku, upravit kontext ...
    e.resume()?;
}
```

## Doručení ve vlákně

Obsluha (`libos::exception::set_handler`) dostane rámec
`abi::exception::frame` (zarovnaný na 64 B): `Report`, obecné registry
a FPU/SIMD v okamžiku výjimky. FP část má hlavičku
`abi::thread::fp::Header` (jako `THREAD_STATE`) a data za ní; její délka
závisí na CPU (x86_64 obraz XSAVE s AVX/AVX-512), takže **rámec má
proměnnou velikost** (`libos::exception::frame_size()`,
`abi::exception::frame::size`). Typový přístup: `Frame::fp_header()`,
`fp()`, `fp_mut()`; nejmenší alternativní zásobník
`libos::exception::min_alt_stack()`. Leží na **alternativním zásobníku**
(`set_alt_stack`, jako `sigaltstack`), je-li nastavený a vlákno na něm
ještě není, jinak pod SP (na x86_64 pod červenou zónou 128 B). Obsluha se
nevrací, končí `handler_return(rámec, RESUME | NEXT)`.

- **Kontrola návratu:** kontext z rámce projde stejnou kontrolou jako
  `THREAD_STATE` (PC a SP v userspace, jen uživatelské příznaky, FPU bez
  rezervovaných bitů). Podvržený rámec tak umí jen to, co vlákno smí samo.
- **NEXT** bere druh a adresu výjimky z kopie jádra, ne z rámce.
- **Přetečení zásobníku:** rámec se zapisuje kontrolovaně; když se
  nevejde (ochranná oblast), obsluha se přeskočí a výjimka jde dál - až ke
  konci procesu, nikdy do rekurze. S alternativním zásobníkem se obsluha
  zavolá.
- **Vnoření:** výjimka v obsluze se doručí znovu nejvýš do hloubky 8
  (`handler::MAX_DEPTH`); další jde rovnou na kanály.
- **Vstup obsluhy** musí ležet ve spustitelné paměti, tedy zapečetěné nebo
  s oprávněním JIT (W^X, [sprava-pameti.md](sprava-pameti.md)).

### API pro signály a SEH

`libos::exception::Frame` je typový přístup k rámci (podklad pro
`ucontext_t` v `cstd` a `CONTEXT` ve Windows vrstvě): `report()`,
`general()` / `set_general()` (`X64General` / `A64General`), `pc()`,
`sp()`, `fp()` a návrat `resume()` / `next()`. `alloc_alt_stack(size)`
alokuje a nastaví alternativní zásobník vlákna.

| Druh (`abi::exception::kind`) | `signal_class` | POSIX | SEH (Wine) |
|---|---|---|---|
| `PAGE_FAULT`, `GENERAL` | `Segv` | `SIGSEGV` | `EXCEPTION_ACCESS_VIOLATION` (`access`: čtení / zápis / spuštění) |
| `ALIGNMENT` | `Bus` | `SIGBUS` | `EXCEPTION_DATATYPE_MISALIGNMENT` |
| `PAGE_FAULT` s `reason` = `BEYOND_END` (za koncem zvětšitelného objektu paměti, [správa paměti](sprava-pameti.md#zvětšitelný-objekt-paměti); `signal_class_of`) | `Bus` | `SIGBUS` | `EXCEPTION_IN_PAGE_ERROR` |
| `DIVIDE` | `Fpe` | `SIGFPE` | `EXCEPTION_INT_DIVIDE_BY_ZERO` |
| `INVALID_OPCODE` | `Ill` | `SIGILL` | `EXCEPTION_ILLEGAL_INSTRUCTION` |
| `BREAKPOINT` | `Trap` | `SIGTRAP` | `EXCEPTION_BREAKPOINT` |

Čísla signálů určuje `cstd`, kódy `EXCEPTION_*` Windows vrstva.

```rust
extern "C" fn on_fault(f: *mut u8) -> ! {
    let mut fr = unsafe { libos::exception::Frame::from_raw(f) };
    if libos::exception::signal_class(fr.report().kind) == SignalClass::Trap {
        // aarch64: PC stojí na brk
        if cfg!(target_arch = "aarch64") { let pc = fr.pc(); fr.set_pc(pc + 4); }
        let _ = fr.resume();
    }
    let _ = fr.next();
    libos::sys::thread_exit()
}
```

## Pozastavení a kontext

- `THREAD_SUSPEND` vrací token, zavření ho vlákno obnoví. Vlákno stojí na
  hranici návratu do userspace; spící v syscallu je pozastavené hned.
- `THREAD_STATE` čte a zapisuje obecné registry (`abi::thread::X64General`,
  `A64General`) a FPU/SIMD (hlavička a obraz XSAVE/FXSAVE, V0-V31 + FPCR + FPSR) pozastaveného
  vlákna nebo vlákna ve výjimce. Zápis vlákno samo použije před návratem.
- `PROCESS_THREADS` dá debuggeru handly všech vláken.

## Bezpečnost

- **Kdo smí:** číst kontext `READ`, zapsat `WRITE`, pozastavit a kanály
  `MANAGE`. **`WRITE` na procesu = plná kontrola** (stejně jako `VM_MAP`):
  rodič je přirozeně debuggerem potomka, jiným se takový handle nedává.
- **Jádro se nedá shodit** kontextem: nekanonické nebo jaderné PC a SP,
  IOPL a změna režimu, rezervované bity MXCSR/FPCR/FPSR se odmítnou nebo
  ořežou (testy v sadě `procesy`).
- **Kill vždy vyhraje** nad pozastavením i čekáním ve výjimce.

## Rozdíly architektur

- **aarch64 dělení nulou nehlásí** (`udiv`/`sdiv` vrátí 0). `cstd`
  (`SIGFPE`) a Wine (`EXCEPTION_INT_DIVIDE_BY_ZERO`) to musí zajistit
  kontrolou v kódu.
- **Breakpoint:** na x86_64 je PC už za `int3`, na aarch64 na `brk` -
  obsluha ho posune o 4.
- **TEB přes GS** (Windows vrstva) je samostatný úkol; v kontextu je zatím
  jen báze FS / `TPIDR_EL0`.

## Klíčové soubory

| Soubor | Co |
|---|---|
| `kernel/src/sched/suspend.rs` | pozastavení, parkování, čekající zápis kontextu, stav vlákna pro výjimky |
| `kernel/src/object/exception.rs` | pořadí obsluh, kanály, token, doručení ve vlákně, `RETURN` |
| `kernel/src/arch/*/ustate.rs` | rámec registrů a FPU, kontrola zápisu, příprava skoku do obsluhy |
| `libs/abi/src/thread.rs`, `exception.rs` | formáty kontextu, zprávy a rámce |
| `libs/libos/src/exception.rs` | API pro obsluhy |

## Testy

`cargo xtask test --only procesy` (x86_64 i aarch64):
`user/init/src/tests/thread_state.rs`, `exceptions.rs`,
`exceptions_fast.rs` - pozastavení a přesměrování PC, neplatný kontext,
kill pozastaveného, kanály a pořadí, breakpoint, vlákno obsluhy, obsluha
ve vlákně (oprava výpadku, podvržený rámec, NEXT, přetečení s
alternativním zásobníkem i bez něj, limit vnoření).
