# libs/libos

Knihovna pro programy v userspace (`no_std`) a **stabilní API pro aplikace**: typy objektů se zavíráním handlů (`Handle`, `Event`, `Channel`, `Vmo`, `Process`, `Thread`), čekání (`wait_one`, `wait_many`, `Port`), čas, náhoda, log (`println!`), start procesu (`entry!`, heap, panic handler, `Startup`), spouštění programů z initfs (`spawn.rs`, `StartHandle`), bariéry pro ovladače (`barrier`), futex a synchronizace (`sync.rs`). Popis: `docs/cs/libos.md`, `docs/cs/syscally.md`.

## Klíčové soubory

- `src/sys.rs` - obálky syscallů nad `abi::syscall`.
- `src/rt.rs`, `rt_own.rs` - vstupní bod a běhové prostředí; `src/spawn.rs` - `spawn`, `service`, `spawn_with`; `src/hw.rs` - přístup k HW pro ovladače.
- Std pro aplikace: `libs/osrt` mapuje Rust std na libos (`docs/cs/rust-std.md`).

## Invarianty

- Aplikace volá libos, nikdy přímo čísla syscallů.
- Typy objektů zavírají handle v `Drop`; nepřidávej cestu, která handle zdvojí nebo nechá viset.
- Veřejné API je hotové až s ověřeným chováním na x86_64 i aarch64.
- Zásobníky (hlavní ve `spawn` i vláken) vždy přes `vm::map_stack` s ochrannou oblastí `vm::STACK_GUARD` pod sebou; uvolnit se musí celá rezervace (`join`).
- Spouštění sdílí kód jen programů z namapovaného initfs (`initfs::offset_of`, neměnný obsah); obraz odjinud se nahrává bez sdílení, aby záměna obrazu nepodstrčila cizí kód.
- Žádné závislosti na službách (`fsd`, `netd`): klienti jsou v `svcclient` a `*proto`.

## Testy

- Většina chování se ověřuje v QEMU testy userspace (`user/init/src/tests/`), např. `cargo xtask test --only zaklad,ipc,procesy,pamet`.
- Čisté části: `cargo test -p libos` (jen to, co se na hostiteli přeloží).

## Pasti

- Změna formátu startovní zprávy = `libs/abi/src/bootstrap.rs` + jádro (`kernel/src/userboot.rs`) + všechny služby, které ji čtou.
- Nový syscall bez obálky zde je pro aplikace neexistující.
