# user/launchd

Spouštěcí služba pro `std::process::Command` (protokol `libs/launchproto`, popis `docs/cs/rust-std.md`, kapitola Procesy). Spustí ji init s obrazem initfs; init drží řídicí kanál (`client` -> kanál klienta) a aplikace dostanou kanál se štítkem `LAUNCH`.

## Klíčové soubory

- `src/main.rs` - smyčka nad portem, klientské kanály, `CONNECT` (kanál pro potomka).
- `src/launch.rs` - `LAUNCH` a `LOOKUP`: registr příkazů z manifestů v initfs, hledání programu (registr -> `/system/bin` v relaci FS volajícího -> initfs; absolutní cesta = soubor s právem `EXECUTE`), manifest, podjob s limity, zúžení sítě, spuštění.

## Invarianty

- launchd nemá kořenová práva a nic nepřidává: job, relaci FS, relaci sítě a stdio dodává volající v požadavku. Potomek = podjob jobu volajícího, síť = relace volajícího zúžená manifestem (netd nikdy nerozšíří), manifest bez `[network]` = bez sítě.
- Program se čte relací FS volajícího (nikdy vlastní), takže potomek nespustí nic, co by rodič sám nepřečetl.
- Chyba nese popis pro uživatele (`launchproto::reply`), u odpojeného svazku kód `NOT_MOUNTED`.

## Testy

- Host: `cargo test -p launchproto -p manifest`. QEMU: `cargo xtask test --only "std procesy"` (aplikace `user/std-proc`).

## Pasti

- Požadavky se obsluhují postupně: dlouhé čtení programu z FS zdrží ostatní klienty.
- Supervisor launchd zatím jen sleduje, po pádu ho nerestartuje (aplikace dostanou `SERVICE_UNAVAILABLE`).
