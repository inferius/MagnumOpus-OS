# libs/libos

Knihovna pro programy v userspace (`no_std`) a **stabilní API pro aplikace**: typy objektů se zavíráním handlů (`Handle`, `Event`, `Fence`, `DmaMap`, `Channel`, `Vmo` vč. `create_pinned`, `Process`, `Thread`), čekání (`wait_one`, `wait_many`, `Port`), čas, náhoda, log (`println!`), start procesu (`entry!`, heap, panic handler, `Startup`), spouštění programů z initfs (`spawn.rs`, `StartHandle`), bariéry pro ovladače (`barrier`), futex a synchronizace (`sync.rs`). Popis: `docs/cs/libos.md`, `docs/cs/syscally.md`.

## Klíčové soubory

- `src/sys.rs` - obálky syscallů nad `abi::syscall`.
- `src/rt.rs`, `rt_own.rs` - vstupní bod a běhové prostředí; `src/spawn.rs` - `spawn`, `service`, `spawn_with`; `src/hw.rs` - přístup k HW pro ovladače.
- Std pro aplikace: `libs/osrt` mapuje Rust std na libos (`docs/cs/rust-std.md`).

## Invarianty

- Aplikace volá libos, nikdy přímo čísla syscallů.
- Typy objektů zavírají handle v `Drop`; nepřidávej cestu, která handle zdvojí nebo nechá viset.
- Veřejné API je hotové až s ověřeným chováním na x86_64 i aarch64.
- Program PIE s `PT_INTERP` (`spawn.rs`): program (šablona z initfs volajícího nebo kopie) a zavaděč od `libd` na náhodné báze (`random_base`), kód zapečetěný, auxv na zásobník, zpráva zavaděče (`LOADER_MAGIC`, proces + spojení s libd) **před** startovní zprávou; startovní zpráva a kanál patří programu. Relaci s libd a lístek (`libd.rs`, feature `libd`; `bootstrap::kind::LIBD` arg 0/1) dostane jen zavaděč a statický program se `Options::libd`; statický spawn bez volby s libd ani initem nemluví. Relace a lístek pro dítě (`libd::child`) **bez čekání**: `Connect` + `ADOPT` (init: `set_provider`). Po `PEER_CLOSED` `RECONNECT` přes lístek. `ET_EXEC` beze změny. Popis: `docs/cs/dynamicke-linkovani.md`.
- Pod zavaděčem (`loader.rs`, tabulka z auxv): `thread::spawn` bere blok TLS od zavaděče a nastaví TP v trampolíně, `join` ho vrátí; `rt_own` po `startup` volá `loader::run_init` (inicializátory modulů, zavaděč je odkládá na program) a po `main` `loader::run_fini`; `loader::{dlopen, dlsym, dlclose}` obalují tabulku (chyba = text z `dlerror`). Bez zavaděče beze změny (`dlopen` = chyba). `spawn::Options::static_tls_reserve` -> auxv `AT_MO_TLS_RESERVE`. `spawn::Options::reserve` (T5): rezervace v potomkovi jako **první** krok (před programem, zavaděčem a zásobníkem), tabulka v bloku auxv (`AT_MO_RESERVE`), program ji čte `loader::reserved()`; statický program = `NOT_SUPPORTED`.
- Zásobníky (hlavní ve `spawn` i vláken) vždy přes `vm::map_stack` s ochrannou oblastí `vm::STACK_GUARD` pod sebou; uvolnit se musí celá rezervace (`join`). Jediná výjimka: `thread::spawn_on_stack` (C `pthread_attr_setstack`) - zásobník i ochranná oblast patří volajícímu, `JoinHandle` má `stack_len` 0 a `join` nic neodmapuje.
- Spouštění sdílí kód jen programů z namapovaného initfs (`initfs::offset_of`, neměnný obsah); obraz odjinud se nahrává bez sdílení, aby záměna obrazu nepodstrčila cizí kód.
- `Vmo::create_resizable`/`set_size`/`size` (`VMO_RESIZABLE`, `VMO_SET_SIZE`, `VMO_SIZE`); `exception::signal_class_of(&Report)` = `Bus` pro výpadek za koncem zvětšitelného objektu (`reason::BEYOND_END`), knihovny mapují signály přes ni, ne přes `signal_class(kind)`.
- `Waiter` (`waiter.rs`): port + jednorázové registrace s jedinečnými klíči (nikdy znovu, neznámý klíč = ignorovat). Služba s proměnným počtem klientů čeká přes něj, ne přes `wait_many` (limit 64; `docs/cs/libos.md`).
- `WaitSet` (`waitset.rs`): náhrada `wait_many` se stejným rozhraním nad portem; objekt = hodnota handlu (s generací), registrace drží mezi voláními, chybějící objekt zapomene. Vlastní klíče mají nejvyšší bit, cizí registrace na sdíleném portu (`port()`) musí mít klíč `< FOREIGN_KEYS`. Port do pole nepatří (jádro `NOT_SUPPORTED`).
- Žádné závislosti na službách (`fsd`, `netd`): klienti jsou v `svcclient` a `*proto`.

## Testy

- Většina chování se ověřuje v QEMU testy userspace (`user/init/src/tests/`), např. `cargo xtask test --only zaklad,ipc,procesy,pamet`.
- Čisté části: `cargo test -p libos` (jen to, co se na hostiteli přeloží).

## Pasti

- Změna formátu startovní zprávy = `libs/abi/src/bootstrap.rs` + jádro (`kernel/src/userboot.rs`) + všechny služby, které ji čtou.
- Nový syscall bez obálky zde je pro aplikace neexistující.
