# libs/filesview

Správce souborů v GUI (`user/gui-files`): stav a zprávy (`app.rs`), rozhraní nad `libs/ui` (`view.rs`), výpis s řazením a filtrem (`listing.rs`), navigace (`nav.rs`), náhled textu a BMP (`preview.rs`), texty hodnot a tři stavy (`format.rs`), rozhraní k systému `Backend` + `TestBackend`. Popis: `docs/cs/spravce-souboru.md`.

## Invarianty

- Neznámá hodnota se nikdy neukáže jako nula: masky `known`/`hidden` (`fsproto::avail`) -> „—“ a důvod v náhledu; řazení dává neznámé na konec.
- Výkon: řazení a filtr jen při změně; texty řádků až pro viditelné řádky (`Listing::row`, `Rc` sdílené s tabulkou). 10 000 položek nesmí stát nic navíc při překreslení (test `ten_thousand_items_are_fast`).
- Žádné vlastní kopie komponent `libs/ui`; co chybí (klávesy aplikace, schránka formátů, cíl puštění, obrázek), je požadavek na vlákno GUI.
- Žádná blokující čekání v aplikaci: kopírování dělá `files-copy`, otevírání služba initu (bez čekání na odpověď).

## Testy

- `cargo test -p filesview` (host, `src/tests.rs`); v QEMU `cargo xtask test --only gui` (test „spravce souboru“, snímek `target/shots/soubory.png`).

## Pasti

- Popisky tlačítek jsou zároveň jména v přístupnosti, podle nich klikají testy: stejný popisek dvakrát v jednom okně (např. „Obnovit“) je chyba.
