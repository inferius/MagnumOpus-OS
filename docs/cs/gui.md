# Grafika a okna (GUI)

Milník 6c. Popisuje ovladače displeje (`virtio-gpu`, `fbdisplay`), kompozitor `compd`, protokol
okna (`libs/guiproto`), nativní knihovnu UI (`libs/ui`), písmo
(`libs/font`), kreslení (`libs/gfx`), systémové dialogy a API akcí.
Odsouhlasená rozhodnutí jsou v [Architektuře](architektura.md#rozhodnutí-k-milníkům-6a-6d-odsouhlaseno)
a v tabulce [Rozhodnutí 6c](#rozhodnutí-6c-odsouhlaseno) níže.

<figure class="diagram">
<svg viewBox="0 0 780 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Části GUI">
<defs><marker id="g1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">aplikace</text><text class="m" x="95" y="64" text-anchor="middle">libs/ui: prvky, text</text><text class="m" x="95" y="80" text-anchor="middle">kreslí do svých bufferů</text>
<rect class="box" x="10" y="120" width="170" height="70" rx="6"/><text class="t b" x="95" y="145" text-anchor="middle">okno terminálu</text><text class="m" x="95" y="164" text-anchor="middle">nativní protokol 6b</text><text class="m" x="95" y="180" text-anchor="middle">jako obsah okna</text>
<rect class="box" x="10" y="220" width="170" height="70" rx="6"/><text class="t b" x="95" y="245" text-anchor="middle">shell, agent</text><text class="m" x="95" y="264" text-anchor="middle">volají akce aplikací</text><text class="m" x="95" y="280" text-anchor="middle">(capability actions)</text>
<rect class="acc" x="280" y="60" width="200" height="200" rx="6"/><text class="t b" x="380" y="88" text-anchor="middle">compd (kompozitor)</text><text class="m" x="380" y="110" text-anchor="middle">strom vrstev všech oken</text><text class="m" x="380" y="128" text-anchor="middle">transakce, takt snímků</text><text class="m" x="380" y="146" text-anchor="middle">dekorace, odznak důvěry</text><text class="m" x="380" y="164" text-anchor="middle">systémové dialogy</text><text class="m" x="380" y="182" text-anchor="middle">fokus, rozdělení vstupu</text><text class="m" x="380" y="200" text-anchor="middle">strom přístupnosti</text><text class="m" x="380" y="218" text-anchor="middle">směrování akcí</text>
<path class="ln" d="M180 55 L278 100" marker-end="url(#g1)"/><text class="m" x="190" y="62">okno: vrstvy, a11y</text>
<path class="ln" d="M180 155 L278 160" marker-end="url(#g1)"/>
<path class="ln" d="M180 255 L278 220" marker-end="url(#g1)"/>
<rect class="box" x="580" y="20" width="190" height="70" rx="6"/><text class="t b" x="675" y="45" text-anchor="middle">inputd</text><text class="m" x="675" y="64" text-anchor="middle">fokus = compd</text><text class="m" x="675" y="80" text-anchor="middle">bezpečná zkratka</text>
<rect class="box" x="580" y="140" width="190" height="70" rx="6"/><text class="t b" x="675" y="165" text-anchor="middle">virtio-gpu (ovladač)</text><text class="m" x="675" y="184" text-anchor="middle">displayproto: snímek,</text><text class="m" x="675" y="200" text-anchor="middle">poškozené oblasti, flush</text>
<rect class="box" x="580" y="260" width="190" height="70" rx="6"/><text class="t b" x="675" y="285" text-anchor="middle">init</text><text class="m" x="675" y="304" text-anchor="middle">identita a oprávnění</text><text class="m" x="675" y="320" text-anchor="middle">aplikací, dialogy</text>
<path class="ln" d="M578 55 L482 100" marker-end="url(#g1)"/><text class="m" x="500" y="62">Record</text>
<path class="ln" d="M482 170 L578 175" marker-end="url(#g1)"/><text class="m" x="500" y="164">Present</text>
<path class="ln" d="M578 290 L482 240" marker-end="url(#g1)"/><text class="m" x="490" y="290">connect, dialog</text>
<rect class="ok" x="280" y="290" width="200" height="50" rx="6"/><text class="t b" x="380" y="312" text-anchor="middle">QEMU virtio-gpu</text><text class="m" x="380" y="329" text-anchor="middle">2D teď, virgl 3D další krok</text>
<path class="ln" d="M675 212 L482 305" marker-end="url(#g1)"/>
</svg>
<figcaption>Části GUI a kdo s kým mluví. Aplikace nikdy nevidí cizí okna ani obrazovku.</figcaption>
</figure>

## Rozhodnutí 6c (odsouhlaseno)

| # | Otázka | Rozhodnutí |
|---|---|---|
| 1 | vykreslování | **softwarové skládání nad virtio-gpu 2D** jako kompatibilní vrstva všude (x86_64 i aarch64); hned potom **virgl 3D** jako druhý backend kompozitoru (stejný strom vrstev) |
| 2 | buffery | **sdílený objekt paměti (VMO)**: alokuje ho aplikace (platí její job), kompozitoru předá handle jen s právy `READ`, `MAP`, `WAIT`, `DUPLICATE`, `TRANSFER`; kreslení bez kopírování |
| 3 | snímky | **takt kompozitoru + transakce**: termín snímku 60 Hz, aplikace dostane `Frame` jen na požádání; transakce jsou atomické i přes více oken a aplikací (synchronizovaná změna velikosti) |
| 4 | písmo | **vlastní parser a rasterizér TrueType** (`libs/font`): cmap, glyf, hmtx, kern, plošné vyhlazení, bez hintingu a shapingu |

⚠️ **Rizika (řečená předem):**

- **virgl 3D v QEMU** potřebuje displej s OpenGL (`-display sdl,gl=on`,
  `gtk,gl=on` nebo `egl-headless`). Testy běží bez okna, takže 3D jde
  testovat jen s `egl-headless`; když ho hostitel nemá, test 3D se přeskočí
  a ověří se jen 2D. Venus (Vulkan) potřebuje port Mesy a v QEMU pro
  Windows chybí, proto je to až pozdější krok.
- **Bez IOMMU** smí `virtio-gpu` přes DMA zapsat kamkoli do paměti
  (stejně jako ostatní ovladače). S IOMMU (`iommu_platform=on`) jen do
  svých bufferů.
- **Aplikace smí svůj buffer měnit i během čtení.** Nejhorší důsledek je
  roztržený obsah jejího vlastního okna; kompozitor obsahu bufferu nikdy
  nevěří (rozměry kontroluje při vytvoření, čtení je omezené na VMO).
- **Softwarové skládání** stojí CPU: při 1280x800 a 60 Hz je to ~250 MB/s
  zápisu. Proto se skládají jen poškozené oblasti a nic se nekreslí, když
  se nic nemění.
- **Konzole jádra a GOP firmwaru (vyřešeno v loaderu):** firmware
  (edk2/OVMF) má ovladač `VirtioGpuDxe` a z virtio-gpu udělá také GOP. Jeho
  framebuffer je obyčejná RAM, kterou na obrazovku posílají příkazy virtio -
  po ExitBootServices by ho nikdo neobnovoval a konzole jádra by byla
  prázdná, dokud obraz nepřevezme `compd`. Loader proto projde všechny GOP,
  přes `EFI_PCI_IO_PROTOCOL` zjistí výrobce a typ zařízení a **virtio-gpu
  (1af4:1050) vezme jen tehdy, když jiný GOP není** (pak varuje v logu).
  Log loaderu vypíše všechny GOP (`GOP na zarizeni PCI 1234:1111`, `GOP mimo
  PCI`) a vybraný framebuffer: x86_64 karta VGA, aarch64 ramfb.
- **Napodobení systémového dialogu** v celoobrazovkové aplikaci nejde
  úplně vyloučit pohledem. Ochrana je v tom, že dialog oprávnění přijme
  jen fyzický vstup (umělý vstup ne), obsahuje bezpečnostní frázi
  uživatele, kterou aplikace nezná, a bezpečná zkratka vždy ukáže
  skutečný systémový dialog.

## Spuštění

```text
init
 ├─ devmgr ── najde PCI 1af4:1050 (virtio-gpu) ── spustí virtio-gpu
 │            initu pošle zprávu `display` + kanál (displayproto)
 ├─ fbdisplay ─ jen když `display` nepřišel: framebuffer z firmwaru
 │              (bootstrap FRAMEBUFFER, FB_CONTROL CLAIM), viz Displeje
 ├─ inputd
 └─ compd ─── init mu pošle `display` (kanál ovladače), `input`
              (nový kanál fokusu od inputd), `secure` (kanál zkratky)
              a pro každou aplikaci `connect <identita>` -> kanál okna
```

Kompozitor dostane fokus vstupu od `inputd` (příkaz `focus`), takže
klávesnici a myš vidí jen on a rozděluje je oknům. Bezpečnou zkratku
dostává také on (kanál `secure`) a ukáže na ni systémovou obrazovku.

## Kreslení a buffery

Pixel je `u32` v pořadí bajtů B, G, R, A (formát virtio-gpu
`B8G8R8A8_UNORM`, v registru `0xAARRGGBB`) s **předem vynásobenou alfou**
(premultiplied). Buffer má šířku, výšku a krok řádku (stride) **v pixelech**.

<figure class="diagram">
<svg viewBox="0 0 780 260" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Zivotni cyklus bufferu">
<defs><marker id="g2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="160" height="60" rx="6"/><text class="t b" x="90" y="55" text-anchor="middle">aplikace kreslí</text><text class="m" x="90" y="74" text-anchor="middle">do bufferu A</text>
<rect class="acc" x="220" y="30" width="160" height="60" rx="6"/><text class="t b" x="300" y="55" text-anchor="middle">COMMIT</text><text class="m" x="300" y="74" text-anchor="middle">vrstva -> A, damage</text>
<rect class="box" x="430" y="30" width="160" height="60" rx="6"/><text class="t b" x="510" y="55" text-anchor="middle">compd čte A</text><text class="m" x="510" y="74" text-anchor="middle">při skládání</text>
<rect class="ok" x="620" y="30" width="150" height="60" rx="6"/><text class="t b" x="695" y="55" text-anchor="middle">RELEASED A</text><text class="m" x="695" y="74" text-anchor="middle">až vrstva ukáže B</text>
<path class="ln" d="M170 60 L218 60" marker-end="url(#g2)"/>
<path class="ln" d="M380 60 L428 60" marker-end="url(#g2)"/>
<path class="ln" d="M590 60 L618 60" marker-end="url(#g2)"/>
<rect class="box" x="10" y="150" width="160" height="60" rx="6"/><text class="t b" x="90" y="175" text-anchor="middle">mezitím kreslí</text><text class="m" x="90" y="194" text-anchor="middle">do bufferu B</text>
<rect class="acc" x="220" y="150" width="160" height="60" rx="6"/><text class="t b" x="300" y="175" text-anchor="middle">COMMIT</text><text class="m" x="300" y="194" text-anchor="middle">vrstva -> B</text>
<path class="ln" d="M170 180 L218 180" marker-end="url(#g2)"/>
<path class="ln dash" d="M380 180 L690 92" marker-end="url(#g2)"/>
<text class="m" x="10" y="245">Dvojitý buffer: aplikace nikdy nečeká na kompozitor, jen nesmí znovu použít buffer před RELEASED.</text>
</svg>
<figcaption>Buffer patří aplikaci, kompozitor ho jen čte mezi COMMIT a RELEASED.</figcaption>
</figure>

- Aplikace vytvoří VMO a pošle `BUFFER_CREATE` s handlem (jen s právy
  `READ`, `MAP` a základními). Kompozitor ho namapuje jen pro čtení
  (`stride * height * 4` bajtů - menší VMO mapování odmítne) a ověří
  rozměry (nejvýš 16384 px).
- Paměť bufferu platí job aplikace: aplikace nemůže vyčerpat paměť
  kompozitoru.
- Kompozitor buffer drží od `COMMIT`, který ho připojil k vrstvě, do zprávy
  `RELEASED`. Tu pošle, jakmile buffer neukazuje žádná vrstva (vrstva
  dostala jiný buffer nebo zanikla). Knihovna `ui` proto kreslí střídavě
  do dvou bufferů a na kompozitor nikdy nečeká.

## Strom vrstev

Okno je strom **vrstev**. Obsah vrstev kreslí aplikace, ale **posun,
průhlednost, ořez, scroll a animace dělá kompozitor**: posunout seznam
nebo rozsvítit tlačítko tak nevyžaduje nové vykreslení aplikací a funguje
plynule i tehdy, když aplikace zrovna počítá.

| Druh vrstvy | Obsah |
|---|---|
| `GROUP` | jen děti; volitelně ořez na svůj obdélník a posun obsahu (scroll) |
| `BUFFER` | obdélník z bufferu aplikace (zdrojový výřez se škáluje na cíl) |
| `SOLID` | jednobarevný obdélník (pozadí, oddělovače, výběr) |

Vlastnosti vrstvy (`LAYER_SET`, každá volitelná): obdélník v souřadnicích
rodiče, pořadí mezi sourozenci, průhlednost 0-255, viditelnost, ořez dětí,
posun obsahu `(sx, sy)`, buffer a zdrojový výřez, barva.

**Animace** (`ANIMATE`): vlastnost (průhlednost, posun x/y, scroll x/y),
cílová hodnota, délka v ms a křivka (`linear`, `ease-out`). Kompozitor ji
počítá v každém snímku sám; aplikace dostane `ANIMATION_DONE`.

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Strom vrstev a strom pristupnosti">
<defs><marker id="g3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="t b" x="20" y="24">strom vrstev</text>
<rect class="acc" x="20" y="36" width="160" height="40" rx="6"/><text class="t" x="100" y="61" text-anchor="middle">1 GROUP okno</text>
<rect class="box" x="20" y="100" width="150" height="40" rx="6"/><text class="t" x="95" y="125" text-anchor="middle">2 SOLID pozadí</text>
<rect class="box" x="190" y="100" width="170" height="40" rx="6"/><text class="t" x="275" y="125" text-anchor="middle">3 GROUP seznam (ořez)</text>
<rect class="box" x="190" y="170" width="170" height="40" rx="6"/><text class="t" x="275" y="195" text-anchor="middle">4 BUFFER řádky</text>
<rect class="box" x="20" y="170" width="150" height="40" rx="6"/><text class="t" x="95" y="195" text-anchor="middle">5 BUFFER tlačítko</text>
<path class="ln" d="M100 76 L95 98" marker-end="url(#g3)"/>
<path class="ln" d="M120 76 L270 98" marker-end="url(#g3)"/>
<path class="ln" d="M275 140 L275 168" marker-end="url(#g3)"/>
<path class="ln" d="M70 76 L70 168" marker-end="url(#g3)"/>
<text class="m" x="190" y="232">scroll = vlastnost vrstvy 3, aplikace nekreslí</text>
<text class="t b" x="440" y="24">strom přístupnosti</text>
<rect class="ok" x="440" y="36" width="190" height="40" rx="6"/><text class="t" x="535" y="61" text-anchor="middle">window "Poznámky"</text>
<rect class="ok" x="440" y="100" width="150" height="40" rx="6"/><text class="t" x="515" y="125" text-anchor="middle">list "Soubory"</text>
<rect class="ok" x="610" y="100" width="160" height="40" rx="6"/><text class="t" x="690" y="125" text-anchor="middle">button "Uložit"</text>
<rect class="ok" x="440" y="170" width="150" height="40" rx="6"/><text class="t" x="515" y="195" text-anchor="middle">item "a.txt" ✓</text>
<path class="ln" d="M515 76 L515 98" marker-end="url(#g3)"/>
<path class="ln" d="M560 76 L680 98" marker-end="url(#g3)"/>
<path class="ln" d="M515 140 L515 168" marker-end="url(#g3)"/>
<path class="ln-acc dash" d="M360 190 L438 190"/>
<path class="ln-acc dash" d="M170 190 Q400 260 690 142"/>
<text class="m" x="440" y="240">uzel míří na vrstvu + obdélník: kompozitor ví,</text>
<text class="m" x="440" y="256">kde prvek je, i bez aplikace (čtečka, agent, test)</text>
</svg>
<figcaption>Dva stromy jednoho okna. Vrstvy určují, co je vidět; přístupnost, co to znamená.</figcaption>
</figure>

## Strom přístupnosti

Struktura okna je součást protokolu. Uzel (`A11Y_UPDATE`) nese:

| Pole | Význam |
|---|---|
| `id`, `parent` | identita a rodič (0 = kořen okna) |
| `role` | `window`, `titlebar`, `button`, `label`, `text-input`, `checkbox`, `list`, `list-item`, `group`, `image`, `progress`, `terminal` |
| `name` | text pro člověka („Uložit“) |
| `value` | hodnota (text pole, procenta průběhu) |
| `state` | bity: `focused`, `checked`, `disabled`, `selected`, `expanded`, `busy` |
| `bounds` | vrstva + obdélník v ní |
| `actions` | bity: `activate`, `focus`, `set-value`, `toggle`, `scroll` |

Kompozitor strom drží a slouží z něj:

- **čtečce obrazovky a lupě** (později) - přes capability, ne aplikaci,
- **testům a agentovi**: „klikni na tlačítko Uložit“ jde přes uzel,
  ne přes souřadnice; kompozitor pošle aplikaci `A11Y_ACTION`,
- **dekoracím**: vlastní titulek je přístupný stejně jako standardní
  (uzel `titlebar`).

## Snímky a transakce

Stav okna je dvojí: **rozpracovaný** (zprávy od posledního `COMMIT`)
a **platný** (ten se skládá). `COMMIT` převede rozpracovaný stav na platný
**atomicky**: nikdy není vidět polovina změny. Jedna transakce smí mít víc
zpráv (limit zprávy kanálu je 64 KiB).

<figure class="diagram">
<svg viewBox="0 0 780 290" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Takt snimku">
<defs><marker id="g4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<path class="ln" d="M20 60 L760 60"/><text class="m" x="20" y="45">compd</text>
<path class="ln" d="M20 160 L760 160"/><text class="m" x="20" y="145">aplikace</text>
<path class="ln" d="M20 250 L760 250"/><text class="m" x="20" y="235">virtio-gpu</text>
<path class="ln-acc" d="M100 50 L100 70"/><text class="m" x="85" y="40">tik N</text>
<path class="ln-acc" d="M367 50 L367 70"/><text class="m" x="350" y="40">tik N+1</text>
<path class="ln-acc" d="M634 50 L634 70"/><text class="m" x="617" y="40">tik N+2</text>
<path class="ln" d="M104 64 L170 156" marker-end="url(#g4)"/><text class="m" x="120" y="100">FRAME seq, deadline</text>
<rect class="box" x="172" y="148" width="120" height="24" rx="4"/><text class="m" x="232" y="165" text-anchor="middle">kreslí, COMMIT</text>
<path class="ln" d="M292 156 L330 66" marker-end="url(#g4)"/>
<rect class="acc" x="370" y="48" width="90" height="24" rx="4"/><text class="m" x="415" y="65" text-anchor="middle">skládání</text>
<path class="ln" d="M460 66 L520 244" marker-end="url(#g4)"/><text class="m" x="490" y="140">PRESENT damage</text>
<path class="ln" d="M560 244 L600 70" marker-end="url(#g4)"/><text class="m" x="585" y="200">PRESENTED</text>
<path class="ln" d="M604 64 L680 156" marker-end="url(#g4)"/><text class="m" x="650" y="100">PRESENTED txn</text>
<text class="m" x="20" y="282">Vstup se doručuje hned (ne s taktem); skládání čeká jen na termín, ne na pomalou aplikaci.</text>
</svg>
<figcaption>Takt 60 Hz: aplikace kreslí po FRAME, kompozitor skládá na tiku, ovladač potvrdí zobrazení.</figcaption>
</figure>

- **Takt:** perioda 16,67 ms. Kompozitor tiká jen tehdy, když něco čeká
  (poškozená oblast, animace, žádost o snímek). Nic se nemění = žádné
  probuzení.
- **Žádost o snímek:** `COMMIT` s příznakem `FRAME` řekne „chci vědět, kdy
  kreslit další“. Na začátku příští periody přijde `FRAME { seq, deadline }`;
  aplikace, která stihne `COMMIT` do `deadline`, je v tom snímku.
- **Priorita vstupu:** kompozitor nejdřív vyřídí vstup (doručí ho oknu),
  teprve potom skládá. Vstup se nikdy nebatchuje do taktu.
- **Pomalá aplikace** nezdrží ostatní: skládá se její poslední platný stav.
- **`PRESENTED { txn, time }`** potvrdí zobrazení (čas od ovladače).

### Synchronizovaná změna velikosti

Kompozitor pošle `CONFIGURE { window, w, h, serial }`. Aplikace překreslí
a v `COMMIT` potvrdí `serial`. Rám okna (dekorace) a nový obsah se ukážou
**ve stejném snímku**. Když aplikace nestihne potvrdit do 200 ms, kompozitor
ukáže starý obsah v novém rámu (ořezaný nebo doplněný pozadím), takže
okno nikdy „neuteče“ z rámu.

### Transakce přes víc oken a aplikací

Pro změny, které se musí ukázat zároveň v různých oknech (okno terminálu
s obsahem z jiného procesu, rozdělení obrazovky dvou aplikací):

1. Koordinátor (aplikace nebo kompozitor) si řekne o skupinu:
   `SYNC_GROUP { count }` -> `SYNC_TOKEN { token }` (128bitové náhodné číslo).
2. Token předá ostatním účastníkům (svým kanálem).
3. Každý účastník pošle `COMMIT` s `token`. Kompozitor drží platné stavy
   všech, dokud nepřijde `count` commitů (nebo nevyprší 200 ms - pak je
   uplatní všechny a skupinu zruší, aby se nic nezaseklo).

## Okna a dekorace

`WINDOW_CREATE { id, mode, w, h, title }`. Rám, titulek a tlačítka kreslí
kompozitor podle režimu ([Vlastní vzhled okna](architektura.md#vlastní-vzhled-okna-vlastní-titulek)):

| Režim | Aplikace kreslí | Systém si nechá |
|---|---|---|
| `STANDARD` | jen obsah | rám, titulek, odznak, tlačítka |
| `CUSTOM_TITLE` | i plochu titulku | odznak na pevném místě, tlačítka (aplikace volí stranu a barvu), přesun a resize přes deklarované oblasti |
| `FRAMELESS`, `FULLSCREEN` | vše | odznak u horního okraje při najetí myší a vždy při změně oprávnění (3 s); bezpečná zkratka a dialogy vždy |

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Rezim vlastni titulek">
<rect class="box" x="20" y="20" width="740" height="200" rx="8"/>
<rect class="acc" x="20" y="20" width="740" height="36" rx="8"/>
<rect class="ok" x="28" y="26" width="150" height="24" rx="5"/><text class="m" x="103" y="43" text-anchor="middle">odznak: Poznámky · síť</text>
<rect class="box" x="196" y="26" width="110" height="24" rx="5"/><text class="m" x="251" y="43" text-anchor="middle">karta 1</text>
<rect class="box" x="312" y="26" width="110" height="24" rx="5"/><text class="m" x="367" y="43" text-anchor="middle">karta 2</text>
<rect class="warn" x="430" y="24" width="230" height="28" rx="4"/><text class="m" x="545" y="43" text-anchor="middle">oblast DRAG (deklarovaná)</text>
<rect class="err" x="672" y="26" width="24" height="24" rx="4"/><rect class="err" x="700" y="26" width="24" height="24" rx="4"/><rect class="err" x="728" y="26" width="24" height="24" rx="4"/>
<text class="m" x="680" y="76">tlačítka: systém</text>
<text class="m" x="200" y="76">karty: aplikace (vlastní titulek)</text>
<text class="t" x="390" y="140" text-anchor="middle">obsah aplikace</text>
<text class="m" x="20" y="242">Přesun okna funguje i se zaseknutou aplikací: kompozitor zná oblasti a nemusí se aplikace ptát.</text>
</svg>
<figcaption>Vlastní titulek: aplikace kreslí karty, systém odznak a tlačítka.</figcaption>
</figure>

**Deklarované oblasti** (`REGIONS`): seznam obdélníků s druhem `DRAG`
(táhne okno), `NO_DRAG` (výjimka uvnitř `DRAG`, např. karta) a `RESIZE`
(okraje). Kompozitor podle nich přesouvá a mění velikost sám, bez dotazu
na aplikaci.

**Odznak důvěry** ukazuje identitu aplikace (jméno z manifestu, které
předal init, ne aplikace) a co právě smí (síť, složky, kamera). Aplikace
ho nemůže překreslit: kompozitor ho skládá až nad obsahem okna.

**Limity na klienta** (ochrana kompozitoru před aplikací): buffer nejvýš
64 MiB a všechny buffery 256 MiB, 256 bufferů, 8192 vrstev, 32 oken,
4096 uzlů přístupnosti na okno, rozpracované zprávy do `COMMIT` nejvýš
16 MiB, titulek 256 znaků, 4 skupiny transakcí. Zprávy, které se
aplikaci nevejdou do fronty kanálu, kompozitor odloží (pohyb myši a ping
smí vypadnout, `RELEASED`, `FRAME` a `CONFIGURE` ne); kdo nevyčte 4096
odložených zpráv, je odpojen.

**Nereagující aplikace:** kompozitor posílá `PING`; bez `PONG` do 3 s
ztlumí okno a nabídne zavření. Přesun, minimalizace a zavření fungují
dál.

## Vzhled podle návrhu (fáze 1)

> **Změna proti předchozímu stavu:** dekorace, lišta, dialogy, prvky UI
> a terminál měly pevné tmavé barvy a rozměry v kódu a písmo Tuffy.
> Teď berou vše z tokenů `design/tokens.toml` a kreslí Interem
> a JetBrains Mono podle prototypů v `design/` ([Vzhled](vzhled.md)).
> Důvod: jeden zdroj pravdy pro prototypy i systém; změna barvy nebo
> rozestupu nevyžaduje zásah do kódu.

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tok tokenu do systemu">
<defs><marker id="vf1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="70" width="160" height="60" rx="6"/><text class="t b" x="90" y="96" text-anchor="middle">design/tokens.toml</text><text class="m" x="90" y="114" text-anchor="middle">vložený při překladu</text>
<path class="ln" d="M170 100 L218 100" marker-end="url(#vf1)"/>
<rect class="box" x="220" y="70" width="150" height="60" rx="6"/><text class="t b" x="295" y="96" text-anchor="middle">libs/theme</text><text class="m" x="295" y="114" text-anchor="middle">libs/conf, kontrast</text>
<path class="ln" d="M370 85 L438 40" marker-end="url(#vf1)"/><path class="ln" d="M370 100 L438 100" marker-end="url(#vf1)"/><path class="ln" d="M370 115 L438 160" marker-end="url(#vf1)"/>
<rect class="ok" x="440" y="15" width="310" height="50" rx="6"/><text class="t b" x="595" y="37" text-anchor="middle">compd: okna, lišta, dialogy</text><text class="m" x="595" y="54" text-anchor="middle">libs/comp/style.rs, render.rs</text>
<rect class="ok" x="440" y="75" width="310" height="50" rx="6"/><text class="t b" x="595" y="97" text-anchor="middle">libs/ui: prvky aplikací</text><text class="m" x="595" y="114" text-anchor="middle">look.rs (trust_*, ai_* nenabízí)</text>
<rect class="ok" x="440" y="135" width="310" height="50" rx="6"/><text class="t b" x="595" y="157" text-anchor="middle">gui-term: barvy term_*, JetBrains Mono</text><text class="m" x="595" y="174" text-anchor="middle">mřížka z type.mono</text>
</svg>
<figcaption>Tokeny jsou data: kompozitor, knihovna UI i terminál čtou stejný soubor.</figcaption>
</figure>

**Snímky z QEMU** (test `compd: vzhled podle navrhu`, `screendump` přes
QMP, 1280x800; porovnání s `design/plocha.html` a `design/okna.html`):

![Světlý motiv: okna ukázky, terminálu a správce úloh, lišta](../../design/snimky/vzhled-faze1-svetly.png)

![Tmavý motiv: aplikace se přebarví zprávou THEME, lišta s rozmazaným pozadím](../../design/snimky/vzhled-faze1-tmavy.png)

![Vysoký kontrast: bez stínů, plná lišta](../../design/snimky/vzhled-faze1-kontrast.png)

**Tokeny jako data.** `libs/theme` přečte `design/tokens.toml` přes
`libs/conf` (nově umí desetinná čísla, např. `frame_ms = 16.7`) a dá
hodnoty pro režim světlý, tmavý a vysoký kontrast: barvy (premultiplied),
typografickou škálu, rozestupy, rozměry, zaobleni, stíny, pohyb
(křivky `cubic-bezier`) a vrstvy. Test `contrast_pairs_pass` ověří
všechny povinné dvojice kontrastu ze sekce `[contrast]` stejně jako
`design/tools/build-tokens.mjs`. Výchozí motiv je vložený při překladu;
`Tokens::parse` přečte i jiný text (uživatelské motivy později).
Kompozitor přepíná režim příkazem `theme light|dark|contrast`.

**Okna** (`design/okna.html`): titulek 40 px (`size.title_h`), zaoblení
18 px (`radius.window`), rám 1 px `border`, stín podle stavu
(`shadow.window` / `window_inactive`, dvě vrstvy), aktivní titulek
`surface_2` a jméno váhy 600, neaktivní `surface` a váha 500 vedlejší
barvou. Tlačítka okna jsou pilulky 28 px s ikonou z design systému;
cíl kliknutí 40 px (`size.hit_min`), překryvy rozhoduje nejbližší
střed. Najetí zvýrazní tlačítko (`surface_3`, zavřít `danger`).
Odznak důvěry 26 px: štít, jméno z manifestu a ikony oprávnění,
barvy `trust_*`; systémová aplikace (`[app] system = true` v manifestu,
posoudí init, `ConnectInfo.system`) má odznak „Systém“ se štítem
s fajfkou. Okno bez fokusu má odznak se sytostí 0,6. Rám je 1 px, ve
vysokém kontrastu 2 px. Nové okno se umístí do pracovní plochy
(obrazovka bez lišty); pod lištu se dostane jen tažením.

**Prvky aplikací** (`libs/ui`): agent AI má vyhrazené barvy `ai_*`
a vždy ikonu agenta a slovo „agent“ (štítek `.agent()`, řádek seznamu
s levým pruhem a štítkem, průběh v barvě `ai`; ve stromu přístupnosti
jméno začíná „agent:“). Nedostupné tlačítko je stejné tlačítko s krytím
`opacity.disabled`. Vybraný řádek má pozadí `accent_soft` a normální
písmo. Rámeček fokusu se kreslí jen v aktivním okně (prvek si fokus
pamatuje). Neviditelný okraj 6 px vně okna mění velikost.
Nereagující aplikace: obsah ztlumený a karta „… nereaguje“.

**Lišta** (`design/plocha.html`, poloha „dole, odsazená“): 52 px,
odsazení 8 px, zaoblení 18 px, pod ní **rozmazané pozadí** (`blur(16px)`
jako v prototypu) a přes něj `surface` s krytím `opacity.bar`, stín
`shadow.bar`. Rozmazání se počítá v polovičním rozlišení a jen když se
pod lištou něco změní (pohyb okna, obsah, motiv), jinak se bere
z mezipaměti; najetí myší, hodiny ani kurzor ho nepřepočítají. Vlevo
spouštěč, oddělovač a tlačítka oken (ikona podle aplikace, značka
běží / aktivní), vpravo pilulka „Vstup ovládá automat“, úlohy se
štítkem agenta, **síť a hlasitost ze skutečného stavu** (`netd`: linka
a adresa, `audiod`: hlasitost a ztlumení; bez spojení nebo ztlumeno
přeškrtnutá ikona), zvonek oznámení (zatím zástupná ikona) a hodiny
(čas a datum v pásmu, které init pošle s příkazem `display`).

Stav pro lištu dává init příkazem `sources net audio` s handly: relace
sítě jen s právem `INFO` (bez spojení kamkoli) a kanál ke zvukové službě.
Kompozitor se ptá každé 2 s a odpověď čte bez blokování (zaseknutá
služba lištu nezdrží).

**Plocha:** barva `bg` a dva měkké radiální přechody `wall_a`,
`wall_b` jako `.desk` v prototypu; spočítá se jednou pro velikost
obrazovky, s jemným rozptylem proti pruhům.

**Systémový dialog:** šířka 440 px, rám 2 px `trust_border`, pruh
„Systém“ s frází, nadpis, blok aplikace, upozornění „jen fyzický
vstup“, tlačítka jako pilulky s rámečkem fokusu 2 px.

**Kreslení (`libs/gfx`):**

| Modul | Co |
|---|---|
| `shape.rs` | zaoblený obdélník a rám ze vzdálenostního pole (stejný vzorec jako shader), rohové záplaty okna |
| `shadow.rs` | Gaussův stín předpočítaný do masky z 9 dílů (3 krabičkové průchody), kreslení jen čte tabulku |
| `icon.rs` | 64 ikon přímo z `design/assets/mo.js` (podmnožina SVG: `path` včetně oblouků, `rect`, `circle`), tah vzdálenostním polem |
| `backdrop.rs` | radiální přechody plochy |
| `text.rs` | rodina řezů (výběr váhy jako v CSS), jemné umístění po 1/4 px, gama tmavého textu |

**Stejné pixely na CPU i GPU:** okno se kreslí jako čtverec, rohy se
dopočítají „záplatou“ (okno přes uložené pozadí s pokrytím zaobleného
tvaru); vícevrstvý stín se složí do jedné barvy a prolne jednou; vrstva
nad okny (lišta, dialog, kurzor) jde i na CPU přes mezibuffer. Proto
test `gpu_list_matches_cpu_frame` dál porovnává snímek pixel po pixelu.

⚠️ **Rizika a odchylky (řečená předem):**

- Písma jsou vložená do programů (každý jen řezy, které kreslí): `compd`
  +1,7 MB, aplikace s `libs/ui` +1,25 MB, `gui-term` +0,55 MB. Sdílená
  paměť s písmy od kompozitoru je úkol G5.
- Rozmazání pod lištou počítá CPU i v GPU backendu (stejné pixely).
  Navazující krok pro Venus: rozmazání shaderem ve `vkcomp`.
- Stíny ze statických masek.
- Velikost písma 200 % (Přístupnost) přijde s G4; motiv systému
  aplikace dostávají zprávou `THEME` (knihovna UI i terminál se
  přebarví).
- Hodnoty, které byly jen v `mo.css` (krytí lišty, ztlumení, šířka
  dialogu, ikony 14 px, značka v liště), jsou nově v tokenech
  (`[size]`, `[opacity]`) a kód je čte odtud.

**Cena snímku na CPU:** `compd` píše do logu čas skládání celé obrazovky
a po 300 snímcích průměr a maximum (`compd: snimek cele obrazovky ...`).

| Kde | Celá obrazovka | Poznámka |
|---|---|---|
| hostitel, 800x600 (test `look_snapshots`) | 3,5 ms, první 11 ms | první s výpočtem plochy, stínů a rozmazání |
| QEMU na Windows (emulace CPU), 1280x800 | 0,5-1,8 s | emulace bez akcelerace je ~100x pomalejší; běžné snímky jsou jen poškozené oblasti |

Kompozitor se proto při připojení displeje nejdřív ukáže jen barvou
pozadí a celý snímek složí v dalším taktu (start nečeká na skládání).

## Vstup v oknech

Kompozitor dostává události od `inputd` ([Vstup](vstup.md)) - z kanálu
fokusu i z kanálu `secure` (v bezpečném režimu jde vstup jen tam) - a
rozděluje je:

- **klávesnice** (`Key`, `Text`) -> okno s fokusem; nové okno jiné
  aplikace dostane fokus jen tehdy, když uživatel aspoň 1,5 s nepsal
  (jinak by si okno na pozadí mohlo „vzít“ rozepsané heslo),
- **myš** -> okno pod ukazatelem (přepočet na souřadnice okna); stisk
  tlačítka mimo oblasti dekorace okno aktivuje a dá mu fokus,
- **kolečko** -> okno pod ukazatelem,
- **bezpečná zkratka** (`secure`) -> systémová obrazovka,
- událost s příznakem `SYNTHETIC` (umělý vstup) jde oknu jen s příznakem;
  systémový dialog ji **ignoruje**,
- **nové vstupní zařízení** (`NewDevice`, ochrana proti BadUSB): kompozitor
  ukáže systémový dialog „Povolit zařízení?“ a do rozhodnutí události
  z toho zařízení zahazuje - zařízení se tedy nemůže povolit samo
  (vypsat Enter). Odpověď pošle `inputd` jako `trust <id>` / `reject <id>`.

Okno dostane `INPUT` se záznamy `InputRecord` (28 B): klávesa (kód
nezávislý na rozložení, akce, znak), vložený text, ukazatel v souřadnicích
okna, opuštění okna, tlačítko s polohou, kolečko a fokus; k tomu
modifikátory, příznak umělého vstupu a čas. Pohyb a uvolnění tlačítka jdou
oknu, které dostalo stisk, i když ukazatel z okna odjede.

## Systémové dialogy

Dialog (oprávnění, heslo, potvrzení akce) kreslí kompozitor ve **zvláštní
vrstvě nad ztlumenými okny**. Do té vrstvy nesmí kreslit žádná aplikace.

- Požádat o dialog smí jen služba s capability `dialog` (init ji dá
  zprostředkovateli oprávnění), nikdy aplikace sama.
- Dialog ukazuje identitu žádající aplikace, text otázky, volby
  a **bezpečnostní frázi** uživatele.
- Klávesnice a myš jdou během dialogu jen do něj; umělý vstup je
  ignorován.
- Výsledek `ANSWER { id, choice }` nebo `ANSWER { id, text }` (heslo)
  jde jen žadateli.
- U potvrzení akce dialog ukazuje skutečné parametry (cestu, příjemce,
  seznam souborů) a potvrzení se váže přesně na ně (viz [API akcí](#api-akcí)).

Zprávy kanálu `dialog` jsou binární (`guiproto::control`):

| Zpráva | Pole |
|---|---|
| `Ask` | id, druh (`permission`, `confirm`, `password`), aplikace, text, volby (1-8) |
| `Cancel` | id |
| `Phrase` | bezpečnostní fráze (nejvýš 64 znaků) |
| `Answer` (odpověď) | id, index volby, text (heslo) |
| `Cancelled` (odpověď) | id |

Klávesy v dialogu: Tab a šipky mění volbu, Enter potvrdí, Esc vybere
poslední volbu (zamítnutí). Myš: klik na tlačítko. Prvních 300 ms po
zobrazení dialog vstup ignoruje - druhý klik dvojkliku nebo klávesa
napsaná do jiného okna tak nepotvrdí dialog, který právě vyskočil.

**Bezpečná obrazovka:** Ctrl+Alt+Delete (zachytí `inputd`, ne aplikace)
ukáže systémovou obrazovku přes všechno. Zavře ji jen fyzický vstup;
kompozitor pak pošle `inputd` zprávu `release` a fokus se vrátí.

## API akcí

Aplikace v manifestu deklaruje **akce** s typovanými parametry
a oprávněními. Volá je shell, průzkumník, asistent i agent - vždy přes
kompozitor, který parametry ověří proti manifestu dřív, než je aplikace
dostane.

```toml
[[action]]
id = "note.create"
title = "Nová poznámka"
needs = ["fs:write"]
confirm = false          # true = potvrdit systémovým dialogem

[[action.param]]
name = "title"
type = "text"            # text, int, bool, path, paths, choice
required = true

[[action.param]]
name = "color"
type = "choice"
choices = ["zluta", "modra"]
```

| Zpráva | Směr | Obsah |
|---|---|---|
| `List` | volající -> compd | odpověď `Actions`: akce běžících aplikací (id aplikace, deklarace) |
| `Call` | volající -> compd | id volání, aplikace, akce, argumenty (typované hodnoty), příznak automatu |
| `ACTION_INVOKE` | compd -> aplikace | ověřené argumenty, id volání, kdo volá |
| `ACTION_RESULT` | aplikace -> compd | `ok` + hodnota, nebo text chyby |
| `Result` | compd -> volající | výsledek s id volání volajícího |

Akce s `confirm = true` nebo volaná z umělého zdroje (agent) se před
předáním potvrdí systémovým dialogem. Volající vidí jen akce, ne okna.

**Potvrzuje se to, co je vidět.** Dialog kreslí systém a ukazuje
**skutečné argumenty** podle jejich typu z manifestu, ne jen název akce:

| Typ | V dialogu |
|---|---|
| `path` | celá cesta (dlouhá se zalomí po znacích, nic se neschová) |
| `paths` | počet položek a náhled prvních 8 celých cest, „... a dalších N“ |
| `text` | hodnota v uvozovkách (příjemce, cíl v síti, zpráva), nejvýš 300 znaků |
| `int`, `bool`, `choice` | hodnota (`ano`/`ne` u `bool`) |

Kompozitor si při zobrazení dialogu uloží zakódované argumenty (celé,
porovnávají se bajt po bajtu - otisk by šel úmyslně zkolidovat) a aplikaci po potvrzení předá **přesně ty argumenty,
které uživatel viděl**. Když by se mezi zobrazením a potvrzením změnily,
volání se odmítne („argumenty se po zobrazení změnily“) a aplikace nic
nedostane. Volající nemá zprávu, kterou by argumenty po potvrzení změnil.

```text
Systém: potvrzení
asistent (automat) chce v aplikaci Pošta provést akci „Odeslat přílohu“.
Akce potřebuje: síť, soubory.
Parametry:
recipient: "jana@example.com"
file (cesta): /home/michal/Dokumenty/rozpocet-2027-final.xlsx
                                        [Povolit]  [Zamítnout]
```

## Řídicí příkazy kompozitoru

Init mluví s `compd` po bootstrap kanálu textovými příkazy (jako s
`inputd`); odpověď je `ok`, `ok <data>` nebo `err <KÓD>`.

| Příkaz | Handly | Odpověď |
|---|---|---|
| `display [pásmo]` | kanál ovladače grafiky | `ok <w> <h>`; pásmo IANA pro hodiny v liště |
| `input` | kanál fokusu od `inputd` | `ok` |
| `secure` | kanál bezpečné zkratky od `inputd` | `ok` |
| `connect` + identita aplikace (binárně) | - | `ok` + kanál okna |
| `dialog` | - | `ok` + kanál systémových dialogů |
| `actions <volající> [automat]` | - | `ok` + kanál volání akcí; `automat` = kanál agenta, každé volání se potvrzuje dialogem (příznak ve zprávě volající neobejde) |
| `inspect` | - | `ok` + kanál inspekce |
| `status` | - | `ok clients=1 windows=2 frames=.. presented=.. dialogs=0 secure=0` |
| `tasks <běží> <agent>` | - | `ok` (ikona úloh v panelu) |
| `theme light\|dark\|contrast` | - | `ok` (režim barev z tokenů) |
| `sources [net] [audio]` | relace sítě (`INFO`), kanál zvuku | `ok` (stav v liště) |

**Inspekce** (`inspect`, pro testy, později čtečku obrazovky a lupu):
seznam oken, strom přístupnosti okna v souřadnicích obrazovky, akce přes
uzel, barva pixelu obrazovky a „počkej na ustálení“ (vše složeno
a zobrazeno).

## Protokol okna (`libs/guiproto`)

Zpráva = kód operace `u32` + pole kódovaná `libs/wire` (little-endian,
řetězec = délka `u16` + UTF-8). Neznámá operace je chyba `NOT_SUPPORTED`
(odpověď `ERROR`), špatná data `INVALID_ARGS`; kompozitor zprávu zahodí
a klienta neodpojí, dokud chyb nepřibývá (limit 64).

Aplikace -> kompozitor:

| Op | Jméno | Pole |
|---|---|---|
| 1 | `HELLO` | verze |
| 2 | `WINDOW_CREATE` | okno, režim, w, h, titulek |
| 3 | `WINDOW_SET` | okno, titulek, min. velikost, strana a barva tlačítek |
| 4 | `WINDOW_DESTROY` | okno |
| 5 | `BUFFER_CREATE` | buffer, w, h, stride (v pixelech) + handle VMO |
| 6 | `BUFFER_DESTROY` | buffer |
| 7 | `LAYER_CREATE` | vrstva, okno, rodič, druh |
| 8 | `LAYER_SET` | vrstva, maska vlastností, hodnoty |
| 9 | `LAYER_DESTROY` | vrstva (i s dětmi) |
| 10 | `DAMAGE` | buffer, obdélník |
| 11 | `ANIMATE` | vrstva, vlastnost, cíl, délka, křivka |
| 12 | `A11Y_UPDATE` | uzly |
| 13 | `A11Y_REMOVE` | id uzlů |
| 14 | `REGIONS` | okno, obdélníky s druhem |
| 15 | `COMMIT` | transakce, příznaky (`FRAME`), serial z `CONFIGURE`, token skupiny |
| 16 | `SYNC_GROUP` | počet účastníků |
| 17 | `PONG` | serial |
| 18 | `ACTION_RESULT` | volání, výsledek |

Kompozitor -> aplikace:

| Op | Jméno | Pole |
|---|---|---|
| 100 | `WELCOME` | verze, w a h obrazovky, škála v procentech |
| 101 | `CONFIGURE` | okno, w, h, stav (fokus, maximalizace), serial |
| 102 | `FRAME` | seq, deadline (ns), perioda |
| 103 | `PRESENTED` | transakce, čas |
| 104 | `RELEASED` | buffer |
| 105 | `INPUT` | okno, záznamy vstupu |
| 106 | `CLOSE` | okno (uživatel klikl na zavřít) |
| 107 | `PING` | serial |
| 108 | `SYNC_TOKEN` | token |
| 109 | `ANIMATION_DONE` | vrstva |
| 110 | `A11Y_ACTION` | uzel, akce, hodnota |
| 111 | `ACTION_INVOKE` | volání, akce, argumenty |
| 112 | `ERROR` | op, kód |
| 113 | `THEME` | režim barev u8 (0 světlý, 1 tmavý, 2 vysoký kontrast); po `WELCOME` a při změně |

Příklad (okno s jedním bufferem):

```text
-> HELLO 1
<- WELCOME 1 1280 800 100
-> WINDOW_CREATE 1 STANDARD 400 300 "Ukázka"
-> BUFFER_CREATE 1 400 300 400         + handle VMO
-> LAYER_CREATE 1 okno=1 rodič=0 BUFFER
-> LAYER_SET 1 rect=0,0,400,300 buffer=1
-> A11Y_UPDATE [1 window "Ukázka"] [2 button "OK" vrstva 1 (150,250,100,30)]
-> COMMIT 1 FRAME
<- CONFIGURE 1 400 300 focused serial=1
<- PRESENTED 1
<- FRAME 1 deadline=...
```

## Protokol displeje (`guiproto::display`)

Kompozitor <-> ovladač grafiky. Ovladač je jednoduchý: kompozitor kreslí
celý obraz do své VMO a říká, které obdélníky se změnily.

| Op | Směr | Pole |
|---|---|---|
| `INFO` | compd -> ovladač | - |
| `MODES` | ovladač -> compd | výstupy: w, h, zapnutý; schopnosti (`VIRGL`, `VENUS`) |
| `ATTACH` | compd -> ovladač | výstup, w, h, stride (v bajtech) + handle VMO (jen čtení) |
| `PRESENT` | compd -> ovladač | seq, obdélníky |
| `PRESENTED` | ovladač -> compd | seq, čas (ns) |
| `SYNC` | compd -> ovladač | token (u64) - značka po restartu kompozitoru |
| `SYNCED` | ovladač -> compd | token; vše před ní patří předchozí instanci ([Obnova](#obnova-po-pádu-kompozitoru)) |
| `OPEN_RENDER` | vlastník displeje -> ovladač | - |
| `RENDER` | ovladač -> vlastník | + handle kanálu render uzlu ([Venus](venus.md#render-uzel-gpuproto)) |

Ovladač zkopíruje poškozené obdélníky do svých bufferů DMA (s IOMMU jediná
paměť, kam smí zařízení zapisovat a číst), pošle `TRANSFER_TO_HOST_2D`
a `RESOURCE_FLUSH` a po dokončení odpoví `PRESENTED`.

## Ovladač virtio-gpu

Inicializace jako u ostatních virtio ([Vstup](vstup.md#ovladač-virtio-input)):
vlastnosti `VERSION_1`, `ACCESS_PLATFORM` (a `VIRGL`, když ho zařízení
nabídne), přerušení MSI-X a fronta 0 (řízení); fronta kurzoru se zatím
nepoužívá (kurzor kreslí kompozitor). Příkazy 2D:

1. `GET_DISPLAY_INFO` - velikost výstupů,
2. `RESOURCE_CREATE_2D` (formát `B8G8R8X8_UNORM`, výstup alfu nepotřebuje),
3. `RESOURCE_ATTACH_BACKING` - seznam kusů DMA paměti (každý nejvýš
   4 MiB souvislé paměti, vždy celé řádky),
4. `SET_SCANOUT`,
5. pro každý snímek `TRANSFER_TO_HOST_2D` po obdélnících + jeden
   `RESOURCE_FLUSH`.

Ovladač spouští `devmgr` pro PCI `1af4:1050` s limitem 40 MiB DMA paměti
a initu hlásí zprávu `display`. V QEMU je zařízení
`virtio-gpu-pci,iommu_platform=on,xres=1280,yres=800` na x86_64
i aarch64.

## Displeje: virtio-gpu a fbdisplay

Kompozitor kreslí celý snímek do své VMO a ovladači displeje posílá jen
poškozené obdélníky (`guiproto::display`). Ovladač tak jde vyměnit bez
změny kompozitoru. Jsou dva:

| | `virtio-gpu` | `fbdisplay` |
|---|---|---|
| kde | QEMU (a jiné hypervizory s virtio-gpu) | cokoli s UEFI GOP: VMware, Hyper-V gen2, VirtualBox, skutečné PC; v QEMU `--gpu none` |
| kdo spouští | `devmgr` (PCI `1af4:1050`), zpráva `display` | `init`, když `display` nepřijde |
| rozlišení | z `GET_DISPLAY_INFO` (zatím pevné po startu) | **pevné**: režim, který nastavil firmware (loader ho nemění) |
| kopie | do bufferů DMA + `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH` | přímý zápis do paměti obrazovky (write-combining), po řádcích |
| formát | `B8G8R8X8_UNORM` (jako kompozitor) | RGB nebo BGR podle firmwaru; převod a odstranění alfy při kopii |
| akcelerace, vsync | virgl/Venus jako další krok; `PRESENTED` po flush | **žádná**; `PRESENTED` hned po kopii (bez čekání na vsync) |
| víc monitorů | víc výstupů (scanout) | jeden (výstup 0) |

**Přednost má virtio-gpu.** Loader proto už při startu vybírá GOP, který
není virtio-gpu (`loader/src/pci_id.rs`): na QEMU x86_64 VGA, na aarch64
ramfb. Bez GOP (nebo s nepodporovaným formátem, např. `PixelBitMask`)
framebuffer není a bez virtio-gpu GUI nenaběhne (`init: grafika
(virtio-gpu ani framebuffer) neni`).

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Výběr ovladače displeje">
<defs><marker id="fbd1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">loader (UEFI)</text><text class="m" x="110" y="64" text-anchor="middle">vybere GOP mimo virtio-gpu</text><text class="m" x="110" y="80" text-anchor="middle">fyz. adresa, w, h, stride, RGB/BGR</text>
<rect class="box" x="10" y="130" width="200" height="80" rx="6"/><text class="t b" x="110" y="155" text-anchor="middle">jádro</text><text class="m" x="110" y="174" text-anchor="middle">log do fb do FB_CONTROL CLAIM</text><text class="m" x="110" y="190" text-anchor="middle">bootstrap FRAMEBUFFER initu</text><text class="m" x="110" y="204" text-anchor="middle">(VMO write-combining)</text>
<path class="ln" d="M110 90 L110 128" marker-end="url(#fbd1)"/>
<rect class="acc" x="290" y="130" width="200" height="80" rx="6"/><text class="t b" x="390" y="155" text-anchor="middle">init</text><text class="m" x="390" y="174" text-anchor="middle">devmgr hlásí display?</text><text class="m" x="390" y="190" text-anchor="middle">ano: virtio-gpu</text><text class="m" x="390" y="204" text-anchor="middle">ne: spustí fbdisplay</text>
<path class="ln" d="M210 170 L288 170" marker-end="url(#fbd1)"/>
<rect class="box" x="570" y="20" width="200" height="70" rx="6"/><text class="t b" x="670" y="45" text-anchor="middle">virtio-gpu</text><text class="m" x="670" y="64" text-anchor="middle">DMA + TRANSFER + FLUSH</text><text class="m" x="670" y="80" text-anchor="middle">(přednost)</text>
<rect class="box" x="570" y="130" width="200" height="80" rx="6"/><text class="t b" x="670" y="155" text-anchor="middle">fbdisplay</text><text class="m" x="670" y="174" text-anchor="middle">kopie obdélníků do fb</text><text class="m" x="670" y="190" text-anchor="middle">převod RGB/BGR, bez alfy</text><text class="m" x="670" y="204" text-anchor="middle">pevné rozlišení</text>
<path class="ln" d="M490 150 L568 70" marker-end="url(#fbd1)"/><text class="m" x="500" y="100">display</text>
<path class="ln" d="M490 175 L568 175" marker-end="url(#fbd1)"/><text class="m" x="500" y="168">jinak</text>
<rect class="ok" x="290" y="240" width="200" height="50" rx="6"/><text class="t b" x="390" y="262" text-anchor="middle">compd</text><text class="m" x="390" y="279" text-anchor="middle">stejný protokol pro oba</text>
<path class="ln" d="M390 210 L390 238" marker-end="url(#fbd1)"/><text class="m" x="398" y="228">kanál displeje (gui::DISPLAY)</text>
</svg>
<figcaption>Výběr ovladače displeje. Kompozitor neví, který z nich běží.</figcaption>
</figure>

### Kdo vlastní framebuffer

Jádro dá initu framebuffer ve startovní zprávě (druh `FRAMEBUFFER`, viz
[Syscally](syscally.md#framebuffer-z-firmwaru)) a kreslí do něj log, dokud
ho vlastník výslovně nepřevezme ([`FB_CONTROL`](syscally.md#fb_control)
`CLAIM`). Po `RELEASE` nebo pádu vlastníka jádro obrazovku smaže a kreslí
log dál; panika jádra kreslí vždy.

O vlastníkovi rozhoduje init na jednom místě (`gui::display`):

| Situace | Vlastník framebufferu |
|---|---|
| virtio-gpu je | nikdo z GUI (obraz jde přes virtio-gpu); framebuffer zůstává pro záchrannou konzoli |
| virtio-gpu není | `fbdisplay` pod kompozitorem (init mu dá duplikát handlu) |
| GUI neběží nebo spadne | záchranná konzole `user/console` (příkaz `display`), viz [Shell](navrh-shell.md) |

Kompozitor skončí -> `fbdisplay` dostane zavřený kanál a skončí; jádro mu
obrazovku vezme zpět při konci procesu (i při pádu), ale jen když ji pořád
drží. Výslovný `RELEASE` `fbdisplay` nevolá: jádro neověřuje volajícího
a po pádu kompozitoru už obrazovku mohla převzít záchranná konzole. `Startup::framebuffer`
initu zůstává, takže ji init může hned předat konzoli.

### Ovladač fbdisplay

`user/fbdisplay` dostane od initu objekt paměti framebufferu a kanál
displeje (kind USER). Geometrii zjistí sám (`FB_CONTROL` `INFO`),
framebuffer namapuje a pak obrazovku převezme (`CLAIM`). Protokol:

1. `INFO` -> `MODES` s jediným výstupem o rozlišení firmwaru, schopnosti 0,
2. `ATTACH` s VMO kompozitoru (jen čtení); jiná velikost než framebuffer
   = `NOT_SUPPORTED`, po připojení se překreslí celá obrazovka,
3. `PRESENT` -> obdélníky se ořežou na obrazovku (`normalize_rects`)
   a zkopírují po řádcích (`fbcopy::copy_rect`): u BGR jen vynulovaná
   alfa, u RGB prohozené R a B; pak bariéra (vyprázdní buffery
   write-combining) a `PRESENTED` s časem.

Framebuffer se mapuje **write-combining** (x86_64: položka PAT 1, kterou
jádro přepíše z WT na WC; aarch64: Normal Non-cacheable). Zápisy se
slučují do celých řádků sběrnice, čtení je pomalé - proto ovladač
z framebufferu nikdy nečte.

**Výkon a omezení:**

- celá obrazovka 1920 x 1080 = 8 MiB na snímek; s write-combining to
  skutečné PC zvládne za jednotky ms (jde o rychlost sběrnice ke grafice).
  Mapování bez cache (UC) by bylo řádově pomalejší - proto WC.
- ⚠️ **Rozlišení je pevné**: GOP po `ExitBootServices` režim měnit nedovolí.
  Jiné rozlišení jen volbou ve firmwaru nebo (později) v loaderu před
  startem jádra.
- ⚠️ **Bez vsync**: snímek se může roztrhnout (kopie běží v době
  vykreslování monitoru). Firmware nedává žádné přerušení snímku.
- ⚠️ **Mapování framebufferu i v přímé mapě jádra** má typ cache podle MTRR
  firmwaru, proces ho má WC. Dvě mapování s různým typem jsou na x86
  formálně nedefinovaná; jádro po `CLAIM` přes svoje mapování nepíše
  (jen při panice), takže v praxi nevadí.

## Nativní knihovna UI (`libs/ui`)

Jedna knihovna pro všechny aplikace, deklarativní v Rustu:

```rust
impl App for Poznamky {
    type Msg = Msg;
    fn title(&self) -> String { "Poznámky".into() }
    fn view(&self) -> Node<Msg> {
        column([
            label("Poznámky").heading(),
            text_input(&self.title).placeholder("Název").on_change(Msg::Title),
            row([
                checkbox("Důležitá", self.important).on_toggle(Msg::Important),
                spacer(),
                button("Uložit").on_click(Msg::Save),
            ]),
            scroll(list(self.items.clone(), self.selected).on_select(Msg::Select)).height(150),
        ])
    }
    fn update(&mut self, m: Msg) { /* změna stavu */ }
}

ui::client::run(kanal_okna, Poznamky::default(), Mode::Standard, 420, 380)
```

Knihovna z popisu sestaví prvky, rozvrhne je (sloupce, řádky, pružné
prvky `grow` a `spacer`), vykreslí je do bufferů (`libs/gfx`, `libs/font`)
a odvodí z nich **strom vrstev i strom přístupnosti** - aplikace nic
z toho nepíše ručně. Vstup (klávesy, text, myš, akce přístupnosti)
převede na zprávy `Msg`.

| Prvek | Myš | Klávesnice | Přístupnost |
|---|---|---|---|
| `label` | - | - | `label` |
| `button` | klik | Enter, mezerník | `button`, `activate` |
| `text_input` | klik = kurzor | text, Backspace, Delete, šipky, Home, End, Enter = `on_submit` | `text-input`, `set-value` |
| `checkbox` | klik | mezerník, Enter | `checkbox`, `toggle` |
| `list` | klik na položku | šipky nahoru a dolů | `list` a `list-item` |
| `progress` | - | - | `progress` (hodnota v procentech) |
| `column`, `row`, `scroll`, `spacer` | kolečko v `scroll` | - | `group` |

Fokus se posouvá klávesou Tab (Shift+Tab zpět). **Posouvaná oblast**
(`scroll`) je vlastní vrstva: kolečko změní jen vlastnost vrstvy
`scroll` a obsah posune kompozitor, aplikace nic nepřekresluje.

Běhové prostředí `ui::client` (vlastnost `os`) drží okno: dvojité buffery
ve VMO, vrstvy, celý strom přístupnosti po každé změně, `COMMIT`
s potvrzením `CONFIGURE`, odpovědi na `PING` a akce. Nižší vrstva
`ui::client::Window` slouží aplikacím, které kreslí samy (okno terminálu).

## Písmo (`libs/font`)

Vlastní čtení TrueType (`.ttf`): tabulky `head`, `hhea`, `maxp`, `cmap`
(formáty 4 a 12), `loca`, `glyf` (jednoduché i složené glyfy), `hmtx`,
`kern` (formát 0), párový kerning z `GPOS` (lookup typu 2, formáty 1 a 2,
i v rozšíření typu 9) a jednoduché záměny z `GSUB` pro zapnuté vlastnosti
(`tnum` tabulkové číslice, `cv11`). Rasterizér převede kvadratické křivky na úsečky
a spočítá **plošné pokrytí pixelu** (vyhlazení bez hintingu). Glyfy se
ukládají do mezipaměti podle velikosti (`gfx::text`).

Soubor písma je nedůvěryhodný vstup: každé čtení kontroluje meze, vadný
glyf je prázdný glyf, nikdy panika (test přepisuje náhodné bajty souboru).
Bez shapingu (arabština, indická písma) a bez hintingu.

Písma systému (`libs/font/fonts/`, licence v `THIRD-PARTY.md`):
**Inter** 300-700 (SIL OFL 1.1) pro UI, **JetBrains Mono** 400 a 600
(SIL OFL 1.1) pro terminál a **Tuffy** (public domain) jako záloha.
Program si cargo vlastnostmi `inter-*`, `mono-*` vloží jen řezy, které
kreslí. Váha z tokenů se mapuje na řez jako v CSS (650 -> 700).
Statické řezy (ne proměnné písmo), protože rasterizér čte `glyf`.

## Okno terminálu

Terminál 6b je v GUI oknem `gui-term` nad svým nativním protokolem:
**vlastní titulek** (aplikace kreslí kartu relace „Relace N“ a deklaruje oblast `DRAG` s výjimkou `NO_DRAG` pro kartu,
odznak a tlačítka kreslí systém), obsah je mřížka znaků `term::screen`
(písmo JetBrains Mono, barvy `term_*` z tokenů, titulek 40 px s kartou
relace) a strom přístupnosti má
uzel `terminal` s viditelným textem. Řádek jde zadat klávesnicí, akcí
přístupnosti `set-value` nebo akcí `terminal.run` (vyžaduje potvrzení).

Obsah je interaktivní terminál `term::ui` (editor se zvýrazněním,
doplňování, bloky výstupu, tabulky, průběhy, výzvy) nad službou relací
`termd`. Kanál k ní okno nedostane přímo: init mu dá kanál připojení a na
`connect` spustí termd; po pádu termd se okno připojí znovu ke stejné
relaci (viz [Init: Terminál](init.md#terminál)).

## Správce úloh

Okno se stejnými daty jako příkaz `tasks` v terminálu (viz [Přehled
úloh](architektura.md#přehled-úloh-odsouhlaseno)) a ikona „běží N úloh“
v panelu kompozitoru.

| Část | Kde | Co dělá |
|---|---|---|
| protokol | `libs/taskproto` (návrh, odsouhlasuje vlákno shellu, které dělá `taskd`) | seznam úloh, odběr změn se snímkem a pořadovým číslem (navázání po výpadku), akce zrušit, pozastavit, pokračovat, výpis; registrace úloh producenty |
| model a okno | `libs/taskview` (testy na hostiteli), program `gui-tasks` | seznam (úlohy agenta nahoře), filtr „jen agent“ a „dokončené“, detail s průběhem a spotřebou z jobu, tlačítka akcí |
| ikona v panelu | `libs/comp` | „Úlohy: N“, oranžově „Úlohy: N (agent M)“, když běží úloha agenta AI; klik dá správce úloh do popředí |

- **Úlohy agenta AI** jsou výrazně označené: štítek „[AGENT AI]“ v seznamu,
  oranžový štítek „Spustil agent AI (kdo)“ v detailu, počet v záhlaví
  a oranžová ikona v panelu.
- **Oprávnění:** tlačítko je aktivní jen tehdy, když akci `taskd`
  volajícímu povolil (bity `actions` úlohy) a dovoluje ji stav úlohy.
  Rozhoduje vždy `taskd` (odpověď `ACCESS_DENIED`); systémová úloha bez
  práva má všechna tlačítka neaktivní a okno to řekne.
- **Teď:** dokud `taskd` není v systému, okno běží nad testovacími daty
  (`TestBackend` se chová jako taskd: kontrola práv, změny stavu
  událostmi) a počet pro ikonu posílá init řídicím příkazem
  `tasks <běží> <agent>`. Až bude `taskd`, dostane init od něj kanál
  (`connect`) pro okno a kompozitor (jen čtení).

```text
Úlohy: běží 4                         [Agent AI: 1 běžících úloh]
[x] Jen úlohy agenta   [x] Zobrazit dokončené
[AGENT AI] Refaktorace projektu (klon) - běží 15 %
Kopírování 3 souborů do /zaloha - běží 40 %
Scrub svazku data - běží 12 %
build.sh - čeká: Přepsat výstup? [a/n]
Stažení aktualizací - hotovo: 3 balíky
```

## 3D: Venus a virgl (stav)

Cílové API systému je Vulkan, ve virtuálním stroji tedy **Venus** (Vulkan
přes virtio-gpu). Přechodně **virgl** (Gallium/OpenGL hostitele) jen pro
skládání kompozitoru.

| Co | Stav |
|---|---|
| `cargo xtask run/test --gpu 2d\|gl\|venus\|none` | volba zařízení: `virtio-gpu-pci`, `virtio-gpu-gl-pci` (virgl), `virtio-gpu-gl-pci,venus=on,blob=on,hostmem=256M`; bez okna displej `egl-headless`; `none` = bez virtio-gpu (GUI přes `fbdisplay`). **`run` s oknem bez `--gpu` = `none`:** okno QEMU ukazuje první kartu (VGA) a na ni je navázaný vstup; s virtio-gpu by kompozitor kreslil na druhou konzoli (v okně QEMU View → virtio-gpu-pci) |
| ovladač | vyjedná `VIRGL`, `RESOURCE_BLOB`, `CONTEXT_INIT` a vypíše capsety, např. `virgl(v1) virgl2(v2)` |
| `vgpu::virgl` | vlastní kodér proudu příkazů virgl (stavové objekty, shadery TGSI, vrcholy, kreslení, scissor) a příkazy 3D virtio-gpu, testy na hostiteli |
| skládání přes GPU | čeká: na tomto hostiteli (QEMU 11.1 pro Windows) se se zapnutým OpenGL (`egl-headless`, `gtk,gl=on`, `sdl,gl=es`) virtuální stroj po startu jádra zastaví |
| Venus | QEMU pro Windows ho nemá („Render server support was not enabled in virglrenderer“). **Funguje ve WSL2:** `cargo xtask run --qemu-wsl --gpu venus` (QEMU 10.1 a virglrenderer s render serverem přeložené v `~/venus/prefix`, Vulkan hostitele lavapipe, displej GTK pod Xvfb); ovladač hlásí capset `venus(v0)` |
| vlastní ovladač Venus | kroky 1-4 hotové (skládání kompozitoru na GPU: `backend gpu`); dřív krok 1: render uzel v ovladači, knihovna `libs/venus` (generovaná z vk.xml), test vypíše jméno fyzického zařízení - viz [Vulkan přes Venus](venus.md) |

⚠️ **Kde testovat Venus:** Linux s fyzickou GPU a QEMU + virglrenderer
s render serverem (nebo crosvm). WSL2 jen s Vulkanem přeloženým přes
D3D12 (nejisté). VMware ne - jeho SVGA3D je jiný protokol. Na skutečném
hardwaru Vulkan přinese port Mesy (milník 7).

## Obnova po pádu kompozitoru

Když `compd` spadne, supervisor initu ho spustí znovu (společný protokol
[Obnova služeb](obnova-sluzeb.md)) a **okna se vrátí sama** se stejnou
identitou (odznak důvěry, id aplikace). Aplikace nic dělat nemusí:
obnovu dělá `ui::client`.

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Obnova oken po pádu kompozitoru">
<defs><marker id="gr1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="gr1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="210" height="120" rx="6"/><text class="t b" x="115" y="45" text-anchor="middle">aplikace (ui::client)</text><text class="m" x="115" y="66" text-anchor="middle">záznam stavu okna</text><text class="m" x="115" y="84" text-anchor="middle">(guiproto::replay)</text><text class="m" x="115" y="102" text-anchor="middle">VMO bufferů drží aplikace</text><text class="m" x="115" y="120" text-anchor="middle">kanál okna + lístek</text>
<rect class="err" x="290" y="20" width="180" height="50" rx="6"/><text class="t b" x="380" y="50" text-anchor="middle">compd gen 1 (spadl)</text>
<rect class="ok" x="290" y="110" width="180" height="60" rx="6"/><text class="t b" x="380" y="135" text-anchor="middle">compd gen 2</text><text class="m" x="380" y="155" text-anchor="middle">SYNC, ATTACH, fokus</text>
<rect class="acc" x="550" y="20" width="200" height="150" rx="6"/><text class="t b" x="650" y="45" text-anchor="middle">supervisor (init)</text><text class="m" x="650" y="68" text-anchor="middle">adaptér CompdService</text><text class="m" x="650" y="86" text-anchor="middle">recept = ConnectInfo</text><text class="m" x="650" y="104" text-anchor="middle">z manifestu</text><text class="m" x="650" y="122" text-anchor="middle">Kept: konec k ovladači</text><text class="m" x="650" y="140" text-anchor="middle">Dep::notify(inputd)</text>
<rect class="box" x="550" y="220" width="200" height="60" rx="6"/><text class="t b" x="650" y="245" text-anchor="middle">ovladač displeje</text><text class="m" x="650" y="264" text-anchor="middle">pád nepozná</text>
<path class="ln dash" d="M220 45 L288 45" marker-end="url(#gr1)"/><text class="m" x="225" y="35">1. PEER_CLOSED</text>
<path class="ln-acc" d="M150 140 C 250 260, 480 250, 548 160" marker-end="url(#gr1a)"/><text class="m" x="230" y="245">2. RECONNECT na lístku</text>
<path class="ln" d="M548 140 L472 140" marker-end="url(#gr1)"/><text class="m" x="480" y="190">3. connect(recept)</text>
<path class="ln-acc" d="M290 150 L222 120" marker-end="url(#gr1a)"/><text class="m" x="225" y="175">4. HELLO + stav okna</text>
<path class="ln" d="M650 170 L650 218" marker-end="url(#gr1)"/><text class="m" x="656" y="200">duplikát konce</text>
</svg>
<figcaption>Pád kompozitoru: aplikace dostane přes lístek novou relaci se stejnou identitou a pošle znovu stav okna.</figcaption>
</figure>

| Krok | Kdo | Co |
|---|---|---|
| 1 | supervisor | adaptér `CompdService` (`user/init/src/gui.rs`) spustí novou instanci a pošle jí `display` s duplikátem konce kanálu k ovladači displeje (držák konců `Kept`) |
| 2 | `compd` | pošle ovladači `SYNC` s náhodným tokenem a zahodí vše, co přijde před `SYNCED` (pozdní odpovědi staré instance), pak `INFO` a znovu `ATTACH` (nová VMO snímku nahradí starou) |
| 3 | adaptér | odpověď na `display` = potvrzení startu; pak nový fokus a kanál `secure` od `inputd` (`take_input`) |
| 4 | aplikace | `ui::client` vidí `PEER_CLOSED`, zavolá `conn.recover()`: supervisor přes lístek vydá relaci podle receptu (`ConnectInfo` z manifestu - identitu dává init, ne aplikace) |
| 5 | aplikace | `HELLO` a poslední **potvrzený** stav okna ze záznamu (`guiproto::replay`): okno, buffery (VMO drží aplikace, pošle je znovu jen pro čtení), vrstvy s vlastnostmi, titulek, oblasti a strom přístupnosti; po `CONFIGURE` `COMMIT` (stejná velikost) nebo událost `Resized` |
| 6 | aplikace | událost `Restored { generation }`; `run` okno nakreslí znovu |

Restart `inputd` (`Dep::notify("inputd")`) kanál fokusu ani `secure`
nezavře: konce na straně `inputd` drží init v držáku konců a předá je nové
instanci. Kompozitor dostane `Focus(false)` (ukončí tažení, zapomene
stisk a oknu s fokusem pošle `Focus(false)` - aplikace pustí klávesy)
a pak `Focus(true)`. Okna zůstanou.

**Co se neobnoví** (pravidla obnovy, odsouhlaseno):

| Rozpracované | Výsledek |
|---|---|
| transakce (zprávy od posledního `COMMIT`) | zahodí se; volání vrátí `SERVICE_RESTARTED`, `run` okno nakreslí znovu (aplikace kreslící sama, např. `gui-term`, totéž) |
| `COMMIT` | záznam ho uplatní: po obnově je stav okna stejný, jako kdyby ho stará instance přijala |
| odpověď na akci (`ACTION_RESULT`) | `SERVICE_RESTARTED`: volající patřil staré instanci a odpověď nedostal; akce v aplikaci proběhla |
| skupina transakcí (`SYNC_GROUP`), animace | token skupiny patří staré instanci; animace skončí rovnou v cílové hodnotě |
| systémový dialog, bezpečná obrazovka | zmizí s instancí; volající (`dialog`, `actions`) vidí `PEER_CLOSED` |

**Pořadí handlů aplikace s oknem** (startovní zpráva, kind USER): 0 = kanál
okna, 1 = lístek, 2+ = handly aplikace (`gui-tasks`: kanál na `taskd`).
`ui::client::Link::from_handles` vezme první dva. Bez supervisoru (testy)
dostane aplikace lístek bez druhého konce: pád kompozitoru okno ukončí.

**Nikdy nevisí**: čekání na lístek má termín (`Conn`, výchozí 10 s), pozdrav
nové instanci 5 s, nejvýš 3 pokusy, když spadne i nová instance. Když to
supervisor vzdá (víc než 5 pádů za 60 s), aplikace dostane
`SERVICE_UNAVAILABLE` a skončí.

**Rizika**: obnova posílá celý stav okna (strom přístupnosti velkého okna
jsou desítky kB, buffery jen handly - pixely se nekopírují). Okno je po
obnově vidět s posledním potvrzeným obsahem, i když aplikace zrovna
kreslila nový (o snímek starší obraz). Kompozitor nové instance okno umístí
znovu (poloha se zatím nepamatuje).

## Stav a odchylky od návrhu

| Co | Teď | Proč to nevadí / kdy jinak |
|---|---|---|
| vykreslování | softwarové skládání nad virtio-gpu 2D | 3D (virgl) je další krok, viz rizika |
| QoS `interactive` pro kompozitor | jádro zatím nemá API pro třídu vlákna | priorita vstupu je ve smyčce kompozitoru; QoS doplní plánovač |
| průhlednost skupiny | násobí se do dětí (bez mezibufferu) | překrývající se děti poloprůhledné skupiny se prolnou i mezi sebou; GPU backend to udělá správně |
| vsync | čas `PRESENTED` od ovladače (konec `RESOURCE_FLUSH`) | virtio-gpu 2D vsync nehlásí |
| kurzor | kreslí ho kompozitor do snímku | hardwarový kurzor (fronta 1) přijde s 3D |
| obnova oken po restartu | aplikace pošle znovu poslední potvrzený stav ([Obnova](#obnova-po-pádu-kompozitoru)) | poloha okna se nepamatuje; rozpracovaná transakce vrátí `SERVICE_RESTARTED` |
| restart kompozitoru | supervisor (adaptér `CompdService`, displej z držáku konců) | dialogy a skupiny transakcí staré instance zaniknou |
| commit držený ve skupině | uplatní se až s celou skupinou, i když klient mezitím poslal další běžný commit | klient ve skupině má čekat na `PRESENTED`; přísné pořadí commitů jednoho klienta přijde s GPU backendem |
| pád ovladače displeje | kompozitor běží dál bez obrazu (okna, dialogy, akce) | restart ovladače zatím není |
| displej bez virtio-gpu | `fbdisplay`: pevné rozlišení z firmwaru, bez akcelerace a vsync | nativní ovladače GPU (pozdější milník) |

## Testy

- **hostitel** (součást `cargo xtask test`): `cargo test -p guiproto -p gfx
  -p font -p vgpu -p fbcopy -p comp -p ui` - kódování zpráv a odolnost proti
  vadným zprávám, barvy a prolínání, ořez, poškozené oblasti, text,
  rasterizér (přesné pokrytí čtverce, háček nad písmenem, náhodně poškozený
  soubor písma), příkazy virtio-gpu, převod RGB/BGR a kopie obdélníků
  s ořezem do framebufferu (`fbcopy`), transakce a jejich atomicita,
  skupiny transakcí (včetně termínu), synchronizovaná změna velikosti,
  rozdělení vstupu a fokus, přesun okna titulkem bez aplikace, deklarované
  oblasti vlastního titulku, odznak nad obsahem aplikace, odznak
  bezrámového okna, panel, dialogy jen z fyzického vstupu, heslo,
  bezpečná obrazovka, nové zařízení (BadUSB), akce (ověření, potvrzení,
  konec aplikace), inspekce, scroll a animace v kompozitoru, ping,
  rozvržení a ovládání prvků UI, editace textu s diakritikou. Testy
  kompozitoru a UI ukládají snímky `target/comp-scene.png`
  a `target/ui-window.png` pro kontrolu okem.
- **QEMU** (`cargo xtask test`, x86_64 i aarch64, sada `gui` poslední):
  kompozitor nad virtio-gpu, okno `gui-demo`, klik přes strom
  přístupnosti, barvy pixelů obrazovky (pozadí aplikace, odznak), klik
  umělým vstupem (`inject`), akce z manifestu (špatný argument odmítnut,
  `note.add`, akce agenta čeká na potvrzení), systémový dialog nepřijme
  umělý Enter a přijme fyzické klávesy z QEMU (QMP), bezpečná obrazovka
  po Ctrl+Alt+Del z QEMU, okno terminálu s vlastním titulkem, **pád
  kompozitoru uprostřed práce** (poslední: kompozitor pod supervisorem
  testu, `gui-demo` v jiném procesu s lístkem; po zabití se okno vrátí se
  stejnou identitou, odznakem a stavem, aplikace dál reaguje, jedno okno
  a jeden klient).
- **QEMU bez virtio-gpu** (`cargo xtask test --gpu none`, x86_64
  i aarch64): stejná sada `gui` nad `fbdisplay` (x86_64 VGA, aarch64
  ramfb). Pixely kontroluje inspekce kompozitoru, ne ovladač, takže sada
  ověří kompozitor a protokol displeje nad fbdisplay; obsah samotného
  framebufferu QEMU test nečte.
