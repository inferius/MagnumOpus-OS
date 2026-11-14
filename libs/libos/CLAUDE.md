# libs/libos

Knihovna pro programy v userspace (`no_std`) a **stabilní API pro aplikace**: typy objektů se zavíráním handlů (`Handle`, `Event`, `Fence`, `Channel`, `Vmo`, `Process`, `Thread`), čekání (`wait_one`, `wait_many`, `Port`), čas, náhoda, log (`println!`), start procesu (`entry!`, heap, panic handler, `Startup`), spouštění programů z initfs (`spawn.rs`, `StartHandle`), bariéry pro ovladače (`barrier`), futex a synchronizace (`sync.rs`). Popis: `docs/cs/libos.md`, `docs/cs/syscally.md`.

## Klíčové soubory

- `src/sys.rs` - obálky syscallů nad `abi::syscall`.
- `src/rt.rs`, `rt_own.rs` - vstupní bod a běhové prostředí; `src/spawn.rs` - `spawn`, `service`, `spawn_with`; `src/hw.rs` - přístup k HW pro ovladače.
- Std pro aplikace: `libs/osrt` mapuje Rust std na libos (`docs/cs/rust-std.md`).

## Invarianty

- Aplikace volá libos, nikdy přímo čísla syscallů.
- Typy objektů zavírají handle v `Drop`; nepřidávej cestu, která handle zdvojí nebo nechá viset.
- Veřejné API je hotové až s ověřeným chováním na x86_64 i aarch64.
- Program PIE s `PT_INTERP` (`spawn.rs`): program (šablona z initfs volajícího nebo kopie) a zavaděč od `libd` na náhodné báze (`random_base`), kód zapečetěný, auxv na zásobník, zpráva zavaděče (`LOADER_MAGIC`, proces + spojení s libd) **před** startovní zprávou; startovní zpráva a kanál patří programu. Relaci s libd a lístek (`libd.rs`, feature `libd`; `bootstrap::kind::LIBD` arg 0/1) dostane jen zavaděč a statický program se `Options::libd`; statický spawn bez volby s libd ani initem nemluví. Relace a lístek pro dítě (`libd::child`) **bez čekání**: `Connect` + `ADOPT` (init: `set_provider`). Po `PEER_CLOSED` `RECONNECT` přes lístek. `ET_EXEC` beze změny. Popis: `docs/cs/dynamicke-linkovani.md`.
- Pod zavaděčem (`loader.rs`, tabulka z auxv): `thread::spawn` bere blok TLS od zavaděče a nastaví TP v trampolíně, `join` ho vrátí; `rt_own` po `main` volá `loader::run_fini`; `loader::{dlopen, dlsym, dlclose}` obalují tabulku (chyba = text z `dlerror`). Bez zavaděče beze změny (`dlopen` = chyba). `spawn::Options::static_tls_reserve` -> auxv `AT_MO_TLS_RESERVE`.
- Zásobníky (hlavní ve `spawn` i vláken) vždy přes `vm::map_stack` s ochrannou oblastí `vm::STACK_GUARD` pod sebou; uvolnit se musí celá rezervace (`join`).
- Spouštění sdílí kód jen programů z namapovaného initfs (`initfs::offset_of`, neměnný obsah); obraz odjinud se nahrává bez sdílení, aby záměna obrazu nepodstrčila cizí kód.
- Žádné závislosti na službách (`fsd`, `netd`): klienti jsou v `svcclient` a `*proto`.

## Testy

- Většina chování se ověřuje v QEMU testy userspace (`user/init/src/tests/`), např. `cargo xtask test --only zaklad,ipc,procesy,pamet`.
- Čisté části: `cargo test -p libos` (jen to, co se na hostiteli přeloží).

## Pasti

- Změna formátu startovní zprávy = `libs/abi/src/bootstrap.rs` + jádro (`kernel/src/userboot.rs`) + všechny služby, které ji čtou.
- Nový syscall bez obálky zde je pro aplikace neexistující.
