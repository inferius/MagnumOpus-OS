# user/volfs

Proces jednoho vyměnitelného média (program se std, `xtask/src/stdsys.rs` `STD_PROGRAMS`). Spouští ho init pro každý oddíl `block-media` (`user/init/src/removable.rs`) s argumenty `volfs <fat|exfat|ntfs> <ro|rw>` a dvěma handly: kanál oddílu (`blockproto`) a kanál k fsd (`libs/volproto`). Popis: `docs/cs/media.md`.

## Klíčové soubory

- `src/lib.rs` - trait `Volume` (operace podle cest), `io_err`; `src/serve.rs` - jedna zpráva `volproto` -> odpověď (`Outcome::stop` po `UNMOUNT`).
- `src/dev.rs` - `Sectors` (medium po sektorech), `Cached` (sdílená vyrovnávací paměť sektorů podle velikosti média `cache_sectors`, 64 KiB až 2 MiB; úklid před načtením, ne po něm, `flush_dirty` na konci každé měnící operace, `discard_dirty` po chybě, `set_read_only`), `MemDisk` pro testy.
- `src/fat.rs` (crate `fatfs`), `src/ntfs.rs` (crate `ntfs`, jen čtení), `src/exfat/` (vlastní: `boot.rs` spouštěcí oblast a součet, `entry.rs` sady položek, hash jména, razítka, `upcase.rs`, `alloc.rs` řetězy, bitmapa, `resize`, `dir.rs` složky, `mod.rs` svazek); `src/time.rs` časy FAT/NTFS.
- `src/main.rs` - napojení na kanály v systému; konec 0 = odpojeno, 2 = chybí handly, 3 = FS nejde připojit, 4 = médium vytaženo (zavřený kanál oddílu).

## Invarianty

- Proces běží ve vlastním jobu s limitem 16 MiB (`user/init/src/removable.rs`). Proces nemá nic kromě kanálu oddílu a kanálu k fsd. Rozsah sektorů a povolení zápisu hlídá ovladač (`diskguard::media`), ne volfs.
- Data z média jsou nedůvěryhodná: nikdy `panic` ani nekonečná smyčka (řetězy omezené počtem clusterů, indexy přes `get`, kontrolovaná aritmetika). Chyba = chyba operace.
- Po odpovědi na měnící operaci jsou data na médiu (`commit` = `flush_dirty`); po chybě se rozpracované změny zahodí (exFAT znovu načte bitmapu).
- exFAT: příznak „špinavý svazek“ se zapíše a vyprázdní před první změnou, `unmount` ho smaže. TexFAT, chyba média, poškozená tabulka velkých písmen nebo záložní spouštěcí oblast = jen pro čtení. Sady s neznámými sekundárními položkami se nemění (`NOT_SUPPORTED`).
- Pole `VolumeFlags` a `PercentInUse` nejsou v kontrolním součtu spouštěcí oblasti; jiné pole boot sektoru se za běhu nemění.

## Testy

- `cargo test -p volfs` (FAT, NTFS, exFAT; exFAT formátuje a nezávisle čte dev-dependency `exfat-fs`). V QEMU: `cargo xtask test --only "vymenna media"`.

## Pasti

- `exfat-fs` neumí přečíst prázdný soubor s prvním clusterem 0 (platné podle specifikace): soubory ověřované jím musí mít data.
- `fatfs` bez std závisí na neudržovaném `core_io`, proto je volfs program se std.
- Změna rozhraní `Volume` nebo `volproto` = úprava fsd (`user/fsd/src/serve_media.rs`) a `docs/cs/media.md` i `docs/en/media.md`.
