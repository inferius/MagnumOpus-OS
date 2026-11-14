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
- Plánovač: vlákno, které ještě běží (`on_cpu`), nesmí být v **cizí** frontě - jiné CPU by na něj čekalo se zakázanými přerušeními a dvě CPU by se mohla zablokovat navzájem. Přesun na jiné CPU a zařazení vlákna probuzeného během usínání dělá `finish_switch`; kradení taková vlákna přeskakuje (`sched/mod.rs`, `sched/balance.rs`).
- Synchronizace mezi CPU nesmí stát na vzoru „zapiš svůj příznak, přečti cizí“ (dvě adresy): QEMU TCG na x86 hostiteli nedodrží pořadí `stlr` -> `ldar` a probuzení se ztratí. Stav, o kterém rozhodují dvě strany, patří do jednoho atomického slova (CAS/swap), jako `Thread::cpu_flags`, nebo pod zámek.
- Selhaný test plánovače má vypsat stav (`sched::dump_state`: fronty, časovače, `on_cpu`) - bez něj se uváznutí nedá rozebrat.

## Testy

- Jádro nemá host testy (`test = false`). Logika, která nezávisí na HW, patří do knihoven (např. `libs/schedcore`, `libs/sizeclass`, `libs/csprng`) a testuje se `cargo test -p <crate>`.
- Přeložitelnost pro druhou architekturu: `cargo xtask check --arch aarch64` (po každé změně mimo `arch/<arch>/` povinná).
- Self-testy jádra a userspace v QEMU: `cargo xtask test` (feature `qemu-test`); cíleně `cargo xtask test --only <sada>`. Sady self-testů jádra (pamet, platforma, log, planovac, objekty, userspace) běží vždy před userspace.

## Pasti

- Změna jen pro x86_64, která projde lokálně, často rozbije aarch64 překlad - vždy `check`.
- Změna ABI (abi/syscall/ struktury ve zprávách) se musí promítnout do `libos`, docs cs+en a obou architektur.
- Komentáře v `.rs` česky bez diakritiky, jen ASCII.
