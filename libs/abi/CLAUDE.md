# libs/abi

Rozhraní jádro <-> userspace (`no_std`, žádná závislost na jádru): kódy chyb (`error.rs`), čísla syscallů (`syscall.rs`), práva a typy handlů (`object.rs`), příznaky mapování (`vm.rs`), joby a limity (`job.rs`, `limits.rs`), plánovač (`sched.rs`), čas (`time.rs`), formát startovní zprávy procesu (`bootstrap.rs`), parametry (`params.rs`), ACPI/napájení (`acpi.rs`), plot (`fence.rs`: stavový automat `Timeline`, `WaitItem`, `Info`), paměť jiného procesu (`process::memory`, práva `Rights::READ_MEMORY`/`WRITE_MEMORY`, syscall `PROCESS_MEMORY`), DMA po kusech (`dma.rs`: příznaky `DMA_MAP`, `Segment`, směr `DMA_SYNC`; rozpočet `job::pool::DMA`, volba `vm::VMO_PINNED`, právo `Rights::DMA`). Dokumentace: `docs/cs/syscally.md`, `docs/cs/chyby.md`, `docs/cs/jadro.md`.

## Invarianty

- Změna hodnoty je nekompatibilní změna rozhraní. Existující čísla syscallů ani kódy chyb se nepřečíslovávají ani nerecyklují.
- Stabilní pro aplikace je `libos`, ne čísla syscallů (ta se smí měnit spolu s libos). Kódy chyb jsou stabilní i pro aplikace (objevují se v logech a docs).
- Nový syscall: postup je v kořenovém `CLAUDE.md` (`COUNT` + `NAMES` v jádře + seznam v `tests.rs` + nadpis v `docs/cs/syscally.md` i `docs/en/syscalls.md` + obálka v libos).
- Nový kód chyby: další volné číslo v `error.rs` a popis v `docs/cs/chyby.md` i `docs/en/errors.md`.
- Struktury sdílené s jádrem jsou `#[repr(C)]` a stejné na x86_64 i aarch64.

## Testy

- `cargo test -p abi`. Test `every_syscall_is_documented` (`src/tests.rs`) čte `docs/` a selže, když chybí nadpis syscallu v cs nebo en; testy času v `time_tests.rs`, parametrů v `params_tests.rs`, plotu v `fence_tests.rs`, DMA v `dma_tests.rs`.

## Pasti

- Jméno syscallu na špatném indexu `NAMES` v `kernel/src/syscall/mod.rs`: pole má délku `COUNT` (chybějící prvek odhalí překlad), ale přehozené jméno se projeví až v logu.
- Konvence volání (`rax`, `rdi`, ...) v `syscall.rs` je popsaná pro x86_64; aarch64 mapování je v `kernel/src/arch/aarch64/`.
