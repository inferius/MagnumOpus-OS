# libs/shlang

Jazyk shellu MagnumOpus: jeden jazyk, dva zápisy (`no_std` + `alloc`). **B** (příkazový, výchozí): `ls ~/Dok -r | where size > 100MB`; **C** (výrazový, blízký Rustu). Oba parsery (`parse/`) dělají stejný strom (`ast.rs`); formátovač (`fmt/`) umí strom zapsat v obou zápisech, interpret (`eval/`) ho vyhodnotí. Dál `lex.rs`, `highlight.rs`, `complete`, `explain`, `history`, `vfs/` (koncept v klonu, undo, náhled). Návrh a gramatika: `docs/cs/navrh-shell.md`, příkazy `docs/cs/prikazy-shellu.md`.

## Invarianty

- Nic nemá vlastní chování podle zápisu: B i C vedou na stejný strom a stejnou sémantiku. Změna jazyka = oba parsery + formátovač + interpret + docs.
- Prostředí (soubory, programy, čas, přerušení) dodává hostitel přes `host::Host`; interpret sám nic nedělá. V OS ho dodává `user/termd`/`user/shsess`, v testech `vfs::mem::MemFs` a testovací hostitel.
- Příkaz `install` (`eval/builtins/install.rs`) je jen průvodce: rozhodnutí a texty jsou v `libs/installplan`, zápis dělá služba `installd` (hostitel dává kanál přes `Host::installer`). Nové kroky instalace patří do `installplan`, ne sem.
- Chyby shellu jsou hodnoty (kód, popis, místo, příčina, rada; kódy `SH_*` nebo názvy chyb systému), ne panic. Hlášení jsou česky.

## Testy

- `cargo test -p shlang`. QEMU: `cargo xtask test --only shell,ulohy`.

## Pasti

- Změna syntaxe se promítne do zvýrazňování, doplňování i formátovače; zapomenuté místo se projeví až jako špatně obarvený nebo špatně formátovaný příkaz.
- Gramatika a rozhodnutí jsou ve schváleném návrhu; nejasnost řeš dotazem (A/B/C), neměň ji z vlastní hlavy.
