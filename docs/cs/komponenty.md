# Knihovna komponent

Specifikace základních ovládacích prvků GUI: rozměry, tokeny, stavy
a ovládání klávesnicí, pro počítač, tablet a televizi. Podle ní se ladí
knihovna `libs/ui`. Na konci je seznam rozdílů proti současné
implementaci (`libs/ui`, ukázka `user/gui-komponenty`).

**Stav:** návrh **[navrženo]**. Hodnoty bere z `design/tokens.toml`.
Čísla označená *navrženo* v tokenech zatím nejsou (seznam v části
[Nové tokeny](#nové-tokeny-navrženo)).

> **Klikací prototyp:** [design/komponenty.html](../../design/komponenty.html).
> Ke každé komponentě je živá ukázka a řádek všech stavů vynucených
> staticky (výchozí, najetí, stisk, fokus, vypnuto, chyba, vybráno).
> Dál je tam krátká specifikace a rámeček s rozdílem proti `libs/ui`.
> Přepínač nahoře mění hustotu (počítač / tablet / televize). Lišta
> prototypu mění motiv, vysoký kontrast, písmo a pohyb.

Navazuje na [Vzhled a ovládání](vzhled.md) (design systém, tokeny)
a [Grafika a okna](gui.md) (jak `libs/ui` funguje uvnitř).

## Společná pravidla stavů

| Stav | Vzhled | Tokeny |
|---|---|---|
| Výchozí | plocha, okraj 1 px | `surface`, `border_strong` (kontrast hranice aspoň 3 : 1), `ink` |
| Najetí | ztmavení plochy | `surface_3`; plné varianty míchají 88 % barvy + `ink` |
| Stisk | další ztmavení, zmenšení na 98 % | 76 % + `ink`, `motion.fast` |
| Fokus | prstenec mimo prvek | `focus`, `size.focus_ring` 2, `size.focus_offset` 2; pole odsazení 0 |
| Vypnuto | krytí 50 %, žádná reakce | `opacity.disabled` |
| Chyba | okraj 2 px, ikona a věta pod prvkem | `danger`, `danger_soft` |
| Vybráno / zapnuto | měkký akcent, u voleb plný | `accent_soft`, `accent`, `accent_ink` |

Zásady:

- **Akce při uvolnění.** Tlačítko se spustí při uvolnění myši i klávesy.
  Když uživatel odjede ze stisknutého prvku, akce se zruší.
- **Fokus jen z klávesnice.** Prstenec se kreslí po ovládání klávesnicí
  nebo ovladačem, ne po kliknutí myší.
- **Chyba nikdy jen barvou.** Vždy ikona `error` a věta, jak chybu
  opravit. Text je spojený s prvkem (u čtečky jako popis) a ohlásí se
  při odeslání. Okraj 2 px se kreslí dovnitř, rozvržení se nehýbe.
- **Výběr a fokus jsou dvě věci.** Vybraný řádek má `accent_soft`.
  Aktivní řádek (kurzor) má prstenec. Můžou být na různých řádcích.
- **Vypnuté prvky.** Vypnuté tlačítko a pole nejsou v pořadí Tab.
  Vypnutá položka nabídky jde najít šipkami a popisek řekne proč.

## Hustoty

Hustotu nevolí aplikace, plyne z kontextu okna: `posture`, `min_hit`,
`pointers`, `viewing_distance` (viz [Tablet a televize](vzhled.md#tablet-a-televize-návrh)).

| Rozměr | Počítač | Tablet | Televize | Zdroj |
|---|---|---|---|---|
| Ovládací prvek | 36 | 44 | 56 | `control_h`, `hit_min_touch`; TV navrženo |
| Malý prvek | 28 | 36 | 44 | `control_h_compact`; tablet, TV navrženo |
| Řádek seznamu, tabulky, stromu | 36 | 44 | 56 | `max(control_h, min_hit)` jako dnes |
| Položka nabídky | 32 | 44 | 56 | navrženo |
| Písmo těla | 14 / 20 | 14 / 20 | 21 / 30 | `type.body`; TV &times; 1,5 navrženo |
| Ikona v prvku | 20 | 20 | 28 | `icon_m`; TV navrženo |
| Zaškrtávátko, radio | 18 | 22 | 28 | navrženo |
| Přepínač (š &times; v, jezdec) | 44 &times; 26, 18 | 52 &times; 32, 24 | 64 &times; 36, 26 | `switch_*`; tablet, TV navrženo |
| Fokus (tah / odsazení) | 2 / 2 | 2 / 2 | 4 / 3, zvětšení 1,04 a stín | navrženo pro TV |
| Najetí (hover) | ano | ne | ne | podle `pointers` |
| Popisek | najetí 500 ms, fokus hned | dlouhý stisk | 800 ms na fokusu | dnes konstanta |

Na televizi je fokus jediné zvýraznění: neexistuje ukazatel, takže
fokus musí být vidět z 3 m (proto 4 px a zvětšení).

## Klávesnice obecně

```
Tab / Shift+Tab   -> dalsi / predchozi ZASTAVKA (skupina)
                     zaskrtavatko, prepinac, tlacitko = 1 zastavka
                     radio, zalozky, segmenty, seznam, tabulka,
                     strom, nabidka, skupina stitku = 1 zastavka
Sipky             -> pohyb UVNITR zastavky (pamatuje si posledni prvek)
Enter             -> vychozi akce (tlacitko, otevreni, potvrzeni dialogu)
Mezernik          -> prepnuti (tlacitko, zaskrtavatko, vyber radku)
Esc               -> zavre nejvnitrnejsi vrstvu a vrati fokus:
                     popisek -> nabidka -> bublina -> dialog
Pismena           -> hledani psanim (seznam, nabidka, vyber, strom; 1 s)
Shift+F10, Menu   -> kontextova nabidka u prvku s fokusem
F6                -> dalsi oblast okna (napr. oznameni)
```

Na televizi platí totéž s ovladačem: šipky, OK = Enter, Zpět = Esc,
tlačítko Menu = kontextová nabídka.

## Komponenty

Úplná specifikace a stavy jsou v prototypu. Tady je souhrn pro
`libs/ui`: rozměry, klíčové chování a stav implementace (**ano**,
**částečně**, **ne**).

| Komponenta | `libs/ui` | Rozměry a tokeny | Klávesnice |
|---|---|---|---|
| Tlačítko: primární, sekundární, nebezpečné, ikonové | ano | výška `--k-h`, min. šířka 2 &times; výška, `s5`, pilulka; primární `accent`, nebezpečné `danger`; stav *probíhá* (kolečko, pevná šířka) | Enter, Mezerník při uvolnění |
| Textové pole, víceřádkové | ano | `--k-h`, `radius.control`, `s4`; popisek nad, nápověda / chyba pod | šipky, Ctrl+šipky, výběr se Shift, schránka, undo; víceřádkové Ctrl+Enter odešle |
| Číslo | ano | jako pole, zarovnání vpravo, jednotka, krokovací tlačítka mimo Tab | &uarr; &darr; krok, PgUp / PgDn 10 kroků, Home / End |
| Heslo | ano | jako pole + oko (`aria-pressed`), varování Caps Lock | jako pole, nikdy návrhy |
| Hledání | ano | lupa vlevo, křížek jen s textem, filtr s prodlevou 150 ms | Esc smaže, druhé Esc opustí, &darr; do výsledků |
| Zaškrtávátko | částečně | políčko 18 / 22 / 28, `radius.xs`, okraj 2 px; neurčitý stav | Mezerník |
| Přepínač | ano | `switch_*`; okamžitý účinek | Mezerník |
| Radio | částečně | kruh 18 / 22 / 28; skupina je jedna zastávka | šipky rovnou vybírají |
| Posuvník | ano | dráha 4 px (dnes 2), jezdec `switch_knob` | šipky, PgUp / PgDn 10 %, Home / End; kolečko jen s fokusem |
| Výběr (select) | ano | jako pole; seznam `radius.card`, max. 8 položek | Enter / Mezerník / Alt+&darr; / F4, šipky, písmena, Esc, Tab zvolí aktivní |
| Záložky | ano | `--k-h`, max. šířka 180, vybraná podtržení 3 px `accent` | &larr; &rarr; Home End s automatickou aktivací, Ctrl+PgUp / PgDn |
| Segmentový přepínač | ano | dráha `surface_3`, vybraný díl `surface` + `accent_ink` | &larr; &rarr; Home End |
| Nabídka, kontextová nabídka | ano | `radius.card`, položka `--k-item`, zkratka vpravo, oddělovač, nadpis skupiny, podnabídka, nebezpečná položka poslední | &uarr; &darr; s cyklem, &rarr; / &larr; podnabídka, písmena, Esc, Tab zavře |
| Popisek (tooltip) | ano | `ink` na `surface`, max. 280, `radius.xs` | ukáže se při fokusu, Esc skryje |
| Bublina (popover) | ne | `radius.card`, šipka 12, šířka 300, nemodální | fokus dovnitř, Esc vrátí na spouštěč |
| Dialog aplikace | ne | `dialog_w`, `radius.window`, `shadow.dialog`, `scrim` | past fokusu, Enter výchozí, Esc zrušit, návrat fokusu |
| Oznámení (toast) | ne | max. 440, min. výška 48, inverzní barvy, 5 s / 8 s s akcí, nejvýš 3 | nebere fokus; F6, Esc |
| Seznam | ano | řádek `--k-row`, `radius.control` | šipky, Ctrl+Mezerník, Shift+šipky, Ctrl+A, písmena, Enter |
| Tabulka | ano | řádek `--k-row`, hlavička `type.caption` na `surface_2`, čísla vpravo | jako seznam, Enter v hlavičce třídí |
| Strom | ne | řádek `--k-row`, odsazení úrovně 20, trojúhelník otočený o 90&deg; | &rarr; rozbalí / dítě, &larr; sbalí / rodič, `*` sourozenci |
| Průběh, kolečko, kostra | částečně | pruh 8 px; kolečko 24 / 16 / 40; kostra 12 px | není zaměřitelný; čtečce po 10 % |
| Odznak, štítek, filtrovací štítek | částečně | odznak 18; štítek 24 `type.caption`; filtrovací `--k-h-sm` | skupina štítků: šipky, Mezerník, Delete |
| Drobečky | ne | `--k-h-sm`, oddělovač `/` v `ink_faint`, zkrácení na „...“ | každý odkaz zastávka Tab |
| Rozbalovací panel | ne | hlavička `--k-h` + 12, `radius.card`, šipka otočená o 180&deg; | Enter / Mezerník, ve skupině &uarr; &darr; |
| Prázdný stav | ne | kruh 72 s ikonou `icon_xl`, nadpis `type.title`, jedna akce | fokus na akci |
| Panel (lišta, Nastavení) | ano | šířka `size.panel_w` (360), řádek nejméně `size.panel_row` (40), mezera ikona-text `size.panel_gap` (10), okraj hlavičky a paty `size.panel_edge` (14), boky `s5`; hlavička s ikonou, tučným názvem (`body_strong`) a souhrnem (`caption`, `ink_muted`), řádek s ikonou, názvem a popisem (tlačítko s najetím `surface_3`, volba radio s fajfkou vpravo, přepínač vpravo, informace), sekce (`caption` velkými), stavový řádek (`surface_3` / `danger_soft`, `radius.control`), pata s čarou `border`. Skládá se z běžných prvků (`libs/ui/src/panel.rs`: `panel_header`, `.panel_row(ikona, popis)`, `panel_section`, `panel_state`, `panel_footer`); popis řádku je hodnota pro čtečku | volby radio jsou skupina (šipky vyberou), Tab mezi skupinou, přepínačem a patou; fokus jako vnitřní rámeček `radius.control` |

Kdy co použít:

- **Přepínač vs. zaškrtávátko.** Přepínač má okamžitý účinek (Wi-Fi).
  Zaškrtávátko se uplatní až potvrzením, nebo vybírá více věcí.
- **Radio vs. výběr.** Radio pro 2 až 5 možností, víc možností = výběr.
- **Záložky vs. segmenty.** Záložky přepínají obsah (panel). Segmenty
  přepínají režim téhož obsahu.
- **Toast vs. dialog vs. systémové oznámení.** Toast je krátká zpráva
  v okně o tom, co se právě stalo, s nejvýš jednou akcí. Chyba, která
  vyžaduje rozhodnutí, patří do dialogu. Systémové oznámení (mimo okno,
  s historií) kreslí systém.
- **Dialog aplikace vs. systémový dialog.** Dialog aplikace je modální jen
  pro své okno. Systémový dialog (oprávnění, heslo, trvalé mazání) kreslí
  kompozitor s pruhem „Systém“ a rámem `trust_border`. Aplikace ho
  nenapodobí, protože barvy `trust_*` nedostane.

## Rozdíly proti libs/ui

Zjištěno z kódu `libs/ui/src/` a ukázky `user/gui-komponenty` (stav
14. 11. 2026). Seřazeno podle dopadu.

**Chybějící widgety:**

1. **Dialog aplikace.** Modální vrstva v okně, past fokusu, výchozí
   a rušicí tlačítko, návrat fokusu. Token `dialog_w` už existuje.
2. **Oznámení (toast).** Oblast oznámení okna, fronta, časovač
   s pauzou, ohlášení čtečce.
3. **Strom.** Potřebuje ho správce souborů a nastavení. Navrhuji ho
   postavit na seznamu (řádek + úroveň + rozbaleno), ne jako novou
   vrstvu.
4. ~~**Číselné pole** a **pole hledání.**~~ Hotovo (`number_input`,
   `search_input`, [Textová pole a schránka](gui.md#textová-pole-a-schránka)).
   **[implementováno]**
5. **Obecná bublina (popover).** Vrstva `popup` dnes umí jen nabídku,
   výběr a návrhy.
6. **Drobečky, rozbalovací panel, prázdný stav, kruhové kolečko,
   kostra, obecný odznak a filtrovací štítek.**

**Chybějící varianty a stavy:**

1. Tlačítko má jen jeden styl. Chybí primární, nebezpečné, ghost,
   přepínací (`pressed`) a stav *probíhá*.
2. Zaškrtávátko nemá neurčitý stav, hover ani vypnutý vzhled
   (`st.disabled` se ignoruje).
3. Radio při vypnutí neztlumí plochu. ~~Skupinu radio skládá aplikace
   sama a šipky mezi radio nefungují.~~ Sourozenci radio jsou skupina
   s jednou zastávkou Tab a šipkami. **[implementováno]**
4. Stav chyba je jen u textových polí. Chybí u výběru, zaškrtávátka,
   posuvníku, záložky a segmentu.
5. ~~Pole nemají hover. Heslo nemá oko ani varování Caps Lock.~~ Hotovo
   (najetí ztmaví okraj, oko, varování Caps Lock). **[implementováno]**
6. ~~Seznam má jen šipky.~~ Hotovo: `list(..).multi_select(true)` sdílí
   logiku výběru s tabulkou (`libs/ui/src/selection.rs`). **[implementováno]**
   Původně: seznam má jen šipky. Chybí Ctrl+Mezerník, Shift+šipky, Ctrl+A
   a hledání psaním. Tabulka to všechno už umí, ať seznam sdílí její
   logiku výběru.
7. Nabídka nemá ~~podnabídky~~, nadpis skupiny, nebezpečnou položku
   ~~ani hledání psaním~~. Podnabídky a mnemoniky jsou hotové.
   **[implementováno]**
8. ~~Výběr: Tab v otevřeném seznamu zavře beze změny.~~ Tab zvolí
   aktivní položku a jde dál; zavřený výběr umí hledání psaním a Home,
   End. **[implementováno]**
9. Záložky: vybraná záložka je jen plocha `surface_2`, která má ve
   světlém motivu proti `surface` kontrast pod 3 : 1. Navrhuji podtržení
   3 px `accent`. Vypnuté záložky se šipkami nepřeskakují.
10. ~~Posuvník mění hodnotu kolečkem i bez fokusu.~~ Kolečko jen
    s fokusem. **[implementováno]** Dráha má 2 px, navrhuji 4.

**Hustota:**

- `control_h_compact` (28) není v `look.rs` napojený.
- Hustota se dnes mění jen výškou přes `min_hit` (výchozí 32 je
  v `context.rs` jako číslo). Chybí škálování písma, ikon, políček
  a fokusu pro televizi a vypnutí hoveru na dotyku.

**Čísla mimo tokeny** (v rozporu s pravidlem „žádná čísla vzhledu mimo
`tokens.toml`“, i když jsou malá):

- `TIP_DELAY` 500 ms a `TIP_MAX_W` 280 (`popup.rs`);
- `TAB_MAX` 180 (`controls.rs`);
- šířka neurčitého pruhu 30 %;
- posun jezdce přepínače o 3 px;
- výchozí `min_hit` 32 (`context.rs`);
- pevná výška tabulky 8 řádků + hlavička.

**Ukázka `user/gui-komponenty`** ukazuje stav chyba jen u hesla.
Vypnutý stav ukazuje jen u tlačítka. Neurčitý stav ani hustotu
neukazuje vůbec. Po doplnění widgetů by měla odpovídat stránce
`design/komponenty.html` (stejné pořadí a stavy), aby šly snímky
vzhledu (`libs/ui/tests/shots/`) porovnat s návrhem.

## Nové tokeny (navrženo)

Návrh doplnění `design/tokens.toml`. Zatím nejsou v tokenech, prototyp
je má jako proměnné `--k-*`.

| Token | Hodnota | Účel |
|---|---|---|
| `size.control_h_tv` | 56 | ovládací prvek a řádek na televizi |
| `size.control_h_compact_touch` / `_tv` | 36 / 44 | malý prvek na tabletu a TV |
| `size.menu_item_h` | 32 | položka nabídky (tablet a TV = výška prvku) |
| `size.check` | 18 (tablet 22, TV 28) | zaškrtávátko a radio |
| `size.slider_track` | 4 | dráha posuvníku |
| `size.tab_max` | 180 | max. šířka záložky (dnes `TAB_MAX`) |
| `size.tip_max_w` | 280 | max. šířka popisku (dnes `TIP_MAX_W`) |
| `size.toast_w` | 440 | max. šířka oznámení v okně |
| `size.focus_ring_tv` / `focus_offset_tv` | 4 / 3 | fokus na televizi |
| `type.scale_tv` | 1.5 | násobek písma na televizi (k uživatelskému zvětšení) |
| `motion.tip_delay` | 500 (TV 800) | prodleva popisku (dnes `TIP_DELAY`) |
| `motion.toast_ms` / `toast_action_ms` | 5000 / 8000 | doba zobrazení oznámení |

Tablet a TV varianty můžou být buď samostatné klíče, nebo nová sekce
`[density.tablet]`, `[density.tv]`. Sekce je čistší, ale `libs/theme`
ji musí umět načíst; o tom rozhodne implementace.

## Rizika

- **Animace výšky rozbalovacího panelu** vyžaduje rozvržení v každém
  snímku. V `libs/ui` navrhuji prolnutí a skok výšky (levné, vejde se
  do rozpočtu snímku).
- **Automatická aktivace záložek** je správně jen tehdy, když se panel
  vykreslí do jednoho snímku. Pomalý panel potřebuje ruční aktivaci
  (šipky jen přesouvají fokus, Enter přepne), jinak šipky zasekávají.
- **Toast nebere fokus.** Uživatel klávesnice se k akci Zpět dostane
  jen přes F6 nebo Ctrl+Z. Proto má být každá akce v toastu dostupná
  i jinak.

## Verze

| Verze | Datum | Co |
|---|---|---|
| v1 | 14. 11. 2026 | první verze: 25 komponent ve všech stavech, hustoty počítač / tablet / TV, klávesnice, rozdíly proti `libs/ui`, návrh nových tokenů |
