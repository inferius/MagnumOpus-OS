# libs/mediaproto

Protokol médií: aplikace <-> `mediad` <-> proces dekodéru (`mediatest`, později `ffdec`). Zprávy relace (`lib.rs`: `Request`, `Event`, `Info`, značka odpovědi), zdroj bajtů (`source.rs`), hodiny A/V a výběr snímku (`clock.rs`), testovací formát `MTM1` s generátorem obrazu NV12 (`testmedia.rs`), klient `MediaClient` za feature `client` (`conn.rs`). Popis: `docs/cs/multimedia.md`.

## Invarianty

- Čtení zprávy nikdy nepanikaří (zprávy čte služba i aplikace od procesu, který zpracovává nedůvěryhodná data); neplatná zpráva = `INVALID_ARGS`, neznámá operace = `NOT_SUPPORTED`, zbytek bajtů za zprávou = chyba.
- Zprávy od služby začínají značkou u32: `TAG_REPLY` (0) = odpověď, jinak číslo události (>= 100). Čísla operací a událostí se nepřečíslovávají (jsou v `docs/cs/multimedia.md`).
- Jeden kanál = jedna relace. Handly nejsou v datech zprávy, jejich pořadí určuje dokumentace operace.
- `testmedia::fill_nv12` a `checksum_nv12` jsou smlouva mezi dekodérem a testy: změna = upravit oboje a sadu `media`.

## Testy

- `cargo test -p mediaproto` (`src/tests.rs`). QEMU: `cargo xtask test --only media`.
