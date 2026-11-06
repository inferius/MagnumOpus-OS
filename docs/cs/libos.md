# libos - knihovna pro programy

`libs/libos` je knihovna, kterou používá každý program v userspace. Je to
**stabilní rozhraní pro aplikace**; syscally pod ní se smí měnit.

## Minimální program

```rust
#![no_std]
#![no_main]

extern crate alloc;

use libos::{Startup, println};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    println!("ahoj");
    0 // návratový kód procesu
}
```

Program potřebuje `build.rs`, který přidá linker skript `user/user.ld`
(viz `user/init/build.rs`), a sestavuje ho `cargo xtask build` (stejný
target jako jádro, ale jiné příznaky překladu). Jméno programu se přidá do
`USER_PROGRAMS` v `xtask/src/main.rs`, aby se dostal do initfs.

## Start procesu

Jádro (u `init`) nebo rodič (u ostatních) spustí první vlákno na symbolu
`_start` z libos s handlem bootstrap kanálu v `rdi`. libos pak:

1. přečte startovní zprávu (`abi::bootstrap`): handly a jejich význam,
2. namapuje heap (4 MiB) a zapne globální alokátor (`alloc::vec::Vec` atd.),
3. zavolá funkci programu z `entry!` se strukturou [`Startup`]:
   - `process` - handle na vlastní proces (mapování paměti, vlákna),
   - `initfs` - obraz initfs (jen `init`),
   - `handles` - další handly od rodiče,
   - `bootstrap` - kanál k rodiči pro další komunikaci,
4. návratovou hodnotu použije jako návratový kód procesu (`PROCESS_EXIT`).

Panika v programu vypíše zprávu do logu a ukončí proces s kódem -1 000 001.

## Typy objektů

Každý typ vlastní handle a zavře ho při zrušení (`Drop`).

| Typ | Účel | Hlavní metody |
|---|---|---|
| `Handle` | obecný handle | `duplicate(práva)`, `into_raw`, `from_raw` |
| `Event` | signál mezi vlákny a procesy | `create`, `signal`, `reset` |
| `Channel` | zprávy s daty a handly | `create`, `write`, `try_read`, `read(deadline)` |
| `Vmo` | objekt paměti | `create`, `read`, `write` |
| `Process` | proces | `create`, `map`, `unmap`, `start`, `kill`, `wait_exit` |
| `Thread` | vlákno | `create`, `start`, `wait_exit` |

Funkce: `wait_one`, `wait_many` (čekání na signály), `time::now`,
`time::deadline_after`, `time::sleep`, `exit(kód)`, `println!` (log jádra).

Chyby jsou `libos::Error` (= `abi::Error`), popsané v [chyby.md](chyby.md).
Konstanty rozhraní (příznaky mapování, kódy procesů) jsou v `libos::abi`.

## Spuštění programu

```rust
let fs = libos::initfs::Initfs::open(&startup)?;          // jen init
let image = fs.find("echo").ok_or(Error::NotFound)?;
let (process, channel) = libos::spawn("echo", image, Vec::new())?;
channel.write(b"ping:1", Vec::new())?;
let reply = channel.read(None)?;
let code = process.wait_exit(None)?;
```

`spawn` vytvoří prázdný proces, namapuje do něj segmenty ELF (s právy podle
segmentů, W^X) a zásobník 256 KiB, pošle startovní zprávu a spustí první
vlákno. Vrátí proces a konec kanálu k potomkovi.

## Nízká úroveň

`libos::sys` obsahuje přímá volání syscallů (jedna funkce na syscall, viz
[syscally.md](syscally.md)). Je veřejné kvůli testům rozhraní; programy mají
používat typy výše.
