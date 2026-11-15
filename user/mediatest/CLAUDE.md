# user/mediatest

Testovací dekodér s rozhraním budoucího `ffdec` (bez FFmpeg): formát `MTM1` z `libs/mediaproto/src/testmedia.rs`, obraz NV12 do poolu (`video.rs`), tón s16 stereo do kruhu `audiod` (`audio.rs`). Spouští ho jen `mediad`. Popis: `docs/cs/multimedia.md`.

## Invarianty

- Vstup jen přes kanál zdroje (`PROBE`); žádné jiné handly.
- Buffer se zapisuje jen, když je volný (vrácený `RELEASE`); plot snímku roste o 1 za dokončený snímek a začíná na hodnotě plotu při `POOL` (obnova po pádu pokračuje).
- `crash_at` v hlavičce = schválný konec kódem 70 (test obnovy).

## Testy

- QEMU: `cargo xtask test --only media`.
