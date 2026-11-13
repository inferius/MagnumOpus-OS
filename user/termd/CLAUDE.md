# user/termd

Prostředník terminálu (milník 6b). Interpret v něm **není**. Bootstrap kanál = spojení s oknem terminálu (nativní protokol `term::proto`); handly `kind::USER` podle pořadí: kanál fsd, kanál registru relací, kanál napájení; volitelné handly se štítkem (brána agenta AI, relace zvukové služby a její lístek); `kind::INITFS` = obraz initfs, odkud se spouští `shsess`. Každá relace shellu běží ve vlastním procesu `user/shsess` (`libs/term::broker`); termd mezi oknem a relacemi směruje zprávy. Popis: `docs/cs/navrh-shell.md#terminál-a-služba-relací`, `docs/cs/init.md#terminál`.

## Klíčové soubory

- `src/main.rs` (jediný soubor, ~300 řádků); logika brokeru a protokolu je v `libs/term`.
- Proces relace: `user/shsess` (`ai.rs`, `audio.rs`, `fs.rs`); interpret: `libs/shlang`.

## Invarianty

- Relace přežijí konec `termd`: registr relací (`term::ctl`) je mimo termd; nový termd je převezme (`Adopt`). Zavřené hlavní spojení relaci neukončí, dokud něco běží.
- Programy v relaci dostanou jen cesty podle manifestu (`libs/term::perm`), ne víc.
- Příkazy `shutdown`/`reboot` jdou přes kanál napájení k initu, ne přímo.

## Testy

- Logika: `cargo test -p term`, `cargo test -p shlang`. QEMU: `cargo xtask test --only shell,ulohy`.

## Pasti

- Logiku nepřidávej sem; patří do `libs/term` (testovatelná na hostiteli).
- Změna pořadí handlů ve startovní zprávě = změna initu (`user/init/src/terminal.rs`) i termd.
