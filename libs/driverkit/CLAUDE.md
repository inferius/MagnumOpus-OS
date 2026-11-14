# libs/driverkit

Sada pro autory ovladačů (DDK, `no_std`). Dělá jednou a pořádně, co ovladač v userspace dělá pořád: typované registry MMIO (`regs`), DMA buffery s kontrolou mezí (`dma`), kruhy deskriptorů a dokončení (`ring`), abstrakce `Platform` (registry, DMA, přerušení, čas, kanál ke klientovi), životní cyklus `Driver` + `run` (probe, start, události, stop), manifest `<jmeno>.driver.toml` (`manifest`), čitelné chyby `DriverError`. Stejný ovladač běží nad jádrem (feature `os`, `src/os/`) i v simulaci na hostiteli (feature `sim`, `src/sim/`). Příručka: `docs/cs/ovladace.md`, ukázka `docs/cs/ovladac-edu.md`.

## Invarianty

- `dk:crash` (`Exit::Crash`) je skutečný pád: bez `stop` a bez uvolnění (runner se zapomene); zařízení zastaví a DMA paměť uklidí jádro (karanténa DMA, `docs/cs/jadro.md`).
- Každé číslo od zařízení je nedůvěryhodný vstup: kruhy odmítnou hlavu mimo kruh, ovladač vrací `DriverError::Device`, nepanikaří a nečte mimo buffer.
- Adresu pro zařízení dává jen `DmaBuffer::iova_of` s kontrolou mezí a souvislosti (`DmaMemory::device_run`); buffer po stránkách (`alloc_dma_pages`) bez IOMMU má víc úseků -> `device_chunks`. Zánik bufferu = odmapování (v simulaci `Region::gone`, kvóta po stránkách se vrací). Test, při kterém zařízení sahá na odmapovanou adresu, jen s `DmaBuffer::is_protected()` (bez IOMMU by zapsalo do uvolněné paměti). Zápis hodnoty, která se nevejde do pole registru, je chyba, ne tichý přepis sousedních bitů.
- Bez IOMMU může zařízení přes DMA zapsat kamkoli; driverkit chrání aspoň stranu ovladače. Tohle omezení řekni výslovně při návrhu ovladače.
- Po pádu ovladače devmgr vypne bus master a nový ovladač začíná resetem zařízení v `probe`.
- Aplikace k ovladači nesmí přímo; ovladač obsluhuje služba (`docs/cs/rozhrani-zarizeni.md`).

## Testy

- `cargo test -p driverkit --features sim` (modely zařízení v `src/sim/`, testy v `src/tests/`).
- Jeden ovladač: `cargo xtask test --driver <jmeno>` (host simulace + QEMU sada `ovladace z manifestu`). Nový ovladač: `cargo xtask new-driver`.

## Pasti

- Backend `os` se přeloží jen pro cíl OS; na hostiteli ho nezapínej.
- Změna `Platform` zasáhne `os` i `sim` backend a všechny ovladače; udržuj obě implementace shodné.
