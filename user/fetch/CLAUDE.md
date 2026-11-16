# user/fetch

Klient HTTP(S) (`libs/httpc`, TLS `libs/tls`) jako proces s minimem capability: síťová relace podle manifestu (TCP 80/443, DNS), `/certs` jen pro čtení, kanál k `revocd`. Příkazy na bootstrap kanálu (`get`, `head`, `post`, `body`, `roots`, `save`, `save-bg`) popisuje hlavička `src/main.rs`.

## Klíčové soubory

- `src/main.rs` - handly a příkazy; `src/save.rs` - stahování do `/Stazene` (`docs/cs/stahovani.md`): `httpc::Sink` do dočasného `.jméno.stahuje`, přejmenování, `sys.origin`, úloha v taskd (zrušení), oznámení, srozumitelné chyby (`explain`).

## Invarianty

- Stahování jen s 4. handlem (relace FS jen `/Stazene` s `ORIGIN`, od initu `downloads.rs`); nikdy nepřepíše existující soubor (`unique`), po chybě dočasný soubor smaže.
- `save-bg` neodpovídá (okna nečtou odpovědi sdíleného kanálu); `save` odpovídá vždy.

## Testy

- QEMU: `cargo xtask test --only stahovani` (stahování) a `--only "TLS a HTTP"` (klient). Host: `cargo test -p httpc`.
