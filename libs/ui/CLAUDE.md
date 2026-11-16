# libs/ui

Nativní knihovna UI (milník 6c): jedna pro všechny aplikace. Aplikace popíše rozhraní deklarativně (`node`), knihovna ho rozvrhne (`layout`), vykreslí do bufferu (`draw`), převede vstup a akce přístupnosti na zprávy aplikace (`event`) a odvodí strom vrstev a strom přístupnosti. `Ui` (jádro) nezávisí na OS. Běhové prostředí pro aplikace v OS (kanál ke kompozitoru, buffery ve VMO, smyčka) je v `src/client/` za feature `os`. Popis: `docs/cs/gui.md#nativní-knihovna-ui-libsui`.

## Invarianty

- Aplikace nic z toho nepíše ručně: strom přístupnosti a vrstev se odvozuje, ne duplikuje.
- Vzhled jen z tokenů (`libs/theme` / `look.rs`), žádná pevná čísla vzhledu v prvcích.
- Jádro `Ui` zůstává bez závislosti na `libos`; všechno OS-specifické patří do `client/` (feature `os`).
- Knihovna nenabízí aplikacím barvy `trust_*` (odznak důvěry, systémové dialogy) ani `ai_*` (jen agent AI); patří jen systému.
- Rozvržení v řádku (`layout.rs`, `stretches`): kontejnery (`Column`, `Scroll`, `Table`, `List`) bez pevné výšky se natáhnou na výšku řádku, listy a vnořené řádky jsou svisle na středu, `.height()` má přednost. Změna pravidla mění vzhled všech aplikací - snímky (`--update-shots`) a test `tests/layout.rs`.
- Protokol s kompozitorem je v `libs/guiproto`; změny drž kompatibilní s `libs/comp`.

## Testy

- `cargo test -p ui` (`src/tests/`: `mod.rs` zakladni prvky, `keys.rs` klice, stav, heslo, `text.rs` editor, `field.rs` textova pole a schranka, `list_multi.rs` vicenasobny vyber v seznamu, `groups.rs` tlacitko pri pusteni, skupina radio, hledani psanim ve vyberu a seznamu, kolecko posuvniku s fokusem, Ctrl+PgUp/PgDn, podnabidky a mnemoniky, `field_behavior.rs` bezne chovani poli (Alt+slova, prepis, delka, jen pro cteni, cislo, hledani, oko, Caps Lock, prstenec jen z klavesnice, blikani, posun tahem, nabidka pole), `table.rs` tabulka, `controls.rs` prepinac, volba, posuvnik, segmenty, zalozky, graf podilu, `popup.rs` vyber, nabidky, bublina, `shots.rs` snimky vzhledu proti `tests/shots/*.png` (prepis jen `cargo xtask test --update-shots`), `context.rs` kontext okna, `spatial.rs` prostorova navigace fokusu, `hover.rs` najeti mysi pres zalozky, tlacitka a radky tabulky, `suggest.rs` seznam navrhu u pole, `layout.rs` natazeni kontejneru v radku, `wrap.rs` zalamovani stitku, `video.rs` plocha videa a pomer stran); `client/` jen s `--features os` pro cíl OS. QEMU: `cargo xtask test --only gui`.

## Pasti

- Najetí: prvek s částmi, které se zvýrazňují zvlášť (záložky, řádky a posuvník tabulky), musí část vracet z `Ui::hover_part`; jinak se pohyb uvnitř prvku nepřekreslí a zvýraznění zůstane na staré části (test `hover.rs`).
- Rozvržení měří podle dostupné šířky (`layout::measure(n, t, avail)`): výška zalamovaného štítku závisí na šířce. `measure` a `place` musí šířky počítat stejně (řádek přes `row_sizes`), jinak se výška štítku a obdélník rozejdou.

- `empty_state` (prázdný stav) a `table(..).plain()` (seznam bez rámečku a čar) jsou v `panel.rs` a `table.rs`.
- Varianty tlačítka jsou vlastnost uzlu `Node::variant` (`.primary()`, `.danger()`, `.ghost()`), ne druh prvku; kreslí `draw.rs` (Button). Nejvýš jedno primární na obrazovce.
- Komponenta panel (`panel.rs`) nemá vlastní druh prvku: vlastnosti uzlu `pad`, `gap`, `deco`, `tone` a `row` (radek s ikonou a popisem pro tlačítko, radio, přepínač, štítek; kreslí `draw_panel_row`, rozměr `row_size`). `Ui::transparent` (bez pozadí, materiál kreslí volající) a `Ui::window_pad` (okraj okna). Test `tests/panel.rs`.
- Režim barev při startu je součást pozdravu: `handshake` nesmí zahodit `THEME` mezi `WELCOME` a `CONFIGURE` (`Window::mode`, `run_video` ho použije před prvním kreslením, obnova pošle `Event::Theme`, když se mezitím změnil; test QEMU `compd: aplikace spustena v tmavem rezimu ...`).
- Změna režimu: `Event::Theme` nastaví `look` i `Context::mode` (cílový režim pro aplikace s vlastním vzhledem). Plynulý přechod dělá kompozitor smícháním snímků; aplikace se překreslí jen jednou.
- Aplikace UI vykresluje do vlastního bufferu; kompozitor mu nevěří (viz `libs/comp/CLAUDE.md`), ale i klient musí ošetřit nesmyslné rozměry od kompozitoru.
- `client/mod.rs` má ~730 řádků; při dalším růstu dělit (obnova po pádu je v `restore.rs`, schránka v `clip.rs`).
- Seznam návrhů u pole: jádro bez stromu `complete.rs` (`Completion`, používá ho i gui-term samostatně), napojení na pole `suggest.rs`. Klávesy, které seznam nevezme, musí jít do pole (`Outcome::Ignored`); heslo seznam nikdy nemá.
- Nabídka: ikona vlevo (`MenuEntry::icon`, sloupec vždy), nadpis skupiny (`menu_header`, nevybírá se, `usable` false), vpravo zkratka nebo důvod zakázání a fajfka; stín `shadow.popover` z `Ui::shadow_masks`, vrstva popoveru (`overlay_rect`) má okraj `popup_shadow_margin`.
- `Ui::open_menu(items, kotva)` otevře nabídku bez prvku (vlastník = kořen pohledu; nabídka zástupce v liště kompozitoru), `Ui::popup_rect` její obdélník.
- Nabídky: `Ui::popup` je vždy nejhlubší úroveň, rodiče podnabídek drží `Ui::popup_stack` (`submenu.rs`); kdo nabídku zavírá, volá `close_popup` (vyprázdní i zásobník), ne `popup = None`.
- Skupina radio = sourozenci se stejným rodičem (`groups.rs`, `tab_stop`): pořadí Tab musí jít přes `tab_stop`, ne přímo přes `focusable`.
- `client/pointer.rs`: kurzor okna, zámek ukazatele a celá obrazovka pro okna, která kreslí sama (SDL); `Event::PointerLock` a `Event::State` (stav z `CONFIGURE` bez fokusu) běhová smyčka `run` ignoruje.
- Kurzor pole v OS kreslí klient do vrstvy `CARET_LAYER` (`client/mod.rs`, `Window::set_caret` podle `Ui::caret`); s `use_caret_layer(true)` ho pole nekreslí a blikání nevrací překreslení. Kompozitor u změny jen viditelnosti / polohy takové vrstvy poškodí jen její obdélník (`libs/comp/src/scene.rs`, `small_layer_change`).
- Výběr řádků (klik, Ctrl, Shift, kotva) je jedna funkce `selection::pick` pro tabulku i seznam; změna pravidla platí pro oba.
- Rozpracovaný znak (mrtvá klávesa, později IME) je `compose.rs` (`Ui::compose` ze zprávy `ServerMsg::Compose`): kreslí se jen u pole s fokusem, nikdy není v hodnotě ani v `on_change`, heslo ho nedostane; končí `Text`, klikem a ztrátou fokusu. Obsah nelogovat.
- Časované chování polí (`caret.rs`: blikání kurzoru, posun tahem za okraj, hledání s prodlevou) jde přes `Ui::next_deadline` a `Ui::timer`; zprávy z časovače vybírá klient `Ui::take_msgs`. Blikání se po `motion.caret_blink_stop` zastaví (žádné překreslování v klidu).
- Prstenec fokusu jen po klávesnici (`Ui::kbd_focus`; klik ho vypne). Kreslení, které má být vidět i po kliknutí (kurzor, akcentový okraj pole), se řídí `State::focused`, ne `focus_visible`.
- Textová pole: editor `text.rs` (grafémy, slova, historie) je bez kreslení a sdílí ho i gui-term; geometrie a klávesy pole jsou v `field.rs`, kreslení a tlačítka v poli (oko, křížek, krokování) v `field_draw.rs`, nabídka pole v `field_menu.rs`, volby a logika čísla ve `fieldopts.rs`. Hodnotu drží aplikace, pole ji po `rebuild` převezme (`sync_fields`).
- Plocha videa (`node::video`, `video.rs`): jádro jen rezervuje místo a počítá obdélník se zachováním poměru stran (`video_rects`, jen mimo rolování). Obraz ukazuje klient (`client/video.rs`, `run_video`) ve vrstvě `900 + id` z bufferů aplikace s ploty získání a uvolnění, bez překreslení okna; tik snímků jen s `App::wants_frames`. Buffery s popisem nejsou v záznamu obnovy (`guiproto::replay`): po pádu kompozitoru je klient vytvoří znovu při dalším snímku (porovnání generace).
