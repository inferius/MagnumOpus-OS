# user/mediad

Služba médií (`docs/cs/multimedia.md`). Řídicí kanál od initu (`media.rs` v initu): jen `CONNECT`. Každá relace běží ve vlastním vlákně (`session.rs`), spouští proces dekodéru ve vlastním jobu s limity (podle prvních bajtů zdroje: `MTM1` = `mediatest` 64 MiB, jinak `ffdec` nad FFmpeg 256 MiB; `ffdec` je PIE, `mediad` má od initu relaci s libd) a obsluhuje jeho čtení zdroje (`source.rs`: VMO nebo kanál aplikace).

## Invarianty

- Dekodér nedostane žádný handle kromě řídicího kanálu, kanálu zdroje (od `mediad`), VMO poolu, plotu snímku a kruhu zvuku. Nikdy handle souboru, sítě ani kanál aplikace.
- Události dekodéru se před přeposláním kontrolují (`forward`): buffer musí být v poolu, `ERROR` se ztrátou dekodéru smí poslat jen `mediad`.
- Relace si drží kopie handlů poolu a kruhu kvůli `REOPEN` po pádu dekodéru.
- Konec kanálu klienta = konec relace (`Drop` dekodéru zabije jeho job).

## Testy

- QEMU: `cargo xtask test --only "media,media ffmpeg"` (`user/init/src/tests/media.rs`, `media_ffmpeg.rs`).

## Pasti

- Obsluha zdroje kanálem aplikace blokuje vlákno relace (čeká až 5 s na aplikaci); ostatní relace běží dál.
- Po pádu libd obnoví `mediad` svou relaci až při dalším spuštění `ffdec` (`RECONNECT` na lístku u supervisoru initu). Během testů supervisor neřídí hlavní smyčka initu, ale test: testy, které spouštějí `ffdec`, proto volají `media_ffmpeg::libd_ready` (jednou za generaci libd spustí `ffdec` ve vedlejším vlákně a mezitím řídí supervisor). Bez toho po sadě `procesy` (test obnovy libd) visí každé `OPEN` do `ffdec`.
