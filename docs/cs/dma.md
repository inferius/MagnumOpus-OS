# DMA po kusech

Ovladač často potřebuje dát zařízení velký buffer: GPU stovky megabajtů
(GTT), síťová karta kruhy paketů, dekodér videa snímky aplikace. Souvislá
paměť pro DMA (`VMO_CREATE_CONTIGUOUS`) má nejvýš 4 MiB a po čase se
fragmentuje. **DMA po kusech** proto dává zařízení nesouvislé připnuté
stránky:

- s IOMMU je zařízení vidí jako **jeden souvislý rozsah IOVA**,
- bez IOMMU jako seznam fyzických úseků.

Je to krok 6a-2 základů pro grafiku a video
([gpu-amd-nvidia.md](gpu-amd-nvidia.md#vlivy-na-základy), bod 4).

Čísla a argumenty syscallů jsou v [syscally.md](syscally.md#dma-po-kusech),
IOMMU a karanténa DMA v [jadro.md](jadro.md#iommu-ochrana-dma) a API ovladačů
v [ovladace.md](ovladace.md).

## Stav

| Část | Co | Stav |
|---|---|---|
| připnutá paměť | `VMO_CREATE` s volbou `VMO_PINNED`: všechny stránky hned, připnuté, v rozpočtu jobu `DMA` (povinný strop kořene) | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `dma`, self-test jádra `DMA: pripnuta pamet ...`) |
| mapování | `DMA_MAP`, `DMA_SEGMENTS`, `DMA_SYNC`; právo `DMA` na handlu objektu paměti; alokátor IOVA (`libs/iova`) s ochrannou stránkou a nápovědou | **[ověřeno v QEMU]** x86_64 (VT-d) i aarch64 (SMMUv3) a bez IOMMU (sady `dma`, `ovladace z manifestu`: edu `sg`) |
| odmapování | zavření handlu = odmapování se synchronním vyprázdněním IOTLB; přístup zařízení na starou adresu zablokuje IOMMU a jádro ho zaloguje | **[ověřeno v QEMU]** x86_64 (VT-d) i aarch64 (SMMUv3) (test `edu: pristup po odmapovani zablokovan`) |
| karanténa | po konci ovladače nejdřív zastavit zařízení, pak odmapovat | **[implementováno]** (stejná cesta jako souvislá paměť; zvlášť netestováno) |
| driverkit | `Platform::alloc_dma_pages`, `DmaBuffer::device_chunks`, `DmaBuffer::is_protected` | **[ověřeno v QEMU]** (ovladač edu) |
| AMD-Vi | mapování na IOVA s právy směru | **[implementováno]** (QEMU `--iommu amd` netestováno) |
| zařízení s 32bitovou adresou | okno IOVA je nad 4 GiB | **[navrženo]** (dnes jen souvislá paměť) |

## Model

<figure>
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="DMA po kusech">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.bad{fill:#fbecec;stroke:#a04040;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
</style>
<defs><marker id="dm1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L10,5 L0,10 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/>
<text class="t b" x="110" y="44" text-anchor="middle">VMO_PINNED (3 stránky)</text>
<text class="m" x="110" y="62" text-anchor="middle">fyzicky: 0x7a000, 0x31000, 0x5c000</text>
<text class="m" x="110" y="78" text-anchor="middle">rozpočet USER + DMA jobu</text>
<path class="ln" d="M210 55 L278 55" marker-end="url(#dm1)"/>
<text class="m" x="244" y="46" text-anchor="middle">DMA_MAP</text>
<rect class="acc" x="280" y="10" width="220" height="90" rx="6"/>
<text class="t b" x="390" y="34" text-anchor="middle">doména zařízení (IOMMU)</text>
<text class="m" x="390" y="54" text-anchor="middle">IOVA 0x1_0000_0000 .. +3 stránky</text>
<text class="m" x="390" y="70" text-anchor="middle">okno nad koncem RAM</text>
<text class="m" x="390" y="86" text-anchor="middle">+ ochranná stránka za rozsahem</text>
<path class="ln" d="M500 55 L568 55" marker-end="url(#dm1)"/>
<rect class="box" x="570" y="20" width="180" height="70" rx="6"/>
<text class="t b" x="660" y="44" text-anchor="middle">DMA_SEGMENTS</text>
<text class="m" x="660" y="62" text-anchor="middle">[(0x1_0000_0000, 12 KiB)]</text>
<text class="m" x="660" y="78" text-anchor="middle">bez IOMMU: 3 fyzické úseky</text>
<rect class="box" x="10" y="150" width="200" height="60" rx="6"/>
<text class="t b" x="110" y="174" text-anchor="middle">zavření handlu</text>
<text class="m" x="110" y="192" text-anchor="middle">(nebo konec ovladače)</text>
<path class="ln" d="M210 180 L278 180" marker-end="url(#dm1)"/>
<rect class="acc" x="280" y="140" width="220" height="80" rx="6"/>
<text class="t b" x="390" y="164" text-anchor="middle">odmapování</text>
<text class="m" x="390" y="182" text-anchor="middle">záznamy pryč + IOTLB synchronně</text>
<text class="m" x="390" y="198" text-anchor="middle">pak IOVA zpět alokátoru,</text>
<text class="m" x="390" y="212" text-anchor="middle">pak odkaz na paměť</text>
<path class="ln" d="M500 180 L568 180" marker-end="url(#dm1)"/>
<rect class="bad" x="570" y="150" width="180" height="60" rx="6"/>
<text class="t b" x="660" y="174" text-anchor="middle">zařízení na starou IOVA</text>
<text class="m" x="660" y="192" text-anchor="middle">zablokováno, log jádra</text>
<text class="m" x="10" y="260">Mapovat smí jen ovladač s prostředkem zařízení; objekt paměti potřebuje právo DMA (a READ/WRITE podle směru).</text>
<text class="m" x="10" y="280">Po pádu ovladače jde mapování do karantény: jádro nejdřív vypne bus master, pak odmapuje.</text>
</svg>
<figcaption>Připnutá paměť, mapování do domény zařízení a odmapování.</figcaption>
</figure>

### Připnutá paměť

`VMO_CREATE` s volbou `abi::vm::VMO_PINNED` (`libos::Vmo::create_pinned`)
vytvoří objekt, jehož stránky vzniknou hned a jsou **připnuté** po celou
dobu života objektu. Připnutou stránku jádro nepřesune, nenahradí kopií
(CoW) ani nevystěhuje, protože do ní může zapisovat zařízení. Pravidla:

- Stránky se strhnou z rozpočtu **uživatelské paměti** a navíc
  z rozpočtu **připnuté paměti** `DMA` (`abi::job::pool::DMA` = 2).
- **Kvóta je povinná.** Kořenový job má strop poloviny RAM, kterou
  spravuje alokátor. `JOB_SET_LIMIT` ho jen sníží, nikdy nezvedne, takže
  připnutá paměť nikdy nezabere celý systém. Podřízené joby dědí limity
  předků; rodič jim může nastavit vlastní.
- Do rozpočtu `DMA` se počítá i souvislá paměť pro DMA
  (`VMO_CREATE_CONTIGUOUS`): i ta je připnutá.
- Připnutý objekt **nejde zapečetit** (`VMO_SEAL` = `NOT_SUPPORTED`):
  obsah může změnit zařízení.
- Klon (`VMO_CLONE`) je běžný objekt: připnuté stránky se do něj
  zkopírují, nesdílí se.

### Mapování (`DMA_MAP`)

`DMA_MAP(resource, vmo, offset, len, flags, iova_hint)` zpřístupní rozsah
připnutého objektu zařízení. Výsledek je handle mapování (`ObjectType::DmaMap`
= 13).

**Kontroly:**

- **Prostředek** musí mít právo `MANAGE` a identitu zařízení
  (`RESOURCE_CREATE` s `device`, od devmgr).
- **Objekt paměti** musí být připnutý (`VMO_PINNED` nebo souvislá paměť
  pro DMA). Jinak `BAD_STATE`; okno na registry (`VMO_CREATE_PHYSICAL`)
  také nejde.
- **Handle objektu** musí mít právo **`DMA`** (`1 << 12`). Výchozí handle
  ho má. Kdo posílá buffer ovladači (aplikace, kompozitor), rozhoduje
  ubráním práva, jestli k němu zařízení smí.
- **Směr** se kontroluje proti právům handlu: `DEVICE_READ` (zařízení čte)
  potřebuje `READ`, `DEVICE_WRITE` (zařízení zapisuje) potřebuje `WRITE`.
  S IOMMU směr vynucuje i hardware; SMMUv3 neumí „jen zápis“, zápis tam
  dovolí i čtení.
- **Kvóta prostředku** `dma_max` se strhne za délku mapování a vrátí se
  odmapováním. U souvislé paměti platí za celý život prostředku.
- **Paměť jádra** za záznam stránek se strhne jobu volajícího.

**Adresy pro zařízení:**

- **S IOMMU** dostane mapování jeden souvislý rozsah IOVA z **okna nad
  koncem RAM**: od RAM zaokrouhlené nahoru na 1 GiB, nejméně od 4 GiB, až po
  šířku překladu domény (VT-d 39 nebo 48 bitů, AMD-Vi 48, SMMUv3 47). Okno
  se tak nikdy nepřekryje s identickým mapováním souvislé paměti (tam je
  adresa pro zařízení fyzická).
  - Rozsah vybírá alokátor IOVA (`libs/iova`, first-fit) a za každým
    rozsahem nechá volnou **ochrannou stránku**. Zařízení, které přeteče
    konec bufferu, pak narazí na chybu překladu, ne na data sousedního
    bufferu.
  - Ovladač může adresu zvolit sám (`iova_hint`). Jádro ověří, že leží
    v okně a nepřekrývá se s jiným mapováním (`ALREADY_EXISTS`).
  - Všechny prostředky jednoho zařízení (i po restartu ovladače) sdílejí
    doménu a alokátor.
- **Bez IOMMU** jsou úseky fyzické adresy stránek (sousední se sloučí).
  Nápověda nejde (`NOT_SUPPORTED`) a směr nikdo nevynutí. Jádro to
  zaloguje jednou na prostředek (`DMA_MAP bez IOMMU: ...`, viz
  [hlasky.md](hlasky.md#jádro-iommu)). Ovladač, který bez ochrany nesmí
  pokračovat, pošle příznak `REQUIRE_IOMMU` a dostane `NOT_SUPPORTED`.

`DMA_SEGMENTS(map, out, max)` vrátí úseky `abi::dma::Segment { addr, len }`.
S IOMMU je úsek jeden.

### Odmapování

Mapování nemá vlastní syscall na odmapování: **zavření handlu** je
odmapování. Jádro postupuje v tomto pořadí:

1. smaže záznamy v doméně a **synchronně** vyprázdní IOTLB (VT-d čeká na
   dokončení invalidace, AMD-Vi na `COMPLETION_WAIT`, SMMUv3 na
   `CMD_SYNC`),
2. vrátí rozsah IOVA alokátoru,
3. pustí odkaz na objekt paměti. Stránky se uvolní, až zanikne i objekt.

Zařízení, které potom použije starou adresu, narazí na chybu překladu.
IOMMU přístup zablokuje a jádro ho zaloguje se zařízením a adresou
(`IOMMU: zablokovan zapis zarizeni ... na adrese 0x... (`, na SMMUv3
`SMMU: zablokovany pristup DMA ... na adresu 0x... (`).

SMMUv3 zatím nemá přerušení fronty událostí. Jádro ji vybírá při každé
invalidaci a při `DMA_SYNC` se směrem `FOR_CPU` (`arch::iommu::poll_faults`),
tedy nejpozději ve chvíli, kdy ovladač čte data po přenosu.

Mapování **nejde předat** jinému procesu (handle nemá `TRANSFER`). Patří
procesu ovladače, aby ho po jeho konci našla karanténa DMA: jádro nejdřív
vypne zařízení bus master a počká na přenosy, teprve potom mapování zruší
([jadro.md](jadro.md#iommu-ochrana-dma)).

### Údržba cache (`DMA_SYNC`)

`DMA_SYNC(map, offset, len, direction)` udržuje cache rozsahu mapování na
nekoherentních platformách (Raspberry Pi):

- `FOR_DEVICE` (0) před přenosem do zařízení vyčistí cache do paměti,
- `FOR_CPU` (1) po přenosu do paměti zahodí staré řádky.

Na x86_64 a QEMU virt (koherentní) nedělá nic kromě výběru hlášení chyb
IOMMU.

## Použití

V ovladači přes driverkit:

```rust
// 12 KiB po stránkách: s IOMMU souvislá IOVA, bez ní fyzické úseky
let sg = p.alloc_dma_pages("edu-sg", 3 * 4096)?;
for (addr, len) in sg.device_chunks(off, len)? {
    // jeden přenos (nebo položka seznamu úseků zařízení) na kus
}
p.dma_sync_for_cpu(&sg); // po přenosu do paměti
```

Přímo nad libos:

```rust
use libos::dma::{DmaMap, map, sync};
let buf = Vmo::create_pinned(1 << 20)?;
let m = DmaMap::new(&resource, &buf, 0, 1 << 20, map::DEVICE_WRITE)?;
let segs = m.segments()?; // [(iova, 1 MiB)] s IOMMU
m.sync(0, 1 << 20, sync::FOR_CPU)?;
drop(m); // odmapování + IOTLB
```

Buffer aplikace: aplikace vytvoří `Vmo::create_pinned` na svůj účet
(rozpočet `DMA` jejího jobu) a pošle službě handle s právem `DMA`. Služba ho
předá ovladači, který ho namapuje.

## Testy

| Test | Kde | Co ověřuje |
|---|---|---|
| `cargo test -p iova` | host | alokátor: first-fit, ochranná stránka, nápověda a překryv, konec okna, náhodný model |
| `cargo test -p abi` | host | kódování `Segment`, slučování fyzických stránek |
| `cargo test -p driverkit --features sim` | host | `device_chunks`, `DmaSplit`, zánik bufferu = chyba zařízení a vrácená kvóta |
| `cargo test -p edu` | host | `sg` tam a zpět, `stale` zachycený jako chyba DMA |
| `DMA: ...` | self-test jádra (`objekty`) | okno IOVA, rozpočet `DMA` a strop kořene, mapování bez IOMMU (úseky, kvóta, meze), s IOMMU (souvislá IOVA, ochranná stránka, nápověda, opakované použití) |
| sada `dma` | QEMU | připnutá paměť v rozpočtu, práva `DMA` a směru, meze, `REQUIRE_IOMMU`, mapování nejde předat |
| sada `ovladace z manifestu` | QEMU | edu `sg`: 3000 B přes hranice stránek oběma směry; edu `stale`: zápis zařízení po odmapování zablokovaný a v logu jádra (jen s IOMMU) |

## Omezení a rizika

- **Bez IOMMU** smí zařízení přes DMA kamkoli. Mapování jen předá
  fyzické adresy a jádro to zaloguje. Test zablokovaného přístupu se
  proto bez IOMMU nepouští: zařízení by zapsalo do uvolněné stránky.
- **Připnutá paměť** se nedá vystěhovat. Strop kořenového jobu (polovina
  RAM) chrání systém, ale ne služby mezi sebou. Rodič může podřízeným
  jobům nastavit vlastní limit `DMA`; init a launchd ho zatím nenastavují
  (aplikace a služby dědí strop kořene) **[navrženo]**: limit z manifestu.
- **Okno IOVA je nad 4 GiB.** Zařízení s 32bitovou adresou DMA (staré
  karty, `edu` s výchozí maskou) ho nevidí a potřebují souvislou paměť.
- **SMMUv3** hlásí zablokované přístupy až při invalidaci nebo `DMA_SYNC`
  (bez přerušení událostí).
- **Cykly čekání** mezi zařízením a pamětí jádro nehlídá: ovladač, který
  zavře mapování uprostřed přenosu, dostane od zařízení chybu (s IOMMU),
  nebo poškodí cizí paměť (bez IOMMU).
