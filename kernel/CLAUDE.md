# kernel

Mikrojádro (`no_std`, `no_main`): paměť, plánovač, objekty s handly a právy, IPC (kanály), syscally, přerušení. Žádné ovladače (ty jsou v userspace) a nic o Windows. Proč a jak: `docs/cs/jadro.md`, `docs/cs/architektura.md`, plánovač `docs/cs/planovac.md`, syscally `docs/cs/syscally.md`.

## Klíčové soubory

- `src/main.rs` - start na bootovacím CPU; vlákno `boot` spustí self-testy a proces `init`.
- `src/arch/api/` - smlouva architektury (traity); `src/arch/mod.rs` - fasáda `crate::arch`; `src/arch/{x86_64,aarch64}/` - implementace; `src/trap.rs` - jediná cesta arch -> jádro.
- `src/mm/` (buddy `phys.rs`, `heap.rs`, `address_space.rs`), `src/sched/` (`qos.rs`, `balance.rs`, `topo.rs`), `src/object/` (`handle.rs`, `channel.rs`, `vmo.rs`, `job.rs`, `process.rs`), `src/syscall/` (dispatch v `mod.rs`).
- `src/sync.rs` - `SpinLock` (po dobu držení zakáže přerušení na CPU).
- `src/tests/` - self-testy jádra: `SUITES` v `tests/mod.rs`, každá oblast má soubor s konstantou `TESTS`.
- `linker.ld` je společný pro obě architektury.

## Invarianty

- Zbytek jádra volá architekturu jen přes fasádu `crate::arch`; nikdy `arch::x86_64::...` ani `arch::aarch64::...`. Kód architektury volá do jádra jen přes `crate::trap` a obecné služby (`log`, `sync`, `error`, `fatal`, `mm`, `acpi::read`). Podrobné pravidlo: `docs/cs/architektura.md#rozhraní-architektury`.
- Obě architektury mají stejné rozložení adresního prostoru, rámec 4 KiB a per-CPU strukturu s ukazatelem na sebe na offsetu 0.
- Vše v `trap` se volá se zakázanými přerušeními; výjimka je `trap::syscall`.
- Žádný root: proces smí jen to, na co dostal handle s právem. Nová operace = kontrola práva handlu, ne identity.
- Syscally: čísla patří do `libs/abi/src/syscall.rs`, postup přidání je v kořenovém `CLAUDE.md`. Návratová hodnota je číslo nebo záporný kód `abi::Error`.
- Obsluha přerušení smí brát jen `SpinLock` (zakazuje přerušení), jinak hrozí deadlock.
- Konec procesu (`Process::finish`) hned zruší adresní prostor (`AddressSpace::teardown`), handly a futexy; zůstane náhrobek. Předpoklad: žádné CPU nemá prostor aktivní - `sched::exit` nastaví `user_gone` a přepne na kořen jádra **před** `thread_exited`, plánovač vláknu s `user_gone` kořen procesu nenahraje. Nová cesta ke konci vlákna musí jít přes `sched::exit`.
- Paměť pro DMA bez IOMMU (`MemoryObject::is_unprotected_dma`) se při konci procesu nevrací alokátoru (zařízení může dál zapisovat); drží ji náhrobek (`Process::parked`) do posledního handlu.
- Stránky objektů paměti vznikají na požádání (`docs/cs/sprava-pameti.md`): každý nový vznik stránky mimo výpadek v daném mapování musí volat `MemoryObject::forget_zero` (jinak někdo dál čte nulovou stránku místo dat). Jádro do paměti procesu nikdy nesahá přes adresy procesu - kopírování jde přes `copy_from_user`/`copy_to_user`, výpadek v jádře nesmí vzniknout.
- Slib (`phys::commit`) se strhává při vytvoření objektu; výpadek bere rámec ze slibu (`alloc_committed_zeroed`), takže nikdy nedojde paměť (žádný OOM kill). Nová alokace pro programy musí jít přes `alloc_user_pages`/`user_room` (respektují slib i rezervu).
- Připnutý rámec (`phys::pin`: DMA, `VMO_PHYS_ADDR`) se nesmí přesunout, nahradit ani vystěhovat; CoW ho nikdy nesdílí.
- Kopie při zápisu: do rámce s dalšími vlastníky (`phys::shares` > 0) nikdo nezapisuje. Každý zápis do stránky objektu jde přes `writable_page` (nebo `commit_page_fast` pod zámkem prostoru, který u sdíleného rámce vrátí `None`). `writable_page` a `forget_*` se nikdy nevolají se zámkem adresního prostoru ani s `cow_lock` (shootdown, zámky jiných prostorů).
- W^X: nikdy W+X. Spustit jde zapečetěná paměť (`MemoryObject::seal`), nezapečetěná jen s `Job::may_jit` cílového procesu (audit `Process::note_jit`). Nejvyšší práva mapování určí `VM_MAP` (`map_with_max`) podle práv handlu a politiky; zapečetěný objekt nikdy nedostane zápis. Pečeť se ověřuje pod zámkem prostoru a mapper se registruje před mapováním (souběh se `seal`).
- Adresní prostor = oblasti (`Mapping`: `vmo: None` = rezervace, přístup vždy chyba). Operace na části oblasti ji nejdřív rozdělí (`Inner::split`), každý kus má vlastní strhnutí; rezervace stojí jen záznam (ne tabulky stránek). `VM_PROTECT` nikdy nad `max_flags` (práva při `VM_MAP`) a nikdy W+X; přidání X přijde až s oprávněním JIT.
- Po `teardown` je prostor mrtvý: `map` -> `BadState`, ostatní operace nic nenajdou; `root()` vrací `None`.
- Plánovač: vlákno, které ještě běží (`on_cpu`), nesmí být v **cizí** frontě - jiné CPU by na něj čekalo se zakázanými přerušeními a dvě CPU by se mohla zablokovat navzájem. Přesun na jiné CPU a zařazení vlákna probuzeného během usínání dělá `finish_switch`; kradení taková vlákna přeskakuje (`sched/mod.rs`, `sched/balance.rs`).

## Testy

- Jádro nemá host testy (`test = false`). Logika, která nezávisí na HW, patří do knihoven (např. `libs/schedcore`, `libs/sizeclass`, `libs/csprng`) a testuje se `cargo test -p <crate>`.
- Přeložitelnost pro druhou architekturu: `cargo xtask check --arch aarch64` (po každé změně mimo `arch/<arch>/` povinná).
- Self-testy jádra a userspace v QEMU: `cargo xtask test` (feature `qemu-test`); cíleně `cargo xtask test --only <sada>`. Sady self-testů jádra (pamet, platforma, log, planovac, objekty, userspace) běží vždy před userspace.

## Pasti

- Změna jen pro x86_64, která projde lokálně, často rozbije aarch64 překlad - vždy `check`.
- Změna ABI (abi/syscall/ struktury ve zprávách) se musí promítnout do `libos`, docs cs+en a obou architektur.
- Komentáře v `.rs` česky bez diakritiky, jen ASCII.
