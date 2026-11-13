# user/devmgr

Správce zařízení. Projde sběrnici PCI Express, zaloguje zařízení a pro známá spustí ovladače z initfs (virtio, síťové karty Intel a Realtek podle ID v `nic.rs`, NVMe, AHCI, xHCI a třídní ovladače USB; na x86 `ps2`). Ovladače z manifestu (`<jmeno>.driver.toml` v initfs, `manifest.rs`) dostanou zařízení bez vestavěného ovladače; init se hlásí zprávou `dev <jmeno>` a po pádu je devmgr obnoví. Rodiči (init) posílá pro každé zařízení zprávu s koncem kanálu ke klientovi ovladače (`block`, `input`, `net`, `rng`, `display`, ...). Startovní zpráva: ROOT_RESOURCE, PCI_ECAM, INITFS. Popis: `docs/cs/ovladace.md`, `docs/cs/init.md`.

## Klíčové soubory

- `src/main.rs` (sběrnice, spouštění), `manifest.rs`, `native.rs`, `nic.rs` (tabulka ID karet), `usb.rs`, `restart.rs` (obnova po pádu ovladače).

## Invarianty

- devmgr drží kořenový Resource **bez** `SET_CLOCK`; ovladači vytváří omezený Resource (jeho BARy, svou stránku konfigurace, kvótu přerušení a DMA).
- Po pádu ovladače devmgr nejdřív vypne bus master, pak ovladač spustí znovu (zařízení může ještě dokončovat DMA).
- Při dlouhém čekání na ovladač disku posílá initu `busy` každých 5 s, aby init start nevzdal (init čeká na zprávu nejvýš 30 s).
- Aplikace nikdy nedostane capability k ovladači; klient ovladače je služba.
- Raw kanál disku (`block-raw <ss> <model>`) jen přeposílá initu; ten ho dá jedině službě `installd`. Zápis na něm hlídá ovladač (`diskguard::install::raw_write_allowed`), ne devmgr.

## Testy

- QEMU: `cargo xtask test --only ovladace,"ovladace z manifestu",disky,usb`. Host: `cargo test -p driverkit --features sim`, `cargo xtask test --driver <jmeno>`.

## Pasti

- Přidání podporovaného zařízení = ID do tabulky + ovladač v initfs (`xtask`) + docs; nový ovladač přes `cargo xtask new-driver`.
- Bez IOMMU (`--iommu none`, stroj bez VT-d/AMD-Vi/SMMU) smí zařízení přes DMA zapsat kamkoli; testy IOMMU pak podle očekávání selžou.
