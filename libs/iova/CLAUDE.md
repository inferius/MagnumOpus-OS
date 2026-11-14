# libs/iova

Alokátor rozsahů adres pro zařízení (IOVA) v jedné doméně IOMMU (`no_std`, jen `alloc`). Používá ho jádro pro `DMA_MAP` (`kernel/src/object/dma.rs`); popis v `docs/cs/dma.md`.

## Invarianty

- Vše po stránkách (4 KiB); rozsah nikdy nepřekročí okno `[base, end)`.
- `alloc` nechá za každým obsazeným rozsahem volnou ochrannou stránku (i za nápovědou); `reserve` (nápověda ovladače) kontroluje jen překryv.
- Okno nad koncem RAM vybírá jádro, ne tahle knihovna (identická mapování souvislé paměti jsou pod ním).

## Testy

- `cargo test -p iova` (`src/tests.rs`, včetně náhodného modelu proti poli stránek).
