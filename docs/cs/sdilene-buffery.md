# Sdílené buffery

Sdílený buffer (obraz okna, snímek videa, výstup GPU) je **VMO plus popis**
z knihovny `libs/bufdesc`. Nový objekt jádra to není. Rozhodnutí 6a je
v [gpu-amd-nvidia.md](gpu-amd-nvidia.md#vlivy-na-základy), bod 2.

## Stav

| Část | Co | Stav |
|---|---|---|
| `libs/bufdesc` | popis (fourcc, modifikátor, roviny, šířka, výška, umístění, cache), ověření proti VMO, kódování 72 B | **[implementováno]** host testy (`cargo test -p bufdesc`) |
| `renderproto` `IMPORT_BUFFER` | popis + VMO pro ovladač GPU | **[implementováno]** v protokolu; Venus `NOT_SUPPORTED` ([render.md](render.md)) |
| okenní protokol | `BUFFER_CREATE_DESC` (NV12, I420, BGRA), `SYNC_POINTS` s plotem získání a uvolnění, převod YUV v kompozitoru (`libs/yuv`, SIMD) | **[ověřeno v QEMU]** krok 6a-4 x86_64 i aarch64 ([gui.md](gui.md#buffery-s-popisem-a-explicitní-synchronizace-video)) |
| převod YUV na GPU | shader `spirv::yuv` ve `vkcomp`, bit po bitu jako `libs/yuv` | **[ověřeno v QEMU]** WSL (vk-probe); kompozitor ho zatím nepoužívá **[navrženo]** |

## Popis

| Pole | Co |
|---|---|
| `fourcc` | formát jako v DRM: `AR24`/`XR24` (B G R A), `AB24`/`XB24` (R G B A), `NV12`, `YU12` (I420), `P010` |
| `modifier` | rozložení jako v DRM: `LINEAR` (0), dlaždice ovladače, `INVALID` = chyba |
| `width`, `height` | 1 až 32768 |
| roviny | 1 až 4 × `{offset, pitch}` v pořadí formátu |
| umístění | systémová paměť / paměť zařízení (fyzická VMO, ovladač ji může odvolat `VMO_REVOKE`) |
| cache | `UNCACHED`, `WRITE_COMBINING`, `CACHED` (hodnoty `abi::vm::cache`) |

Hodnoty fourcc a modifikátorů jsou stejné jako v Linuxu, aby šly předat Mese
a ovladačům beze změny.

**Kdo smí co** neurčuje popis, ale práva handlu VMO:

- konzument dostane `READ | MAP`,
- zapisovatel navíc `WRITE`,
- zařízení jen s právem `DMA` ([dma.md](dma.md)).

**Synchronizace** je jen explicitní. Ploty získání a uvolnění jdou ve
zprávách protokolu ([ploty.md](ploty.md)). Implicitní `dma_resv` jako v Linuxu
tu není.

## Ověření (nedůvěryhodný vstup)

Popis přichází od jiného procesu. Příjemce proto volá
`BufDesc::validate(velikost VMO)` dřív, než z bufferu čte nebo ho předá
zařízení:

- **Známý formát s `LINEAR`:** počet rovin podle formátu a krok řádku
  aspoň řádek pixelů. Poslední řádek každé roviny musí končit ve VMO
  (`offset + pitch × (řádky - 1) + řádek`).
- **Dlaždicový modifikátor:** rozložení zná jen ovladač, proto se kontroluje
  jen hrubá mez `offset + pitch × výška` ve VMO.
- **Aritmetika** všude s kontrolou přetečení: nepřátelský offset
  `u64::MAX - 10` skončí chybou, nepřeteče.
- **Kódování** má jediný tvar (72 B): nepoužité roviny nulové, rezervní
  bajt 0. Dva různé zápisy téhož popisu by šly zneužít k obejití
  porovnání.

```rust
use bufdesc::{BufDesc, format};
// producent: NV12 1920x1080, řádky zarovnané na 64 B
let d = BufDesc::linear(format::NV12, 1920, 1080, 64)?;
let vmo = Vmo::create(d.min_size()?)?;
// příjemce
let d = BufDesc::decode(&msg)?;
d.validate(vmo.size()?)?;
```
