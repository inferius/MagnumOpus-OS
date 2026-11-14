# user/gui-player

Přehrávač nad `libs/ui` a službou `mediad` (`docs/cs/multimedia.md`, etapa F4 s testovacím dekodérem). `main.rs` otevře vestavěnou ukázku `MTM1`, pool bufferů NV12 s plotem snímku, proud `audiod` (kruh předá dekodéru) a povrch videa; `player.rs` drží frontu snímků, hodiny (zvuk, jinak monotónní čas), výběr snímku `mediaproto::clock::decide` a vracení bufferů po plotu uvolnění.

## Invarianty

- Buffer se vrací dekodéru (`RELEASE`) až po plotu uvolnění kompozitoru, nebo hned, když se snímek zahodí (nikdy nebyl ukázán).
- Kanál relace má smyčka (kopie handlu) i `MediaClient`; po každém volání se vybírají události zařazené klientem (`drain`).
- Bez kanálu k audiod (žádná zvuková karta) hraje bez zvuku.

## Testy

- QEMU: `cargo xtask test --only gui` (test „prehravac: plocha videa z mediad, prehrat a pozastavit“, `user/init/src/tests/gui_player.rs`).
