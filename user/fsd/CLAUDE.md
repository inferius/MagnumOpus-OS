# user/fsd

Služba souborového systému nad knihovnou `libs/fs`. Startovní zpráva (kind USER): kanál k ovladači disku (`blockproto`), kanál pro klienta (`fsproto`). První zpráva na bootstrap kanálu je konfigurace (`debug` = povolit testovací operace). Služba obsluhuje požadavky, po `COMMIT_INTERVAL` ukládá commit (fsync klienta je trvalý hned přes intent log), při nečinnosti po malých krocích defragmentuje a deduplikuje, scrub jednou týdně a na povel. Popis: `docs/cs/souborovy-system.md#služba-fsd`, protokol `docs/cs/souborovy-system.md#protokol-fsproto`.

## Klíčové soubory

- `src/main.rs` - smyčka a konfigurace; `blkdev.rs` - adaptér ovladače disku na `BlockDevice`; `serve.rs` + `serve_data.rs`, `serve_hist.rs` (historie), `serve_perm.rs` (oprávnění), `serve_v2.rs`; `tasks.rs` (přehled úloh).
- Protokol: `libs/fsproto`; klient `libs/fsproto` (`FsClient`).

## Invarianty

- Oprávnění a identita relace jsou vynucené tady: identitu a připojené cesty relace určuje `CONNECT_AS` (`libs/fsproto/src/perm.rs`); `docs/cs/souborovy-system.md#oprávnění-a-identita-relací`.
- Hodnoty z disku i požadavky klientů jsou nedůvěryhodný vstup.
- Pád `fsd` nesmí poškodit data (commit je atomický, `libs/fs`); klienti se po restartu připojí přes lístky (`docs/cs/obnova-sluzeb.md`).
- Pozdní práce (defrag, scrub) musí ustoupit požadavkům klientů (malé kroky mezi požadavky).

## Testy

- Logika FS: `cargo test -p fs --features std`. Služba v QEMU: `cargo xtask test --only "souborovy system"` (další sady `historie FS`, `data FS`, `vykon FS`, `obnova fsd`).

## Pasti

- Změna protokolu (`fsproto`) zasahuje všechny klienty (např. `shsess`, `accountd`, init); najdi je přes `grep fsproto */*/Cargo.toml`.
- Přidání operace do `fsproto` bez úpravy docs cs+en nechá protokol nezdokumentovaný.
