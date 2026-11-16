# libs/term

Chytrý terminál (milník 6b), `no_std` + `alloc`. Moduly: `proto` (nativní protokol okno <-> služba relací: typované hodnoty, tabulky se schématem, odkazy, průběh, výzvy, bloky výstupu; žádné escape sekvence), `session` (služba relací), `broker` (prostředník `termd`), `ctl` (řídicí zprávy termd <-> relace, registr relací), `vt` (adaptér na VT100 pro sériovou linku a převzaté programy), `screen` (textová obrazovka a vlastní písmo do framebufferu, záchranná konzole), `theme` + `render` (motiv terminálu, `theme.toml`, ANSI), `perm` (oprávnění programů z manifestu na cesty relace fsd), `select` (výběr textu myší v mřížce okna GUI), `ui/` (okno terminálu; vložení a klik do řádku `ui/pointer.rs`, nabídka doplňování kreslená oknem `ui/menu.rs`), `tasks`, `taskboard`, `activity`, `program` (program ze shellu přes službu exec initu: `Env::exec_start`, `Running`; obsluhu dělá vlákno řádku v `fg.rs`: výstup po řádcích, stdin z roury nebo výzvy po 300 ms nebo hned skrytě, když program vypne ECHO přes `ttyproto`; systémová výzva služby účtů se štítkem `program::SYSTEM_LABEL`, Ctrl+C). Výzvy `Ask`/`Prompt` mají pole `label` (odznak zdroje, okno i VT ho kreslí odlišně; program ho výstupem nenapodobí). Interpret je v `libs/shlang`, ne tady. Popis: `docs/cs/navrh-shell.md`, `docs/cs/prikazy-shellu.md`.

## Invarianty

- Relace a její běžící příkazy patří službě (proces `user/shsess`), ne oknu: po zavření okna běží dál a jde se k nim vrátit. Relace přežijí i pád `termd` (registr relací, `Adopt` novým termd).
- Okno ani termd interpret neobsahují; termd jen směruje zprávy.
- Prostřednictvím `perm` dostane program jen cesty povolené manifestem (capability, ne identita).
- Protokol je nativní a typovaný; escape sekvence jen v adaptéru `vt`.
- Dotaz na text (`ServerMsg::Ask`, odpověď `ClientMsg::Text`, `Host::ask`): `secret = true` okno maskuje a do výpisu bloku jde jen `***` (heslo v průvodci `install`). Řádek ve vlákně jde přes `fg.rs` (`LinePrompt.text`, `Answer::Text`, `answer_of`); `ClientMsg::Text` má tag 14.
- Nabídka doplňování: položky a výběr drží vždy `App::menu`. Okno GUI nastaví `float_menu` (snímek pak nabídku nekreslí) a kreslí ji komponentou `ui::complete` (`user/gui-term/src/compl.rs`); výběr, přijetí a zavření hlásí `menu_select` / `menu_accept` / `menu_close`, kotvu dá `menu_col` (`input_col`). Konzole kreslí nabídku řádky jako dřív.
- Pilulka složky v okně GUI (`ui/pill.rs`): šířku i posun řádku vstupu počítá jen `pill::cwd_pill` ze jména změřeného písmem, kterým se kreslí; text začíná `s3` za pilulkou (ne na buňce za výzvou). `gui-term` ji volá pro kreslení i rozvržení (`rows::cwd_pill`, `Grid::set_indent`). Test `tests_pill.rs` (skutečné písmo UI).
- Výzva: `App::cwd_lead` = buňky před složkou, do kterých okno GUI kreslí ikonu a okraj pilulky (počítá je z tokenů, `Look::cwd_lead`); konzole `0`. `input_col` i `Role::Input` s nimi počítají. Složku zkracuje `short_path` zprostředka po celých složkách.
- Instalátor přes `Env::installer_send` / `installer_recv` (krátké čekání): řádek ve vlákně volá hlavní vlákno opakovaně, to mezi voláními obsluhuje okno.

## Testy

- `cargo test -p term` (`tests*.rs`, `tests_ui.rs`, `tests_broker.rs`, `tests_contrast.rs`, ...). QEMU: `cargo xtask test --only shell,ulohy` (relace a shell), `--only gui` (okno terminálu).

## Pasti

- `ui/mod.rs` má ~1290 řádků (blízko limitu ~1500): nové věci dávej do podmodulů.
- Změna `proto` zasahuje `user/termd`, `user/shsess`, `user/gui-term` a `libs/shlang` hostitele; ověř všechny.
