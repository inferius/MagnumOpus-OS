# user/mediad

Služba médií (`docs/cs/multimedia.md`). Řídicí kanál od initu (`media.rs` v initu): jen `CONNECT`. Každá relace běží ve vlastním vlákně (`session.rs`), spouští proces dekodéru (obraz `mediatest` z initfs) ve vlastním jobu s limity a obsluhuje jeho čtení zdroje (`source.rs`: VMO nebo kanál aplikace).

## Invarianty

- Dekodér nedostane žádný handle kromě řídicího kanálu, kanálu zdroje (od `mediad`), VMO poolu, plotu snímku a kruhu zvuku. Nikdy handle souboru, sítě ani kanál aplikace.
- Události dekodéru se před přeposláním kontrolují (`forward`): buffer musí být v poolu, `ERROR` se ztrátou dekodéru smí poslat jen `mediad`.
- Relace si drží kopie handlů poolu a kruhu kvůli `REOPEN` po pádu dekodéru.
- Konec kanálu klienta = konec relace (`Drop` dekodéru zabije jeho job).

## Testy

- QEMU: `cargo xtask test --only media` (`user/init/src/tests/media.rs`).

## Pasti

- Obsluha zdroje kanálem aplikace blokuje vlákno relace (čeká až 5 s na aplikaci); ostatní relace běží dál.
