# Odchylky GUI od návrhu

Porovnání skutečného GUI s prototypy v `design/` a s `design/tokens.toml`.
Slouží jako zadání oprav pro vlákno GUI (milník 6c). Popis návrhu je ve
[Vzhledu a ovládání](vzhled.md), specifikace prvků v
[Knihovně komponent](komponenty.md).

**Stav:** seznam k 16. 11. 2026 nad `main` `d6078624`. Systém běžel přes
`cargo xtask run --release --headless`, x86_64, vývojový profil, QEMU
1280 &times; 800. Snímky se pořizovaly přes QMP `screendump` v noci, takže
vzhled *Automaticky* byl tmavý. Snímky jsou v
[design/snimky/odchylky/](../../design/snimky/odchylky/01-plocha-okna.png).

Neověřeno:

- světlý motiv systémových prvků;
- dialog oprávnění a výběr souboru (nepodařilo se je vyvolat);
- přihlášení s výběrem účtu;
- aplikace Nastavení. V systému zatím není: chybí ve spouštěči i v paletě.

## Jak číst seznam

| Pole | Význam |
|---|---|
| Priorita | **P1** vypadá rozbitě nebo jako jiný systém; **P2** viditelně jinak než návrh, ale funkční; **P3** detail |
| Velikost | **S** do půl dne, **M** 1 až 2 dny, **L** 3 až 5 dní (odhad včetně testů) |
| Knihovna | `ui` (libs/ui), `theme`, `comp` (libs/comp, kompozitor), `aplikace` (`user/gui-*`) |
| ID | **K** = společná komponenta (oprava jednou, projeví se všude), **O** = jedna obrazovka |

## Společné komponenty (opravit první)

Tyhle odchylky nejsou chyba jedné obrazovky. Pochází z jednoho místa
v `ui` nebo `comp` a projevují se všude, kde se prvek použije. Oprava
jednou opraví celý systém, proto jdou první.

| ID | Co je jinak | Prototyp | Knihovna | P | Velikost |
|---|---|---|---|---|---|
| K1 | **Aplikace nepřejdou do tmavého režimu.** Obsah všech oken zůstává světlý, i když lišta, titulky a spouštěč jsou tmavé (Soubory, Správce úloh, Komponenty, Ukázka, Přehrávač, Terminál). Kompozitor posílá `ServerMsg::Theme` po `HELLO` (`scene.rs`) i po změně (`style.rs`) a `ui` ho převede na `Event::Theme` (`client/mod.rs`), ale okno se novým `Look` nepřekreslí. Snímek [01](../../design/snimky/odchylky/01-plocha-okna.png).<br>**Opraveno:** příčina byla jen jedna - klient `ui` zahodil `THEME` z pozdravu (`b16ea897`); překreslení po `Event::Theme` fungovalo (snímek `dark.png` z testu vzhledu). Ověřeno testem „aplikace spuštěná v tmavém režimu je tmavá od prvního snímku“ (Ukázka i terminál), snímek [01 po](../../design/snimky/odchylky/01-plocha-okna-po.png). | [komponenty.html](../../design/komponenty.html) (přepínač motivu) | ui | P1 | S až M |
| K2 | **Panely a nabídky lišty nejsou z `ui`.** Panel Vzhled, Síť, Zvuk, Oznámení, Vývojář a nabídka zástupce aplikace kreslí `flyout.rs` jako obdélník s řádky textu. Chybí:<br>- ikony, nadpis s ikonou a souhrnem, oddělovače a sekce;<br>- stav najetí a fokus, otevřený stav ikony v liště (`surface_3` + podtržení `accent`);<br>- stín `shadow.popover`, materiál (rozmazání);<br>- ovládací prvky (přepínač, posuvník, dlaždice, tlačítko v patě).<br>Rozměry jsou jiné: šířka podle textu (prototyp 360), řádek 24 px (prototyp 40). Spouštěč a paleta už jdou přes `ui` (`sysui.rs`), panely ne. Návrh: společná komponenta *panel* v `ui` (hlavička, řádek s ikonou a popisem, sekce, pata, přepínač, posuvník) a panely jako pohledy `sysui`. Snímky [02](../../design/snimky/odchylky/02-panel-vzhled.png), [03](../../design/snimky/odchylky/03-panel-zvuk.png), [04](../../design/snimky/odchylky/04-panel-vyvojar.png), [06](../../design/snimky/odchylky/06-nabidka-liste.png).<br>**Částečně opraveno:** komponenta panel v `ui` (`panel.rs`) a panely Vzhled, Síť, Zvuk, Oznámení, Vývojář jako pohledy (`barpanel.rs`) s rozměry z tokenů, stínem, materiálem s pojistkou rozpočtu a otevřeným stavem ikony. Zbývá: nabídka zástupce aplikace (O7) a ovládání zvuku přes službu zvuku (O2). Snímky [02 po](../../design/snimky/odchylky/02-panel-vzhled-po.png), [oznámení po](../../design/snimky/odchylky/02b-panel-oznameni-po.png). | [bubliny-panely.html](../../design/bubliny-panely.html) | comp, ui | P1 | L |
| K3 | **Ikonové tlačítko bez ikony.** V ukázce Komponenty (záložka Pole) je třetí tlačítko prázdná pilulka. Fokus kolem ní je vidět. Snímek [15](../../design/snimky/odchylky/15-komponenty-pole.png).<br>**Opraveno:** ukázce chyběla ikona (`.icon("settings")`); `ui` tlačítko s ikonou kreslí správně. | [komponenty.html#tlacitka](../../design/komponenty.html#tlacitka) | ui | P1 | S |
| K4 | **Jen jeden styl tlačítka.** Tlačítko „Primární“ vypadá jako sekundární (okraj, bez výplně `accent`). Chybí primární, nebezpečné a ghost varianta. Soubory tak mají 13 stejných pilulek a nic nevede oko.<br>**Opraveno v `ui`:** `.primary()`, `.danger()`, `.ghost()` (`node::Variant`, najetí plných variant ztmavené k `ink`, nedostupné s krytím `opacity.disabled`); ukázka Komponenty je ukazuje. Použití v Souborech (jedna hlavní akce) až s E. Snímek [K4 po](../../design/snimky/odchylky/k4-tlacitka-po.png). | [komponenty.html#tlacitka](../../design/komponenty.html#tlacitka) | ui | P2 | M |
| K5 | **Plovoucí vrstvy bez stínu.** Nabídka a seznam výběru mají jen okraj 1 px, chybí `shadow.popover`. Na světlém obsahu splývají s oknem. Fajfka zvolené položky je vlevo, prototyp ji má vpravo. Snímky [10](../../design/snimky/odchylky/10-vyber.png), [11](../../design/snimky/odchylky/11-nabidka-tabulka.png).<br>**Opraveno:** nabídky a seznam výběru mají stín `shadow.popover` (masky v mezipaměti `ui`, vrstva popoveru s okrajem pro stín), fajfka zvolené položky je vpravo, důvod zakázání vpravo místo zkratky. Snímek [11 po](../../design/snimky/odchylky/11-nabidka-po.png). | [komponenty.html#vyber](../../design/komponenty.html#vyber), [#nabidky](../../design/komponenty.html#nabidky) | ui | P2 | S |
| K6 | **Položky nabídky bez ikon a nadpisu skupiny.** Kontextové nabídky Souborů i tabulky jsou jen text a zkratka.<br>**Opraveno:** `MenuEntry::icon` a `menu_header` (nadpis skupiny se nedá vybrat, čtečka ho hlásí jako štítek); ikony v nabídce Souborů, koše, pole (Zpět, Kopírovat, ...), zástupce v liště (nadpis „Okna“) a napájení ve spouštěči. Snímek [06 po](../../design/snimky/odchylky/06-nabidka-liste-po.png). | [kontextove-nabidky.html](../../design/kontextove-nabidky.html), [komponenty.html#nabidky](../../design/komponenty.html#nabidky) | ui | P2 | S |
| K7 | **Vybraná záložka** je plocha `surface_2` bez podtržení 3 px `accent`. Ve světlém motivu je skoro neviditelná.<br>**Opraveno:** vybraná záložka text `accent_ink` a podtržení 3 px `accent` (`size.bar_indicator_h`), pod řadou čára `border`, najetí `surface_3`. Snímek [K7 po](../../design/snimky/odchylky/k7-zalozky-po.png). | [komponenty.html#zalozky](../../design/komponenty.html#zalozky) | ui | P2 | S |
| K8 | **Prázdný výsledek bez prázdného stavu.** Paleta s dotazem „nast“ ukáže prázdný rámeček místo „Nic nenalezeno“ s nabídkou. Snímek [12](../../design/snimky/odchylky/12-paleta-prazdna.png).<br>**Opraveno:** `ui::empty_state` (kruh s ikonou, nadpis, popis, akce); paleta bez výsledku ukáže „Nic nenalezeno“. Snímek [12 po](../../design/snimky/odchylky/12-paleta-prazdna-po.png). | [komponenty.html#prazdno](../../design/komponenty.html#prazdno) | ui, comp | P2 | S |
| K9 | **Seznam jako tabulka.** Spouštěč má vnořený rámeček tabulky, čáru pod každým řádkem a úzký pravý sloupec („běží, + no...“). Prototyp má řádky bez čar a sekce.<br>**Opraveno:** tabulka `.plain()` (bez rámečku a čar, zaoblený výběr) ve spouštěči a paletě. Snímek [05 po](../../design/snimky/odchylky/05-spoustec-po.png). | [spoustec.html](../../design/spoustec.html) (varianta d) | ui, comp | P2 | M |
| K10 | **Segmentový přepínač** je roztažený na celou šířku. Prototyp ho má jen tak široký jako obsah. | [komponenty.html#zalozky](../../design/komponenty.html#zalozky) | ui | P3 | S |
| K11 | **Posuvník** má dráhu 2 px a šedý okraj jezdce (`border_strong`). Prototyp má dráhu 4 px a okraj `accent`. | [komponenty.html#posuvnik](../../design/komponenty.html#posuvnik) | ui | P3 | S |
| K12 | **Bublina lišty** má jen jeden řádek. Prototyp má tučný titulek a pod ním popis (stav, zkratka). | [bubliny-panely.html](../../design/bubliny-panely.html) (Bublina) | comp | P3 | S |

## Lišta a plocha

| ID | Co je jinak | Prototyp | Knihovna | P | Velikost |
|---|---|---|---|---|---|
| O1 | **Panel Vzhled** (podnět uživatele) je nadpis a čtyři řádky textu, zvolený má fajfku. Vypadá jako obdélník se seznamem textů, ne jako nabídka. Řeší ho K2. Prototyp jsem doplnil, dosud ho neměl:<br>- radio skupina se třemi volbami (ikona, název, u Automaticky popis „podle východu a západu slunce“), fajfka vpravo, šipky;<br>- řádek stavu „Teď světlý, podle východu a západu slunce. Tmavý od 16:21“;<br>- přepínač Vysoký kontrast a tlačítko *Nastavení vzhledu*.<br>Snímek [02](../../design/snimky/odchylky/02-panel-vzhled.png).<br>**Opraveno:** radio skupina s ikonami, popisem a fajfkou vpravo (šipky vyberou), řádek stavu („Tmavý od 16:21 (západ slunce)“), přepínač Vysoký kontrast, vypnuté tlačítko *Nastavení vzhledu* s důvodem. Snímek [02 po](../../design/snimky/odchylky/02-panel-vzhled-po.png). | [bubliny-panely.html](../../design/bubliny-panely.html) (ikona měsíce) | comp, ui | P1 | v K2 |
| O2 | **Panel Zvuk** je jen „Hlasitost 100 %“. Chybí posuvník, ztlumení, výstup a vstup, hlasitost aplikací a *Nastavení zvuku*. Snímek [03](../../design/snimky/odchylky/03-panel-zvuk.png).<br>**Opraveno:** posuvník a ztlumení, výstup a vstup (skupiny radio), indikátor nahrávání, hlasitost aplikací podle identity z manifestu (Wine z víc procesů = jedna položka), vypnuté *Nastavení zvuku*; nové operace `audioproto` `OVERVIEW`, `APPS`, `APP_VOLUME`, `SELECT_INPUT` (test v sadě `zvuk`). Snímek [03 po](../../design/snimky/odchylky/03-panel-zvuk-po.png). | [bubliny-panely.html](../../design/bubliny-panely.html) (Zvuk) | comp | P1 | M (po K2) |
| O3 | **Panel Síť** je jen „Pevná síť: připojeno“. Chybí dlaždice Wi-Fi / Letadlo / VPN, řádek s rychlostí a IP, štítek *aktivní* a *Nastavení sítě*.<br>**Částečně:** řádek Ethernet se stavem, adresou IP z `netd` a štítkem „aktivní“. Rychlost linky `netd` nehlásí. Dlaždice Wi-Fi / Letadlo / VPN **čekají na služby** (ovladač Wi-Fi, VPN nejsou); *Nastavení sítě* vypnuté s důvodem. | [bubliny-panely.html](../../design/bubliny-panely.html) (Síť) | comp | P2 | M (po K2) |
| O4 | **Panel Režim vývojáře** má akci jako řádek textu („Zapnout režim vývojáře“), bez přepínače. Zapnutý stav nemá varování v panelu. Prototyp jsem doplnil (přepínač, varovný řádek `danger_soft`, pilulka v liště). Snímek [04](../../design/snimky/odchylky/04-panel-vyvojar.png).<br>**Opraveno:** panel s přepínačem a varováním `danger_soft` (K2), pilulka v liště s ikonou (`code`, zapnuto `shield-alert` na `danger`). | [bubliny-panely.html](../../design/bubliny-panely.html) (Vývojář) | comp | P2 | S (po K2) |
| O5 | **Hodiny** po kliknutí nic neotevřou. Prototyp má kalendář měsíce a světový čas.<br>**Opraveno:** klik na hodiny otevře panel s kalendářem měsíce (dnešek `accent`, listování šipkami), světovým časem (Londýn, New York, Tokio z `libs/tz`, jen jiná pásma než místní) a vypnutým *Nastavením času*; události připravujeme. Snímek [O5 po](../../design/snimky/odchylky/o5-hodiny-po.png). | [bubliny-panely.html](../../design/bubliny-panely.html) (Hodiny) | comp | P2 | M |
| O6 | **Oznámení** jsou jen „Žádná oznámení“. Chybí přepínač Nerušit a pata.<br>**Čeká na službu oznámení** (`notifyproto`, vlákno Plánovač): panel je nad prázdným seznamem. | [bubliny-panely.html](../../design/bubliny-panely.html) (Oznámení) | comp | P2 | S (po K2) |
| O7 | **Nabídka zástupce v liště** je textová karta: „Soubory / Domov - Soubory / + Nové okno“, bez ikon a najetí. Snímek [06](../../design/snimky/odchylky/06-nabidka-liste.png).<br>**Opraveno:** nabídka knihovny `ui` s najetím, klávesami, fajfkou aktivního okna, zkratkou a vypnutou položkou s důvodem (ikony a nadpis skupiny přidá K6). Snímek [06 po](../../design/snimky/odchylky/06-nabidka-liste-po.png). | [kontextove-nabidky.html](../../design/kontextove-nabidky.html) (ikona v liště) | comp | P2 | v K2 |
| O8 | **Ikona Úlohy** otevře Správce úloh místo panelu s průběhy. `flyout.rs` to dělá záměrně, když Správce běží. Návrh a kód se rozcházejí; je potřeba rozhodnout, co platí. | [bubliny-panely.html](../../design/bubliny-panely.html) (Úlohy) | comp | P3 | S |
| O9 | **Značky běžících aplikací**: po startu má většina oranžovou značku (`warn` = žádá pozornost). Má ji být šedá (`ink_faint`) a jen aktivní `accent`. Žádost o pozornost hned po startu je šum.<br>**Opraveno:** okna otevřená na pozadí do `STARTUP_QUIET` (15 s) po otevření relace (automaticky spuštěné aplikace) o pozornost nežádají; značka je šedá, aktivní `accent`. Okno otevřené samo později dál žádá. | [plocha.html](../../design/plocha.html) (lišta) | comp, aplikace | P2 | S |
| O10 | **Ikona otevřeného panelu** v liště nemá otevřený stav (`surface_3` + podtržení `accent`), takže není poznat, k čemu panel patří.<br>**Opraveno:** ikona otevřeného panelu má `surface_3` a podtržení `accent`. | [bubliny-panely.html](../../design/bubliny-panely.html) | comp | P2 | v K2 |
| O11 | **Pravé tlačítko na plochu** nic neotevře. Prototyp má nabídku plochy. | [kontextove-nabidky.html](../../design/kontextove-nabidky.html) (plocha) | comp | P3 | M |
| O12 | **Kurzor nad spouštěčem** ukazuje šipky změny velikosti okna, které je pod ním. Spouštěč nezakrývá okraje okna pro tvar kurzoru. Snímek [05](../../design/snimky/odchylky/05-spoustec.png).<br>**Opraveno:** nad spouštěčem, paletou a panelem lišty je kurzor šipka (`Comp::over_system`; host test). | - | comp | P2 | S |

## Spouštěč

| ID | Co je jinak | Prototyp | Knihovna | P | Velikost |
|---|---|---|---|---|---|
| O13 | **Pata spouštěče** ukazuje „Přihlášený uživatel“ místo avataru a jména a chybí ikona Nastavení.<br>**Opraveno:** kolečko s počátečním písmenem a zobrazované jméno, Nastavení (vypnuté s důvodem). Snímek [05 po](../../design/snimky/odchylky/05-spoustec-po.png). | [spoustec.html](../../design/spoustec.html) (d) | comp | P2 | S |
| O14 | **Sekce spouštěče** jsou Běží / Další... (1) / Systém, prototyp má Běží / Aplikace / Nedávné. „Další...“ na klik nereaguje. Pole hledání nemá lupu a nápovědu Super.<br>**Opraveno:** sekce Běží / Aplikace / Nedávné bez „další...“, pole s lupou a nápovědou Super v koncovém slotu pole (`Trailing::Key`, jen u prázdného pole). | [spoustec.html](../../design/spoustec.html) (d) | comp | P2 | M |

## Okna a dekorace

| ID | Co je jinak | Prototyp | Knihovna | P | Velikost |
|---|---|---|---|---|---|
| O15 | **Odznak důvěry** u Souborů a Správce úloh říká „Systém“. Prototyp ukazuje jméno aplikace a ikonu oprávnění (Soubory + domov).<br>**Opraveno:** odznak ukazuje jméno aplikace, systémovou aplikaci značí štít s fajfkou (jméno i značku dává init z manifestu). | [okna.html](../../design/okna.html) (Odznak důvěry), [soubory.html](../../design/soubory.html) | comp | P2 | S |
| O16 | **Tlačítka okna s vlastním titulkem** (Terminál) jsou šedá jako u neaktivního okna, i když okno má fokus. Snímek [08](../../design/snimky/odchylky/08-terminal.png). | [okna.html](../../design/okna.html) (Režimy okna) | comp | P3 | S |

## Soubory

| ID | Co je jinak | Prototyp | Knihovna | P | Velikost |
|---|---|---|---|---|---|
| O17 | **Kontextová nabídka souboru má všechny akce vypnuté.** Pravé tlačítko řádek sice vybere, ale nabídka se sestaví ze stavu před výběrem. Vypadá rozbitě. Snímek [07](../../design/snimky/odchylky/07-kontextova-soubory.png).<br>**Opraveno** v `ui`: otevřená kontextová nabídka po přestavbě pohledu převezme položky z nového pohledu (`refresh_context`), takže odpovídá výběru po pravém kliku (host test `context_menu_reflects_selection_made_by_right_click`). | [kontextove-nabidky.html](../../design/kontextove-nabidky.html) (soubor) | aplikace | P1 | S |
| O18 | **Nástrojová lišta** jsou dva řádky textových pilulek (Zpět, Vpřed, Nahoru, Domov, Znovu načíst, Nová složka, Přejmenovat, Do koše, Kopírovat, Vyjmout, Vložit). Prototyp má jeden řádek: ikony Zpět / Vpřed, drobečky cesty, hledání s lupou a jednu akci. Akce nad souborem jsou v nabídce a v řádku pod seznamem. | [soubory.html](../../design/soubory.html) | aplikace, ui (drobečky) | P2 | M |
| O19 | **Panel náhledu přetéká** přes pravý okraj okna („Smíte: číst, měnit, přidávat, mazat, sprav...“ je uříznuté). Text se nezalamuje. | [soubory.html](../../design/soubory.html) (Podrobnosti) | aplikace, ui | P2 | S |
| O20 | **Nadpisy „Místa“ a „Náhled“** velkým písmem a místa bez ikon. Prototyp nadpisy nemá a místa mají ikony. Čas souboru je „dnes 0:20 UTC“ místo místního času. | [soubory.html](../../design/soubory.html) | aplikace | P3 | S |

## Terminál a ukázkové aplikace

| ID | Co je jinak | Prototyp | Knihovna | P | Velikost |
|---|---|---|---|---|---|
| O21 | **Terminál je světlý v tmavém režimu** (stejná příčina jako K1, ale terminál kreslí vlastní barvy `term_*`). Ověřit spolu s K1.<br>**Opraveno** s K1: terminál bere režim z pozdravu, snímek [01 po](../../design/snimky/odchylky/01-plocha-okna-po.png). | [terminal.html](../../design/terminal.html) | ui, aplikace | P1 | v K1 |
| O22 | **Správce úloh** je seznam textů „Aplikace X - běží“. Prototyp má tabulku se sloupci, průběhem a akcemi.<br>**Opraveno:** tabulka se sloupci Úloha (ikona původu), Kdo spustil, Stav, Průběh (pruh, u agenta `ai`) a Paměť; řádek agenta má podklad `ai_soft`, levý pruh a štítek „agent“; filtr segmenty Vše / Moje / Agent / Systém; v každém řádku ikonová tlačítka pozastavit (u pozastavené pokračovat) a zrušit (`Cells::actions`, neaktivní, když je `taskd` nepovolí; Šipka vpravo na ně přejde, Enter stiskne); dole pruh detailu se jménem, kdo spustil, stavem a výpisem. Snímek [O22 po](../../design/snimky/odchylky/o22-ulohy-po.png). | [plocha.html](../../design/plocha.html) (Správce úloh) | aplikace | P2 | M |
| O23 | **Pole jen ke čtení** v ukázce Komponenty ukazuje text posunutý (začátek uříznutý). | [komponenty.html#pole](../../design/komponenty.html#pole) | ui | P3 | S |

## Systémové dialogy, zámek a přihlášení

| ID | Co je jinak | Prototyp | Knihovna | P | Velikost |
|---|---|---|---|---|---|
| O24 | **Zamčená obrazovka v tmavém motivu: hodiny, datum a jméno nejsou vidět.** Kreslí se barvou `on_accent` (`greeter.rs`), která je v tmavém motivu tmavě fialová (`#1d0f40`) na tmavé tapetě. Prototyp má čas `ink`, datum `ink_muted`. Jméno je „Vyvojar“ bez diakritiky. Snímek [14](../../design/snimky/odchylky/14-zamcena-obrazovka.png).<br>**Opraveno:** hodiny a jméno `ink`, datum a texty bez účtu `ink_muted` (jako prototyp); `on_accent` jen na akcentu (avatar, tlačítko). Vývojový účet se jmenuje „Vývojář“ (bylo jen jméno účtu bez diakritiky).<br>**Hlídá test** `zamek_citelny_ve_vsech_rezimech` (`libs/comp`): kontrast hodin, data a jména z pixelů proti pozadí ≥ 4,5 : 1 ve světlém, tmavém i kontrastním režimu. Test našel i světlý motiv: `scrim` z pozadí dělal středně tmavou plochu (datum 2,2 : 1); zámek ho teď má jen v tmavém a kontrastním režimu (prototyp ho nemá vůbec). Snímky [14 po](../../design/snimky/odchylky/14-zamcena-obrazovka-po.png), [světlý po](../../design/snimky/odchylky/14b-zamcena-obrazovka-svetla-po.png). | [prihlaseni.html](../../design/prihlaseni.html) | comp | P1 | S |
| O25 | **Odeslání prázdného hesla** nic neřekne (žádná zpráva pod polem). | [prihlaseni.html](../../design/prihlaseni.html) (chyba hesla) | comp | P3 | S |
| O26 | **Bezpečná obrazovka** odpovídá návrhu. Rozdíly jsou drobné: tlačítka bez výchozího (primárního) a bez dlaždice ikony podle závažnosti. Snímek [13](../../design/snimky/odchylky/13-bezpecna-obrazovka.png). | [dialogy.html](../../design/dialogy.html) | comp | P3 | S |

## Doplněno do prototypů

- **Panel Vzhled** v [bubliny-panely.html](../../design/bubliny-panely.html)
  (ikona měsíce) obsahuje:
  - volby Světlý / Tmavý / Automaticky jako radio skupinu s ikonami a fajfkou;
  - řádek stavu podle volby („Teď světlý, podle východu a západu
    slunce. Tmavý od 16:21 (západ slunce v Praze)“, u ruční volby „Teď
    tmavý, nastaveno ručně“);
  - přepínač Vysoký kontrast;
  - tlačítko *Nastavení vzhledu*;
  - bublinu „Vzhled: automaticky (teď světlý)“.
- **Režim vývojáře** v témže prototypu obsahuje:
  - pilulku v liště (zapnutý stav `danger`);
  - panel s přepínačem a popisem;
  - varovný řádek při zapnutí;
  - patu „Jen vývojové sestavení“.

## Doporučené pořadí oprav

1. **K1** (tmavý režim v aplikacích) a **O24** (zámek). Jsou malé a
   nejvíc viditelné.
2. **K2** (panely lišty přes `ui`). Opraví O1 až O7 a O10 najednou,
   proto je to největší úloha.
3. **K3, K4, K5, K6** (tlačítka, stín, ikony nabídek) a **O17** (nabídka
   Souborů).
4. Ostatní P2 po obrazovkách, P3 průběžně.

## Verze

| Verze | Datum | Co |
|---|---|---|
| v1 | 16. 11. 2026 | první porovnání nad `d6078624`: 12 odchylek společných komponent, 26 po obrazovkách; doplněn panel Vzhled a Režim vývojáře v prototypu |
