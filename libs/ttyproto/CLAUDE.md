# libs/ttyproto

Protokol řídicího kanálu terminálu (`termios.h`): režim (pole `struct termios` s hodnotami jako Linux), velikost okna, událost změny velikosti. Kanál dostane proces spuštěný v terminálu ve startovní zprávě se štítkem `abi::params::tag::TTY`; data jdou dál proudy stdin/stdout/stderr. Popis: `docs/cs/cstd.md` (kapitola termios).

## Klíčové soubory

- `src/lib.rs` - konstanty příznaků a indexů `c_cc`, `Mode` (`cooked`, `make_raw`), `Size`, `Request` (encode/decode), odpověď (`encode_reply`, `decode_reply`: `i32` stav + tělo), `Tty` = stav strany terminálu (filtr podporovaných příznaků, změna velikosti).
- `src/tests.rs` - host testy.

## Invarianty

- Hodnoty příznaků a indexy `c_cc` jsou jako Linux/glibc (cstd je předává beze změny; test `not_a_terminal` v `libs/cstd/src/termios_tests.rs` hlídá hlavičku `termios.h`).
- Řádková disciplína (kanonický režim, echo, Ctrl+C) je na straně terminálu, ne v klientovi.
- Na kanálu je rozpracovaný nejvýš jeden požadavek; odpověď na `WATCH_SIZE` nese jeden handle události.

## Testy

- `cargo test -p ttyproto`; klient `cargo test -p cstd` (`termios_tests.rs`); QEMU `cargo xtask test --only cstd` (krok `termios`, testovací terminál v `user/init/src/tests/cstd.rs`).

## Pasti

- Strana okna terminálu (`shsess`) ještě není: dnes kanál dává jen testovací server initu.
