# Vzhled a ovládání

Návrh, jak MagnumOpus-OS vypadá a jak se ovládá: design systém (barvy,
písmo, mřížka, ikony, pohyb), okna, systémové prvky, čtyři klíčové
aplikace a přístupnost. Navazuje na odsouhlasené principy v
[Architektuře](architektura.md#vlastní-vzhled-okna-vlastní-titulek)
(dekorace kreslí systém, odznak důvěry, systémové dialogy ve vlastní
vrstvě, [plynulost](architektura.md#plynulost-uživatelského-rozhraní-plán),
[přehled úloh](architektura.md#přehled-úloh-odsouhlaseno)).

Tahle stránka je **návrh**, ne popis kódu. Kompozitor `compd`, knihovnu UI
`libs/ui` a okno terminálu dělá milník 6c (stránka *Grafika a okna*).
Odsud si bere vzhled a chování; hodnoty bere jako data z
`design/tokens.toml`.

> **Příručka grafického jazyka:** [design/prirucka.html](../../design/prirucka.html)
> (česky i anglicky). Pravidla pro každého, kdo kreslí okno, píše aplikaci,
> text nebo ikonu: hodnoty, logo, barvy, písmo, ikony, rozvržení,
> komponenty, pohyb, hlas a texty, důvěra a agent, přístupnost, s příklady
> „správně / špatně“. Tahle stránka je technický návrh a plán; příručka
> je stručný výklad pro autory. Odkaz funguje v repozitáři (otevři soubor
> v prohlížeči) i na webu dokumentace (`cargo xtask docs` kopíruje složku
> `design/` do `target/docs/design/`).

**Klikací prototypy** jsou v `design/` (otevři `design/index.html`
v prohlížeči). Každá stránka má přepínač motivu, vysokého kontrastu,
velikosti písma a omezení pohybu.

| Prototyp | Co ukazuje |
|---|---|
| `design/prirucka.html` | **příručka grafického jazyka** (cs / en) |
| `design/design-system.html` | barvy s tabulkou kontrastu, typografie, mřížka, zaoblení, stíny, ikony, pohyb, komponenty |
| `design/okna.html` | anatomie okna, odznak důvěry (odebrání oprávnění), režimy okna, stavy |
| `design/plocha.html` | lišta v 6 polohách, přepínání oken, oznámení, systémové dialogy, výběr souboru, bezpečná obrazovka, správce úloh, agent v liště |
| `design/terminal.html` | bloky výstupu, tabulky se schématem, doplňování podle typů, nanečisto a undo, chyby, panel úloh, zápis B / C |
| `design/soubory.html` | verze souboru, „zkusit bokem“, převzetí vybraných změn, konflikt, agent pracující bokem |
| `design/nastaveni.html` | profily zabezpečení, samostatné volby, vzhled, přístupnost |
| `design/prihlaseni.html` | zamčená obrazovka, výběr účtu, bezpečnostní fráze, režim s Ctrl+Alt+Del |
| `design/varianty-stylu.html` | tři původní varianty stylu (archiv rozhodnutí) |
| `design/start.html` | startovací obrazovka: logo s prstencem, log na Esc, zdržení kroku, chyba startu, přechod do avataru |
| `design/start-varianty.html` | tři varianty startovací obrazovky (archiv rozhodnutí) |
| `design/pad.html` | obrazovka kritického pádu: klidný popis, odpočet, `D` technická zpráva, oznámení po dalším startu |
| `design/pad-varianty.html` | tři varianty obrazovky pádu (archiv rozhodnutí) |

## Rozhodnutí ke stylu (odsouhlaseno)

Uživatel vybíral ze tří variant vedle sebe (`design/varianty-stylu.html`):
A Klidný (minimalistický), B Technický (hustý, linky místo stínů),
C Vřelý (hravější, výrazný akcent).

| # | Otázka | Rozhodnutí | Důsledek |
|---|---|---|---|
| 1 | celkový charakter | **C, Vřelý** | fialový akcent, teplé pozadí, zaoblení 18 px u oken a 10 px u prvků, tlačítka jako pilulky, barevné měkké stíny |
| 2 | umístění lišty | **nastavitelné**, výchozí **odsazený pruh dole** | 6 poloh: dole odsazená, plovoucí dok, dole přes šířku, vlevo, vpravo, nahoře |
| 3 | styl ikon | **linkové, zaoblené** | tah 1,75 px, mřížka 24 px, barva z textu |
| 4 | písmo | **Inter + JetBrains Mono** | obě pod SIL OFL 1.1; Tuffy a Noto Sans Mono zůstanou jako záloha |
| 5 | výchozí motiv | **podle denní doby** | světlý ve dne, tmavý po západu slunce v nastaveném pásmu |
| 6 | označení agenta AI | **vyhrazená barva + ikona + slovo** | korálová barva jen pro agenta, vždy s ikonou a slovem „agent“ (nikdy jen barvou) |
| 7 | logo | **symbol M + nápis MagnumOpus OS** (vektory od uživatele) | `design/assets/logo/`; fialová `#6b3fd6` na světlém, levandulová `#b79bff` na tmavém, bílá na barevném |
| 8 | startovací obrazovka | **A (logo s prstencem) + podrobný log z B**, na konci **přechod znaku do avataru** | vybráno ze tří variant v `design/start-varianty.html` |
| 9 | obrazovka kritického pádu | **A Klid** pro uživatele, **klávesa D přepne na technickou zprávu** (Dílna); ve vývojové verzi rovnou Dílna | vybráno ze tří variant v `design/pad-varianty.html` |

## Principy

1. **Důvěra je vidět.** Identita aplikace a její oprávnění jsou vždy na
   stejném místě (odznak vlevo v titulku) a v barvě, kterou žádná aplikace
   nedostane. Co kreslí systém, aplikace nenapodobí.
2. **Plynulost před efekty.** Animuje kompozitor (průhlednost, posun,
   měřítko vrstev), ne aplikace. Animace nečeká na aplikaci a nikdy
   neblokuje vstup.
3. **Přístupné od základu.** Strom přístupnosti je součást protokolu okna.
   Vše jde klávesnicí, písmo jde zvětšit na 200 % bez oříznutí.
4. **Agent je vždy poznat.** Co dělá agent AI, je označené vyhrazenou
   barvou, ikonou a slovem.
5. **Bezpečné zkoušení.** Rizikové změny jdou nanečisto nebo „bokem“
   v klonu; UI je nabízí jako běžnou cestu, ne jako schovanou volbu.
6. **Vlastní tvář.** Inspirace ano, kopie cizího systému ne: zaoblené
   tvary, teplé pozadí, odznak důvěry a lišta s úlohami jsou naše.

## Design systém

### Tokeny jako data

Všechny hodnoty (barvy, písma, rozestupy, zaoblení, stíny, pohyb, vrstvy)
jsou v jednom souboru `design/tokens.toml`. Knihovna UI je načte jako
data, takže změna barvy nebo rozestupu nevyžaduje zásah do kódu prvků.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tok tokenu">
<defs><marker id="vz1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="80" width="170" height="70" rx="6"/><text class="t b" x="95" y="106" text-anchor="middle">design/tokens.toml</text><text class="m" x="95" y="124" text-anchor="middle">jediný zdroj pravdy</text><text class="m" x="95" y="140" text-anchor="middle">světlý, tmavý, kontrast</text>
<path class="ln" d="M180 100 L268 60" marker-end="url(#vz1)"/>
<path class="ln" d="M180 130 L268 170" marker-end="url(#vz1)"/>
<rect class="box" x="270" y="20" width="200" height="74" rx="6"/><text class="t b" x="370" y="44" text-anchor="middle">build-tokens.mjs</text><text class="m" x="370" y="62" text-anchor="middle">kontrola kontrastu WCAG</text><text class="m" x="370" y="78" text-anchor="middle">chyba = konec s kódem 1</text>
<rect class="box" x="270" y="136" width="200" height="74" rx="6"/><text class="t b" x="370" y="160" text-anchor="middle">libs/ui (6c)</text><text class="m" x="370" y="178" text-anchor="middle">načte TOML při startu</text><text class="m" x="370" y="194" text-anchor="middle">motiv přepne kompozitor</text>
<path class="ln" d="M470 57 L558 57" marker-end="url(#vz1)"/>
<rect class="ok" x="560" y="20" width="190" height="74" rx="6"/><text class="t b" x="655" y="44" text-anchor="middle">assets/tokens.css + .js</text><text class="m" x="655" y="62" text-anchor="middle">prototypy v design/</text><text class="m" x="655" y="78" text-anchor="middle">tabulka kontrastu</text>
<path class="ln" d="M470 173 L558 173" marker-end="url(#vz1)"/>
<rect class="ok" x="560" y="136" width="190" height="74" rx="6"/><text class="t b" x="655" y="160" text-anchor="middle">aplikace a dekorace</text><text class="m" x="655" y="178" text-anchor="middle">stejné hodnoty všude</text><text class="m" x="655" y="194" text-anchor="middle">trust-* jen pro systém</text>
</svg>
<figcaption>Tokeny jsou data. Prototypy i knihovna UI berou hodnoty ze stejného souboru.</figcaption>
</figure>

Ukázka formátu (podmnožina TOML: sekce, `klíč = "text"` nebo číslo):

```toml
[radius]
control = 10
window = 18

[color.light]
accent = "#6b3fd6"
on_accent = "#ffffff"

[contrast]
on_accent = "on_accent accent 4.5"   # popředí, pozadí, minimum
```

```bash
node design/tools/build-tokens.mjs          # zapíše design/assets/tokens.css a tokens.js
node design/tools/build-tokens.mjs --check  # jen kontrola (kontrast + aktuálnost výstupu)
```

Jednotky: `px` jsou logické pixely před škálováním HiDPI, čas je v ms.
Kompozitor škáluje zlomkově (125 %, 150 %), proto mají rozměry násobky 4.

### Barvy

Barvy jsou **sémantické** (role), ne odstíny. Každý motiv (světlý, tmavý,
vysoký kontrast) má stejné klíče.

| Role | Světlý | Tmavý | Použití |
|---|---|---|---|
| `bg` | `#fbf1ea` | `#17111c` | pozadí plochy (s teplým přechodem `wall-a`, `wall-b`) |
| `surface` | `#fffdfb` | `#221a28` | plocha okna |
| `surface-2` | `#fff4ee` | `#2b2232` | zvýšená plocha: titulek, postranní panel |
| `surface-3` | `#f6e9e2` | `#362b3e` | najetí myší, výplň prvků |
| `border` / `border-strong` | `#eedad0` / `#8f7a87` | `#3d3146` / `#8a7894` | dělicí čára / hranice prvku (3 : 1) |
| `ink` / `ink-muted` | `#2a1f2d` / `#665569` | `#f6edf3` / `#bba9c4` | text / vedlejší text |
| `accent` / `accent-soft` / `accent-ink` | `#6b3fd6` / `#efe7ff` / `#5027bd` | `#b79bff` / `#382b5c` / `#d8c9ff` | hlavní akce / výběr / text akcentu |
| `focus` | `#6b3fd6` | `#c9b4ff` | rámeček fokusu 2 px s odsazením 2 px |
| `trust-bg` / `trust-ink` / `trust-border` | `#ffe4d6` / `#7a2806` / `#e9a586` | `#4d2414` / `#ffd2bf` / `#9a5130` | **jen systém**: odznak důvěry, systémové dialogy |
| `ai` / `ai-soft` / `ai-ink` | `#c3264d` / `#ffe4ea` / `#9c1b3d` | `#ff8fa8` / `#4a1d2a` / `#ffc2cf` | **jen agent AI** |
| `ok`, `warn`, `danger` | `#1b7543`, `#8a5000`, `#bf283f` | `#6fd39a`, `#f2b54f`, `#ff7d92` | stavy (s `-soft` pozadím) |
| `term-*` | 9 barev | 9 barev | terminál: pozadí, text, tlumený, červená až azurová |

**Vyhrazené barvy:**

- `trust-*` patří jen kompozitoru (odznak, pruh systémového dialogu,
  bezpečná obrazovka). Knihovna UI je aplikacím nenabízí. Aplikace může
  podobný odstín nakreslit do svého bufferu, ale ne na místo odznaku a ne
  do vrstvy dialogů (viz [Systémové dialogy](#systémové-dialogy)).
- `ai-*` je jen pro agenta AI, vždy spolu s ikonou agenta a slovem
  „agent“. Barvoslepý uživatel tak agenta pozná podle ikony a textu.

**Kontrast (WCAG 2.2):** text alespoň 4,5 : 1 (1.4.3), hranice prvků,
ikony a fokus alespoň 3 : 1 (1.4.11). Ve vysokém kontrastu text 7 : 1
(AAA) a hranice 4,5 : 1. Povinné dvojice jsou v sekci `[contrast]`
tokenů; generátor je ověří pro všechny tři motivy (teď 102 z 102
dvojic v pořádku) a při nesplnění skončí chybou.

**Výchozí motiv podle denní doby:** světlý od východu do západu slunce
v nastaveném časovém pásmu (systém zná čas i pásmo, viz
[Reálný čas](cas.md)), jinak tmavý. Přechod je prolnutí 240 ms
a odloží se, dokud uživatel píše (nejdéle 2 minuty).

### Typografie

| Písmo | Použití | Proč |
|---|---|---|
| **Inter** | celé UI | navržené pro obrazovky, velká výška malých písmen, tabulkové číslice, plná čeština |
| **JetBrains Mono** | terminál, kód, cesty | odlišené `0 O`, `1 l I`, široké znaky, čitelné od 12 px |
| Tuffy, Noto Sans Mono | záloha | už přibalené v `libs/font/fonts/` (6c) |

Typografická škála (velikost / řádek v px, váha):

| Token | Velikost | Řádek | Váha | Použití |
|---|---|---|---|---|
| `caption` | 12 | 16 | 500 | vedlejší popisky, štítky |
| `small` | 13 | 18 | 400 | metadata, nápověda |
| `body` | 14 | 20 | 400 | základní text UI |
| `body-lg` | 16 | 24 | 400 | text dialogů, úvodní odstavce |
| `title` | 18 | 24 | 600 | nadpis dialogu, okna |
| `heading` | 22 | 28 | 650 | nadpis stránky aplikace |
| `display` | 32 | 40 | 700 | uvítání, velké nadpisy |
| `mono` | 13 | 20 | 400 | terminál (jedna buňka = šířka znaku) |
| `clock` | 64 | 72 | 300 | hodiny na zamčené obrazovce |

- V tabulkách, časech a hodinách tabulkové číslice (`tnum`).
- V terminálu **bez ligatur**: jeden znak = jedna buňka mřížky.
- **Velikost písma** (Přístupnost): 100, 125, 150, 200 %. Škáluje se celé
  rozhraní a rozvržení se přepočítá, nic se neořízne.

⚠️ **Rizika písma (řečená předem):**

- ~~`libs/font` čte kerning jen z tabulky `kern`~~ vyřešeno ve v1.5:
  `libs/font` čte párový kerning z `GPOS` („Te“ −160, „Vo“ −104 jednotek
  u Interu).
- Inter je hlavně proměnné písmo; přibalit se mají **statické řezy**
  (400, 500, 600, 700) ve formátu TTF s obrysy `glyf`, které rasterizér
  umí. Velikost: ~300 kB na řez, celkem ~1,5 MB s JetBrains Mono.
- Bez hintingu je text pod 12 px měkčí. Proto je nejmenší velikost
  12 px a základ 14 px.

### Mřížka a rozměry

Vše leží na **mřížce 4 px**. Rozestupy: 2, 4, 8, 12, 16, 20, 24, 32, 40,
48, 64 px (`s1` až `s11`). Uvnitř prvku menší mezery než mezi prvky.

| Token | Hodnota | Použití |
|---|---|---|
| `hit-min` | 40 px | nejmenší cíl kliknutí a dotyku (dotyk je výchozí vstup); volba „větší cíle“ dá 48 px |
| `control-h` | 36 px | tlačítko, pole (hustá varianta 28 px) |
| `title-h` | 40 px | titulek okna |
| `title-btn` | 28 px | viditelné tlačítko okna; cíl 40 px přesahuje do okraje titulku |
| `badge-h` | 26 px | odznak důvěry |
| `bar-h` | 52 px | lišta; odsazení od okraje 8 px |
| `resize-edge` | 6 px | neviditelný okraj pro změnu velikosti (roh 18 px) |

### Zaoblení a stíny

| Token | Hodnota | Použití |
|---|---|---|
| `xs` | 6 px | klávesa, malý štítek |
| `badge` | 8 px | odznak důvěry |
| `control` | 10 px | pole, položka seznamu |
| `card` | 14 px | karta, nabídka, oznámení |
| `window` | 18 px | okno, lišta, dialog |
| `pill` | 999 | tlačítka a štítky |

Stín říká, co je výš: neaktivní okno < aktivní okno < nabídka < dialog.
Ve světlém motivu má stín okna fialový nádech (`rgba(107,63,214,.16)`),
v tmavém je černý. Na GPU kreslí kompozitor stín shaderem a výsledek si
pamatuje podle velikosti okna, takže pohyb okna stín nezdraží. Záložní 2D
cesta použije předpočítanou texturu z 9 dílů (rohy, hrany, střed). Ve
vysokém kontrastu stíny nahradí rámeček 2 px.

### Ikony

Linkové a zaoblené: mřížka 24 px, tah 1,75 px, zaoblené konce a spoje,
okraj 2 px. Velikosti 16, 20, 24 a 32 px. Barvu dává text (`currentColor`),
takže ikony samy přejdou do tmavého i kontrastního motivu. Sada má 64 ikon
(`design/assets/mo.js`, objekt `ICONS`); ikona agenta jsou dvě jiskry.

Ikona nikdy nenese význam sama: v liště má popisek nebo tooltip, ve stromu
přístupnosti jméno.

### Logo

Logo tvoří **symbol** (písmeno M v oblouku) a **nápis** „MagnumOpus OS“.
Obojí jsou uzavřené Bézierovy křivky bez písem a bitmap
(`design/assets/logo/`, popis v `CTI-ME.txt`).

| Soubor | Použití |
|---|---|
| `logo-svetle-pozadi.svg` | fialový symbol `#6b3fd6` a tmavý nápis `#221a28`: světlý motiv |
| `logo-tmave-pozadi.svg` | levandulový symbol `#b79bff` a světlý nápis `#fffdfb`: tmavý motiv |
| `logo-jednobarevne.svg` | jedna barva (tisk, vysoký kontrast) |
| `symbol-fialovy.svg`, `symbol-levandulovy.svg`, `symbol-bily.svg` | samotná značka: ikona systému, lišta, start, odznak „Systém“ |

Barvy loga jsou přesně tokeny `accent` (světlý a tmavý motiv)
a `surface` tmavého motivu, takže logo vždy ladí s akcentem. V systému se
symbol kreslí s výplní z tokenu `accent` (jedna cesta, barvu dá motiv);
ve vysokém kontrastu bílý nebo žlutý akcent. Nejmenší velikost symbolu
16 px, kolem něj volné místo aspoň čtvrtinu jeho šířky.

### Pohyb

Snímek při 60 Hz má 16,7 ms. Kompozitor plánuje podle termínu snímku
([Plynulost](architektura.md#plynulost-uživatelského-rozhraní-plán));
animace mění jen vlastnosti vrstev, které skládá sám (průhlednost, posun,
měřítko, ořez). Aplikace během animace nic nekreslí, a proto se animace
nezasekne, ani když aplikace počítá.

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Rozpocet snimku">
<rect class="box" x="10" y="40" width="70" height="44" rx="4"/><text class="t b" x="45" y="66" text-anchor="middle">vstup</text>
<rect class="acc" x="82" y="40" width="300" height="44" rx="4"/><text class="t b" x="232" y="60" text-anchor="middle">skládání poškozených oblastí</text><text class="m" x="232" y="76" text-anchor="middle">~5 ms softwarově, méně s 3D</text>
<rect class="ok" x="384" y="40" width="120" height="44" rx="4"/><text class="t b" x="444" y="66" text-anchor="middle">flush ~2 ms</text>
<rect class="box" x="506" y="40" width="244" height="44" rx="4"/><text class="t" x="628" y="66" text-anchor="middle">rezerva do termínu</text>
<path class="ln" d="M10 104 L750 104"/><text class="m" x="10" y="122">0 ms</text><text class="m" x="750" y="122" text-anchor="end">16,7 ms (vsync)</text>
<rect class="warn" x="82" y="132" width="420" height="26" rx="4"/><text class="m" x="292" y="150" text-anchor="middle">aplikace kreslí svůj buffer souběžně, mimo cestu animace</text>
</svg>
<figcaption>Orientační rozpočet jednoho snímku. Animace jsou jen změny vlastností vrstev.</figcaption>
</figure>

| Token | Délka | Křivka | Použití |
|---|---|---|---|
| `fast` | 90 ms | standard | najetí, stisk, přepínač |
| `base` | 160 ms | standard | nabídka, změna stavu |
| `slow` | 240 ms | enter | otevření okna, dialog, změna motivu |
| `max` | 320 ms | - | horní mez |
| `reduced` | 90 ms | prolnutí | náhrada všech posunů při „Omezit pohyb“ |

Křivky: standard `cubic-bezier(0.2, 0, 0, 1)`, enter
`(0.05, 0.7, 0.1, 1)`, exit `(0.3, 0, 1, 1)`.

**Pravidla:**

- Odchod je kratší než příchod (zavřít = `fast`, otevřít = `slow`).
- Každá animace jde přerušit: nový vstup ji ukončí v aktuálním stavu
  a pokračuje odtud.
- Přesun a změna velikosti okna sledují ukazatel přímo, bez zpoždění
  a pružení.
- Nic nebliká rychleji než 3× za sekundu (WCAG 2.3.1).
- Když kompozitor nestihne termín, vynechá mezisnímek animace, ale nikdy
  nezpozdí reakci na vstup.
- **Omezit pohyb** (Přístupnost nebo systémové nastavení): posuny
  a měřítka nahradí prolnutí 90 ms.

## Okna

### Anatomie

<figure class="diagram">
<svg viewBox="0 0 760 260" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Anatomie okna">
<rect class="box" x="60" y="20" width="640" height="200" rx="18"/>
<rect class="acc" x="60" y="20" width="640" height="40" rx="18"/>
<rect class="warn" x="72" y="27" width="150" height="26" rx="8"/><text class="m" x="147" y="45" text-anchor="middle">odznak: Poznámky · složka</text>
<text class="t b" x="380" y="45" text-anchor="middle">nakup.md - Poznámky</text>
<circle class="box" cx="616" cy="40" r="14"/><circle class="box" cx="646" cy="40" r="14"/><circle class="err" cx="676" cy="40" r="14"/>
<text class="t" x="380" y="140" text-anchor="middle">obsah: kreslí aplikace, strom přístupnosti popisuje vše</text>
<text class="m" x="147" y="80" text-anchor="middle">1 odznak (systém)</text>
<text class="m" x="380" y="80" text-anchor="middle">2 titulek (text od aplikace)</text>
<text class="m" x="646" y="80" text-anchor="middle">3 tlačítka (systém)</text>
<text class="m" x="700" y="236" text-anchor="end">5 okraj 6 px, roh 18 px (kompozitor)</text>
<text class="m" x="60" y="236">4 obsah (aplikace)</text>
<path class="ln" d="M40 20 L40 60"/><text class="m" x="34" y="44" text-anchor="end">40</text>
</svg>
<figcaption>Standardní okno. Odznak, titulek, tlačítka a okraje drží kompozitor, takže fungují i u zaseknuté aplikace.</figcaption>
</figure>

1. **Odznak důvěry** vlevo: jméno z manifestu (předá ho init, ne aplikace)
   a ikony toho, co aplikace právě smí. Klik nebo Enter otevře přehled.
2. **Titulek** na středu (vlevo je místo vyhrazené odznaku), nejvýš 256
   znaků, zkrácený třemi tečkami. Dvojklik = maximalizovat.
3. **Tlačítka okna** minimalizovat, maximalizovat, zavřít; aplikace volí
   jen stranu (vlevo / vpravo).
4. **Obsah** kreslí aplikace do svých bufferů.
5. **Okraje** pro změnu velikosti drží kompozitor.

### Režimy

| Režim | Aplikace kreslí | Systém si nechá | Ovládání |
|---|---|---|---|
| **standardní** (výchozí) | jen obsah | rám, titulek, odznak, tlačítka | běžné |
| **vlastní titulek** | i plochu titulku (karty, vyhledávání, nástroje) | odznak vlevo, tlačítka okna, přesun přes deklarované oblasti `DRAG` / `NO_DRAG` | titulek 44 px; kompozitor táhne okno sám, i když aplikace nereaguje |
| **bez rámu** | vše | odznak u horního okraje při najetí a na 3 s při změně oprávnění | přesun jen přes oblast `DRAG` |
| **celá obrazovka** | vše | po najetí k hornímu okraji sjede pruh s odznakem a tlačítkem „Ukončit celou obrazovku“; **Esc podržený 1 s** ji ukončí vždy | bezpečná zkratka a systémové dialogy fungují vždy |

### Odznak důvěry

- Tvar: pilulka 26 px v barvách `trust-*`, ikona štítu, jméno aplikace,
  za oddělovačem ikony právě platných oprávnění (složka, síť, kamera,
  mikrofon, snímání obrazovky). **Červená tečka** = zařízení právě běží.
- Systémové aplikace mají místo jména slovo „Systém“ a štít s fajfkou.
- Klik, Enter nebo mezerník otevře **přehled oprávnění** (kreslí ho
  kompozitor ve vrstvě nabídek): aplikace, podpis, seznam oprávnění
  s časem posledního použití a tlačítkem **Odebrat**. Odebrání platí
  hned; aplikace o právo znovu požádá systémovým dialogem.
- Při změně oprávnění odznak třikrát pulzuje (1 s) a čtečka oznámí změnu.
- Strom přístupnosti: role `trust-badge`, jméno „Poznámky, ověřená
  aplikace. Smí: složka Dokumenty“.

### Stavy okna

| Stav | Vzhled |
|---|---|
| **aktivní** | titulek `surface-2`, tučné jméno, stín `window`, odznak v plných barvách; má fokus klávesnice |
| **neaktivní** | titulek splyne s obsahem, jméno vedlejší barvou, stín `window-inactive`; obsah se **neztlumí** (dá se číst) |
| **nereaguje** | po 3 s bez `PONG` kompozitor obsah zešedí a ukáže volby „Počkat“ / „Ukončit“; přesun, minimalizace a zavření fungují dál |
| **agent jedná** | rámeček 3 px v barvě `ai` a štítek „agent jedná v tomto okně“ nahoře; v liště tlačítko „Zastavit“ |

## Systémové prvky

### Vrstvy kompozitoru

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Vrstvy kompozitoru">
<rect class="err" x="120" y="10" width="520" height="30" rx="6"/><text class="t b" x="380" y="30" text-anchor="middle">2000 bezpečná obrazovka (Ctrl+Alt+Del)</text>
<rect class="warn" x="120" y="44" width="520" height="30" rx="6"/><text class="t b" x="380" y="64" text-anchor="middle">1000 systémové dialogy</text>
<rect class="warn" x="120" y="78" width="520" height="30" rx="6"/><text class="t" x="380" y="98" text-anchor="middle">900 ztlumení (scrim) přes všechno, i přes lištu</text>
<rect class="acc" x="120" y="118" width="520" height="30" rx="6"/><text class="t" x="380" y="138" text-anchor="middle">400 oznámení</text>
<rect class="acc" x="120" y="152" width="520" height="30" rx="6"/><text class="t" x="380" y="172" text-anchor="middle">300 nabídky, přehled oprávnění, spouštěč</text>
<rect class="acc" x="120" y="186" width="520" height="30" rx="6"/><text class="t" x="380" y="206" text-anchor="middle">200 lišta</text>
<rect class="box" x="120" y="220" width="520" height="30" rx="6"/><text class="t" x="380" y="240" text-anchor="middle">100 okna aplikací (+ odznak a tlačítka nad obsahem)</text>
<rect class="box" x="120" y="254" width="520" height="30" rx="6"/><text class="t" x="380" y="274" text-anchor="middle">0 plocha</text>
<text class="m" x="110" y="64" text-anchor="end">jen kompozitor</text>
<text class="m" x="110" y="30" text-anchor="end">jen kompozitor</text>
<text class="m" x="650" y="172">systém</text>
<text class="m" x="650" y="240">aplikace</text>
</svg>
<figcaption>Pořadí skládání. Do vrstev 900 a výš smí kreslit jen kompozitor.</figcaption>
</figure>

### Lišta

Výchozí poloha je **odsazený pruh dole** (8 px od okrajů, zaoblení 18 px,
výška 52 px). Polohu jde změnit v Nastavení > Vzhled nebo pravým tlačítkem
na liště: plovoucí dok, dole přes šířku, vlevo, vpravo, nahoře (40 px).

Obsah zleva doprava:

| Část | Co dělá |
|---|---|
| **Aplikace** | spouštěč: vyhledávání aplikací, souborů a akcí (API akcí), mřížka aplikací, účet, zamknout, vypnout |
| **Okna** | připnuté a běžící aplikace; tečka pod ikonou = běží, delší akcentová = aktivní; klik = aktivovat / minimalizovat |
| **Agent** | když agent AI jedná, korálová pilulka „Agent ovládá Soubory“ s tlačítkem **Zastavit** |
| **Úlohy** | počet běžících úloh a štítek agenta; klik otevře správce úloh |
| **Stav** | síť, hlasitost, oznámení s počtem nových |
| **Hodiny** | čas a datum (tabulkové číslice) |

Ve svislé poloze se popisky skryjí a zůstanou ikony s tooltipem; hodiny
se napíšou svisle.

### Přepínání oken

**Alt+Tab** otevře přepínač uprostřed obrazovky: náhledy oken v pořadí
posledního použití, předvybrané je předchozí okno. Další Tab (Shift+Tab
zpět) nebo šipky posouvají výběr, uvolnění Alt přepne, Esc zruší.
Náhledy jsou vrstvy, které kompozitor už má (nic se nepřekresluje).

### Oznámení

Vyjíždějí vpravo nahoře (posun 24 px + prolnutí, 240 ms), po 6 s zmizí do
centra oznámení (zvonek v liště). Oznámení agenta má levý pruh v barvě `ai`
a ikonu agenta. Pro čtečku jsou v oblasti `status` (oznámí se, ale nepřeruší).
Ztišení do určitého času je v centru oznámení.

### Systémové dialogy

Dialog oprávnění, hesla, výběru souboru nebo nového zařízení kreslí
kompozitor ve vrstvě 1000. Podvrh v okně aplikace vypadá vždy jinak,
protože:

1. **Ztlumení přes všechno, i přes lištu.** Aplikace nemůže ztlumit lištu
   ani jiná okna.
2. **Pruh „Systém“** nahoře v barvách `trust-*` s **bezpečnostní frází**
   uživatele (nastavuje se v Nastavení > Zabezpečení). Aplikace frázi nezná.
3. **Jen fyzický vstup**: umělý vstup (`SYNTHETIC`) dialog ignoruje.
4. **300 ms ochrana**: tlačítka jsou první 300 ms neaktivní (druhý klik
   dvojkliku ani rozepsaná klávesa nic nepotvrdí).
5. **Ctrl+Alt+Del** vždy ukáže skutečný systém.

Ovládání: Tab a šipky mění volbu, Enter potvrdí, Esc vybere poslední volbu
(zamítnutí). Fokus je uvnitř dialogu, dokud se nezavře.

Druhy v prototypu (`design/plocha.html`):

| Dialog | Volby |
|---|---|
| **oprávnění** (kamera) | Jen teď / Vždy pro aplikaci / Nepovolit |
| **heslo** (zmírnění zabezpečení) | pole hesla; Potvrdit / Zrušit |
| **výběr souboru** (powerbox) | systémový seznam souborů; aplikace dostane handle **jen na vybrané soubory**, jen pro čtení |
| **nová klávesnice** (BadUSB) | kód 3 číslic se napíše **na nové klávesnici**; do té doby se její stisky zahazují; Povolit / Zablokovat zařízení |

### Bezpečná obrazovka

Ctrl+Alt+Delete zachytí `inputd` dřív než kompozitor. Obrazovka ve vrstvě
2000 ukáže pruh „Systém“ s frází a volby Zamknout, Správce úloh, Přepnout
uživatele, Odhlásit, Zpět. Zavře ji jen fyzický vstup.

### Správce úloh

Okno se stejnými daty jako příkaz `tasks` ([Přehled úloh](architektura.md#přehled-úloh-odsouhlaseno)):

| Sloupec | Obsah |
|---|---|
| Úloha | lidsky čitelný název |
| Kdo spustil | ty (terminál), aplikace, služba, **agent** (štítek), neregistrovaný proces |
| Stav, průběh | běží / na pozadí / hotovo před 2 min; pruh průběhu (agent v barvě `ai`) |
| Paměť | spotřeba z rozpočtu jobu |
| Akce | pozastavit, zrušit; detail: připojit, log |

Filtry Vše / Moje / Agent / Systém. Řádky agenta mají levý pruh `ai`
a světlé pozadí `ai-soft`. Proces bez registrace je vidět jako „neznámý
proces v jobu N“. Řádky jdou procházet šipkami.

## Aplikace

### Terminál

Okno v režimu **vlastní titulek**: karty relací (relace patří službě
`termd`, odpojená relace má štítek), přepínač zápisu **B / C**, odznak
a tlačítka systému. Obsah:

- **Bloky výstupu**: každý příkaz je blok s levým pruhem podle stavu
  (`ok` zelený, běží akcentový, chyba červený, nanečisto žlutý, agent
  korálový), řádkem příkazu, dobou a časem. Akce bloku (sbalit, kopírovat,
  spustit znovu, porovnat dva běhy) se ukážou při najetí nebo fokusu.
- **Tabulky se schématem**: sloupce s typem (`size` velikost v jednotkách,
  `mtime` místní čas), řazení klikem na hlavičku, tabulkové číslice.
- **Doplňování podle typů** (Tab): na místě příkazu příkazy, za `where`,
  `sort`, `select` pole vstupu ze schématu předchozího příkazu, jinde cesty.
  Seznam je `listbox` u kurzoru; šipky, Enter / Tab, Esc. Šedý návrh
  z historie za kurzorem přijme šipka vpravo.
- **Nanečisto a undo**: příkaz nad limitem zámku (výchozí 20 souborů)
  ukáže blok se změnami a tlačítky **Převzít** / **Zahodit**; po převzetí
  se v řádku nápovědy objeví tlačítko **Vrátit** (`undo`).
- **Chyby**: rámeček s kódem (`NOT_FOUND`, `SH_UNKNOWN`), popisem,
  místem v řádku, návrhem opravy („Myslel jsi `~/Projekty`?“) a odkazem
  `explain`.
- **Panel úloh** vpravo: async úlohy a úlohy na pozadí s průběhem;
  úlohy agenta se štítkem a korálovým pruhem.
- **Zápis B / C**: přepínač v titulku nebo Ctrl+Shift+F přepíše
  zobrazené příkazy do druhého zápisu (dělá to formátovač `shlang`).

Barvy terminálu jsou `term-*` z tokenů a řídí se motivem. Vlákno 6b
udělalo animovaný návrh terminálu jako ukázku v konverzaci; prototyp
`design/terminal.html` ho sjednocuje s design systémem a umí stejné
přehrávání ukázek dokola (s pozastavením) i ruční psaní.

### Soubory

Okno: nástrojová lišta (zpět, vpřed, cesta, hledání, **Zkusit bokem**),
postranní panel (Místa, Úložiště = repozitáře FS v3, Bokem, Koš), seznam
a panel podrobností.

- **Verze**: časová osa verzí vybraného souboru (automatické verze, commity
  s popisem, změny převzaté od agenta). Výběr verze nabídne **Obnovit**
  (současná verze jde do koše, nic se neztratí), **Kopie vedle**,
  **Porovnat**.
- **Zkusit bokem** vytvoří klon složky (`aside_begin`). Žlutý pruh říká
  „Pracuješ bokem… Změny se neprojeví, dokud je nepřevezmeš“. Změněné
  řádky mají štítek (smazáno, přejmenováno, nové).

<figure class="diagram">
<svg viewBox="0 0 760 220" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Zkusit bokem v pruzkumniku">
<defs><marker id="vz2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="80" width="120" height="56" rx="6"/><text class="t b" x="70" y="104" text-anchor="middle">složka</text><text class="m" x="70" y="122" text-anchor="middle">originál</text>
<path class="ln" d="M130 108 L178 108" marker-end="url(#vz2)"/><text class="m" x="134" y="98">bokem</text>
<rect class="warn" x="180" y="80" width="140" height="56" rx="6"/><text class="t b" x="250" y="104" text-anchor="middle">klon</text><text class="m" x="250" y="122" text-anchor="middle">ty nebo agent mění</text>
<path class="ln" d="M320 108 L368 108" marker-end="url(#vz2)"/>
<rect class="acc" x="370" y="80" width="150" height="56" rx="6"/><text class="t b" x="445" y="104" text-anchor="middle">kontrola rozdílu</text><text class="m" x="445" y="122" text-anchor="middle">vybrat změny</text>
<path class="ln" d="M520 96 L578 52" marker-end="url(#vz2)"/>
<path class="ln" d="M520 108 L578 108" marker-end="url(#vz2)"/>
<path class="ln" d="M520 122 L578 168" marker-end="url(#vz2)"/>
<rect class="ok" x="580" y="24" width="170" height="50" rx="6"/><text class="t b" x="665" y="46" text-anchor="middle">převzít vybrané</text><text class="m" x="665" y="63" text-anchor="middle">atomicky, jako commit</text>
<rect class="err" x="580" y="84" width="170" height="50" rx="6"/><text class="t b" x="665" y="106" text-anchor="middle">konflikt</text><text class="m" x="665" y="123" text-anchor="middle">sloučit / uložit vedle</text>
<rect class="box" x="580" y="144" width="170" height="50" rx="6"/><text class="t b" x="665" y="166" text-anchor="middle">zahodit klon</text><text class="m" x="665" y="183" text-anchor="middle">jedním krokem</text>
</svg>
<figcaption>Zkusit bokem: změny vznikají v klonu a do originálu jdou jen vybrané.</figcaption>
</figure>

- **Zkontrolovat a převzít** otevře seznam změn se zaškrtávacími poli.
  Konflikt (soubor se mezitím změnil i v originálu) se nepřevezme sám:
  nabídne **Sloučit** nebo **Uložit vedle** (`aside_apply` konflikty jen
  ohlásí). Po převzetí zelený pruh „Převzato N změn“ s tlačítkem Vrátit.
- **Agent bokem**: složka, kde agent pracuje, má v postranním panelu ikonu
  agenta a korálový pruh „Asistent uklízí tuto složku bokem. Vidíš
  originál; agent má přístup jen ke klonu.“ s tlačítky **Zkontrolovat
  návrh** a **Zastavit agenta**.

### Nastavení a profily zabezpečení

Kategorie vlevo (role `tablist`, šipky nahoru a dolů), obsah vpravo.
**Profil** nastaví volby najednou; každou volbu jde změnit zvlášť.
Změněná volba má štítek „upraveno“, hodnotu podle profilu a tlačítko
**Vrátit**; profil ukáže „upraveno: N“ a nabídne „Vrátit vše podle profilu“.
**Zmírnění** (profil Volný nebo volnější hodnota) potvrzuje systémový
dialog s heslem; zpřísnění ne.

| Volba | Standardní (výchozí) | Přísný | Volný |
|---|---|---|---|
| ověření nového vstupního zařízení (BadUSB) | jen klávesnice | všechna vstupní zařízení | vypnuto |
| nové síťové spojení aplikace | dotaz u aplikací bez manifestu | dotaz u každého nového cíle | podle manifestu, bez dotazu |
| spouštění programů | nepodepsané se zeptá | jen podepsané | povolit vše |
| umělý vstup | zeptat se pokaždé | zakázat | povolit s indikátorem |
| snímání obrazovky | pamatovat povolení | zeptat se pokaždé | pamatovat povolení |
| zámek nebezpečných operací v terminálu | nad 20 souborů | nad 5 souborů | nad 100 souborů |
| cloudové modely AI | vypnuto | vypnuto a zamčeno | povolit s dotazem |
| agent pracuje jen bokem | vždy kontrola rozdílu | kontrola + heslo u převzetí | převzít bez kontroly do 5 změn |
| automatické zamčení | po 5 minutách | po 1 minutě | po 15 minutách |

Umělý vstup je ve všech profilech **viditelný** (lišta ukáže, kdo ovládá
klávesnici); profil mění jen to, zda se systém ptá.

Další stránky prototypu: **Vzhled** (motiv podle denní doby / světlý /
tmavý, poloha lišty s náhledy, písmo) a **Přístupnost** (velikost písma,
vysoký kontrast, čtečka obrazovky, omezit pohyb, vždy zvýraznit fokus,
větší cíle kliknutí). V prototypu tyto volby opravdu přepínají vzhled.

### Přihlášení

- **Zamčená obrazovka**: velké hodiny (64 px, váha 300), datum, souhrn
  oznámení (bez obsahu) a štítek, že agent čeká na kontrolu. Klik nebo
  klávesa otevře přihlášení; v Přísném profilu je nutné **Ctrl+Alt+Del**.
- **Přihlášení**: výběr účtu (role `radiogroup`), pole hesla jako pilulka
  s tlačítkem šipky, upozornění na Caps Lock, **bezpečnostní fráze**
  (ukáže se až po výběru účtu, podvržená obrazovka ji nezná). Chybné
  heslo: zatřesení polem (240 ms, při Omezit pohyb jen barva) a text;
  po 3 pokusech prodleva 30 s. Další metody: bezpečnostní klíč.
- Dole vlevo přístupnost (vysoký kontrast, čtečka **Ctrl+Alt+S**)
  a rozložení klávesnice, vpravo síť a vypnutí.

### Start systému

Startovací obrazovku kreslí init do framebufferu ještě před kompozitorem
(`design/start.html`). Proto jen levné efekty: plné barvy, přechody,
průhlednost a posun s měřítkem jedné vrstvy, žádné rozmazání. Kreslí se
jen změněná oblast kolem znaku, desítky kB za snímek.

| Čas | Co se děje |
|---|---|
| 0 ms | firmware; init převezme framebuffer (UEFI GOP) bez bliknutí, černé pozadí |
| +300 ms | teplé pozadí plochy a znak (symbol v prstenci) se prolnou za 900 ms, znak začne jemně dýchat, pod ním nápis |
| průběžně | **prstenec průběhu** se plní podle skutečných kroků initu: jádro, zařízení, soubory, síť, plocha (ne podle odhadu času) |
| krok déle než 2 s | pod znakem „čeká na: síť (DHCP)“; **Esc** kdykoli ukáže podrobný log startu a zpět |
| konec + 200 ms | **přechod do avataru**: znak se zmenší tak, aby prstenec přesně obkreslil kruh avataru (88 px), vjede do něj (600 ms, křivka standard) a rozplyne se (300 ms), zatímco se objeví avatar |
| +120 ms | jméno, pole hesla a bezpečnostní fráze najedou postupně; kompozitor převezme poslední snímek startu, takže přihlášení je pokračování téhož obrazu |

- **Chyba startu**: místo zaseknutého loga obrazovka s tím, co selhalo
  (služba, kód), a volbami **Zkusit znovu**, **Spustit předchozí verzi**
  (snapshot FS) a **Záchranný režim**.
- **Motiv**: podle posledního přihlášení (uložený u zavaděče), bez něj
  podle denní doby.
- **Omezit pohyb**: bez dýchání, prstenec se plní skokem, místo přesunu do
  avataru prolnutí 90 ms.
- Varianty, ze kterých se vybíralo (A dech, B kroky, C přechod do hodin),
  jsou v `design/start-varianty.html`.

### Pád systému

Obrazovka pro **kritický pád**: panika jádra nebo pád initu
(`design/pad.html`). Pády služeb, ovladačů a kompozitoru řeší
[zotavení](#jak-se-gui-vzpamatuje) a tahle obrazovka se u nich neukáže.

Kreslí ji **jádro samo** do framebufferu z firmwaru (UEFI GOP): bez
kompozitoru, bez ovladačů, bez alokace paměti. Písmo a symbol jsou
předrasterizované v obrazu jádra (`40 kB), barvy jsou pevné (tmavý motiv),
přepnutí pohledu je okamžité a jediný pohyb je odpočet.

| Pohled | Co ukazuje |
|---|---|
| **pro uživatele** (výchozí) | tmavě fialové pozadí, symbol v prstenci, který se plní odpočtem do restartu (15 s); „Systém musel zastavit“, ujištění, že soubory jsou v bezpečí a rozpracovaný stav se vrátí; kód chyby a id pádu (`PAGE_FAULT_KERNEL · #MO-7F3A-21C9`); QR odkaz na vysvětlení chyby v dokumentaci |
| **Dílna** (klávesa **D**) | vlevo stručný popis a klávesy, vpravo technická zpráva: druh paniky, verze, CPU, adresa, vlákno a job, zpětná stopa se symboly a řádky, registry, posledních 5 zpráv logu, kam se hlášení uloží; ve **vývojové verzi** systému se ukáže rovnou |

Klávesy: **Enter** restart hned, **D** přepnout pohled, **P** pozastavit
nebo obnovit odpočet, **šipky** posun technické zprávy. Stav se zapíše
i na sériovou linku.

- **Hlášení**: jádro ho uloží do rezervované paměti, která přežije teplý
  restart; init ho po dalším startu zapíše do `/system/pady/` a po
  přihlášení ukáže oznámení „Systém minule spadl“ (Zobrazit hlášení, Uložit
  na USB). **Nic se samo neodesílá**; QR kód jen odkazuje na dokumentaci.
- **Opakovaný pád**: 3 pády při startu za sebou = další start nabídne
  předchozí verzi systému (snapshot FS) nebo záchranný režim.
- **Přístupnost**: text aspoň 7 : 1, nic nebliká, odpočet jde pozastavit.

## Přístupnost

Strom přístupnosti je součást protokolu okna (`guiproto::a11y`). Knihovna
UI ho odvozuje z popisu prvků; aplikace nic nepíše ručně. Pravidla:

- **Každý prvek má roli a jméno.** Tlačítko jen s ikonou má jméno
  („Minimalizovat“), odznak má jméno s oprávněními, stav (vybráno,
  zaškrtnuto, rozbaleno, nedostupné) je vlastnost uzlu.
- **Jen klávesnicí**: Tab / Shift+Tab mezi skupinami, šipky uvnitř skupiny
  (seznam, karty, přepínač, segmenty, tabulka), Enter / mezerník aktivuje,
  Esc zavře nabídku nebo dialog. Fokus je vždy vidět (2 px `focus`).
  Každá stránka prototypu má odkaz „Přeskočit na obsah“.
- **Velké písmo** do 200 % bez oříznutí; rozvržení se přepočítá.
- **Vysoký kontrast**: černé pozadí, bílý text, žlutý akcent, rámečky 2 px,
  bez stínů a průhlednosti.
- **Pohyb**: Omezit pohyb nahradí posuny prolnutím.
- Živé oblasti: výstup terminálu `log`, oznámení `status`, chyby `alert`.

Role, které prototypy používají a které `guiproto::a11y::Role` zatím nemá
(6c má `window`, `titlebar`, `button`, `label`, `text-input`, `checkbox`,
`list`, `list-item`, `group`, `image`, `progress`, `terminal`). Návrh
doplnění:

| Role | Kde | Akce přístupnosti |
|---|---|---|
| `switch` | přepínače v Nastavení | `toggle` |
| `radio-group`, `radio` | segmenty, profily, výběr účtu | `select` |
| `tab-list`, `tab`, `tab-panel` | karty terminálu, kategorie Nastavení | `select` |
| `table`, `row`, `cell`, `column-header` | tabulky terminálu, správce úloh, seznam souborů | `sort` (hlavička) |
| `combobox`, `listbox`, `option` | příkazový řádek s doplňováním | `expand`, `select` |
| `menu`, `menu-item` | nabídka lišty | `activate` |
| `dialog`, `alert-dialog` | dialogy aplikací / systémové dialogy (jen kompozitor) | `dismiss` |
| `log`, `status`, `alert` | výstup terminálu, oznámení, chyby | - |
| `trust-badge` | odznak důvěry (jen kompozitor) | `activate` (přehled oprávnění) |

## Klávesové zkratky

| Zkratka | Co dělá |
|---|---|
| Ctrl+Alt+Del | bezpečná obrazovka (zachytí ji `inputd`) |
| Alt+Tab, Alt+Shift+Tab | přepínání oken |
| Super | spouštěč aplikací s vyhledáváním |
| Super+šipky | přichytit okno k polovině / maximalizovat / obnovit |
| Esc (podržet 1 s) | ukončit celou obrazovku |
| Ctrl+Alt+S | čtečka obrazovky (i na přihlášení) |
| Tab / Shift+Tab, šipky, Enter, Esc | ovládání prvků a dialogů |
| v terminálu: Tab, ↑ ↓, Ctrl+C, Ctrl+Shift+P, Ctrl+Shift+F | doplnit, historie, přerušit, nanečisto, přepnout zápis B / C |

## Svižnost, plynulost a zotavení (požadavek)

Základ celého GUI je, aby bylo **svižné a plynulé** a aby se **samo
vzpamatovalo** z pádu kterékoli části, a zároveň **hezké**. Obojí platí
stejně: efekty (rozmazání, stíny, animace) jsou navržené tak, aby běžely
na GPU v rozpočtu snímku, a když by ho překročily, systém je zjednoduší
(menší rozmazání, kratší animace), místo aby zpozdil odezvu. Svižnost
a zotavení mají měřitelné cíle a testy selžou, když je systém nesplní;
vzhled hlídají testy snímků obrazovky proti prototypům.

### Cíle

Měří se na referenčním stroji a v QEMU s virgl; při 120 Hz platí stejné
cíle v milisekundách, animace jsou časové, ne po snímcích.

| Ukazatel | Cíl |
|---|---|
| pohyb myši -> kurzor na obrazovce | do 1 snímku (16,7 ms při 60 Hz); kurzor v hardwarové rovině |
| stisk klávesy -> znak v textovém poli | do 2 snímků |
| klik -> viditelná reakce prvku | v dalším snímku |
| vynechané snímky při animaci | 0 v běžném provozu; v testu nejvýš 1 z 60 v QEMU |
| otevření spouštěče, přepínače oken, nabídky | do 50 ms |
| první snímek nově spuštěné aplikace | do 150 ms (kostra z uloženého stavu UI) |
| změna velikosti okna | okraj sleduje ukazatel v každém snímku, i když aplikace nestíhá |
| kompozitor v klidu | 0 % CPU a GPU (když se nic nemění, nic se nekreslí) |
| obraz po pádu kompozitoru | do 500 ms, okna na původních místech, fokus zachovaný |
| lišta po pádu shellu plochy | do 300 ms; okna se pádu vůbec nedotknou |
| pád nebo zaseknutí ovladače GPU | nejvýš 1 vynechaný snímek (přechod na 2D), po restartu ovladače zpět na GPU |

### Jak se svižnost zajistí

- **Kompozitor nikdy nečeká na aplikaci.** Skládá poslední potvrzený
  buffer okna. Opožděná aplikace má v okně starší snímek, ostatní okna
  ani kurzor to nezdrží. Při změně velikosti čeká na nový buffer nejvýš
  1 snímek, pak použije starý roztažený s výplní okraje.
- **Skládání co nejpozději před vsync**: kompozitor měří, jak dlouho
  skládání trvá, a začne až tak, aby stihl termín s rezervou 2 ms. Poloha
  ukazatele a nejnovější vstup se dosadí na poslední chvíli
  (late latching). Kurzor je v hardwarové rovině, takže se hýbe i když
  skládání trvá déle.
- **Celá obrazovka bez skládání**: hra nebo video na celé obrazovce jde
  přímo na výstup (direct scanout), kompozitor jen přidá odznak, když je
  potřeba.
- **Priority**: kompozitor a okno s fokusem mají QoS `interactive`,
  priorita se dědí přes IPC do služeb, čtení pro UI předběhne práci na
  pozadí ([Plynulost](architektura.md#plynulost-uživatelského-rozhraní-plán)).
- **Rychlý start aplikací**: výchozí tokeny jsou zkompilované v kódu
  (žádné čtení souboru při startu), písmo je ve **sdílené mezipaměti
  glyfů** (objekt paměti jen pro čtení, který plní služba písma), aplikace
  ukáže první snímek z uloženého stavu UI a teprve pak načítá data.
  Spouštěč, přepínač a nabídky jsou součást už běžícího shellu plochy.
- **Práce mimo UI vlákno**: knihovna UI dělá I/O asynchronně; UI vlákno
  jen reaguje a kreslí.

### Jak se GUI vzpamatuje

| Co spadne | Co se stane | Co uživatel uvidí |
|---|---|---|
| **aplikace** | jen její okno zmizí; po restartu se vrátí ze stavu v jejím datasetu | nic jiného se nezmění |
| **aplikace se zasekne** | `PING` bez `PONG` 3 s: okno zešedne, nabídne Počkat / Ukončit | ostatní okna běží dál |
| **shell plochy** (lišta, spouštěč, oznámení na obrazovce) | supervisor ho spustí znovu; stav lišty je v konfiguraci, úlohy v `taskd`, oznámení ve službě oznámení | lišta na chvíli zmizí (do 300 ms), okna zůstanou |
| **služba oznámení** | supervisor ji restartuje; fronta je uložená v jejím datasetu | nic se neztratí |
| **ovladač GPU** | kompozitor pozná pád nebo zaseknutí (plot nesplněný do 250 ms), přepne stejný strom vrstev na záložní 2D cestu, supervisor restartuje ovladač, kompozitor se vrátí na GPU | na chvíli zmizí rozmazání, obraz běží dál |
| **kompozitor** | viz níže | obraz zmizí nejvýš na 500 ms, okna se vrátí na svá místa |
| **kompozitor 3× za minutu** | init přepne na záchranný režim: textový terminál na framebufferu (`term::screen`) s možností restartovat GUI | systém zůstane ovladatelný |

**Restart kompozitoru bez ztráty oken:**

1. Kompozitor si průběžně ukládá **stav scény** do objektu paměti, který
   drží init (přežije pád kompozitoru): okna podle identity aplikace
   a klíče okna, poloha, velikost, pořadí, režim, fokus, motiv. Zápis je
   levný (jen změny) a atomický (dvě kopie, platná se přepne).
2. Po pádu ho supervisor spustí znovu; nový kompozitor načte stav scény
   a ukáže plochu a lištu hned.
3. Knihovna UI v aplikaci (`ui::client`) pozná zavřený kanál, připojí se
   znovu a **pošle celý stav okna v jednom `COMMIT`**: strom vrstev, strom
   přístupnosti a handly svých bufferů. Buffery patří aplikaci, takže se
   nic nepřekresluje: okno je zpět v dalším snímku.
4. `inputd` mezitím drží vstup pro vlastníka fokusu (nejvýš 1 s, pak ho
   zahodí), aby rozepsaný text nešel jinému oknu. Nový kompozitor nejdřív
   obnoví fokus, pak dostane podržené události.
5. Odznak důvěry nepotřebuje nic od aplikace: identitu a oprávnění dá
   kompozitoru znovu init.

Aplikace kreslící samy (bez `libs/ui`) dostanou totéž v nízké vrstvě
`ui::client::Window`: zpětné volání „připojeno znovu“, ve kterém pošlou
svůj strom vrstev.

**Hlídací pes:** kompozitor posílá supervisoru tep každý snímek, kdy něco
kreslí, a v klidu jednou za sekundu. Bez tepu 2 s (zablokování, nekonečná
smyčka) ho supervisor ukončí a spustí znovu podle postupu výše.

## Plán implementace

**Předpoklad:** kompozitor dostane GPU backend (virgl hned, Vulkan přes
Venus později). Návrh proto počítá s rozmazáním pod lištou a nabídkami,
jemnými stíny a leskem. Softwarové skládání nad virtio-gpu 2D zůstane
jako **záložní cesta** (stroj bez podporované GPU, záchranný režim):
stejný strom vrstev, jen bez rozmazání (lišta má pak plné pozadí
`surface` s krytím 90 %) a se stíny z předpočítaných textur.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Poradi vlaken implementace">
<defs><marker id="vz3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="m" x="10" y="16">fáze 1</text><text class="m" x="270" y="16">fáze 2</text><text class="m" x="530" y="16">fáze 3 (souběžně)</text>
<rect class="acc" x="10" y="30" width="200" height="56" rx="6"/><text class="t b" x="110" y="53" text-anchor="middle">G1 vykreslování na GPU</text><text class="m" x="110" y="71" text-anchor="middle">tvary, stíny, blur, animace</text>
<rect class="acc" x="10" y="110" width="200" height="56" rx="6"/><text class="t b" x="110" y="133" text-anchor="middle">G2 písmo a ikony</text><text class="m" x="110" y="151" text-anchor="middle">Inter, GPOS, atlas glyfů</text>
<rect class="box" x="10" y="190" width="200" height="56" rx="6"/><text class="t b" x="110" y="213" text-anchor="middle">S1 taskd</text><text class="m" x="110" y="231" text-anchor="middle">služba úloh (vlákno shellu)</text>
<rect class="box" x="10" y="270" width="200" height="56" rx="6"/><text class="t b" x="110" y="293" text-anchor="middle">S2 fsproto: klony, diff</text><text class="m" x="110" y="311" text-anchor="middle">vlákno FS</text>
<path class="ln" d="M210 58 L268 70" marker-end="url(#vz3)"/><path class="ln" d="M210 138 L268 80" marker-end="url(#vz3)"/>
<path class="ln" d="M210 58 L268 150" marker-end="url(#vz3)"/><path class="ln" d="M210 138 L268 160" marker-end="url(#vz3)"/>
<rect class="acc" x="270" y="46" width="200" height="56" rx="6"/><text class="t b" x="370" y="69" text-anchor="middle">G3 systémová vrstva</text><text class="m" x="370" y="87" text-anchor="middle">dekorace, odznak, dialogy</text>
<rect class="acc" x="270" y="126" width="200" height="56" rx="6"/><text class="t b" x="370" y="149" text-anchor="middle">G4 prvky UI</text><text class="m" x="370" y="167" text-anchor="middle">libs/ui, role a11y</text>
<rect class="warn" x="270" y="206" width="200" height="56" rx="6"/><text class="t b" x="370" y="229" text-anchor="middle">G5 zotavení a svižnost</text><text class="m" x="370" y="247" text-anchor="middle">restart compd, měření</text>
<path class="ln" d="M210 58 L268 230" marker-end="url(#vz3)"/>
<path class="ln" d="M470 154 L528 50" marker-end="url(#vz3)"/><path class="ln" d="M470 154 L528 110" marker-end="url(#vz3)"/><path class="ln" d="M470 154 L528 170" marker-end="url(#vz3)"/><path class="ln" d="M470 154 L528 230" marker-end="url(#vz3)"/><path class="ln" d="M470 154 L528 290" marker-end="url(#vz3)"/>
<path class="ln" d="M470 74 L528 290" marker-end="url(#vz3)"/>
<path class="ln" d="M210 218 L528 54" marker-end="url(#vz3)"/><path class="ln" d="M210 298 L528 176" marker-end="url(#vz3)"/>
<rect class="ok" x="530" y="26" width="220" height="44" rx="6"/><text class="t b" x="640" y="53" text-anchor="middle">A1 shell plochy</text>
<rect class="ok" x="530" y="86" width="220" height="44" rx="6"/><text class="t b" x="640" y="113" text-anchor="middle">A2 terminál</text>
<rect class="ok" x="530" y="146" width="220" height="44" rx="6"/><text class="t b" x="640" y="173" text-anchor="middle">A3 soubory</text>
<rect class="ok" x="530" y="206" width="220" height="44" rx="6"/><text class="t b" x="640" y="233" text-anchor="middle">A4 nastavení</text>
<rect class="ok" x="530" y="266" width="220" height="44" rx="6"/><text class="t b" x="640" y="293" text-anchor="middle">A5 přihlášení</text>
</svg>
<figcaption>Vlákna a závislosti. G1 a G2 běží souběžně, potom G3, G4 a G5, potom aplikace.</figcaption>
</figure>

### Fáze 1: základ (souběžně)

> **Změna proti v1.4:** fáze 1 je v systému hotová v 2D cestě a ve
> skládání na GPU se stejnými pixely (bez rozmazání). Stíny jsou
> předpočítané masky, atlas glyfů je zatím mezipaměť na CPU. Popis
> a odchylky: [GUI: Vzhled podle návrhu](gui.md#vzhled-podle-návrhu-fáze-1).

| Vlákno | Obsah | Kde | Hotovo, když |
|---|---|---|---|
| **G1 vykreslování na GPU** | zaoblené obdélníky a rámečky jako shader (vzdálenostní pole, vyhlazené hrany v jednom průchodu); stíny jako rozmazaný tvar (dvouprůchodový Gauss nebo dual Kawase) s mezipamětí podle velikosti; **rozmazání pozadí** pro lištu, nabídky a ztlumení (jen pod oblastí, v poloviční velikosti); přechody; průhlednost skupin přes mezibuffer; **časová osa animací** v `compd` (křivky a délky z tokenů, přerušitelné, vynechání snímku místo zpoždění vstupu); načtení tokenů a přepínání motivu | `compd`, `libs/comp`, backend virgl | snímek s 10 okny, stíny a rozmazanou lištou do 4 ms GPU času v QEMU; animace otevření okna bez vynechaného snímku; záložní 2D cesta vykreslí totéž bez rozmazání |
| **G2 písmo a ikony** | statické řezy Inter (400, 500, 600, 700) a JetBrains Mono (400, 600) v `libs/font/fonts/`; párový kerning z `GPOS`; **atlas glyfů na GPU** se subpixelovým umístěním (1/4 px); mezipaměť pro škálování 100-200 % a HiDPI; ikony: cesty z `design/assets/mo.js` převést při sestavení do formátu, který rasterizér umí (stejné křivky jako glyfy), a kreslit je do atlasu jako glyfy | `libs/font`, `libs/gfx`, build skript | „Te“, „Vo“ s kerningem; čeština bez chyb; test s poškozeným souborem písma dál bez paniky; 64 ikon ve 4 velikostech |

### Fáze 2: systém a prvky (souběžně, po fázi 1)

| Vlákno | Obsah | Kde | Hotovo, když |
|---|---|---|---|
| **G3 systémová vrstva** | dekorace podle [Anatomie](#anatomie) a [Režimů](#režimy) (titulek 40 / 44 px, tlačítka 28 px s cílem 40 px, okraj 6 px); stavy okna včetně „agent jedná“; **odznak důvěry** a přehled oprávnění s odebráním (data od initu a zprostředkovatele oprávnění); systémové dialogy ve vzhledu prototypu (pruh „Systém“, fráze, 300 ms); bezpečná obrazovka; přepínač oken Alt+Tab (náhledy jsou vrstvy, které kompozitor má); celoobrazovkový pruh a Esc 1 s | `compd`, `guiproto::control`, init | všechny režimy a stavy z `design/okna.html` v QEMU; odebrání oprávnění se projeví do jednoho snímku; dialog nepřijme umělý vstup |
| **G4 prvky UI** | nové prvky: přepínač, segmenty, karty, tabulka s řazením, combobox s doplňováním, nabídka, plovoucí panel, tooltip, oznámení, dialog aplikace, posuvník; styl podle tokenů (zaoblení, pilulky, fokus 2 px); ovládání klávesnicí podle [Přístupnosti](#přístupnost); **nové role** v `guiproto::a11y::Role`; velké písmo a vysoký kontrast | `libs/ui`, `libs/guiproto` | ukázková aplikace se všemi prvky odpovídá `design/design-system.html` v obou motivech a v kontrastu; testy stromu přístupnosti pro každý prvek |
| **G5 zotavení a svižnost** | [restart kompozitoru bez ztráty oken](#jak-se-gui-vzpamatuje) (stav scény v objektu paměti u initu, opětovné připojení v `ui::client`, podržení vstupu v `inputd`); přechod na 2D při pádu ovladače GPU; hlídací pes; záchranný režim; skládání před vsync s late latching, hardwarový kurzor, direct scanout; sdílená mezipaměť glyfů; **měření**: kompozitor zapisuje časy vstupu, skládání a výstupu každého snímku a `cargo xtask` z nich dělá zprávu | `compd`, `libs/ui`, `inputd`, init, supervisor | všechny [cíle](#cíle) splněné v QEMU testem; test zabije kompozitor, shell plochy a ovladač GPU a změří návrat |

### Fáze 3: aplikace (souběžně, po G4)

| Vlákno | Obsah | Závisí na | Hotovo, když |
|---|---|---|---|
| **A1 shell plochy** | nová systémová aplikace s capability pro vrstvy 200-400: lišta v 6 polohách, spouštěč s vyhledáváním (API akcí), centrum oznámení (služba oznámení), správce úloh, pilulka „agent jedná“ se Zastavit | G3, G4, **S1 `taskd`** (vlákno shellu) | `design/plocha.html` bez systémových dialogů (ty jsou G3) |
| **A2 terminál** | `gui-term` kreslí bloky, tabulky, chyby a nanečisto jako prvky UI nad `termd`; doplňování jako combobox; panel úloh; přepínač B / C | G4, `termd` | ukázky z `design/terminal.html` proti skutečnému `termd` |
| **A3 soubory** | průzkumník: seznam, podrobnosti, časová osa verzí a obnova, zkusit bokem, převzetí vybraných změn, konflikty | G4, **S2 klony a diff v `fsproto`** (vlákno FS) | celý tok „bokem“ z `design/soubory.html` nad skutečným FS |
| **A4 nastavení** | aplikace Nastavení a **konfigurační služba** (TOML se schématem, verzovaná s FS); profily zabezpečení a samostatné volby napojené na `inputd` (BadUSB), `fwd` (síť), bránu spuštění (zatím jen plán), zámek v terminálu; vzhled a přístupnost | G4, G3 (dialog s heslem) | změna profilu se projeví ve službách bez restartu; zmírnění chce heslo |
| **A5 přihlášení** | zamčená obrazovka a přihlášení ve vrstvě systému, správce relací uživatelů, ukládání hesel (pomalý hash, např. Argon2id), zamykání po nečinnosti, režim s Ctrl+Alt+Del | G3, G4, init (relace = sada capabilities) | přihlášení, zamčení, odemčení a přepnutí uživatele v QEMU |

**Proč takhle:** po fázi 1 a G3 už systém „vypadá jako on“ (okna,
dekorace, odznak, stíny, písmo), i když aplikace jsou ještě jednoduché.
Aplikace pak běží souběžně, protože se navzájem nepotřebují; brzdit je
budou jen služby mimo GUI (`taskd`, klony v `fsproto`, konfigurace).

**Testy (všechna vlákna):**

- **snímky obrazovky**: QEMU `screendump` přes QMP a porovnání s referenčním
  obrázkem s tolerancí (motiv světlý, tmavý, kontrast; písmo 100 a 200 %);
- **plynulost a svižnost**: kompozitor zapisuje časy každého snímku; test
  selže, když při animaci okna vypadne víc než 1 snímek z 60 nebo když
  odezva vstupu překročí [cíle](#cíle);
- **zotavení**: test zabije kompozitor, shell plochy, službu oznámení
  a ovladač GPU a ověří, že se okna vrátila na svá místa v daném čase
  a žádný stisk klávesy nešel jinému oknu;
- **přístupnost**: strom přístupnosti každé obrazovky má u všech
  ovládacích prvků roli a jméno; průchod Tabem dosáhne každého prvku;
- **kontrast**: `node design/tools/build-tokens.mjs --check` v `cargo xtask test`.

**Rozhodnutí k implementaci (odsouhlaseno 7. 11. 2026):**

| Otázka | Možnosti | Rozhodnutí |
|---|---|---|
| jak se tokeny dostanou do kódu | A) `build.rs` vygeneruje Rust konstanty z `tokens.toml`; B) knihovna čte TOML za běhu; C) A pro výchozí hodnoty + B pro uživatelské motivy | **C**: rychlý start bez parseru, motivy jdou přidat bez překladu |
| kdo kreslí lištu a spouštěč | A) kompozitor; B) samostatná aplikace s capability pro systémové vrstvy | **B**: pád lišty neshodí kompozitor, supervisor ji restartuje |
| služba oznámení | A) součást shellu plochy; B) samostatná služba | **B**: oznámení musí přežít restart lišty a jde je posílat i bez GUI (terminál, log) |

⚠️ **Rizika (řečená předem):**

- **Kerning a písmo**: bez čtení `GPOS` bude Inter bez kerningu (viz
  [Typografie](#typografie)). Dá se to vyřešit předzpracováním při sestavení.
- **Rozmazání a GPU v QEMU**: virgl potřebuje hostitele s OpenGL
  (`egl-headless` pro testy); kde chybí, testy poběží jen se záložní 2D
  cestou a rozmazání se neověří. Rozmazání stojí GPU čas úměrný ploše:
  proto jen pod lištou, nabídkami a dialogem, v poloviční velikosti
  a jen když se pod ním něco změnilo.
- **Motiv podle denní doby** potřebuje polohu (východ a západ slunce).
  Bez polohy se použije pevný čas 7:00-20:00 podle pásma.
- **Napodobení systémového dialogu v celoobrazovkové aplikaci** nejde
  vyloučit jen vzhledem; ochrana je fráze, fyzický vstup a Ctrl+Alt+Del
  (stejné riziko jako v návrhu 6c).
- **Prototypy načítají písma z Google Fonts** (jen kvůli náhledu
  v prohlížeči); bez sítě se použije systémové písmo prohlížeče a vzhled
  se mírně liší. Systém bude mít písma přibalená.

## Verze návrhu

| Verze | Datum | Co |
|---|---|---|
| v1 | 7. 11. 2026 | první návrh: styl C Vřelý, design systém, okna, systémové prvky, 4 aplikace, přístupnost, tokeny |
| v1.1 | 7. 11. 2026 | plán implementace ve 3 fázích; počítá s GPU backendem (rozmazání, stíny shaderem), 2D jako záložní cesta |
| v1.2 | 7. 11. 2026 | svižnost, plynulost a zotavení jako požadavek s měřitelnými cíli; restart kompozitoru bez ztráty oken; vlákno G5; rozhodnutí k implementaci (C, B, B) |
| v1.3 | 7. 11. 2026 | logo (vektory od uživatele) v prototypech a v design systému; startovací obrazovka s přechodem do avataru |
| v1.4 | 7. 11. 2026 | obrazovka kritického pádu (A Klid + technická zpráva na D), oznámení po dalším startu; příručka grafického jazyka |
| v1.5 | 8. 11. 2026 | **implementace fáze 1** v systému: tokeny jako data (`libs/theme`), Inter a JetBrains Mono s kerningem `GPOS`, okna, odznak, lišta, plocha, dialog a terminál podle prototypů; odchylky v [GUI: Vzhled podle návrhu](gui.md#vzhled-podle-návrhu-fáze-1) |
