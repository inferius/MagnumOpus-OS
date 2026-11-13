# libs/installview

Grafický instalátor (`design/instalace.html`): stav průvodce a rozhraní nad `libs/ui`. Binárka je `user/gui-install` (jen lepidlo: kanály, vlákna autopilota). Kroky, plán disku, mapa disku (`installplan::map`), validace a texty jsou v `libs/installplan`, zápis dělá služba `user/installd`. Popis: `docs/cs/instalace.md#grafický-průvodce`.

## Invarianty

- Žádná logika instalace navíc: stejné kroky a požadavky jako textový `install` (`libs/shlang/src/eval/builtins/install.rs`). Nové pravidlo nebo krok patří do `installplan`, ne sem.
- Se službou installd je rozpracovaný nejvýš jeden požadavek (`waiting`), další čekají ve frontě; odpovědi nemají číslo požadavku.
- Heslo a PIN jen v `password_input` (do stromu přístupnosti nejdou, ověřuje test).
- Rozvržení podle třídy šířky okna: `compact` bez seznamu kroků.

## Testy

- `cargo test -p installview` (`src/tests.rs`, falešný installd nad skutečným plánem). QEMU: `cargo xtask test --only instalace-gui` (autopilot, snímky kroků v `target/shots/instalace-gui-<arch>/`).

## Pasti

- Autopilot (testovací běh) se posouvá zprávou `AUTO_STEP` na stejném kanálu jako odpovědi installd; když přijde během čekání na installd, provede se až po odpovědi (`auto_pending`).
- Značky `gui-install: krok N`, `gui-install: HOTOVO` čte `xtask/src/usb/install_test.rs`; změna textu = změna testu.
