# user/accountd

Služba uživatelských účtů; logika je v `libs/accounts`, protokol v `libs/accountproto`. Handly ve startovní zprávě (kind USER): relace FS (`/db` = `/system/accounts`, účet 902), její lístek, kanál k initu (`accountproto::ToInit`). První zpráva bootstrapu = konfigurace (TOML: `kdf = "qemu" | "hw"`, `dev = true` jen ve vývojovém běhu), odpověď `ok`. Pak init posílá `login` / `setup` / `key` + handle a `status`. Popis: `docs/cs/ucty.md`.

## Klíčové soubory

- `src/main.rs` (smyčka, konfigurace), `service.rs` (obsluha požadavků), `admin.rs` (správa), `store.rs` (ukládání na FS), `os.rs` (vazba na OS).

## Invarianty

- Tajemství se nikdy nevypisují; přijaté zprávy se po zpracování nulují (`zeroize`).
- Žádný root: správa počítače podle schválených rozhodnutí v `docs/cs/ucty.md`.
- Data účtů jsou v `/system/accounts` přes vlastní relaci FS (účet 902), ne pod identitou volajícího.
- `dev = true` a `kdf = "qemu"` nesmějí do produkčního běhu.

## Testy

- Logika: `cargo test -p accounts`. Služba v QEMU: `cargo xtask test --only ucty`.

## Pasti

- Nový druh přihlášení = protokol (`accountproto`) + `libs/accounts` + init + přihlašovací obrazovka kompozitoru (`libs/comp::greeter`) + docs cs+en.
- Změna formátu uložených záznamů zasahuje existující data na discích.
