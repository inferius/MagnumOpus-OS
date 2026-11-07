# Grafika a okna (GUI)

Milník 6c. Popisuje ovladač `virtio-gpu`, kompozitor `compd`, protokol
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
(premultiplied). Buffer má šířku, výšku a krok řádku (stride, násobek 4).

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

- Aplikace vytvoří VMO a pošle `BUFFER_CREATE` s handlem. Kompozitor ověří,
  že `stride * height` se vejde do VMO a že rozměry nepřekročí limit
  (16384 px), a namapuje ho jen pro čtení.
- Paměť bufferu platí job aplikace: aplikace nemůže vyčerpat paměť
  kompozitoru.
- Kompozitor buffer drží od `COMMIT`, který ho připojil k vrstvě, do zprávy
  `RELEASED` (vrstva dostala jiný buffer nebo zanikla).

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

**Nereagující aplikace:** kompozitor posílá `PING`; bez `PONG` do 3 s
ztlumí okno a nabídne zavření. Přesun, minimalizace a zavření fungují
dál.

## Vstup v oknech

Kompozitor dostává události od `inputd` ([Vstup](vstup.md)) a rozděluje je:

- **klávesnice** (`Key`, `Text`) -> okno s fokusem,
- **myš** -> okno pod ukazatelem (přepočet na souřadnice okna); stisk
  tlačítka mimo oblasti dekorace okno aktivuje a dá mu fokus,
- **kolečko** -> okno pod ukazatelem,
- **bezpečná zkratka** (`secure`) -> systémová obrazovka,
- událost s příznakem `SYNTHETIC` (umělý vstup) jde oknu jen s příznakem;
  systémový dialog ji **ignoruje**.

Okno dostane `INPUT` se stejným 32bajtovým záznamem jako `inputproto`,
jen `a`, `b` u ukazatele jsou souřadnice v okně.

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

```text
-> dialog ask 7 permission "Poznámky" "Povolit přístup k síti?" allow,deny
<- answer 7 allow
```

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
type = "text"            # text, int, bool, path, choice
required = true

[[action.param]]
name = "color"
type = "choice"
choices = ["zluta", "modra"]
```

| Zpráva | Směr | Obsah |
|---|---|---|
| `ACTIONS_LIST` | volající -> compd | akce běžících aplikací (id aplikace, id akce, parametry) |
| `ACTION_CALL` | volající -> compd | aplikace, akce, argumenty (typované hodnoty) |
| `ACTION_INVOKE` | compd -> aplikace | ověřené argumenty, id volání, kdo volá |
| `ACTION_RESULT` | aplikace -> compd -> volající | `ok` + hodnota, nebo kód chyby |

Akce s `confirm = true` nebo volaná z umělého zdroje (agent) se před
předáním potvrdí systémovým dialogem. Volající vidí jen akce, ne okna.

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
| 5 | `BUFFER_CREATE` | buffer, w, h, stride + handle VMO |
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

Příklad (okno s jedním bufferem):

```text
-> HELLO 1
<- WELCOME 1 1280 800 100
-> WINDOW_CREATE 1 STANDARD 400 300 "Ukázka"
-> BUFFER_CREATE 1 400 300 1600        + handle VMO
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
| `MODES` | ovladač -> compd | výstupy: w, h, zapnutý; schopnosti (`3D`) |
| `ATTACH` | compd -> ovladač | výstup, w, h, stride + handle VMO (jen čtení) |
| `PRESENT` | compd -> ovladač | seq, obdélníky |
| `PRESENTED` | ovladač -> compd | seq, čas (ns) |

Ovladač zkopíruje poškozené obdélníky do svých bufferů DMA (s IOMMU jediná
paměť, kam smí zařízení zapisovat a číst), pošle `TRANSFER_TO_HOST_2D`
a `RESOURCE_FLUSH` a po dokončení odpoví `PRESENTED`.

## Ovladač virtio-gpu

Inicializace jako u ostatních virtio ([Vstup](vstup.md#ovladač-virtio-input)):
vlastnosti `VERSION_1`, `ACCESS_PLATFORM` (a `VIRGL` pro 3D), přerušení
MSI-X, fronta 0 (řízení) a 1 (kurzor). Příkazy 2D:

1. `GET_DISPLAY_INFO` - velikost výstupů,
2. `RESOURCE_CREATE_2D` (formát `B8G8R8A8_UNORM`),
3. `RESOURCE_ATTACH_BACKING` - seznam kusů DMA paměti (každý nejvýš
   4 MiB, souvislá DMA paměť),
4. `SET_SCANOUT`,
5. pro každý snímek `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH`.

## Nativní knihovna UI (`libs/ui`)

Jedna knihovna pro všechny aplikace, deklarativní v Rustu:

```rust
fn view(s: &State) -> Node<Msg> {
    column([
        label("Poznámky").heading(),
        text_input(&s.title).on_change(Msg::Title),
        row([
            checkbox("Důležitá", s.important).on_toggle(Msg::Important),
            button("Uložit").on_click(Msg::Save),
        ]),
        list(s.items.iter().map(|i| list_item(i))),
    ])
}
```

Knihovna z popisu sestaví prvky, rozvrhne je (sloupce, řádky, pevné
a pružné rozměry), vykreslí je do bufferů (`libs/gfx`, `libs/font`)
a odvodí z nich **strom vrstev i strom přístupnosti** - aplikace nic
z toho nepíše ručně. Vstup (klávesy, text, myš, akce přístupnosti)
převede na zprávy `Msg`. Fokus se posouvá klávesou Tab.

Prvky: `label`, `button`, `text_input`, `checkbox`, `list`, `progress`,
`text_grid` (mřížka znaků pro okno terminálu), kontejnery `column`, `row`,
`scroll`.

## Písmo (`libs/font`)

Vlastní čtení TrueType (`.ttf`): tabulky `head`, `hhea`, `maxp`, `cmap`
(formáty 4 a 12), `loca`, `glyf` (jednoduché i složené glyfy), `hmtx`,
`kern` (formát 0). Rasterizér převede kvadratické křivky na úsečky
a spočítá **plošné pokrytí pixelu** (vyhlazení bez hintingu). Glyfy se
ukládají do mezipaměti podle velikosti.

Soubor písma je nedůvěryhodný vstup: každé čtení kontroluje meze, vadný
glyf je prázdný glyf, nikdy panika. Bez shapingu (arabština, indická
písma) a bez hintingu - to přijde s přechodem na plné vykreslování textu.

## Okno terminálu

Terminál 6b se v GUI stane oknem nad svým nativním protokolem: okno
terminálu je aplikace `libs/ui` s prvkem `text_grid`, která mluví se
službou relací (`termd`) protokolem `term::proto`. V 6c je hotová ukázka
`gui-term` (mřížka znaků, kurzor, vstup z klávesnice do řádku); napojení
na `termd` dělá vlákno terminálu.

## Testy

- **hostitel:** `cargo test -p guiproto -p gfx -p font -p ui` - kódování
  zpráv, skládání vrstev, ořez a průhlednost, rasterizér (glyf proti
  očekávané bitmapě), rozvržení prvků, vstup a fokus, strom přístupnosti.
- **QEMU** (`cargo xtask test`, x86_64 i aarch64): ovladač virtio-gpu
  ukáže obraz, kompozitor složí okno ukázkové aplikace, test přes strom
  přístupnosti klikne na tlačítko, umělý vstup do systémového dialogu
  neprojde, skutečný (QMP) ano, transakce skupiny se uplatní zároveň.
