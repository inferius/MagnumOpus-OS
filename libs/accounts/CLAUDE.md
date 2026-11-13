# libs/accounts

Logika služby účtů `accountd` bez závislosti na jádru (`no_std`, testy na hostiteli): záznamy účtů (`record.rs`), heslo Argon2id (`kdf.rs`), PIN vázaný na klíč zařízení (`device.rs`), zpomalení po chybách (`throttle.rs`), bezpečnostní klíče CTAP2 (`auth.rs`), obaly klíče domova a obnovovací klíč (`wrap.rs`), správa (`manage.rs`), jádro `engine.rs`. Popis: `docs/cs/ucty.md`. Služba: `user/accountd`, protokol `libs/accountproto`.

## Invarianty

- Žádná vlastní kryptografie: jen RustCrypto (argon2, chacha20poly1305, hkdf, hmac, sha2, blake2).
- Tajemství se nikdy nevypisují (ani do logu) a po zpracování se nulují (`zeroize`).
- Čísla účtů se nerecyklují (viz `engine.rs`, metadata starší než účty).
- Správa počítače bez roota; schválená rozhodnutí jsou v `docs/cs/ucty.md`.

## Testy

- `cargo test -p accounts` (`src/tests/accounts.rs`, `crypto.rs`). Služba v QEMU: `cargo xtask test --only ucty`.

## Pasti

- V testech a QEMU se používají zlehčené parametry KDF (`kdf = "qemu"` v konfiguraci accountd); na HW musí platit silné (`hw`). Neověřuj bezpečnost na zlehčených parametrech.
- Změna formátu záznamu účtu zasahuje i uložená data v `/system/accounts`.
