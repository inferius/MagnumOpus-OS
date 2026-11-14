# user/init

První proces v userspace: spouští služby (`timed`, `acpid`, `devmgr`, `fsd`, `inputd`, `netd`, `fwd`, `dnsd`, `ntpd`, `revocd`, `taskd`, `launchd`, `accountd`, GUI `compd`, zvuk), pak běží jako **supervisor** (`src/supervisor/`: zařízení připojená za běhu, restart spadlých služeb, registr služeb se závislostmi a lístky). V testovacím běhu (příznak `TEST_RUN` od jádra) místo supervisoru spustí testy userspace (`src/tests/`) a skončí s výsledkem. Popis: `docs/cs/init.md`, `docs/cs/obnova-sluzeb.md`.

## Klíčové soubory

- `src/main.rs` - pořadí startu; `src/system.rs`, `gui.rs`, `accounts.rs`, `session.rs`, `terminal.rs`, `audio.rs`, ... - start jednotlivých služeb.
- `src/launch.rs` - spouštěcí služba `launchd` (std::process): start, klientské kanály (`launch::client`, štítek `LAUNCH`).
- `src/files.rs` - `files-copy` pro každou relaci a handly okna Soubory; `src/open.rs` - služba „otevři cestu“ (asociace `[open]`, kanál pro každou relaci, volání akce přes kompozitor, spuštění aplikace v relaci).
- `src/supervisor/` - `mod.rs` smyčka (`Supervisor::step`), `lifecycle.rs` restart a závislosti, `leases.rs` lístky (vč. `ADOPT` s limity a `queue_lease` pro lístky vydané mimo kolo supervisoru), `service.rs` adaptér služby; `src/libd.rs` - služba knihoven (adaptér `LibdService`, poskytovatel relací pro děti initu).
- `src/tests/mod.rs` - `SUITES` (jména sad pro `--only`), makra `check!` / `check_eq!`; jeden soubor na oblast (`gui.rs`, `net.rs`, `fs.rs`, ...).

## Invarianty

- init drží kořenová práva a z nich vytváří užší pro služby (job s limity podle manifestu); služba nedostane víc, než potřebuje.
- Služba s restartem se po pádu spustí znovu s odkladem 100 ms, 200 ms, ... (max 5 s); víc než 5 restartů za 60 s = vzdá to. Klienti se k nové instanci připojují přes lístky (`libs/svcclient`).
- Po pádu `compd` se spustí záložní konzole; po pádu `termd` zůstanou relace (`shsess`).
- Instalátor (`src/install.rs`): `installd` se spouští až na první `OpenInstaller` a jen jednou dostane raw kanály disků (`Devices.raw`). Při startu se čte `/system/config/system.toml` (pásmo pro timed, rozložení klávesnice pro inputd); chybějící soubor = výchozí. Soubor `install-answers` v initfs spustí test instalace (jen obraz testu, nikdy instalovaný initfs); s `rozhrani = "gui"` ho dostane grafický instalátor (autopilot) místo textové cesty. S `rozhrani = "gui-vstup"` init jen najde inspekcí kompozitoru pole pásma a tlačítko Pokračovat a zaloguje jejich střed na obrazovce (klik QMP v `xtask/src/usb/install_test.rs`). Grafický instalátor `gui-install` je aplikace plochy jen na živém USB (`gui::desktop_apps`, `Devices.stick`) a dostane kanál k installd; installd dostane kanál producenta do taskd.
- Uživatelská čísla služeb: ACCOUNTD 902, LOGD 903 - nové číslo nesmí kolidovat.
- Render uzly (`src/render.rs`): poskytovatele si init vyžádá od displeje jednou (`OPEN_PROVIDER`) dřív, než displej dostane kompozitor, drží ho (`Kept`) a dává duplikát službě `gpud` (restart supervisorem). Render uzel pro kompozitor (`backend gpu`) a testy otevírá `render::open_render` přes gpud, nikdy přes displej.
- Kanál služby „otevři cestu“ dostane jen okno terminálu přihlášeného uživatele (`terminal.rs`, štítek `term::ctl::tag::OPEN`, relace = `Client.open_session`); záložní konzole ne.
- `open::poll` nesmí spouštět aplikaci se zámkem služby (spuštění Souborů si bere `open::endpoint`): spouštění až po `drop` zámku.
- Pořadí startu je závislostní: `fsd` čeká na disk od `devmgr`, `netd` na síťovou kartu atd. Nové pořadí = upravit `docs/cs/init.md`.

## Testy

- Testy userspace jsou v QEMU, ne na hostiteli: `cargo xtask test --only <sada>[,<sada>]` (jména sad viz `SUITES`, např. `gui`, `sit`, `firewall`, `shell`, `supervisor`, `obnova sluzeb`). `--repeat N` hledá nestabilní testy.
- Nový test: funkce v `src/tests/<oblast>.rs`, zapsat do `TESTS` oblasti a do `SUITES`. Selhání vrací `Err(String)`, nepanikaří.
- Plný běh (všechny sady, obě architektury) pouští koordinátor.

## Pasti

- Test delší než 20 s se v logu označí `[pomaly]`; každé další kolo `--repeat` přidá 60 s k limitu.
- Žádné pevné časování bez termínu: testy čekají s deadlinem, jinak visí celý běh.
- Sada, která si vezme ovladač z `display_native::NATIVES`, ho musí na konci vrátit (běžná instance, ne obnova), jinak další kolo `--repeat` ani další sady ovladač nenajdou. Kompozitoru v testu dávat duplikát konce klienta, ne jediný konec (jeho konec by ukončil ovladač).
- `src/tests/` je velké (přes 17 000 řádků celého crate): nové testy do nových souborů, ne do existujících nad ~1500 řádků.
