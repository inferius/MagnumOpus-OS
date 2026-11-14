# Grafika NVIDIA a AMD

**Stav: [navrženo]** (2026-11-14). Nic z této stránky není implementované.
Stránka je průzkum a návrh: kterou převzatou cestou jít, co od ní dostaneme
a **co je potřeba změnit v základech systému už teď**, aby se ovladač
později nemusel roubovat (sekce [Vlivy na základy](#vlivy-na-základy)).

Pořadí: **NVIDIA první** (k testování jsou karty NVIDIA), AMD jako druhý
ovladač ve stejné vrstvě. Intel má vlastní ovladač v Rustu
([Nastavení obrazu](displej.md)).

## Shrnutí

| Otázka | Doporučení |
|---|---|
| Ovladač NVIDIA | **nouveau** z Linuxu (licence MIT) beze změny logiky, v C, jako izolovaný proces |
| Generace NVIDIA | Kepler až Blackwell. Turing a novější přes firmware GSP-RM (plné takty, Vulkan); Maxwell 2 až Volta jen obraz a pomalá akcelerace (karta zůstane na startovních taktech) |
| Vulkan NVIDIA | **NVK** z Mesy (MIT), stejné jádro nouveau |
| Ovladač AMD | **amdgpu** + display core (MIT), stejná vrstva, druhý krok |
| Vrstva kompatibility | vlastní „LinuxKPI“ v Rustu nad driverkitem; vzor FreeBSD drm-kmod (BSD) |
| Bez IOMMU | nativní ovladač GPU **nespouštět**, zůstat na framebufferu firmwaru (návrh, otázka B) |
| Základy už teď | vlastní objekt plotu (fence), sdílené buffery se životním cyklem, velká DMA paměť po kusech, nahrávání firmwaru, třída „render“ pro Vulkan |

## Architektura

<figure class="diagram">
<svg viewBox="0 0 780 420" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Architektura ovladace GPU">
<defs><marker id="gn1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="230" height="74" rx="6"/><text class="t b" x="125" y="34" text-anchor="middle">aplikace (Vulkan)</text><text class="m" x="125" y="53" text-anchor="middle">Mesa NVK / RADV v procesu</text><text class="m" x="125" y="69" text-anchor="middle">aplikace, „libdrm“ nad kanálem</text>
<rect class="acc" x="275" y="10" width="230" height="74" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">služba render (gpud)</text><text class="m" x="390" y="53" text-anchor="middle">povolení, kontexty, kvóty,</text><text class="m" x="390" y="69" text-anchor="middle">přidělí render uzel</text>
<rect class="acc" x="540" y="10" width="230" height="74" rx="6"/><text class="t b" x="655" y="34" text-anchor="middle">compd</text><text class="m" x="655" y="53" text-anchor="middle">třída displej</text><text class="m" x="655" y="69" text-anchor="middle">(protokol jako intel-display)</text>
<path class="ln" d="M240 47 L273 47" marker-end="url(#gn1)"/>
<path class="ln dash" d="M125 84 L125 150 L218 150" marker-end="url(#gn1)"/><text class="m" x="132" y="120">render uzel (kanál)</text>
<path class="ln" d="M390 84 L390 128" marker-end="url(#gn1)"/>
<path class="ln" d="M655 84 L655 128" marker-end="url(#gn1)"/>
<rect class="ok" x="220" y="130" width="550" height="200" rx="6"/><text class="t b" x="495" y="152" text-anchor="middle">proces gpu-nvidia (izolovaný, omezený Resource, IOMMU)</text>
<rect class="box" x="235" y="165" width="250" height="70" rx="4"/><text class="t" x="360" y="187" text-anchor="middle">nouveau + DRM core (C, MIT)</text><text class="m" x="360" y="205" text-anchor="middle">KMS, GEM, TTM, plánovač,</text><text class="m" x="360" y="221" text-anchor="middle">GSP-RM, NVK uAPI (VM_BIND)</text>
<rect class="box" x="505" y="165" width="250" height="70" rx="4"/><text class="t" x="630" y="187" text-anchor="middle">most tříd (Rust)</text><text class="m" x="630" y="205" text-anchor="middle">displej = KMS atomic,</text><text class="m" x="630" y="221" text-anchor="middle">render = ioctl DRM přes kanál</text>
<rect class="warn" x="235" y="250" width="520" height="66" rx="4"/><text class="t" x="495" y="272" text-anchor="middle">LinuxKPI (Rust, naše): paměť, DMA, IOMMU, přerušení, zámky, workqueue,</text><text class="m" x="495" y="290" text-anchor="middle">časovače, firmware, PCI, i2c; dma-fence a dma-buf jako naše objekty plotů a bufferů</text><text class="m" x="495" y="306" text-anchor="middle">nad driverkitem a libos (vzor: FreeBSD linuxkpi)</text>
<path class="ln" d="M495 330 L495 358" marker-end="url(#gn1)"/>
<rect class="box" x="10" y="360" width="760" height="50" rx="6"/><text class="t b" x="390" y="382" text-anchor="middle">jádro</text><text class="m" x="390" y="400" text-anchor="middle">Resource, BAR (WC), MSI-X, DMA po kusech, doména IOMMU, objekty plotů a bufferů, firmware z FS</text>
<rect class="box" x="10" y="190" width="190" height="120" rx="6"/><text class="t b" x="105" y="212" text-anchor="middle">firmware</text><text class="m" x="105" y="232" text-anchor="middle">GSP-RM (Turing+),</text><text class="m" x="105" y="248" text-anchor="middle">PMU/SEC2/ACR,</text><text class="m" x="105" y="264" text-anchor="middle">AMD: PSP, SMU,</text><text class="m" x="105" y="280" text-anchor="middle">DMCUB, VCN</text><text class="m" x="105" y="296" text-anchor="middle">služba firmwaru</text>
<path class="ln" d="M200 250 L233 250" marker-end="url(#gn1)"/>
</svg>
<figcaption>Aplikace nikdy nedostane capability k ovladači: render uzel jí přidělí služba render, obraz jde přes třídu displej. Převzaté C běží v jednom procesu nad naší vrstvou kompatibility.</figcaption>
</figure>

- **Jeden proces na GPU** (`gpu-nvidia`, později `gpu-amd`): převzatý kód
  DRM (jádro DRM, TTM, plánovač, nouveau nebo amdgpu) v C beze změny logiky,
  pod ním naše vrstva kompatibility, nad ním most do tříd zařízení.
- **Třída displej**: stejný protokol jako `intel-display` a `bochs-display`
  (konektory, režimy, flip ve vsync, hotplug). Most ho překládá na atomický
  commit KMS. Kompozitor tak nepozná, jaký ovladač pod ním je.
- **Třída render** (nová): Vulkan v aplikaci (Mesa NVK/RADV) potřebuje
  ioctl DRM (alokace bufferů, `VM_BIND`, `EXEC`, ploty). Posílá je jako
  zprávy po kanálu „render uzlu“. Kanál přidělí **služba render**
  (povolení z manifestu, kontext na aplikaci, kvóty paměti GPU), ne
  aplikace sama ([Rozhraní zařízení](rozhrani-zarizeni.md)).
- **Pád**: GPU se po nastavení režimu nedá vrátit na framebuffer firmwaru
  (GOP už nezobrazuje). Obnova = reset funkce PCI (FLR, nebo reset
  sběrnice) a nová instance; mezitím se snímek kompozitoru neukazuje.
  Viz [Rizika](#rizika).

## NVIDIA: která cesta pokryje které generace

Ověřeno ve zdrojích Linuxu a Mesy (2026-11-14):

| Cesta | Licence | Generace | Obraz (KMS) | Akcelerace | Pro nás |
|---|---|---|---|---|---|
| **nouveau** (Linux) | MIT text v hlavičkách (`nouveau_drm.c`, `nvkm/...`, `gsp/...`), modul „GPL and additional rights“ | NV04 až Ada, tabulky čipů i Blackwell (GB100/GB102, GB202 až GB207) | ano, všechny | Kepler+; od Turingu přes GSP-RM, výchozí (`NvGspRm` = true) | **doporučeno** |
| **NVK** (Mesa) | MIT (jádro Mesy) | Kepler až Ada, spotřebitelské Blackwell (RTX 5000) | – | Vulkan 1.4 konformní; potřebuje nouveau s `VM_BIND`/`EXEC` | **Vulkan** |
| **open-gpu-kernel-modules** (NVIDIA) | soubory MIT, jako modul MIT/GPLv2 | Turing+ (jen s GSP firmwarem stejného vydání) | `nvidia-modeset` | jen s uzavřeným userspace NVIDIA | jen fakta, případně jen displej |
| **nova** (Linux, Rust) | GPL-2.0 | GSP (Turing+) | zatím ne (seznam úkolů displej nemá) | rozpracované (GSP, FIFO, GR) | **jen fakta** |

Z toho:

1. **Turing (RTX 20, GTX 16) až Blackwell (RTX 50)**: nouveau + firmware
   GSP-RM (verze 570.144 v nouveau). GSP sám řídí takty a napájení, takže
   karta jede plnou rychlostí. Vulkan přes NVK. **Hlavní cíl.**
2. **Maxwell 2 (GTX 900), Pascal (GTX 10), Volta**: obraz ano, akcelerace
   ano, ale takty nejdou zvýšit (firmware pro řízení napájení musí být
   podepsaný NVIDIA). Na plochu stačí, na hry ne.
3. **Kepler (GTX 600/700), Maxwell 1 (GTX 750)**: obraz, akcelerace,
   ruční přetaktování, Vulkan NVK. Nízká priorita.
4. Starší než Kepler: jen obraz, nepodporovat.

`open-gpu-kernel-modules` dává lepší podporu nových funkcí, ale bez
uzavřeného userspace NVIDIA nemá Vulkan ani OpenGL. Jejich zobrazovací
část (`nvidia-modeset`) je pod MIT a mohla by sloužit jako záloha pro
Turing+, kdyby nouveau u konkrétní karty selhal - za cenu druhé obří
kódové základny. Nedoporučuji.

**Co potřebujeme od uživatele**: přesný seznam karet NVIDIA (model a ID
zařízení, ve Windows Správce zařízení → Podrobnosti → ID hardwaru,
`PCI\VEN_10DE&DEV_xxxx`), a u každé, zda je v desktopu, nebo v notebooku
s Optimus (integrovaná Intel + NVIDIA). Podle ID se karta zařadí do
generace výše.

## AMD (druhý krok)

| Část | Licence (ověřeno v hlavičkách) |
|---|---|
| `amdgpu` (`amdgpu_device.c`) | MIT text |
| display core (`dc/core/dc.c`, `amdgpu_dm.c`) | MIT text, `amdgpu_dm.c` SPDX MIT |
| řízení napájení (`pm/swsmu/amdgpu_smu.c`) | MIT text |

amdgpu jde do stejné vrstvy kompatibility jako nouveau (stejné jádro DRM,
TTM, plánovač). Navíc potřebuje firmware pro téměř vše (PSP, SMU, DMCUB pro
obraz, VCN pro video, mikrokód grafiky a SDMA). Vulkan: RADV z Mesy.
Rozsah generací (GCN 1.2+ a RDNA výchozí, starší experimentálně) a
licence firmwaru ověřit při startu práce na AMD (zdroje firmwaru byly při
průzkumu nedostupné).

## Licence a co nepřevzít

Převzatý kód DRM je až na výjimky pod MIT nebo duálně MIT/GPL. Ověřené
hlavičky:

| Soubor | Licence | Postup |
|---|---|---|
| `drm_gem.c`, `drm_atomic.c`, `drm_syncobj.c`, `scheduler/sched_main.c` | MIT text | převzít |
| `ttm/ttm_bo.c`, `drm_exec.c` | GPL-2.0 OR MIT | převzít pod MIT |
| `drm_gpuvm.c` | GPL-2.0-only OR MIT | převzít pod MIT |
| `dma-buf/dma-resv.c` | MIT | převzít |
| **`dma-buf/dma-fence.c`** | **GPL-2.0-only** | **nepřevzít**: API `dma_fence_*` implementovat ve vrstvě kompatibility nad naším objektem plotu |
| `include/linux/*.h` (hlavičky jádra) | převážně GPL | nepřevzít: hlavičky LinuxKPI vlastní (vzor FreeBSD, BSD-2) |

Pravidlo: **před převzetím každého souboru ověřit licenci** a GPL-only
soubory nahradit vlastní implementací (jen podle API a dokumentace). Seznam
se udržuje automaticky skriptem importu (hlavička SPDX / text MIT).

## Vrstva kompatibility (LinuxKPI)

Ovladač DRM čeká služby jádra Linuxu. Vrstva je naše (Rust + tenké C
hlavičky), vzor je FreeBSD `sys/compat/linuxkpi` (BSD), ze kterého jde
převzít i kód, kde se hodí.

| Skupina | Co ovladač volá | Na co se mapuje u nás |
|---|---|---|
| paměť | `kmalloc`, `vmalloc`, `alloc_pages`, `kref`, `idr`/`xarray`, seznamy, rb-strom | alokátor procesu (`cstd`), stránky VMO |
| DMA | `dma_alloc_coherent`, `dma_map_sg`, `dma_map_page`, masky | DMA po kusech + doména IOMMU zařízení |
| MMIO | `ioremap`, `ioremap_wc`, `readl`/`writel` | `Platform::map_bar`, typ cache WC |
| přerušení | `request_irq`, MSI/MSI-X, `tasklet` | `Platform::enable_irq`, vlákno přerušení |
| souběh | spinlock, mutex, rwsem, RCU, atomiky, `completion`, `wait_queue` | zámky `cstd`, futex/událost |
| práce | `workqueue`, `kthread`, `timer`, `hrtimer`, `jiffies`, `msleep` | vlákna procesu, časovače libos |
| PCI | konfigurace, BARy, `pci_enable_device`, napájení D0/D3 | stránka konfigurace + služba napájení |
| firmware | `request_firmware` | **služba firmwaru** |
| i2c/DDC | `i2c_adapter`, `i2c_transfer` | v ovladači (nouveau/amdgpu mají vlastní) |
| synchronizace | `dma_fence`, `sync_file`, `dma_resv`, `dma_buf` | **naše objekty plotů a bufferů** |
| ladění | `printk`, `dev_*`, `WARN_ON`, debugfs | log driverkitu; debugfs ne |

Odhad rozsahu (neověřeno měřením): vrstva 15-30 tisíc řádků (FreeBSD
linuxkpi je v tomto řádu), převzaté jádro DRM + TTM + plánovač desítky
tisíc řádků, nouveau stovky tisíc, amdgpu miliony (převážně generované
hlavičky registrů).

**Aktualizace z upstreamu**: držet se jedné LTS verze Linuxu (jako
drm-kmod), import skriptem (`xtask`): stáhnout tag, zkontrolovat licence,
použít naši sadu záplat (záplaty jen ve vrstvě a v sestavení, ne v logice
ovladače), přeložit, host testy vrstvy, test na HW. Přechod na novou LTS
jednou ročně.

## Etapy

| Etapa | Obsah | Předpoklady |
|---|---|---|
| (a) obraz | třída displej přes KMS: převzetí od GOP, režimy z EDID, flip, hotplug, více monitorů | vrstva (bez plánovače a GEM pro aplikace), firmware (Turing+: GSP), IOMMU |
| (b) 2D/kopírování | buffery v paměti GPU pro kompozitor, kopírovací engine, nulová kopie snímku | sdílené buffery, ploty |
| (c) Vulkan | NVK/RADV v aplikacích, třída render, služba render, kvóty | port Mesy (vlákno jazyků/libc), render uzel |
| (d) napájení a teploty | runtime PM (D3cold, Optimus), teploty a ventilátory do služby napájení | ACPI `_DSM`/`_PR3` přes acpid |

Etapa (a) dá obraz na všech kartách od Kepleru (pro Turing+ s GSP).
Akcelerace kompozitoru (b) je přínos i bez Vulkanu.

## IOMMU

GPU je nejsilnější DMA zařízení v počítači a ovladač má stovky tisíc
řádků převzatého C plus firmware (GSP je vlastní procesor s přístupem
k paměti). **Bez IOMMU může chyba v ovladači nebo firmwaru přepsat
libovolnou paměť včetně jádra.**

Návrh:

- s IOMMU: zařízení má vlastní doménu (už dnes), DMA jen do paměti, kterou
  mu ovladač namapuje; po pádu devmgr vypne bus master a doménu zruší;
- **bez IOMMU nativní ovladač GPU nespouštět**: obraz zůstane na
  framebufferu firmwaru (`fbdisplay`) a uživatel dostane hlášení, proč;
  volitelně ruční povolení s varováním (otázka B);
- remapování přerušení (dnes vypnuté) zapnout dřív, než poběží GPU
  (zařízení s DMA by jinak mohlo podvrhnout MSI).

## Testování

QEMU grafiku NVIDIA ani AMD neemuluje. Možnosti:

1. **Skutečný stroj** (hlavní): obraz na USB flashce, log po sériové lince
   nebo do souboru ([Nastavení obrazu](displej.md#test-na-skutečném-stroji)).
2. **Předání karty do VM (VFIO)**: vyžaduje Linux jako hostitele s IOMMU;
   na Windows (Hyper-V DDA) jen ve Windows Server. Varianta: druhý stroj
   s Linuxem a QEMU + VFIO; urychlí cyklus (bez restartu stroje).
3. **Simulace v driverkitu**: testy vrstvy kompatibility na hostiteli
   (zámky, workqueue, DMA, ploty) a přehrávání záznamu MMIO z reálného
   běhu (záznam z našeho ovladače, ne z cizího kódu).

Hardware: na Turing+ (GSP, plné takty, Vulkan) postavit etapy (a)-(c);
jednu kartu Maxwell 2/Pascal pro ověření cesty bez GSP. Pro AMD později
jedna karta RDNA 2 nebo novější.

## Vlivy na základy

Co je dobré změnit **už teď** (nezávisle na samotném ovladači), aby GPU
nevyžadovalo zásahy do hotových částí. Rozsah: S = dny, M = 1-2 týdny,
L = víc.

| # | Oblast | Změna | Proč teď | Rozsah |
|---|---|---|---|---|
| 1 | **objekt plotu (fence)** | objekt jádra: stav (nesplněn, splněn, chyba), časová osa (timeline, 64bit hodnota), čekání ve `wait_many`, předání handlem, signalizuje jen držitel práva `SIGNAL` | `dma-fence.c` je GPL - musíme mít vlastní; zároveň základ pro Vulkan semafory, `sync_file`, `drm_syncobj` a explicitní synchronizaci v protokolu displeje; dnes ho nemá ani Venus (ploty jen uvnitř ovladače) | M |
| 2 | **sdílený buffer** | objekt bufferu nad VMO: velikost, umístění (systém / paměť zařízení), typ cache, formát + modifikátor (dlaždice, komprese), krok řádku, kdo smí zapisovat; předání handlem mezi aplikací, kompozitorem a ovladači; odvolání při pádu | ekvivalent `dma-buf`; rozhraní zařízení už vyžaduje „celý životní cyklus bufferu“ (bod 5) - teď ho určit jednou pro displej, Venus i budoucí GPU | M |
| 3 | **třída displej v2** | volitelné: buffer klienta jako snímek bez kopie (s plotem získání a uvolnění), formáty a modifikátory, více rovin (kurzor, překryv), atomický commit (režim + buffery najednou), úplné časování v `SET_MODE` | dnešní protokol kopíruje stín; GPU ovladač i Intel mohou zobrazit buffer přímo - přidat teď jako schopnosti, ať se nemění zprávy | S-M |
| 4 | **DMA po kusech a IOMMU** | alokace velké připnuté paměti po stránkách (scatter-gather, ne jen souvislé 4 MiB), mapování rozsahu VMO do domény zařízení na IOVA zvolenou ovladačem, odmapování se synchronním vyprázdněním TLB IOMMU, kvóty DMA v GB | GPU mapuje stovky MB až GB (TTM, GTT); dnešní `dma_memory` je souvislá a malá | M |
| 5 | **remapování přerušení** | zapnout v jádře (VT-d IR, AMD-Vi, GICv3 ITS už je) | bez něj může DMA zařízení podvrhnout MSI | M |
| 6 | **služba firmwaru** | `request_firmware` jako služba: soubory z `/system/firmware`, kontrolní součty, metadata licence, předání VMO jen pro čtení; obraz a instalátor firmware přibalí | GSP-RM (desítky MB), AMD, Wi-Fi, Realtek NIC (`rtl_fw`), budoucí zvuk; dnes nic takového není | S-M |
| 7 | **paměť GPU a rozpočty** | rozpočet jobu i pro paměť zařízení (VRAM, GTT); objekty, které ovladač smí vystěhovat (TTM) a znovu připnout; mapování VRAM přes BAR jako WC (už je typ cache) | kvóty Vulkanu na aplikaci; spravedlnost mezi aplikacemi | M |
| 8 | **třída render a služba render** | protokol render uzlu obecně (z Venusu: kontext, buffer, `VM_BIND`, submit s ploty, dotazy) + služba, která render uzel přidělí podle manifestu a hlídá kvóty | aplikace nesmí dostat capability k ovladači; Venus dnes dává render uzel přes init - sjednotit dřív, než přibude další ovladač | M |
| 9 | **běhové prostředí převzatého C** | vlákna v procesu ovladače (kthread, workqueue), přesné časovače, čekání s timeoutem; `cstd` s vlákny | jádro DRM i nouveau mají vlastní vlákna a časovače; stejné potřebuje Wi-Fi | M |
| 10 | **napájení zařízení** | protokol stavů napájení zařízení (D0/D3hot/D3cold) přes službu napájení; volání ACPI `_DSM`/`_PRx` z ovladače přes acpid | notebooky s Optimus vypínají dGPU; jinak baterie | S-M |
| 11 | **obnova GPU po pádu** | devmgr: reset funkce PCI (FLR / reset sběrnice) před novou instancí; kompozitor bez displeje vydrží (už umí) | po nastavení režimu se nelze vrátit na GOP; obnova jen resetem | S |

Body 1, 2, 4 a 8 doporučuji začít hned: dotýkají se jádra a protokolů,
které už používají displej a Venus, a později by se měnily na víc místech.

## Rizika

- **Rozsah**: stovky tisíc řádků převzatého C; aktualizace z upstreamu
  jsou trvalá práce (proto jedna LTS verze a záplaty jen ve vrstvě).
- **Firmware NVIDIA a AMD je uzavřený** (binární, redistribuce podle
  licence v linux-firmware - **neověřeno**, zdroj byl při průzkumu
  nedostupný). Bez něj na Turing+ nepojede nic; před přibalením do obrazu
  ověřit licenci.
- **Bez IOMMU** neúnosné riziko (viz výše).
- **Notebooky s Optimus**: obraz vede integrovaná Intel; NVIDIA jen
  počítá - vyžaduje sdílené buffery mezi ovladači (bod 2) a napájení (bod 10).
- **Testy jen na skutečném hardwaru**: pomalejší cyklus, nutná sériová
  linka nebo log na disk.
- **Pády firmwaru GSP**: obnova jen resetem zařízení, na chvíli bez obrazu.
- **Licence se mohou měnit** souborově: kontrola licencí při každém importu.

## Otázky

Viz zpráva koordinátorovi; po rozhodnutí se doplní sem.
