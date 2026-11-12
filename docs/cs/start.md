# Startovní obrazovka

Co uživatel vidí od zapnutí po plochu: logo s prstencem průběhu na teplém
pozadí, bez textu firmwaru, loaderu a jádra, a plynulý přechod do plochy.
Návrh je `design/start.html` (varianta A, log z varianty B, přechod do
avataru); původní varianty jsou v `design/start-varianty.html`.

| Kód | Co dělá |
|---|---|
| `libs/bootsplash` | rozvržení, logo a název z `design/assets`, prstenec, záře, pozadí; stejný obraz ve všech fázích |
| `loader/src/splash.rs`, `loader/src/logger.rs` | první snímek hned po převzetí GOP, výpis loaderu jen na sériovou linku |
| `kernel/src/console.rs` | se `SPLASH` jádro do framebufferu nekreslí a nemaže ho |
| `user/splash` | samostatný proces: rozsvícení, dýchání, prstenec po krocích startu, „čeká na“, obrazovka chyby |
| `user/init/src/splash.rs` | spuštění procesu `splash`, kroky startu, ukončení před displejem |
| `user/compd/src/splash.rs` | první snímek kompozitoru = startovní obraz, přechod do plochy |

## Průběh

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Prubeh startovni obrazovky">
<defs><marker id="st1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="140" height="80" rx="6"/><text class="t b" x="80" y="44" text-anchor="middle">loader</text><text class="m" x="80" y="63" text-anchor="middle">černá + znak</text><text class="m" x="80" y="80" text-anchor="middle">prázdný prstenec</text>
<rect class="box" x="165" y="20" width="140" height="80" rx="6"/><text class="t b" x="235" y="44" text-anchor="middle">jádro</text><text class="m" x="235" y="63" text-anchor="middle">drží obraz</text><text class="m" x="235" y="80" text-anchor="middle">log jen na sériovou</text>
<rect class="acc" x="320" y="20" width="200" height="80" rx="6"/><text class="t b" x="420" y="44" text-anchor="middle">proces splash</text><text class="m" x="420" y="63" text-anchor="middle">rozsvícení, dýchání,</text><text class="m" x="420" y="80" text-anchor="middle">prstenec po krocích</text>
<rect class="ok" x="535" y="20" width="235" height="80" rx="6"/><text class="t b" x="652" y="44" text-anchor="middle">compd</text><text class="m" x="652" y="63" text-anchor="middle">stejný snímek, prstenec doběhne,</text><text class="m" x="652" y="80" text-anchor="middle">znak se zmenší a rozplyne</text>
<path class="ln" d="M150 60 L163 60" marker-end="url(#st1)"/><path class="ln" d="M305 60 L318 60" marker-end="url(#st1)"/><path class="ln" d="M520 60 L533 60" marker-end="url(#st1)"/>
<text class="m" x="157" y="125">SPLASH</text><text class="m" x="300" y="125">CLAIM</text><text class="m" x="470" y="125">RELEASE bez smazání,</text><text class="m" x="470" y="141">display ... splash=promile</text>
<line class="ln" x1="10" y1="170" x2="770" y2="170"/>
<text class="m" x="10" y="192">jádro</text><text class="m" x="200" y="192">zařízení</text><text class="m" x="360" y="192">soubory</text><text class="m" x="500" y="192">síť</text><text class="m" x="640" y="192">plocha</text>
<text class="m" x="10" y="225">kroky prstence (5): init posílá `krok n`; krok, který trvá přes 2 s, ukáže pod názvem „čeká na: ...“</text>
</svg>
<figcaption>Kdo kreslí startovní obraz a kdy. Obraz se nikdy nesmaže: každý další ho převezme tak, jak je.</figcaption>
</figure>

1. **Loader** **[implementováno]** vybere GOP (jako dřív) jako první věc
   po převzetí řízení a vykreslí černou se znakem a prázdným prstencem.
   Pak teprve načte jádro a initfs. Od té chvíle jde jeho výpis jen na
   sériovou linku (protokol UEFI Serial I/O), textová konzole firmwaru by
   psala přes obraz. Do `BootInfo::boot_flags` dá `SPLASH`.
2. **Jádro** **[implementováno]** se `SPLASH` nezakládá textovou konzoli
   nad framebufferem (ta by obrazovku smazala); log jde na sériovou linku
   a do bufferu logu. `FB_CONTROL RELEASE` ani konec vlastníka obrazovku
   nemažou. Initu předá `bootstrap::flags::SPLASH`.
3. **Proces `splash`** **[implementováno]** spustí init z initfs
   (handly: objekt paměti framebufferu, řídicí kanál; třída QoS
   Background, na obraz nikdo nečeká). Obraz převezme (`CLAIM`), rozsvítí
   pozadí (900 ms), ukáže název, nechá dýchat záři (2,4 s) a plní
   prstenec po krocích, které mu init posílá. Před spuštěním displeje
   kompozitoru (nebo záložní konzole) pošle init `konec`: proces dokreslí
   plný prstenec (200 ms), odpoví `hotovo 1000` a skončí; jádro obraz
   při konci procesu nemaže. Neodpoví-li do 1,5 s, init ho ukončí.
   Samostatný proces místo vlákna initu: chyba v kreslení shodí jen jeho,
   ne kritický init.

   | Zpráva (init -> splash) | Význam | Odpověď |
   |---|---|---|
   | `krok <n>` | hotovo `n` kroků z `bootsplash::STEPS` | - |
   | `konec` | dokreslit plný prstenec, skončit | `hotovo <promile>` |
   | `chyba <text>` | obrazovka chyby startu, skončit | `ok` |
4. **Kompozitor** **[implementováno]** dostane prstenec, na kterém obraz
   skončil (`display <pásmo> splash=<promile>`), první snímek je tentýž
   obraz, prstenec doběhne (500 ms), 200 ms se drží, znak se zmenší na
   velikost avataru (600 ms) a se startovním pozadím se rozplyne do
   plochy (300 ms). Plocha má stejné pozadí (`gfx::backdrop`), takže
   přechod je jen zmizení znaku.

Dýchání záře se počítá z času od startu systému, `splash` i kompozitor tak
navazují bez skoku. Plocha se během přechodu skládá dál (do bufferu
v RAM), takže klienti dostávají snímky a start aplikací se nezdrží.

## Obraz

Rozměry a časy jsou z prototypu (logické px, měřítko = výška / 800,
1280 x 800 = 1:1):

| Prvek | Hodnota |
|---|---|
| znak | 132 px, logo 84 px, prstenec poloměr 62 px a tah 3 px (podklad akcent 22 %) |
| záře | akcent 42 % do 65 % paprsku čtverce 184 px; dýchání 0,25-0,7 a 0,92-1,05 |
| název | výška 44 px, 16 px pod znakem, barva `ink` |
| „čeká na“ | 8 px pod názvem, styl `small`, barva `ink_muted` |
| pozadí | jako plocha: `bg` a dva radiální přechody `wall_a`, `wall_b` (světlý motiv) |

Barvy jsou vždy ze světlého motivu: start je před načtením nastavení
uživatele. Logo a název se kreslí přímo z vektorových dat
`design/assets/logo/symbol-fialovy.svg` a `design/assets/logo-wordmark.js`
(pravidlo sudá-lichá, `font::Rasterizer::finish_even_odd`).

## Ladicí start

Textový výpis na obrazovce zůstal pro ladění **[implementováno]**:

- klávesa **D** v nabídce loaderu (2 s, spolu s **L** pro vytažení logů),
- soubor `\os\bootlog` na ESP (pak je každý start ladicí).

Loader pak dá `VERBOSE` místo `SPLASH` a jádro kreslí log jako dřív.
Sériová linka má plný výpis vždy.

## Chyby

| Situace | Co se ukáže |
|---|---|
| panika jádra | obrazovka pádu (`kernel/src/crashscreen.rs`, návrh `design/pad.html`) - kreslí se vždy **[implementováno]** |
| init skončí | jádro obnoví textovou konzoli a ukáže log s kódem konce initu **[implementováno]** |
| GUI ani záložní konzole nenaběhne | proces `splash` (na pokyn initu `chyba`) místo znaku ukáže „Systém se nespustil“ s důvodem a kódem chyby **[implementováno]** |
| GUI nenaběhne, konzole ano | obraz se předá záložní konzoli (jako dřív) **[implementováno]** |
| proces `splash` spadne | obraz zůstane (jádro ho nemaže), start pokračuje, displej ho převezme **[implementováno]** |

## Bez GOP a jiné displeje

| Případ | Co se ukáže |
|---|---|
| bez framebufferu (headless, firmware bez GOP) | loader nic nekreslí ani nedá `SPLASH`; jádro, init i kompozitor jako dřív **[implementováno]** |
| x86_64 `--gpu 2d` (virtio-gpu) | obraz na kartě firmwaru (VGA), kompozitor na virtio-gpu začne stejným obrazem ve svém rozlišení (640 x 480, měřítko 0,75) a přejde do plochy **[ověřeno v QEMU]** (ruční běh, `9e865be`) |
| nativní displej (bochs-display, Intel) | proces `splash` skončí, nativní ovladač převezme framebuffer (`CLAIM`), kompozitor naváže jako u fbdisplay **[ověřeno v QEMU]** pro bochs-display (výchozí `xtask run`) |
| aarch64 v QEMU (ramfb) | obraz na ramfb (800 x 600, měřítko 0,75), proces `splash` animuje (pod TCG ~170 ms na snímek), kompozitor na bochs-display (1280 x 800) začne startovním obrazem a přejde do plochy **[ověřeno v QEMU]** (ruční běh TCG, `9e865be`) |

Režim vytažení logů proces `splash` nespouští (loader a jádro obraz
drží, nikdo ho neanimuje). Testovací jádro (`qemu-test`, `cargo xtask
test`) obraz ignoruje a kreslí log na obrazovku jako dřív: kreslení logu
start zpomaluje a bez něj se měnilo časování testů (test `shell: termd:
relace, soubory, undo, odpojeni` pak našel zbylý klon, viz Navazuje).

## Výkon

Měřeno: release, `cargo xtask run --accel whpx --res 1920x1080`, QEMU 11.1
pro Windows s WHPX. Časy od řádku `os loader` na sériové lince
(hostitel), průměr 3 běhů; obsahuje i 2 s nabídky loaderu (stejné před
i po).

| Úsek (ms od `os loader`) | před (`77ac8ee`) | po (`9e865be`) |
|---|---|---|
| první obraz | text loaderu | startovní obraz za 33 ms (kreslení 46-59 Mcyklů) |
| jádro startuje | 2511 | 2491 |
| init běží | 5489 | 5310 |
| první snímek kompozitoru (`kompozitor bezi`) | 7401 | 7627 |
| první okno (`klient 1`) | 8061 | 7961 |

Start se nezpomalil: první okno je do 100 ms stejně (rozptyl běhů je
~±250 ms). První snímek kompozitoru je o ~230 ms později: pozadí plochy
počítá už při připojení (první snímek je startovní obraz na tomto
pozadí, ~430 ms ve WHPX), dřív ho počítal v prvním taktu, a init čeká
~190 ms, než `splash` dokreslí plný prstenec. Okna se tím nezdrží.
Proces `splash` kreslí snímek za 12-14 ms (dýchání 25 fps, třída
Background).

**Velikost loaderu** (release, x86_64): 86,5 KB před, 271,9 KB po
(vektorová data loga a názvu, motiv z `design/tokens.toml`, rasterizér).
Načtení z ESP je o desetiny ms delší.

Kreslení samotné (`cargo test --release -p bootsplash -- --ignored
perf_numbers`, hostitel, 1920 x 1080): první snímek loaderu 1,4 ms, celý
snímek rozsvícení 2,0 ms, rozplynutí do plochy 3,2 ms, oblast znaku
(244 x 244 px) 0,27 ms, výpočet pozadí 9,2 ms (jednou).

⚠️ **Programy na x86_64 běží bez SSE** (cíl `x86_64-unknown-none`,
plovoucí čárka v softwaru, ~30-40x pomalejší). Proto se obraz kreslí
celočíselně: podklad po řádcích (kopie, ztlumení, prolnutí) a znak
v klidu z tabulky (`libs/bootsplash/src/cache.rs`: vzdálenost, úhel,
pokrytí prstence a loga pro každý pixel). Float zůstal jen pro konce
oblouku a pro posunutý znak během přechodu v kompozitoru. Snímek initu
tím klesl z ~50 ms na 12-14 ms (WHPX). Pozadí plochy (~400 ms ve float)
se počítá jednou: v procesu `splash`, v kompozitoru ho přechod převezme
od plochy (`Comp::backdrop`).

## Determinismus

Loader kreslí s FPU, proces `splash` a kompozitor s plovoucí čárkou
v softwaru. Obraz musí být bit po bitu stejný, jinak by přechod mezi nimi
poskakoval. Knihovna proto používá jen základní operace IEEE 754 (sčítání,
násobení, dělení, převody; vlastní odmocnina, sinus a arkustangens), které
jsou přesně zaokrouhlené v hardwaru i softwaru, a žádné `mul_add`.
Hlídají to dva testy (níže). Jediný známý rozdíl: znak v klidu se kreslí
z celočíselné tabulky, posunutý (přechod v kompozitoru) ve float; liší se
nejvýš o 2 úrovně barvy (test `integer_mark_matches_float`), okem
neviditelné.

## Testy

- `cargo test -p bootsplash` **[implementováno]**: rozvržení, sudá-lichá
  výplň, první snímek, prstenec po směru hodin, shoda pozadí s plochou,
  RGB framebuffer, posun znaku, celočíselná proti float cestě,
  `reference_output_is_stable` (otisky snímků pro 1280 x 800,
  1920 x 1080 a 1024 x 768) a `self_check_matches`.
- QEMU, sada `displej` **[ověřeno v QEMU]**:
  - `startovni obraz: kresba bez FPU = reference hostitele` - otisk
    `bootsplash::self_check()` v initu (plovoucí čárka v softwaru) se
    rovná hodnotě z hostitele,
  - `bochs-display: compd navaze na startovni obraz, nastavi rezim, zmena
    rozliseni za behu` - kompozitor s `splash=600`, odpovědi během
    přechodu a změna režimu po něm.
- Neověřeno automaticky: řádek „čeká na“ a obrazovka chyby (v QEMU se
  žádný krok nezdrží a start neselže).

## Navazuje

- **Zbylý klon po převzetí relace** (shell): když úloha na pozadí
  (`bg`) doběhne, zatímco relaci přebírá nový termd, zůstane v
  `/tmp/.aside` jeden klon. Projevilo se jen s rychlejším startem bez
  kreslení logu (deterministicky na x86_64 i aarch64); testy teď běží
  s logem jako dřív, chyba v shellu zůstává.

- **Esc = podrobný log** na startovní obrazovce **[navrženo]**: init do
  startu `inputd` nemá vstup.
- **Cíl přechodu = avatar přihlášení** **[implementováno]**: init přidá
  do `display` slovo `login`, když není otevřená žádná relace
  (`session::any_open`); kompozitor pak vezme cíl z
  `Comp::greeter_avatar(false)` (`login_target` v
  `user/compd/src/splash.rs`). Přihlašovací obrazovka se ukáže po
  přechodu a avatar kreslí na stejném místě. S otevřenou relací
  (vývojové autopřihlášení) se znak zmenší na místě a rozplyne do plochy.
- **Omezit pohyb**: přechod umí jen prolnutí 90 ms (`reduced`), volba
  v nastavení zatím není **[navrženo]**.
- **Rozlišení**: loader režim GOP nemění (firmware nebo EDID z `--res`);
  obraz se jen škáluje. Volba režimu v loaderu by na skutečném HW dala
  bliknutí a stovky ms.
