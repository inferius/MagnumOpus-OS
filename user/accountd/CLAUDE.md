# user/accountd

Služba uživatelských účtů; logika je v `libs/accounts`, protokol v `libs/accountproto`. Handly ve startovní zprávě (kind USER): relace FS (`/db` = `/system/accounts`, účet 902), její lístek, kanál k initu (`accountproto::ToInit`). První zpráva bootstrapu = konfigurace (TOML: `kdf = "qemu" | "hw"`, `dev = true` jen ve vývojovém běhu), odpověď `ok`. Pak init posílá `login` / `setup` / `key` + handle a `status`. Popis: `docs/cs/ucty.md`.

## Klíčové soubory

- `src/main.rs` (smyčka, konfigurace), `service.rs` (obsluha požadavků), `admin.rs` (správa: `ELEVATE`, popis úkonu v dialogu včetně `SYSTEM_FILE`, granty, audit), `devmode.rs` (režim vývojáře, jen feature `dev-mode`), `store.rs` (ukládání na FS), `os.rs` (vazba na OS), `perms.rs` (povolení aplikací: kanál `permissions` pro dotazy služby zařízení, `permissions-admin` pro init, dialog Povolit / Jen tentokrát / Nepovolit, uložení `/db/permissions/<uid>`).

## Invarianty

- Tajemství se nikdy nevypisují; přijaté zprávy se po zpracování nulují (`zeroize`).
- Žádný root: správa počítače podle schválených rozhodnutí v `docs/cs/ucty.md`.
- Data účtů jsou v `/system/accounts` přes vlastní relaci FS (účet 902), ne pod identitou volajícího.
- `dev = true` a `kdf = "qemu"` nesmějí do produkčního běhu.
- Režim vývojáře je jen za feature `dev-mode` (zapíná ji xtask jen s vývojovým profilem) a jediné jeho místo je politika potvrzení v `Service::elevate`; žádné `cfg` jinde ve službě. Vývojový účet má roli správy, ale platí jen s `dev = true` (`Engine::is_admin`), do `admins()` se nepočítá.
- Povolení aplikací: bez kompozitoru (dialogu) nebo pro neznámého uživatele vždy Nepovolit; dialogy správy a povolení sdílejí čísla (`Admin::next_dialog_id`), odpověď povolení se pozná v `dialog_answers` dřív než správa.

## Testy

- Logika: `cargo test -p accounts`. Služba v QEMU: `cargo xtask test --only ucty`.

## Pasti

- Nový druh přihlášení = protokol (`accountproto`) + `libs/accounts` + init + přihlašovací obrazovka kompozitoru (`libs/comp::greeter`) + docs cs+en.
- Změna formátu uložených záznamů zasahuje existující data na discích.
