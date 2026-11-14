# Grafika NVIDIA a AMD

**Stav: [navrženo]** (2026-11-14). Nic z této stránky není implementované.
Stránka je průzkum a návrh: kterou převzatou cestou jít, co od ní dostaneme
a **co je potřeba změnit v základech systému už teď**, aby se ovladač
později nemusel roubovat (sekce [Vlivy na základy](#vlivy-na-základy)).

Pořadí: **NVIDIA první** (k testování jsou karty NVIDIA): **vlastní
ovladač v Rustu pro GSP** (rozhodnuto 2026-11-14). AMD a 3D na Intelu
později přes izolovanou vrstvu LinuxKPI ([strategie](#strategie-pro-amd-a-3d-na-intelu)).
Obraz na Intelu řeší vlastní ovladač ([Nastavení obrazu](displej.md)).

## Shrnutí

| Otázka | Rozhodnutí / doporučení |
|---|---|
| Ovladač NVIDIA | **rozhodnuto**: vlastní ovladač v Rustu jen pro GSP, pevná verze firmwaru 570.144, definice RPC generované z hlaviček nouveau ([srovnání](#srovnání-nouveau-v-c-nebo-vlastní-ovladač-v-rustu-pro-gsp)) |
| Licence ovladače NVIDIA | **rozhodnuto v návrhu**: GPL-2.0-only v části `gpl/` - start GSP, VBIOS, Falcon/FWSEC a fronty převzít z nova (Rust, GPL), zobrazení, kanály, MMU a uAPI pro NVK přeložit z nouveau (MIT) ([licence ovladače](#licence-ovladače-mit-nebo-gpl)) |
| Generace NVIDIA | **Turing až Blackwell** (GSP: plné takty, Vulkan). Pascal a starší zatím framebuffer firmwaru; port cesty bez GSP z nouveau jen na přání |
| Vulkan NVIDIA | **NVK** z Mesy (MIT) beze změny logiky; render uzel napodobí uAPI nouveau |
| Ovladač AMD | **amdgpu** + display core (MIT) přes LinuxKPI, druhý krok ([strategie](#strategie-pro-amd-a-3d-na-intelu)) |
| Vrstva kompatibility | vlastní „LinuxKPI“ v Rustu nad driverkitem, jen pro AMD a 3D na Intelu, izolovaná uvnitř procesu ovladače; vzor FreeBSD drm-kmod (BSD) |
| Bez IOMMU | **rozhodnuto**: spustit s varováním (jako disky a síť), karanténa DMA po pádu, rada zapnout VT-d / AMD-Vi |
| Základy | **rozhodnuto**: body 1, 2, 4 a 8 (plot, sdílený buffer, DMA po kusech, třída render) jako jaderná práce po dynamickém linkování a před FFmpeg ([architektura](architektura.md#převzetí-cizího-software-pořadí)) |
| Firmware | **rozhodnuto**: přibalit do obrazu; licence NVIDIA i AMD to dovolují (podmínky níže) |

## Architektura

<figure class="diagram">
<svg viewBox="0 0 780 420" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Architektura ovladace GPU">
<defs><marker id="gn1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="230" height="74" rx="6"/><text class="t b" x="125" y="34" text-anchor="middle">aplikace (Vulkan)</text><text class="m" x="125" y="53" text-anchor="middle">Mesa NVK v procesu aplikace,</text><text class="m" x="125" y="69" text-anchor="middle">winsys: uAPI nouveau = zprávy</text>
<rect class="acc" x="275" y="10" width="230" height="74" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">služba render (gpud)</text><text class="m" x="390" y="53" text-anchor="middle">povolení, kontexty, kvóty,</text><text class="m" x="390" y="69" text-anchor="middle">přidělí render uzel</text>
<rect class="acc" x="540" y="10" width="230" height="74" rx="6"/><text class="t b" x="655" y="34" text-anchor="middle">compd</text><text class="m" x="655" y="53" text-anchor="middle">třída displej</text><text class="m" x="655" y="69" text-anchor="middle">(protokol jako intel-display)</text>
<path class="ln" d="M240 47 L273 47" marker-end="url(#gn1)"/>
<path class="ln dash" d="M125 84 L125 150 L218 150" marker-end="url(#gn1)"/><text class="m" x="132" y="120">render uzel (kanál)</text>
<path class="ln" d="M390 84 L390 128" marker-end="url(#gn1)"/>
<path class="ln" d="M655 84 L655 128" marker-end="url(#gn1)"/>
<rect class="ok" x="220" y="130" width="550" height="200" rx="6"/><text class="t b" x="495" y="152" text-anchor="middle">proces gpu-nvidia (Rust, izolovaný, omezený Resource, IOMMU)</text>
<rect class="box" x="235" y="165" width="250" height="70" rx="4"/><text class="t" x="360" y="187" text-anchor="middle">jádro GSP (překlad z nouveau)</text><text class="m" x="360" y="205" text-anchor="middle">start GSP, RPC r570, objekty RM,</text><text class="m" x="360" y="221" text-anchor="middle">MMU, kanály, displej přes RM</text>
<rect class="box" x="505" y="165" width="250" height="70" rx="4"/><text class="t" x="630" y="187" text-anchor="middle">třídy zařízení</text><text class="m" x="630" y="205" text-anchor="middle">displej (jako intel-display),</text><text class="m" x="630" y="221" text-anchor="middle">render (uAPI nouveau, VM_BIND)</text>
<rect class="warn" x="235" y="250" width="520" height="66" rx="4"/><text class="t" x="495" y="272" text-anchor="middle">driverkit: BAR (WC), MSI-X, DMA po kusech, doména IOMMU,</text><text class="m" x="495" y="290" text-anchor="middle">ploty a sdílené buffery jádra, služba firmwaru</text><text class="m" x="495" y="306" text-anchor="middle">(LinuxKPI až pro AMD a 3D na Intelu, uvnitř jejich procesu)</text>
<path class="ln" d="M495 330 L495 358" marker-end="url(#gn1)"/>
<rect class="box" x="10" y="360" width="760" height="50" rx="6"/><text class="t b" x="390" y="382" text-anchor="middle">jádro</text><text class="m" x="390" y="400" text-anchor="middle">Resource, BAR (WC), MSI-X, DMA po kusech, doména IOMMU, ploty, buffery jako VMO + bufdesc, firmware z FS</text>
<rect class="box" x="10" y="190" width="190" height="120" rx="6"/><text class="t b" x="105" y="212" text-anchor="middle">firmware</text><text class="m" x="105" y="232" text-anchor="middle">GSP-RM 570.144,</text><text class="m" x="105" y="248" text-anchor="middle">booter, FWSEC (VBIOS),</text><text class="m" x="105" y="264" text-anchor="middle">AMD: PSP, SMU,</text><text class="m" x="105" y="280" text-anchor="middle">DMCUB, VCN</text><text class="m" x="105" y="296" text-anchor="middle">služba firmwaru</text>
<path class="ln" d="M200 250 L233 250" marker-end="url(#gn1)"/>
</svg>
<figcaption>Aplikace nikdy nedostane capability k ovladači: render uzel jí přidělí služba render, obraz jde přes třídu displej. Ovladač NVIDIA je v Rustu nad driverkitem; většinu práce dělá firmware GSP.</figcaption>
</figure>

- **Jeden proces na GPU**: `gpu-nvidia` v Rustu nad driverkitem (jádro
  GSP přeložené z nouveau). Později `gpu-amd` s převzatým C nad LinuxKPI
  ve vlastním procesu - ven ze všech jen naše rozhraní.
- **Třída displej**: stejný protokol jako `intel-display` a `bochs-display`
  (konektory, režimy, flip ve vsync, hotplug). Na Turing+ ho ovladač
  překládá na řídicí volání GSP-RM a kanál displeje. Kompozitor nepozná,
  jaký ovladač pod ním je.
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
| **nouveau** (Linux) | MIT text v hlavičkách (`nouveau_drm.c`, `nvkm/...`, `gsp/...`), modul „GPL and additional rights“ | NV04 až Ada, tabulky čipů i Blackwell (GB100/GB102, GB202 až GB207) | ano, všechny | Kepler+; od Turingu přes GSP-RM, výchozí (`NvGspRm` = true) | **zdroj překladu** (části pro GSP) |
| **NVK** (Mesa) | MIT (jádro Mesy) | Kepler až Ada, spotřebitelské Blackwell (RTX 5000) | – | Vulkan 1.4 konformní; potřebuje nouveau z Linuxu 6.6+ (uAPI `VM_BIND`/`EXEC`) | **Vulkan** |
| **open-gpu-kernel-modules** (NVIDIA) | soubory MIT, jako modul MIT/GPLv2 | Turing+ (jen s GSP firmwarem stejného vydání) | `nvidia-modeset` | jen s uzavřeným userspace NVIDIA | jen fakta, případně jen displej |
| **nova** (Linux, Rust) | GPL-2.0 | GSP: Turing až Blackwell (výčet TU102 až GB207) | ne (nova-drm má jen `INFO`, `GEM_CREATE`, `GEM_INFO`) | start GSP, fronta příkazů, VBIOS, Falcon/FWSEC, booter, MMU částečně; kanály ne | **zdroj převzetí** pro start GSP a fronty (ovladač pak GPL v `gpl/`) |

Z toho:

1. **Turing (RTX 20, GTX 16) až Blackwell (RTX 50)**: náš ovladač
   (překlad částí nouveau pro GSP) + firmware GSP-RM 570.144. GSP sám řídí
   takty a napájení, takže karta jede plnou rychlostí. Vulkan přes NVK.
   **Zvolený cíl.**
   **Blackwell** (GB202 až GB207, RTX 50) je v nouveau jen přes GSP: i
   zobrazování jde přes GSP-RM (`disp/gb202.c`: „GB20x is GSP-only“). Bez
   firmwaru GSP tedy ani obraz.
2. **Maxwell 2 (GTX 900), Pascal (GTX 10), Volta**: obraz ano, akcelerace
   ano přes firmware podepsaný NVIDIA, který se zavádí bezpečným startem
   ACR (`nvidia/gp108/acr`, `.../gr/*_sig.bin` v linux-firmware); NVK je
   pro tyto generace zapnutý od dubna 2025. Takty ale nejdou zvýšit
   (firmware pro řízení napájení NVIDIA nevydala), karta jede na
   startovních taktech. Na plochu stačí, na hry ne. **U nás zatím
   framebuffer firmwaru**; port této cesty z nouveau jen na přání.
3. **Kepler (GTX 600/700), Maxwell 1 (GTX 750)**: obraz, akcelerace,
   ruční přetaktování, Vulkan NVK. U nás framebuffer firmwaru.
4. **Fermi a starší** (např. GTX 560 Ti, GF114/GF110): nouveau umí obraz
   a 3D, ale řízení napájení je rozpracované („je to pomalé“) a NVK Fermi
   nepodporuje. Návrh: nepodporovat, nechat framebuffer firmwaru
   (`fbdisplay`).

`open-gpu-kernel-modules` dává lepší podporu nových funkcí, ale bez
uzavřeného userspace NVIDIA nemá Vulkan ani OpenGL. Jejich zobrazovací
část (`nvidia-modeset`) je pod MIT a mohla by sloužit jako záloha pro
Turing+, kdyby nouveau u konkrétní karty selhal - za cenu druhé obří
kódové základny. Nedoporučuji.

### Karty k testu a co na nich ověřit

Karty uživatele (stav 2026-11-14):

| Karta | Čip / generace | Cesta | Co na ní testovat |
|---|---|---|---|
| **RTX 4090** (aktivně, z USB) | AD102, Ada | náš ovladač GSP, NVK | **hlavní karta**: etapa (a) obraz (převzetí, režimy, flip, hotplug, víc monitorů), (b) kopírování, (c) Vulkan; obnova po pádu (reset GSP) |
| **RTX 5070 Ti** | GB203, Blackwell | náš ovladač GSP (i obraz přes GSP), NVK | druhá karta Turing+: ověřit, že jde čistě přes GSP včetně zobrazení; nejnovější firmware GSP; Vulkan NVK |
| RTX 2080 Ti (možná) | TU102, Turing | náš ovladač GSP, NVK | nejstarší generace s GSP: kompatibilita s verzí firmwaru, jinak stejné testy jako 4090 |
| **GT 1030** (aktivně, z USB) | GP108, Pascal | framebuffer firmwaru (`fbdisplay`) | ovladač kartu bez GSP odmítne a obraz zůstane na `fbdisplay`; souběh s kartou Turing+ v jednom stroji (dvě GPU) |
| GTX 560 Ti | GF114/GF110, Fermi | nepodporovat | jen ověřit, že ovladač kartu odmítne a obraz zůstane na `fbdisplay` (bezpečný návrat) |

Pořadí: **RTX 4090** pro etapy (a), (b) a (c); **RTX 5070 Ti** pro
Blackwell (obraz jen přes GSP); **RTX 2080 Ti** (když bude) pro Turing;
**GT 1030** a GTX 560 Ti jen pro bezpečné odmítnutí nepodporované karty.

Blackwell v jednotlivých cestách: nouveau má čipy GB100/GB102 a GB202 až
GB207 (GB203 = RTX 5070 Ti, zobrazení `gb202_disp` přes GSP-RM), NVK
spotřebitelský Blackwell podporuje (Mesa 25.2), open-gpu-kernel-modules
„Turing nebo novější“, nova má GB20x ve výčtu čipů (`gpu.rs`), ale bez
zobrazení.

## Srovnání: nouveau v C, nebo vlastní ovladač v Rustu pro GSP

**Stav: rozhodnuto 2026-11-14 - varianta B (vlastní ovladač v Rustu jen
pro GSP).** Turing až Blackwell, části přeložené z nouveau (licence MIT
překlad dovoluje) a start GSP z nova (GPL, ovladač proto v `gpl/`), pevná verze firmwaru
570.144, definice RPC generované z hlaviček nouveau. Pascal a starší
zatím framebuffer firmwaru. LinuxKPI se odkládá na AMD a 3D na Intelu.
Varianta A (nouveau v C přes LinuxKPI) zůstává zapsaná jako zvažovaná.

### Co dělá GSP a co zbývá hostiteli

Od Turingu běží na GPU firmware GSP-RM (Resource Manager NVIDIA). Převezme
takty, napájení, inicializaci enginů a plánování kanálů; ovladač s ním
mluví přes RPC. Podle nouveau (`nvkm/subdev/gsp`, `gsp/rm/r535`,
`gsp/rm/r570`; v `gsp/` 11 souborů `.c`, v `rm/` 12, v `rm/r535/` 18,
plus generované definice zpráv `nvrm/`):

| Část | Co musí udělat ovladač | Kde v nouveau | Odhad Rust |
|---|---|---|---|
| start GSP | najít FWSEC ve VBIOS a spustit FWSEC-FRTS (oblast WPR2), připravit metadata WPR, zavést `booter_load`, firmware `gsp` a bootloader, předat adresu bufferu libos | `gsp/fwsec.c`, `gsp/tu102.c`, `rm/r535/gsp.c` | 3-5 tis. řádků |
| fronty RPC | sdílená paměť: fronta příkazů a zpráv (`cmdq`/`msgq`, po 256 KiB), ukazatele čtení a zápisu, kódování zpráv, asynchronní události od GSP | `rm/r535/rpc.c`, `gsp.c` | 2-3 tis. |
| objekty RM | alokace objektů (klient, zařízení, kanál, ...) a řídicí volání - zprávy s číselnými strukturami NVIDIA | `rm/r535/alloc.c`, `ctrl.c`, `client.c`, `device.c` + `nvrm/` | 2-4 tis. + definice |
| paměť GPU | tabulky stránek virtuálních adres GPU, BAR1/BAR2, VRAM | `rm/r535/vmm.c`, `bar.c`, `subdev/mmu` | 4-6 tis. |
| kanály | alokace kanálů přes RM, push buffery, semafory; plánování dělá GSP | `rm/r535/fifo.c`, `ce.c`, `gr.c` | 3-5 tis. |
| obraz | výstupy, DP, HDMI, podsvícení přes řídicí volání `NV0073_CTRL_*`; ovladač plní kanál displeje (metody) a čte jeho registry | `rm/r535/disp.c`, `engine/disp/tu102.c` až `gb202.c` | 6-10 tis. |
| uspání a obnova | uložení VRAM při uspání (fbsr), reset po pádu GSP | `rm/r535/fbsr.c` | 1-2 tis. |
| uAPI pro NVK | viz níže | `nouveau_drm.h`, `nouveau_uvmm.c`, `nouveau_exec.c` | 4-6 tis. |

Celkem odhad (neměřeno): **25-40 tisíc řádků Rustu** + generované
definice zpráv RPC.

### Verze RPC a firmwaru

- nouveau má rozhraní GSP-RM pro dvě verze firmwaru: `r535` (535.113.01)
  a `r570` (570.144), vybírá podle dostupného firmwaru (tabulky v
  `gsp/tu102.c`). Struktury zpráv se mezi verzemi liší; NVIDIA stabilní
  ABI nedrží.
- Pro nás: **pevně jedna verze firmwaru** (nejnovější podporovaná
  v nouveau, dnes 570.144, pokrývá Turing až Blackwell). Definice zpráv
  generovat z hlaviček nouveau (MIT) do Rustu skriptem v `xtask`; vrstva
  „rm“ s rozhraním jako `rm/rm.h` v nouveau, aby další verze byla nová
  sada definic a malé rozdíly. Firmware a ovladač se aktualizují spolu.

### Co potřebuje NVK

NVK (Mesa) mluví s jádrem přes uAPI nouveau (`include/uapi/drm/nouveau_drm.h`,
licence MIT): `GETPARAM`, `CHANNEL_ALLOC`/`FREE`, `VM_INIT` (musí být
první), `VM_BIND`, `EXEC`, `GEM_NEW`, `GEM_INFO` (vrací `map_handle` pro
mapování), `GEM_CPU_PREP`/`FINI`. Synchronizace jde přes `drm_nouveau_sync`
se syncobj nebo timeline syncobj (vlastní ioctl na ploty nemá).

Návrh pro obě varianty: render uzel ([bod 8](#vlivy-na-základy)) nabídne
**přesně tuto sémantiku** jako zprávy po kanálu a „winsys“ v Mesa nahradí
`ioctl` zprávami; logika NVK zůstane beze změny. Syncobj = náš objekt
plotu (bod 1), `map_handle` = VMO bufferu (bod 2), `VM_BIND` = mapování do
virtuálních adres kontextu GPU. Přesná vrstva uvnitř Mesy neověřena
(zdroj Mesy byl při průzkumu nedostupný).

### Srovnání variant

| | A: nouveau v C přes LinuxKPI (zvažovaná) | **B: vlastní Rust jen pro GSP (zvolená)** |
|---|---|---|
| generace | Kepler až Blackwell (Maxwell 2 až Volta pomalé) | Turing až Blackwell; Pascal a starší jen framebuffer firmwaru |
| kód u nás | vrstva 15-30 tis. + převzaté C (stovky tisíc) | 25-40 tis. vlastního Rustu |
| bezpečnost | C v izolovaném procesu | Rust, menší plocha útoku |
| aktualizace | import LTS, záplaty jen ve vrstvě | změny z nouveau přenášet ručně; nový čip = práce |
| LinuxKPI | hned | až pro AMD |
| NVK | uAPI nouveau přes render uzel | totéž uAPI napodobit |
| riziko | rozsah a údržba vrstvy | chyby, které v nouveau řeší komunita, řešíme sami |
| karty uživatele | 4090, 5070 Ti, 2080 Ti plně; GT 1030 pomalu | 4090, 5070 Ti, 2080 Ti plně; GT 1030 jen framebuffer firmwaru |

Zvolena B: pro Turing+ je menší a čistší, protože GSP dělá většinu
práce. Cena: Pascal a starší bez akcelerace a změny z nouveau přenášíme
sami. Body základů 1, 2, 4 a 8 platí beze změny.

### Licence ovladače: MIT, nebo GPL

Licenční politika projektu (schváleno 2026-11-14): kód pod GPL smí být
převzatý nebo přeložený, ale **jen v oddělené části `gpl/`** jako
samostatný proces ovladače pod GPL-2.0-only, který mluví naším veřejným
protokolem; nikdy v jádře, knihovnách ani službách.

Co nova (GPL-2.0, Linux master 2026-11-14) obsahuje:

| Část | nova-core | nouveau |
|---|---|---|
| VBIOS (hledání FWSEC v tabulce PMU) | ano (`vbios.rs`) | ano |
| Falcon, FWSEC, booter, SEC2, FSP | ano (`falcon/`, `firmware/`) | ano (FSP jen nové čipy) |
| start GSP, sekvencer | ano (`gsp/boot.rs`, `sequencer.rs`) | ano |
| fronta příkazů a zpráv | ano (`gsp/cmdq`) | ano |
| definice zpráv podle verze firmwaru | rozpracované („GSP firmware abstraction“) | ano (`rm/r535`, `rm/r570`) |
| MMU, BAR, VRAM | částečně (`mm/`, alokátor VRAM otevřený) | ano |
| kanály, GR, CE | ne (otevřené úkoly) | ano |
| zobrazení | ne (ani nova-drm) | ano (přes RM) |
| uAPI pro Vulkan | ne (nova-drm 3 ioctl) | ano (`VM_BIND`, `EXEC`, ... pro NVK) |

| | MIT (jen nouveau) | **GPL-2.0-only (nova + nouveau)** |
|---|---|---|
| umístění | kdekoli (ovladač, sdílené knihovny) | jen `gpl/`, samostatný proces |
| start GSP a fronty | přeložit z C | převzít hotový Rust z nova (méně práce, odhad -5 až -8 tis. řádků překladu) |
| zobrazení, kanály, MMU, uAPI NVK | přeložit z nouveau | totéž (nouveau, MIT - v GPL díle smí být) |
| nové čipy po Blackwellu | čekat na nouveau | nova je směr upstreamu pro GSP (cíl: nahradit nouveau pro GSP); přebírat z obou |
| sdílení kódu s ostatním systémem | volně | z ovladače nic nesmí zpět do MIT částí; společné věci (definice RPC z hlaviček nouveau, trait třídy displej) držet v MIT knihovnách |
| závislosti | driverkit | driverkit + tenká náhrada API `kernel::` z Rustu v Linuxu (PCI, DMA, IO, firmware, devres), na které nova stojí |
| licence binárky | MIT | GPL-2.0-only; zdroj je veřejný (projekt je open source), naše MIT knihovny v ní zůstávají MIT |
| firmware | beze změny (licence NVIDIA pro OSI systémy) | beze změny |

**Rozhodnutí v návrhu: GPL-2.0-only v `gpl/drivers/gpu-nvidia`.** Hlavní
důvody: start GSP a fronty jsou v nova hotové v Rustu (stejný jazyk,
podobné abstrakce jako driverkit), a nova je dlouhodobý směr upstreamu
pro karty s GSP - nové generace se tam objeví dřív. Zbytek (zobrazení,
kanály, MMU, uAPI pro NVK) se stejně překládá z nouveau.

Důsledky a pravidla:

- ovladač je **jeden proces v `gpl/`**, ven jen naše protokoly (třída
  displej, třída render, ploty, sdílené buffery) - stejné jako u varianty
  MIT, takže rozhodnutí jde později otočit bez změny zbytku systému;
- **nic z `gpl/` se nelinkuje do MIT částí**; sestavení (`xtask`) to
  hlídá (vlivy na základy, bod 12);
- definice zpráv RPC generovat **z hlaviček nouveau (MIT)** do MIT
  knihovny - zůstanou použitelné i mimo `gpl/`;
- verzi firmwaru, kterou umí start GSP v nova, ověřit proti zvolené 570.144
  (start ze VBIOS a booter jsou na verzi skoro nezávislé, zprávy RPC ne);
- každý převzatý soubor nese původ a licenci v hlavičce a v `THIRD-PARTY.md`.

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
| **`dma-buf/dma-fence.c`** | **GPL-2.0-only** | mimo `gpl/` nepřevzít; v ovladači pod `gpl/` smí být, ale ploty jádra jsou naše tak jako tak - `dma_fence_*` musí stejně předávat stav našemu objektu plotu |
| `include/linux/*.h` (hlavičky jádra) | převážně GPL | nepřevzít: hlavičky LinuxKPI vlastní (vzor FreeBSD, BSD-2) |

Pravidlo: **před převzetím každého souboru ověřit licenci**. GPL-only
soubory smí jen do procesu ovladače v `gpl/` (celý proces je pak
GPL-2.0-only); mimo něj vlastní implementace podle API a dokumentace.
Seznam se udržuje automaticky skriptem importu (hlavička SPDX / text MIT).

### Licence firmwaru

Ověřeno v kopii linux-firmware (zrcadlo na GitHubu, 2026-11-14):

| Firmware | Licence | Redistribuce | Podmínky |
|---|---|---|---|
| NVIDIA (`LICENCE.nvidia`, „License For Customer Use of NVIDIA Software“) | proprietární | jen „pro použití v operačních systémech šířených pod licencí schválenou OSI“ - náš systém je MIT OR Apache-2.0, tedy ano | binárky beze změny, přiložit text licence; zákaz zpětného inženýrství, dekompilace a překladu; jen pro procesory navržené NVIDIA |
| AMD (`LICENSE.amdgpu`) | proprietární, bez poplatků | binárně ano | přiložit copyright a text licence; zákaz zpětného inženýrství; exportní předpisy USA |

**Otevřené:** v dostupné kopii souboru `WHENCE` chybí záznam pro
`nvidia/<čip>/gsp/` (kopie je zřejmě starší). Před přibalením ověřit
v aktuálním linux-firmware, že firmware GSP spadá pod `LICENCE.nvidia`.
Obraz pak nese firmware v `/system/firmware/` beze změny a licence vedle
něj; `THIRD-PARTY.md` uvede verzi linux-firmware.

## Strategie pro AMD a 3D na Intelu

Pro AMD (a pro 3D akceleraci na Intelu, kde obraz už máme v Rustu) se
převezme hotový ovladač DRM z Linuxu (amdgpu, případně xe) v C. Vrstva
LinuxKPI je přitom **uzavřená uvnitř procesu ovladače**:

- ven z procesu jdou **jen naše rozhraní**: třída displej, třída render
  (obálka: kontexty, buffery, ploty, kvóty), ploty a sdílené buffery
  (VMO + `libs/bufdesc`), služba firmwaru, protokol napájení. Žádné API Linuxu
  (struktury jádra, čísla ioctl mimo užitečná data render uzlu, sysfs)
  nepřekročí hranici procesu;
- uvnitř render uzlu se pro Mesu (RADV, Intel ANV) nese uAPI daného
  ovladače jako užitečná data - stejně jako u NVK. Náhradní ovladač musí
  totéž uAPI zachovat, aby Mesa zůstala beze změny;
- **ovladač jde proto později nahradit vlastním v Rustu bez změny zbytku
  systému** (služby, kompozitor, aplikace, Mesa).

**Licence (politika `gpl/`)**: s převzatým C ovladačem je proces AMD
stejně kandidát na `gpl/` (amdgpu je MIT, ale vrstva by mohla převzít
i GPL infrastrukturu Linuxu - `dma-fence`, pomocné knihovny z `lib/` -
místo vlastní implementace). Postup: základ LinuxKPI z FreeBSD (BSD) a
vlastní, GPL kód Linuxu jen tam, kde ušetří skutečnou práci; pak je celý
proces AMD v `gpl/` pod GPL-2.0-only. Rozhodnout při startu AMD.

**Trend k firmwaru**: stejně jako NVIDIA s GSP přesouvají i AMD a Intel
plánování do firmwaru - AMD od RDNA 3 (GFX11) plánuje fronty mikrokódem
MES (Micro Engine Scheduler), Intel `xe` odesílá práci jen přes firmware
GuC (podle dokumentace a zdrojů Linuxu; v tomto průzkumu podrobně
neověřeno). Hostitelská část pro novější generace se tím zmenšuje a
vlastní ovladač v Rustu (jako pro NVIDIA) může být levnější než udržovat
LinuxKPI. **Rozhodnutí před začátkem AMD**: znovu porovnat vlastní Rust
pro RDNA 3+ (MES) s amdgpu přes LinuxKPI.

**Náklady údržby LinuxKPI**: Linux nemá stabilní vnitřní API - funkce
jádra a DRM se mění s každou verzí (za poslední roky přibyly např.
`drm_exec`, `drm_gpuvm`, změny `dma-fence` a `dma-resv`). Proto:

- držet **jednu LTS verzi** Linuxu a přecházet na další jednou ročně,
  jedním velkým krokem (jako FreeBSD drm-kmod, který Linux následuje po
  verzích se sadou záplat);
- import skriptem: seznam použitých symbolů Linuxu se generuje a hlídá
  (nový nebo změněný symbol = práce ve vrstvě, ne v ovladači), kontrola
  licencí každého souboru;
- host testy vrstvy (zámky, workqueue, DMA, ploty) v simulaci driverkitu;
- odhad: přechod na novou LTS = týdny práce na vrstvě a testy na HW
  (neměřeno); bez toho ovladač zastará a ztratí opravy a nové čipy.

## Vrstva kompatibility (LinuxKPI)

Platí pro AMD a 3D na Intelu ([strategie](#strategie-pro-amd-a-3d-na-intelu)),
ne pro NVIDIA.

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
| synchronizace | `dma_fence`, `sync_file`, `dma_resv`, `dma_buf` | **naše ploty a buffery (VMO + `libs/bufdesc`)** |
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
| (a) obraz | třída displej: start GSP, převzetí od GOP, režimy z EDID, flip, hotplug, více monitorů (na Turing+ přes RM) | jádro GSP (start, RPC r570, objekty RM), služba firmwaru, IOMMU |
| (b) 2D/kopírování | buffery v paměti GPU pro kompozitor, kopírovací engine, nulová kopie snímku | sdílené buffery, ploty |
| (c) Vulkan | NVK/RADV v aplikacích, třída render, služba render, kvóty | port Mesy (vlákno jazyků/libc), render uzel |
| (d) napájení a teploty | runtime PM (D3cold, Optimus), teploty a ventilátory do služby napájení | ACPI `_DSM`/`_PR3` přes acpid |

Etapa (a) dá obraz na kartách Turing až Blackwell. Akcelerace kompozitoru
(b) je přínos i bez Vulkanu. Pro AMD stejné etapy přes LinuxKPI.

## IOMMU

GPU je nejsilnější DMA zařízení v počítači a ovladač má stovky tisíc
řádků převzatého C plus firmware (GSP je vlastní procesor s přístupem
k paměti). **Bez IOMMU může chyba v ovladači nebo firmwaru přepsat
libovolnou paměť včetně jádra.**

Rozhodnutí (2026-11-14, jednotně s disky a sítí):

- s IOMMU: zařízení má vlastní doménu (už dnes), DMA jen do paměti, kterou
  mu ovladač namapuje; po pádu devmgr vypne bus master a doménu zruší;
- **bez IOMMU se ovladač GPU spustí s varováním**: hláška v logu a
  v systémových oznámeních, že DMA není omezené, s radou zapnout ve
  firmwaru VT-d (Intel) nebo AMD-Vi / IOMMU (AMD). Po pádu ovladače platí
  **karanténa DMA** z jádra (bus master vypnout, počkat na dokončení
  přenosů, paměť pustit až potom - [Jádro](jadro.md));
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

Hardware: viz [Karty k testu](#karty-k-testu-a-co-na-nich-ověřit) - RTX
4090 (Turing+ s GSP) a GT 1030 (Pascal bez GSP) jsou k dispozici. Pro AMD
později jedna karta RDNA 2 nebo novější (uživatel žádnou nemá).

## Vlivy na základy

Co je dobré změnit **už teď** (nezávisle na samotném ovladači), aby GPU
nevyžadovalo zásahy do hotových částí. Rozsah: S = dny, M = 1-2 týdny,
L = víc.

| # | Oblast | Změna | Proč teď | Rozsah |
|---|---|---|---|---|
| 1 | **objekt plotu (fence)** | objekt jádra: stav (nesplněn, splněn, chyba), časová osa (timeline, 64bit hodnota), čekání ve `wait_many`, předání handlem, signalizuje jen držitel práva `SIGNAL`; **[ověřeno v QEMU]** krok 6a-1 ([ploty.md](ploty.md)) | `dma-fence.c` je GPL - musíme mít vlastní; zároveň základ pro Vulkan semafory, `sync_file`, `drm_syncobj` a explicitní synchronizaci v protokolu displeje; dnes ho nemá ani Venus (ploty jen uvnitř ovladače) | M |
| 2 | **sdílený buffer** | **rozhodnuto (6a): VMO + popis v knihovně `libs/bufdesc`, ne nový objekt jádra.** Popis: formát (fourcc) + modifikátor (dlaždice, komprese), roviny `{offset, pitch}`, šířka a výška, umístění (systém / paměť zařízení), typ cache. Kdo smí zapisovat, určují práva handlu VMO (`READ`/`MAP`, zapisovatel i `WRITE`). Příjemce popis ověří proti velikosti VMO (nedůvěryhodný vstup). Synchronizace jen explicitní: ploty získání a uvolnění ve zprávách protokolu (bod 1), žádný implicitní `dma_resv`. Odvolání: paměť zařízení je fyzická VMO a ovladač ji odvolá `VMO_REVOKE`; systémovou paměť drží handly. Linuxový `dma-buf` je objekt hlavně kvůli implicitní synchronizaci (`dma_resv`) a připojení zařízení; obojí u nás řeší ploty ve zprávách a `DMA_MAP` nad VMO (bod 4) **[navrženo]** | ekvivalent `dma-buf`; rozhraní zařízení už vyžaduje „celý životní cyklus bufferu“ (bod 5) - teď ho určit jednou pro displej, Venus i budoucí GPU | M |
| 3 | **třída displej v2** | volitelné: buffer klienta jako snímek bez kopie (s plotem získání a uvolnění), formáty a modifikátory, více rovin (kurzor, překryv), atomický commit (režim + buffery najednou), úplné časování v `SET_MODE` | dnešní protokol kopíruje stín; GPU ovladač i Intel mohou zobrazit buffer přímo - přidat teď jako schopnosti, ať se nemění zprávy | S-M |
| 4 | **DMA po kusech a IOMMU** | alokace velké připnuté paměti po stránkách (scatter-gather, ne jen souvislé 4 MiB), mapování rozsahu VMO do domény zařízení na IOVA zvolenou ovladačem, odmapování se synchronním vyprázdněním TLB IOMMU, kvóty DMA v GB | GPU mapuje stovky MB až GB (TTM, GTT); dnešní `dma_memory` je souvislá a malá | M |
| 5 | **remapování přerušení** | zapnout v jádře (VT-d IR, AMD-Vi, GICv3 ITS už je) | bez něj může DMA zařízení podvrhnout MSI | M |
| 6 | **služba firmwaru** | `request_firmware` jako služba: soubory z `/system/firmware`, kontrolní součty, metadata licence, předání VMO jen pro čtení; obraz a instalátor firmware přibalí | GSP-RM (desítky MB), AMD, Wi-Fi, Realtek NIC (`rtl_fw`), budoucí zvuk; dnes nic takového není | S-M |
| 7 | **paměť GPU a rozpočty** | rozpočet jobu i pro paměť zařízení (VRAM, GTT); objekty, které ovladač smí vystěhovat (TTM) a znovu připnout; mapování VRAM přes BAR jako WC (už je typ cache) | kvóty Vulkanu na aplikaci; spravedlnost mezi aplikacemi | M |
| 8 | **třída render a služba render** | protokol render uzlu obecně (z Venusu: kontext, buffer, `VM_BIND`, submit s ploty, dotazy) + služba, která render uzel přidělí podle manifestu a hlídá kvóty | aplikace nesmí dostat capability k ovladači; Venus dnes dává render uzel přes init - sjednotit dřív, než přibude další ovladač | M |
| 9 | **běhové prostředí převzatého C** | vlákna v procesu ovladače (kthread, workqueue), přesné časovače, čekání s timeoutem; `cstd` s vlákny | jádro DRM a amdgpu mají vlastní vlákna a časovače; stejné potřebuje Wi-Fi. Ovladač NVIDIA v Rustu to nepotřebuje - až s AMD | M |
| 10 | **napájení zařízení** | protokol stavů napájení zařízení (D0/D3hot/D3cold) přes službu napájení; volání ACPI `_DSM`/`_PRx` z ovladače přes acpid | notebooky s Optimus vypínají dGPU; jinak baterie | S-M |
| 12 | **oddělená část `gpl/`** | sestavení programů z `gpl/` zvlášť (licence GPL-2.0-only v manifestu a v initfs), kontrola, že žádný MIT program ani knihovna nezávisí na `gpl/`, licence ve výpisu `THIRD-PARTY.md` | politika schválená 2026-11-14; ovladač NVIDIA bude první program v `gpl/` | S |
| 11 | **obnova GPU po pádu** | devmgr: reset funkce PCI (FLR / reset sběrnice) před novou instancí; kompozitor bez displeje vydrží (už umí) | po nastavení režimu se nelze vrátit na GOP; obnova jen resetem | S |

Body 1, 2, 4 a 8 doporučuji začít hned: dotýkají se jádra a protokolů,
které už používají displej a Venus, a později by se měnily na víc místech.

## Rizika

- **Rozsah NVIDIA**: 25-40 tisíc řádků vlastního Rustu; změny a opravy
  z nouveau a nové čipy přenášíme ručně; firmware GSP a ovladač se musí
  aktualizovat spolu (pevná verze 570.144).
- **Rozsah AMD**: stovky tisíc řádků převzatého C a údržba LinuxKPI
  (Linux nemá stabilní vnitřní API) - viz [strategie](#strategie-pro-amd-a-3d-na-intelu).
- **Pascal a starší** (GT 1030) bez akcelerace, jen framebuffer firmwaru.
- **Firmware NVIDIA a AMD je uzavřený** (binární; redistribuce dovolená,
  viz [Licence firmwaru](#licence-firmwaru)). Bez něj na Turing+ nepojede
  nic, ani obraz na Blackwellu. Licence NVIDIA platí jen pro open-source
  systémy - kdyby se licence systému změnila, firmware by nešlo přibalit.
- **Bez IOMMU** (rozhodnuto: spustit s varováním) může chyba v ovladači nebo
  firmwaru přepsat paměť jádra; karanténa DMA chrání jen po pádu, ne za běhu.
- **Notebooky s Optimus**: obraz vede integrovaná Intel; NVIDIA jen
  počítá - vyžaduje sdílené buffery mezi ovladači (bod 2) a napájení (bod 10).
- **Testy jen na skutečném hardwaru**: pomalejší cyklus, nutná sériová
  linka nebo log na disk.
- **Pády firmwaru GSP**: obnova jen resetem zařízení, na chvíli bez obrazu.
- **Licence se mohou měnit** souborově: kontrola licencí při každém importu.

## Rozhodnutí a otázky

| # | Otázka | Rozhodnutí (2026-11-14) |
|---|---|---|
| 1 | cesta pro NVIDIA | **B**: vlastní Rust jen pro GSP (Turing až Blackwell), firmware 570.144, RPC generované z hlaviček nouveau; Pascal framebuffer firmwaru; LinuxKPI až pro AMD a 3D na Intelu ([srovnání](#srovnání-nouveau-v-c-nebo-vlastní-ovladač-v-rustu-pro-gsp)) |
| 2 | bez IOMMU | **B**: spustit s varováním, karanténa DMA, rada zapnout VT-d / AMD-Vi |
| 3 | firmware | **A**: přibalit do obrazu po ověření licence (ověřeno, zbývá záznam GSP ve `WHENCE`) |
| 4 | základy | body 1, 2, 4, 8 jako jaderná práce **po dynamickém linkování a před FFmpeg** (ploty a sdílené buffery využije i video) |
| 5 | licence ovladače NVIDIA | **GPL-2.0-only v `gpl/`** (nova + nouveau), rozhodnuto v návrhu podle politiky `gpl/` ([licence ovladače](#licence-ovladače-mit-nebo-gpl)) |
