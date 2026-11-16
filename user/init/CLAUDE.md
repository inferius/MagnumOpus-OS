# user/init

První proces v userspace: spouští služby (`timed`, `acpid`, `devmgr`, `fsd`, `inputd`, `netd`, `fwd`, `dnsd`, `ntpd`, `revocd`, `taskd`, `launchd`, `accountd`, GUI `compd`, zvuk, média `mediad`), pak běží jako **supervisor** (`src/supervisor/`: zařízení připojená za běhu, restart spadlých služeb, registr služeb se závislostmi a lístky). V testovacím běhu (příznak `TEST_RUN` od jádra) místo supervisoru spustí testy userspace (`src/tests/`) a skončí s výsledkem. Popis: `docs/cs/init.md`, `docs/cs/obnova-sluzeb.md`.

## Klíčové soubory

- `src/main.rs` - pořadí startu; `src/system.rs`, `gui.rs`, `accounts.rs`, `session.rs`, `terminal.rs`, `audio.rs`, ... - start jednotlivých služeb.
- `src/audio.rs` - audiod pod supervisorem; relace aplikací s receptem (`app\0` + `audioproto::session::Recipe`, `gui::audio_handles` předá štítky `AUDIO`/`AUDIO_LEASE` při spuštění aplikace), propojení s `accountd` kanálem `permissions` při každém startu audiod; `src/perms.rs` - příkaz shellu `permissions` (kanál `permissions-admin`, uživatel podle relace terminálu; slova shellu anglicky, `microphone`, výpis `app\tperm\tallow|deny`) a jména účtů pro `acl` (powerproto `Accounts`, ze služby účtů kanálem `names`; shell kanál nedostává, duplikát sdílený relacemi by míchal odpovědi).
- `src/launch.rs` - spouštěcí služba `launchd` (std::process): start, klientské kanály (`launch::client`, štítek `LAUNCH`).
- `src/files.rs` - `files-copy` pro každou relaci a handly okna Soubory (pevné pořadí: FS, `files-copy`, parametry, „otevři cestu“, `names` u accountd; chybějící = prázdný kanál); `src/open.rs` - služba „otevři cestu“ (asociace `[open]`, kanál pro každou relaci, volání akce přes kompozitor, spuštění aplikace v relaci).
- `src/supervisor/` - `mod.rs` smyčka (`Supervisor::step`), `lifecycle.rs` restart a závislosti, `leases.rs` lístky (vč. `ADOPT` s limity a `queue_lease` pro lístky vydané mimo kolo supervisoru), `service.rs` adaptér služby; `src/libd.rs` - služba knihoven (adaptér `LibdService`, poskytovatel relací pro děti initu).
- `src/appearance.rs` - volba vzhledu: `/system/config/vzhled/{system,<uid>}.toml` (jen system, bez grantů), předání kompozitoru (`appearance ...` před `session open`, po pádu v `give_accounts`), změna z lišty (kanál `launcher`), příkaz `theme` (`powerproto::Request::Appearance`; shell převádí svá anglická slova do uloženého tvaru `schema svetly`; systémová volba jen přes `ELEVATE` `APPEARANCE_SYSTEM` ve vlákně, grant ověří na `grants`).
- `src/sysfiles.rs` - služba úkonu `SYSTEM_FILE` (příkaz `system`: zápis, smazání, složka v `/system`): kontrola cesty (`/system/accounts` nikdy), `ELEVATE` za relaci shellu ve vlákně, grant na `grants`, pak změna relací FS initu. Jiná cesta lidí do `/system` není.
- `src/sshd.rs` - služba SSH (`docs/cs/ssh.md`): hostitelský klíč v `/system/config/ssh`, zapnutí přes `ELEVATE` (`SSHD`), start při bootu jen když je zapnutá, kanál `claim <listek>` pro sshd; `src/remote.rs` - vzdálené relace (oddělené od `session::SESSIONS`, bez kompozitoru a `ELEVATE`), jednorázový lístek, pilulka v liště (`remote` kompozitoru).
- `src/tests/mod.rs` - `SUITES` (jména sad pro `--only`), makra `check!` / `check_eq!`; jeden soubor na oblast (`gui.rs`, `net.rs`, `fs.rs`, ...); `dynlink_disk.rs` knihovny z disku (DL-4c, sada `procesy`, kopíruje `dl4c/` z initfs do `/dl4c` na disku).

## Invarianty

- init drží kořenová práva a z nich vytváří užší pro služby (job s limity podle manifestu); služba nedostane víc, než potřebuje.
- Služba s restartem se po pádu spustí znovu s odkladem 100 ms, 200 ms, ... (max 5 s); víc než 5 restartů za 60 s = vzdá to. Záměrné ukončení testem (`Supervisor::kill_for_test`) restartuje stejně, ale do limitu se nepočítá (jinak `--repeat` službu vzdá). Klienti se k nové instanci připojují přes lístky (`libs/svcclient`).
- Po pádu `compd` se spustí záložní konzole; po pádu `termd` zůstanou relace (`shsess`).
- Instalátor (`src/install.rs`): `installd` se spouští až na první `OpenInstaller` a jen jednou dostane raw kanály disků (`Devices.raw`). Při startu se čte `/system/config/system.toml` (pásmo pro timed, rozložení klávesnice pro inputd); chybějící soubor = výchozí. Soubor `install-answers` v initfs spustí test instalace (jen obraz testu, nikdy instalovaný initfs); s `rozhrani = "gui"` ho dostane grafický instalátor (autopilot) místo textové cesty. S `rozhrani = "gui-vstup"` init jen najde inspekcí kompozitoru pole pásma a tlačítko Pokračovat a zaloguje jejich střed na obrazovce (klik QMP v `xtask/src/usb/install_test.rs`). Grafický instalátor `gui-install` je aplikace plochy jen na živém USB (`gui::desktop_apps`, `Devices.stick`) a dostane kanál k installd; installd dostane kanál producenta do taskd.
- Role `Gui` (relace okna pro potomky): adaptér `CompdService::narrow` přijme jen prázdné zúžení; `NARROW` váže nový lístek na rodiče (limit jako `ADOPT`), `Supervisor::revoke_leases` ruší lístky podle receptu i celý strom odvozených; `gui::revoke` = lístky + `revoke <id>` v kompozitoru. Testy `tests/gui_role.rs` (program `user/gui-probe`).
- Vzdálená relace vzniká jen na žádost `accountd` (`ToInit::OpenRemote`) po ověření podpisu; lístek platí jednou a porovnává se v konstantním čase. Funkce `session::*` hledající podle čísla relace musí znát i `remote::*`.
- Do společného `/tmp` lidé nezapisují (skupina jen `LIST|TRAVERSE`, `accounts::layout`). Každá relace (místní i vzdálená) dostane soukromou `/tmp/<uid>` (`session::create_tmp`, `fsproto::perm::user_tmp`) s klony shellu `/tmp/<uid>/.aside`; úklid zbytků při startu prochází i ty (`terminal::leftover_clones`). Test `ucty: shell uzivatele zapisuje (soukromy /tmp)`.
- Uživatelská čísla služeb: ACCOUNTD 902, LOGD 903 - nové číslo nesmí kolidovat.
- Render uzly (`src/render.rs`): poskytovatele si init vyžádá od displeje jednou (`OPEN_PROVIDER`) dřív, než displej dostane kompozitor, drží ho (`Kept`) a dává duplikát službě `gpud` (restart supervisorem). Render uzel pro kompozitor (`backend gpu`) a testy otevírá `render::open_render` přes gpud, nikdy přes displej.
- Kanál služby „otevři cestu“ dostane jen okno terminálu přihlášeného uživatele (`terminal.rs`, štítek `term::ctl::tag::OPEN`, relace = `Client.open_session`); záložní konzole ne.
- `open::poll` nesmí spouštět aplikaci se zámkem služby (spuštění Souborů si bere `open::endpoint`): spouštění až po `drop` zámku.
- Supervisor čeká přes `libos::WaitSet` (`Supervisor::waits`, vznikne v prvním `step`, protože `new` je `const`); port lístků je duplikát portu množiny, klíče lístků `< libos::FOREIGN_KEYS`, pakety vybírá `lease_packets` z `take_foreign` (port se nečte přímo, jinak by kradl pakety množiny).
- Pořadí startu je závislostní: `fsd` čeká na disk od `devmgr`, `netd` na síťovou kartu atd. Nové pořadí = upravit `docs/cs/init.md`.

## Testy

- Testy userspace jsou v QEMU, ne na hostiteli: `cargo xtask test --only <sada>[,<sada>]` (jména sad viz `SUITES`, např. `gui`, `sit`, `firewall`, `shell`, `supervisor`, `obnova sluzeb`). `--repeat N` hledá nestabilní testy.
- Nový test: funkce v `src/tests/<oblast>.rs`, zapsat do `TESTS` oblasti a do `SUITES`. Selhání vrací `Err(String)`, nepanikaří.
- Plný běh (všechny sady, obě architektury) pouští koordinátor. Sady v `ON_REQUEST` (`tests/mod.rs`, např. `wine` s diskem z `--wine`) běží jen s `--only`.

## Pasti

- Test delší než 20 s se v logu označí `[pomaly]`; každé další kolo `--repeat` přidá 60 s k limitu.
- Žádné pevné časování bez termínu: testy čekají s deadlinem, jinak visí celý běh.
- Sada, která si vezme ovladač z `display_native::NATIVES`, ho musí na konci vrátit (běžná instance, ne obnova), jinak další kolo `--repeat` ani další sady ovladač nenajdou. Kompozitoru v testu dávat duplikát konce klienta, ne jediný konec (jeho konec by ukončil ovladač).
- `src/tests/` je velké (přes 17 000 řádků celého crate): nové testy do nových souborů, ne do existujících nad ~1500 řádků.
