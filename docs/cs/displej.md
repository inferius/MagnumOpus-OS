# Nastavení obrazu (nativní ovladače displeje)

Nadstavba nad framebufferem z firmwaru (UEFI GOP, ovladač `fbdisplay`):
výčet konektorů a monitorů, režimy z EDID, změna rozlišení za běhu,
připojení monitoru za běhu (hotplug) a page flip ve vsync. **Bez
akcelerace** – obraz dál skládá kompozitor `compd` na procesoru.

| Část | Co dělá |
|---|---|
| `libs/display` | společná část pro všechny výrobce: EDID, DisplayID, časování (CVT, DMT), DisplayPort (AUX, DPCD, navázání linky, MST – příprava), HDMI (infoframy, TMDS, SCDC) |
| `guiproto::modes` + `guiproto::display` | protokol mezi ovladačem a kompozitorem (jeden pro všechny ovladače) |
| `libs/displaydrv` | kostra nativního ovladače: obsluha protokolu, dvojitý buffer s vsync, záznam stavu firmwaru |
| `user/bochs-display` | vzorový ovladač pro QEMU (`bochs-display`, `secondary-vga`) |
| `libs/intelgfx` + `user/intel-display` | Intel Gen9 (Skylake) až Xe2 (Lunar Lake, Battlemage) |
| `user/devmgr` (`native.rs`), `user/init` (`display_native.rs`) | spuštění, výběr displeje, obnova po pádu |

## Přehled

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Vrstvy nastaveni obrazu">
<defs><marker id="dp1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="270" y="10" width="220" height="56" rx="6"/><text class="t b" x="380" y="33" text-anchor="middle">compd (kompozitor)</text><text class="m" x="380" y="52" text-anchor="middle">snímek ve VMO, příkazy mode / outputs</text>
<path class="ln" d="M380 66 L380 96" marker-end="url(#dp1)"/>
<text class="m" x="392" y="86">guiproto::display + modes: CONNECTORS, SET_MODE, ATTACH, PRESENT, HOTPLUG, RESET</text>
<rect class="box" x="10" y="100" width="160" height="56" rx="6"/><text class="t b" x="90" y="123" text-anchor="middle">fbdisplay</text><text class="m" x="90" y="142" text-anchor="middle">GOP, jediný režim</text>
<rect class="box" x="190" y="100" width="160" height="56" rx="6"/><text class="t b" x="270" y="123" text-anchor="middle">virtio-gpu</text><text class="m" x="270" y="142" text-anchor="middle">režim od hostitele</text>
<rect class="ok" x="370" y="100" width="180" height="56" rx="6"/><text class="t b" x="460" y="123" text-anchor="middle">bochs-display</text><text class="m" x="460" y="142" text-anchor="middle">QEMU, flip, režimy</text>
<rect class="ok" x="570" y="100" width="180" height="56" rx="6"/><text class="t b" x="660" y="123" text-anchor="middle">intel-display</text><text class="m" x="660" y="142" text-anchor="middle">Gen9 až Xe2</text>
<path class="ln" d="M460 156 L560 196" marker-end="url(#dp1)"/>
<path class="ln" d="M660 156 L600 196" marker-end="url(#dp1)"/>
<rect class="ok" x="440" y="200" width="310" height="50" rx="6"/><text class="t b" x="595" y="221" text-anchor="middle">libs/displaydrv</text><text class="m" x="595" y="239" text-anchor="middle">protokol, dvojitý buffer, vsync, záznam firmwaru</text>
<path class="ln" d="M595 250 L595 268" marker-end="url(#dp1)"/>
<rect class="ok" x="440" y="272" width="310" height="50" rx="6"/><text class="t b" x="595" y="293" text-anchor="middle">libs/display</text><text class="m" x="595" y="311" text-anchor="middle">EDID, DisplayID, CVT/DMT, DP, HDMI</text>
<rect class="box" x="10" y="200" width="400" height="122" rx="6"/><text class="t b" x="210" y="223" text-anchor="middle">devmgr + init</text><text class="m" x="210" y="244" text-anchor="middle">devmgr spustí ovladač podle PCI a pošle initu sadu pro restart</text><text class="m" x="210" y="262" text-anchor="middle">init: výběr virtio-gpu, nativní, jinak fbdisplay;</text><text class="m" x="210" y="280" text-anchor="middle">drží konec kanálu ovladače a záznam firmwaru;</text><text class="m" x="210" y="298" text-anchor="middle">po pádu: nová instance obnoví obraz firmwaru</text>
</svg>
<figcaption>Kompozitor mluví se všemi ovladači stejným protokolem; nativní ovladače sdílejí kostru a knihovnu formátů.</figcaption>
</figure>

## Protokol displeje

Doplňuje původní zprávy (`INFO`, `ATTACH`, `PRESENT`, `SYNC`) o nastavení
obrazu. Čísla 6–15 / 106–115 a bity schopností 2–7 patří nastavení obrazu
(5/105 a bit 1 má Venus).

| Zpráva | Směr | Obsah |
|---|---|---|
| `CONNECTORS` (6) → `CONNECTORS` (106) | klient → ovladač | konektory: číslo výstupu, typ (DP, eDP, HDMI, DVI, VGA, virtuální, firmware), připojen, jméno monitoru z EDID, rozměr v mm, režimy (š × v, obnova v mHz, pixel clock, příznaky `PREFERRED`, `CURRENT`) |
| `SET_MODE` (7) → `MODE_SET` (107) | klient → ovladač | nastaví režim výstupu; obnova 0 = nejvyšší. Pak klient pošle nové `ATTACH` s novou velikostí. Při chybě zůstává původní režim |
| `HOTPLUG` (108) | ovladač → klient | událost bez požadavku: konektor se změnil, klient znovu pošle `CONNECTORS` |
| `RESET` (109) | ovladač → klient | nová instance ovladače po pádu: předchozí `ATTACH` neplatí, klient znovu vybere výstup |

| Schopnost | Bit | Význam |
|---|---|---|
| `MODESET` | 2 | ovladač umí víc než jeden režim |
| `HOTPLUG` | 3 | ovladač posílá `HOTPLUG` |
| `VSYNC` | 4 | `PRESENT` se zobrazí celým snímkem ve svislém zatemnění, `PRESENTED` nese čas vsync |
| `MULTI` | 5 | víc výstupů |

**Page flip.** Kompozitor dál kreslí do své VMO (stín) a v `PRESENT` pošle
poškozené obdélníky. Ovladač je zkopíruje do zadního bufferu a ten ve vsync
přepne. Zadní buffer je o snímek pozadu, proto se do něj kopíruje i
poškození předchozího snímku. Po `ATTACH` a změně režimu se kopírují celé
oba buffery. Kompozitor tak nemusí vědět, kolik bufferů ovladač má, a
rozhraní se nekříží se seznamem vrstev pro backend Vulkan (`libs/vkcomp`).

**Kompozitor** vybere hlavní připojený konektor (přednost má vestavěný
panel) a jeho aktuální režim. Když žádný režim nastavený není (monitor
právě připojen), nastaví preferovaný. Příkazy na řídicím kanálu:
`mode <š> <v> [mHz]` (změna rozlišení, okna se přesunou na obrazovku,
okna na celou obrazovku a maximalizovaná dostanou novou velikost) a
`outputs` (konektory jako text).

```text
compd                              ovladač
  SYNC / INFO / CONNECTORS  ───►
                            ◄───   MODES, CONNECTORS (1920x1080 CURRENT, ...)
  mode 1024 768 (od uživatele)
  SET_MODE 0 1024 768 0     ───►   nastaví režim, zahodí stín
                            ◄───   MODE_SET 1024x768 @ 60004 mHz
  ATTACH (nová VMO)         ───►   kopie do obou bufferů, flip
                            ◄───   ATTACHED
  PRESENT seq=1 [obdélníky] ───►   kopie (obdélníky + předchozí) do zadního, flip
                            ◄───   PRESENTED seq=1 čas vsync
```

## libs/display

| Modul | Obsah |
|---|---|
| `edid` | základní blok EDID 1.3/1.4: výrobce, produkt, rozměry, vstup (bitů na barvu, rozhraní), ustálená, standardní a detailní časování, jméno a sériové číslo, rozsah frekvencí, čtení po blocích s opakováním |
| `cta` | rozšíření CTA-861: kódy VIC (+ tabulka časování), audio, reproduktory, HDMI VSDB (fyzická adresa, hluboká barva, max. TMDS), HDMI Forum VSDB a HF-SCDB (600 MHz, SCDC) |
| `displayid` | DisplayID 1.3 a 2.0: časování typu I (10 kHz) a VII (kHz), jméno produktu |
| `cvt`, `dmt` | CVT 1.2 (standardní, RB v1, RB v2) v celých číslech; tabulka DMT |
| `modes` | seznam režimů ze všech zdrojů: sloučení duplicit (detailní > VIC > DMT > CVT), limity zdroje a monitoru, řazení, výběr |
| `dp` | DPCD, schopnosti (i tabulka rychlostí eDP 1.4), výběr a záložní konfigurace linky, M/N; `auxch` (kódování AUX, DEFER, I2C přes AUX pro EDID), `train` (obnova hodin + vyrovnání kanálu, TPS1–4), `mst` (sideband zprávy s CRC – jen příprava) |
| `hdmi` | infoframy AVI, vendor (HDMI VIC pro 4K), SPD, audio; TMDS clock podle hloubky barvy a barevného prostoru; rozhodnutí o scramblingu, registry SCDC |

Testy (`cargo test -p display`, součást `cargo xtask test`) běží na
reálných dumpech EDID z databáze [linuxhw/EDID](https://github.com/linuxhw/EDID)
(monitor AOC 2243W s HDMI, panel notebooku LG LP133WH2) a na blocích
sestavených v testu (DisplayID, HF-VSDB). Hodnoty CVT se porovnávají
s kalkulátorem VESA. Navázání linky DP a čtení EDID přes AUX se testuje
proti simulovanému přijímači (DEFER, krátké odpovědi I2C, vadný kabel,
pád na nižší rychlost).

## Výběr displeje v initu

1. virtio-gpu od správce zařízení (stroj ve virtualizaci),
2. nativní ovladač (`display-native` od devmgr), který odpoví na `SYNC`
   a hlásí připojený monitor s režimem; na x86 má přednost karta s nižší
   adresou PCI (tu používá firmware),
3. jinak `fbdisplay` nad framebufferem firmwaru.

devmgr spouští `bochs-display` pro `1234:1111` (bochs-display, stdvga,
secondary-vga) a `intel-display` pro Intel třídy 03. Initu pošle zprávu
`display-native <program> <b> <d> <f>` s konci kanálu (klient, ovladač),
prostředkem zařízení, konfigurací PCI, záznamem firmwaru a procesem.
Intel navíc dostane rozsah stolen paměti (BDSM/GGC), bez DMA a přerušení.

## Bezpečnost obrazu

Cíl: **nikdy nezůstat s černou obrazovkou.**

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Navrat na obraz firmwaru">
<defs><marker id="dp2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">start ovladače</text><text class="m" x="95" y="64" text-anchor="middle">jen čte registry,</text><text class="m" x="95" y="80" text-anchor="middle">uloží záznam firmwaru</text>
<path class="ln" d="M180 55 L218 55" marker-end="url(#dp2)"/>
<rect class="ok" x="220" y="20" width="170" height="70" rx="6"/><text class="t b" x="305" y="45" text-anchor="middle">ATTACH / SET_MODE</text><text class="m" x="305" y="64" text-anchor="middle">první zápis registrů</text><text class="m" x="305" y="80" text-anchor="middle">až teď</text>
<path class="ln" d="M390 55 L428 55" marker-end="url(#dp2)"/>
<rect class="warn" x="430" y="20" width="150" height="70" rx="6"/><text class="t b" x="505" y="45" text-anchor="middle">pád ovladače</text><text class="m" x="505" y="64" text-anchor="middle">supervisor initu</text><text class="m" x="505" y="80" text-anchor="middle">spustí restore</text>
<path class="ln" d="M580 55 L618 55" marker-end="url(#dp2)"/>
<rect class="acc" x="620" y="20" width="130" height="70" rx="6"/><text class="t b" x="685" y="45" text-anchor="middle">obraz firmwaru</text><text class="m" x="685" y="64" text-anchor="middle">RESET → compd</text><text class="m" x="685" y="80" text-anchor="middle">připojí znovu</text>
<rect class="box" x="10" y="120" width="380" height="70" rx="6"/><text class="t b" x="200" y="145" text-anchor="middle">chyba před převzetím</text><text class="m" x="200" y="164" text-anchor="middle">neznámá generace, dlaždicový framebuffer, žádný</text><text class="m" x="200" y="180" text-anchor="middle">výstup: ovladač skončí → init použije fbdisplay</text>
<rect class="box" x="430" y="120" width="320" height="70" rx="6"/><text class="t b" x="590" y="145" text-anchor="middle">chyba SET_MODE</text><text class="m" x="590" y="164" text-anchor="middle">ovladač vrátí předchozí nastavení,</text><text class="m" x="590" y="180" text-anchor="middle">compd ponechá snímek</text>
</svg>
<figcaption>Ovladač mění registry obrazu až po uložení záznamu firmwaru; po pádu ho nová instance zapíše zpět.</figcaption>
</figure>

- **Záznam firmwaru** (`displaydrv::snapshot`): seznam (registr, hodnota)
  všeho, co ovladač mění, v pořadí obnovy (adresa povrchu poslední, tím se
  změna projeví celým snímkem). Zapisuje se do objektu paměti, který drží
  init – přežije pád ovladače. Magic se zapisuje naposled: pád uprostřed
  zápisu nechá záznam neplatný a nová instance nic nepřepisuje.
- **Ovladač nesáhne na registry**, dokud nemá záznam uložený (backend vrací
  `BAD_STATE`), a dokud kompozitor nepřipojí snímek.
- **Pád**: init drží konec kanálu ovladače (`Kept`), takže kompozitor kanál
  neztratí. Supervisor spustí novou instanci s příznakem `restore`: ta
  zapíše záznam zpět, pošle `RESET` a dál slouží **jen režim firmwaru**
  (bez `MODESET`, jeden buffer). Kompozitor se znovu připojí.
- **Čistý konec** (vypnutí) obraz firmwaru také vrátí.
- **Log jádra**: ovladač vybraný pro GUI dostane od initu framebuffer
  firmwaru a převezme ho (`FB_CONTROL CLAIM`, jako `fbdisplay`) – jádro pak
  nekreslí log do paměti, kterou ukazuje GUI. Init kvůli tomu instanci od
  devmgr nahradí svou (ta první registry obrazu ještě neměnila).

Co pokryté není (rizika):

- **Zaseknutí** ovladače (nekonečná smyčka bez pádu) se nepozná – nemá
  hlídání živosti.
- Když padá i instance v režimu obnovy, supervisor to po 5 pádech za
  minutu vzdá; obraz pak zůstane, jak ho nechala poslední obnova.
- Záchranná konzole (`user/console`) kreslí do framebufferu firmwaru; když
  nativní ovladač běží na stejné kartě s jiným rozlišením, je její obraz
  posunutý, dokud se nevrátí režim firmwaru.

## bochs-display (QEMU)

Registry DISPI (Bochs VBE) v MMIO BAR2 od 0x500, obraz v BAR0, EDID od
offsetu 0 v BAR2 (generuje ho QEMU: „QEMU Monitor“, 13 režimů). Stejné
registry má `secondary-vga` a `VGA` na x86, proto `xtask` žádné zařízení
přidávat nemusel (aarch64 už `bochs-display` měl).

- **Změna rozlišení**: `ENABLE` = 0, `BPP` 32, `XRES`, `YRES`,
  `VIRT_WIDTH`, `VIRT_HEIGHT` = 2 × výška, `ENABLE` = zapnuto + LFB. Zpětné
  čtení ověří, že zařízení režim přijalo (jinak návrat k předchozímu).
- **Page flip**: dva buffery ve VRAM, přepnutí registrem `Y_OFFSET`.
  Zařízení nemá přerušení vsync – zatemnění simulují hodiny s periodou
  podle obnovy (`VblankClock`), `PRESENTED` přijde v jejich taktu.
- `-device bochs-display` neumí `GETCAPS` (maximum rozlišení); pak
  režimy omezuje jen velikost VRAM.

Test (`cargo xtask test --only displej`, x86_64 i aarch64): vlastní
instance kompozitoru nad posledním `bochs-display` (na x86 druhá karta,
první má log jádra) – výčet režimů z EDID, nastavení režimu kompozitorem,
`mode 800 600`, `mode 1024 768 60000`, odmítnutí neplatného režimu, růst
počtu zobrazených snímků, pak zabití ovladače, nová instance s
`restore`, `RESET` a kontrola, že platí režim firmwaru (x86: OVMF kartu
zapne na 1280x800; aarch64: výstup vypnutý).

## Intel (intel-display)

### Rozsah: displej verze 9 až 20

| Displej | Platformy | Stav |
|---|---|---|
| 9 | Skylake, Kaby Lake, Coffee/Whiskey/Comet Lake | převzetí, flip, scaler, EDID (AUX, GMBUS), hotplug, znovunavázání linky DP (stejná rychlost) |
| 11 | Ice Lake, Elkhart/Jasper Lake | převzetí, flip, scaler, EDID, hotplug |
| 12, 13 | Tiger Lake, Rocket Lake, Alder Lake, Raptor Lake, DG2 | převzetí, flip (integrovaná), scaler, EDID, hotplug; DG2 jen jeden buffer (framebuffer v paměti karty) |
| 14, 20 | Meteor/Arrow Lake, Lunar Lake, Battlemage | převzetí a scaler bez EDID (AUX je jinde – XELPDP), jeden buffer |
| 30 | Panther Lake | jen rozpoznání, zůstává fbdisplay |

Proč od Gen9: Skylake zavedl „universal planes“, DDI s jednotným
`TRANS_DDI_FUNC_CTL`, scaler pipe a GGTT ve formátu gen8 – tyto registry
se do Xe2 mění jen v detailech. Starší generace (Broadwell a níž) mají
jiné roviny i porty a dnes jsou vzácné. Gen9 je zároveň nejrozšířenější
(stroje 2015–2020).

### Co ovladač dělá

1. Rozpozná generaci podle PCI ID (`intelgfx::platform`, seznamy z
   `include/drm/intel/pciids.h`).
2. Přečte stav od GOP: zapnutý transcoder (EDP, A–D), časování, port DDI
   (`TRANS_DDI_FUNC_CTL`), rovinu 1 (`PLANE_CTL`, `STRIDE`, `SURF`).
   Převezme jen lineární XRGB8888 (to GOP nastavuje).
3. EDID: DP a eDP přes AUX (`DDI_AUX_CTL`), HDMI a DVI přes GMBUS (piny podle
   PCH). Obnovu změří čítačem snímků pipe, jinak z EDID.
4. Framebuffer GOP najde přes položky GGTT (stolen paměť nebo paměť karty)
   a namapuje ho. Druhý buffer pro flip dá do stolen paměti za něj a
   zapíše mu položky GGTT na volné adrese (0xE0000000).
5. **Flip** = zápis `PLANE_SURF` (hardware ho převezme ve vsync);
   dokončení = `PLANE_SURFLIVE` má novou adresu a čítač snímků se posunul.
6. **Změna rozlišení** = menší režimy z EDID přes scaler pipe (`PIPESRC`,
   `PS_CTRL`, `PS_WIN_POS`, `PS_WIN_SZ`; okno se zachováním poměru stran).
   Časování monitoru, PLL a linka zůstávají od firmwaru.
7. Hotplug: periodické čtení `SDEISR`, `GEN8_DE_PORT_ISR`,
   `GEN11_DE_HPD_ISR` (úroveň HPD) – změna = událost `HOTPLUG`.

### Co ovladač nedělá (vědomě)

- **Jiná obnovovací frekvence ani vyšší rozlišení než od firmwaru**: to
  vyžaduje nastavit PLL (DPLL/combo PHY/Type-C PHY podle generace), znovu
  zapnout DDI a navázat linku. Knihovna to připravuje (`libs/display::dp`:
  výběr linky, M/N, trénink; `intelgfx::phy` pro Gen9), ale celé sekvence
  zapnutí z i915 přeložené nejsou.
- Druhý monitor (jen výstup zapnutý firmwarem), MST, DSC, HDR, PSR.
- Přerušení (vsync se zjišťuje dotazem na registry každou 1 ms během flipu).

### Co na hardwaru ověřené není

**Nic z Intel části zatím neběželo na skutečném stroji** – QEMU grafiku
Intel neemuluje. Host testy (`cargo test -p intelgfx`) jedou nad
simulovanými registry (`intelgfx::sim`): scénáře „eDP na transcoderu EDP“
a „HDMI na pipe B“ s reálnými dumpy EDID, zaznamenané posloupnosti zápisů
(flip, scaler, obnova záznamu, trénink linky Gen9). Simulace je jen tak
věrná, jak hardware popisuje i915 a PRM. Nejistá místa:

- transcoder EDP krmí pipe A (předpoklad podle toho, jak to nastavuje GOP),
- přiřazení AUX kanálu a pinu GMBUS portům Type-C na ADL+ a diskrétních kartách,
- změna `PIPESRC` za běhu bez vypnutí pipe (i915 to dělá jako „fastset“),
- zápis položek GGTT v oblasti 0xE0000000 (ověřuje se, že je volná),
- velikost stolen paměti z registru GGC (kódování Gen9+).

### Test na skutečném stroji

1. `cargo xtask image` → `target/os-x86_64-debug.img`; zapsat na USB
   flashku (Linux `dd if=... of=/dev/sdX bs=4M`, Windows Rufus v režimu
   DD). Secure Boot vypnout.
2. **Sériová linka** (deska s COM portem nebo USB-sériový adaptér na
   portu, který jádro používá): 115200 8N1, zaznamenat celý log. Hledané
   řádky: `devmgr: pci ... 8086:xxxx trida 03`, `intel-display ...:
   <platforma>, vystup ..., <š>x<v> @ ... mHz, monitor ...`,
   `init: displej je nativni ovladac intel-display`, `compd: displej`.
3. Bez sériového portu: log na disk jádro zatím neumí (plán v
   architektuře) – vyfotit obrazovku; zachráněná konzole po pádu GUI
   ukáže poslední řádky logu.
4. Odpojit a připojit monitor (HDMI/DP) a sledovat v logu `HOTPLUG`.
   Změnu rozlišení zatím umí jen řídicí kanál kompozitoru (příkaz `mode`
   od initu a testů); nastavení obrazu pro uživatele je další krok.
5. Pokud obraz zčerná: počkat 5 s (restart ovladače v režimu obnovy),
   případně restart stroje – záznam firmwaru se při startu ukládá znovu.

## Původ a licence

Kód je napsaný znovu v Rustu. Z Linuxu jsme brali postupy a fakta
(hodnoty registrů, pořadí kroků) jen ze souborů pod licencí MIT (ověřeno
v hlavičce každého souboru); kód pod GPL nepoužit. Přehled je v
`THIRD-PARTY.md` v kořeni repozitáře. Dumpy EDID v testech: linuxhw/EDID,
CC BY 4.0.

## Testy

| Úroveň | Příkaz | Co |
|---|---|---|
| host | `cargo test -p display` | EDID, CTA, DisplayID, CVT/DMT, AUX, trénink linky, MST CRC, HDMI (35 testů) |
| host | `cargo test -p displaydrv` | dvojitý buffer, vsync, změna režimu, události, záznam firmwaru |
| host | `cargo test -p intelgfx` | platformy, stav od GOP, AUX, GMBUS, GGTT, flip, scaler, obnova, hotplug, trénink Gen9 |
| host | `cargo test -p guiproto`, `-p comp` | zprávy protokolu, `Comp::resize` |
| QEMU | `cargo xtask test --only displej` (x86_64, aarch64) | bochs-display + compd: režimy, změna rozlišení, pád a obnova |
