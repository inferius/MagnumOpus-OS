# Ovladač edu

Ukázkový ovladač pro výukové zařízení QEMU `edu` (PCI `1234:11e8`,
`-device edu`). Je celý na sadě driverkit a je to první ovladač
v [příručce](ovladace.md#první-ovladač-krok-za-krokem): registry v BAR0,
výpočet faktoriálu s přerušením MSI a přenos DMA tam a zpět.

- kód: `user/edu/src/lib.rs` (generický ovladač), `src/main.rs` (start
  nad jádrem), `src/model.rs` (model zařízení pro simulaci),
  `src/tests.rs`,
- manifest: `user/edu/edu.driver.toml` (BAR0, 1 přerušení, 16 KiB DMA),
- fakta o zařízení: QEMU `docs/specs/edu.rst`; vlastní kód (MIT OR
  Apache-2.0), z QEMU (GPL) nic převzaté není.

## Zařízení

| Registr | Offset | Význam |
|---|---|---|
| `ID` | `0x00` | `0xRRrr00ed`: hlavní a vedlejší verze |
| `LIVENESS` | `0x04` | čte se zapsaná hodnota s převrácenými bity |
| `FACTORIAL` | `0x08` | zápis spustí výpočet, čtení vrátí výsledek (32 bitů) |
| `STATUS` | `0x20` | bit 0 výpočet běží, bit 7 přerušení po výpočtu |
| `IRQ_STATUS` | `0x24` | důvod přerušení: bit 0 faktoriál, bit 8 DMA |
| `IRQ_ACK` | `0x64` | zápis smaže zapsané bity `IRQ_STATUS` |
| `DMA_SRC`, `DMA_DST`, `DMA_COUNT` | `0x80`, `0x88`, `0x90` | adresy a délka přenosu |
| `DMA_CMD` | `0x98` | bit 0 start, bit 1 směr do paměti, bit 2 přerušení |

Vnitřní buffer zařízení pro DMA je na adrese `0x40000` (z pohledu
zařízení), 4 KiB. Přenos trvá v QEMU asi 100 ms. xtask přidává zařízení
s `dma_mask=0xffffffffffffffff` (výchozí maska je 28 bitů a adresy DMA nad
256 MiB by zařízení ořízlo). Rozběhnutý přenos edu nejde zrušit; po konci
ovladače ho zastaví devmgr vypnutím bus master.

## Protokol

| Zpráva | Odpověď |
|---|---|
| `fact <n>` | `fact <n> <n!>` (32 bitů, přeteče jako hardware) |
| `dma <text>` | `dma <text>`: text projde DMA do zařízení a zpět (1-4096 B) |
| jiný příkaz | `chyba <popis>` |
| `dk:ping`, `dk:info`, `dk:restart`, `dk:crash` | společné zprávy driverkitu |

Během výpočtu nebo přenosu odpoví ovladač na další příkaz
`chyba zarizeni je zaneprazdnene`.

## Testy

- v simulaci (`cargo test -p edu`): faktoriál s přerušením, DMA tam
  a zpět bez přístupu mimo přidělenou paměť, chybné příkazy, kvóta DMA
  z manifestu,
- v QEMU (`cargo xtask test --driver edu`, sada `ovladace z manifestu`):
  ping, faktoriál přes MSI, 1000 B přes DMA a IOMMU, obnova po `dk:crash`
  a restart po `dk:restart`. Na x86_64 (VT-d) i aarch64 (SMMUv3).
