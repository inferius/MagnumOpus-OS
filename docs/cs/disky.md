# Disky: NVMe, AHCI a ochrana cizích disků

Disky obsluhují tři ovladače v userspace. Všechny mluví s klientem
(souborový systém `fsd`, init, testy) stejným protokolem
[`blockproto`](souborovy-system.md):

| ovladač | zařízení | kde | spouští ho |
|---|---|---|---|
| `virtio-blk` | virtuální disk virtio 1.x | QEMU | `devmgr` podle ID zařízení virtio |
| `nvme` | řadič NVMe (PCI třída 01:08:02) | VMware, skutečné PC, QEMU `-device nvme` | `devmgr` podle třídy PCI |
| `ahci` | řadič SATA AHCI (PCI třída 01:06:01) | VMware (SATA), skutečné PC, QEMU `-device ahci` / q35 | `devmgr` podle třídy PCI |
| `usb-storage` | disk USB (třída 08/06/50, SCSI přes BOT) | flashky, externí disky, [živé USB](zive-usb.md), QEMU `-device usb-storage` | `devmgr` podle třídy rozhraní USB ([USB](usb.md#disky-usb-usb-storage)) |

Společná obsluha požadavků je v `blockproto::server`: kontrola rozsahu,
překlad na náš oddíl GPT a zákaz zápisu na cizí disk jsou na jednom místě
pro všechny tři ovladače. Ovladač dodá jen čtení, zápis, flush a TRIM
v sektorech zařízení (trait `Disks`). Disky s logickým sektorem 512 B
i 4 KiB fungují stejně (viz [Velikost sektoru](#velikost-sektoru)).

**Čtení bez čekání a spojování** **[ověřeno v QEMU]**: `BlockClient::read`
pošle u delšího čtení až 32 požadavků po 32 KiB bez čekání (odpovědi chodí
v pořadí, po chybě klient vybere i zbylé odpovědi); `write_queued` totéž
pro zápisy. Ovladač s drahým příkazem vrátí větší `Disks::max_read` (dnes
`usb-storage`) a server navazující čtení jednoho klienta, která už čekají
ve frontě, přečte jedním voláním a odpoví na každé zvlášť. Když spojené
čtení selže, každé se zopakuje samo, takže chybu dostane to, kterého se
týká. Ostatní ovladače zůstávají po 32 KiB.

<figure class="diagram">
<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Spuštění ovladačů disků a ochrana cizích disků">
<defs><marker id="dk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="76" rx="6"/><text class="t b" x="85" y="44" text-anchor="middle">PCI Express</text><text class="m" x="85" y="63" text-anchor="middle">01:08:02 NVMe</text><text class="m" x="85" y="80" text-anchor="middle">01:06:01 AHCI</text>
<rect class="box" x="210" y="20" width="150" height="76" rx="6"/><text class="t b" x="285" y="44" text-anchor="middle">devmgr</text><text class="m" x="285" y="63" text-anchor="middle">omezený Resource</text><text class="m" x="285" y="80" text-anchor="middle">BAR, MSI(-X), DMA</text>
<path class="ln" d="M160 58 L208 58" marker-end="url(#dk1)"/>
<rect class="acc" x="410" y="12" width="190" height="92" rx="6"/><text class="t b" x="505" y="36" text-anchor="middle">nvme / ahci</text><text class="m" x="505" y="55" text-anchor="middle">fronty, příkazy, přerušení</text><text class="m" x="505" y="71" text-anchor="middle">DMA jen přes IOMMU</text><text class="m" x="505" y="87" text-anchor="middle">jeden kanál na disk</text>
<path class="ln" d="M360 58 L408 58" marker-end="url(#dk1)"/><text class="m" x="364" y="50">pci, bar</text>
<rect class="acc" x="410" y="140" width="190" height="80" rx="6"/><text class="t b" x="505" y="164" text-anchor="middle">diskguard</text><text class="m" x="505" y="183" text-anchor="middle">čte MBR a GPT</text><text class="m" x="505" y="199" text-anchor="middle">rozhodne: náš / cizí</text>
<path class="ln" d="M505 104 L505 138" marker-end="url(#dk1)"/>
<rect class="ok" x="640" y="120" width="110" height="56" rx="6"/><text class="t b" x="695" y="143" text-anchor="middle">náš oddíl</text><text class="m" x="695" y="161" text-anchor="middle">jen oddíl, zápis</text>
<rect class="warn" x="640" y="190" width="110" height="56" rx="6"/><text class="t b" x="695" y="213" text-anchor="middle">cizí disk</text><text class="m" x="695" y="231" text-anchor="middle">celý, jen čtení</text>
<path class="ln" d="M600 170 L638 150" marker-end="url(#dk1)"/><path class="ln" d="M600 195 L638 215" marker-end="url(#dk1)"/>
<rect class="box" x="210" y="160" width="150" height="60" rx="6"/><text class="t b" x="285" y="185" text-anchor="middle">init</text><text class="m" x="285" y="204" text-anchor="middle">disk s FS -&gt; fsd</text>
<path class="ln dash" d="M408 80 L362 175" marker-end="url(#dk1)"/><text class="m" x="300" y="130">block + kanál, done</text>
<rect class="box" x="10" y="160" width="150" height="60" rx="6"/><text class="t b" x="85" y="185" text-anchor="middle">fsd</text><text class="m" x="85" y="204" text-anchor="middle">blockproto</text>
<path class="ln" d="M210 190 L162 190" marker-end="url(#dk1)"/>
<text class="m" x="10" y="270">Ochrana je v ovladači, ne ve fsd: chybný klient (fsd, test, nástroj) nemůže zapsat mimo náš oddíl.</text>
<text class="m" x="10" y="290">Zápis a TRIM na cizí disk vrací READ_ONLY; FEATURES hlásí READ_ONLY, takže FS se připojí jen pro čtení.</text>
<text class="m" x="10" y="310">Klient našeho oddílu vidí sektor 0 = první sektor oddílu; tabulku oddílů ani cizí oddíly nevidí.</text>
<text class="m" x="10" y="330">Při jakékoli pochybnosti (poškozená GPT, překryv oddílů, chyba čtení) platí „cizí disk“.</text>
</svg>
<figcaption>Spuštění ovladačů disků a rozhodnutí, co klient uvidí.</figcaption>
</figure>

## Riziko a ochrana cizích disků

> **Riziko.** Na VMware s připojeným fyzickým diskem a hlavně na skutečném
> počítači vidí ovladač NVMe nebo AHCI i disky, které nám nepatří: Windows,
> Linux, zálohy. Jediný chybný zápis (formátování, přehrání logu FS, test,
> který píše do „testovacího“ disku) na špatný disk zničí cizí data a může
> způsobit, že se jiný systém už nespustí. Chyba nemusí být zlý úmysl:
> stačí, že init vybere špatný disk, nebo že test pozná testovací disk
> jen podle velikosti.

Proto ovladač **před obsluhou klienta** disk rozebere (`libs/diskguard`)
a podle výsledku omezí, co klient vidí a smí:

| disk | klient vidí | zápis a TRIM |
|---|---|---|
| GPT s **naším oddílem**: typ `C123ADA0-51BA-48C5-A536-C2CE6B2B3581` nebo jméno oddílu přesně `MagnumOpus` | jen tento oddíl (sektor 0 = první sektor oddílu) | povolen, jen uvnitř oddílu; atribut oddílu „jen pro čtení“ (bit 60) ho zakáže |
| celý disk s **naším FS** bez tabulky oddílů (superblok `OSFS` na začátku některého z bloků 1..8 po 4 KiB, sektor 0 bez podpisu MBR) | celý disk | povolen (datový disk v QEMU, starší rozložení) |
| GPT bez našeho oddílu | celý disk | **zakázán** (`READ_ONLY`) |
| náš oddíl je neplatný (mimo disk, mimo použitelnou oblast, **překrývá jiný oddíl** nebo MBR, hlavičky či tabulky GPT) | celý disk | **zakázán** |
| MBR (bez GPT), ochranný MBR s poškozenými hlavičkami GPT | celý disk | **zakázán** |
| prázdný nebo neznámý disk, chyba čtení | celý disk | **zakázán** |
| zařízení hlásí ochranu proti zápisu (NVMe write protect, virtio `RO`) | podle tabulky | **zakázán** |
| navíc: oddíl FAT, exFAT nebo NTFS (MBR, GPT, superfloppy; ne ESP, rozšířený oddíl, překryv, disk s naším oddílem) | další kanál `block-media` jen na tento oddíl pro proces média `volfs` | jen vyměnitelné USB (RMB) bez ochrany proti zápisu a jen FAT/exFAT, jinak **zakázán** ([Vyměnitelná média](media.md#kdy-se-na-médium-smí-zapisovat)) **[ověřeno v QEMU]** |

Podrobnosti:

- **Ve výchozím stavu jsou cizí disky jen pro čtení.** Globální přepínač
  zápisu neexistuje. Jedinou výjimkou je **raw kanál instalátoru**
  (`block-raw`): dostane ho jen služba `installd` a ovladač i na něm pustí
  zápis jen do GPT a do oddílů instalátoru (`MagnumOpus`, `MagnumOpus
  EFI`), nikdy do cizího oddílu **[ověřeno v QEMU]** (`--only instalace`).
  Podrobně: [Instalace](instalace.md#ochrana-cizích-oddílů).
- GPT se ověřuje celá: podpis, CRC hlavičky i tabulky, poloha hlavičky,
  rozsah použitelné oblasti. Oddíl nesmí zasahovat do MBR, hlaviček ani
  tabulek GPT, i kdyby hlavička (s platným CRC, např. od chybného
  nástroje) hlásila použitelnou oblast přes ně. Při poškozené primární hlavičce se použije
  záložní na konci disku. Když neprojde ani jedna, disk je cizí.
- Vybere se **první platný** náš oddíl (podle pořadí v tabulce). Další
  naše oddíly zatím klient nevidí (TODO: kanál na každý oddíl).
- Ochrana je v procesu ovladače, ne až ve `fsd`. Chyba ve `fsd`, v initu
  nebo v testu tak nezapíše mimo náš oddíl. Rozsah, který klient vidí,
  hlídá jedna funkce (`Decision::translate`, včetně přetečení LBA).
- FS na cizím disku se připojí jen pro čtení: ovladač v odpovědi na
  `FEATURES` nastaví `READ_ONLY` a smaže `DISCARD`.

Zbytková rizika (ochrana je nepokrývá):

- **Bez IOMMU** (`--iommu none`, starší počítač, IOMMU vypnutá ve
  firmwaru) může chybný nebo napadený ovladač přes DMA zapsat do libovolné
  paměti, a tím i obejít ochranu disků (např. přepsat paměť jiného
  ovladače). S IOMMU smí zařízení jen do DMA paměti svého ovladače (test
  `IOMMU chrání paměť mimo DMA`).
- Ochrana věří tabulce oddílů. Když uživatel omylem označí cizí oddíl naším
  typem nebo jménem `MagnumOpus`, ovladač do něj zapisovat bude.
- Ovladač samotný posílá příkazy řadiči. Chyba v kódu ovladače mimo
  `blockproto::server` (např. špatné LBA v příkazu) ochranu obejde; proto
  je obsluha požadavků společná a ovladače dělají jen překlad na příkazy.
- Jmenné prostory NVMe s metadaty (ochrana dat T10 PI) ovladač vynechá
  a zaloguje.

## Velikost sektoru

Klient (`blockproto`) adresuje vždy po 512 B. Disk může mít větší logický
sektor: NVMe naformátované na 4 KiB, SATA 4Kn, virtio s `blk_size`.
Ovladač nahlásí skutečnou velikost sektoru (`Disks::sector_size`) a zbytek
dělá společný server:

- **GPT** se čte v sektorech disku: na disku 4 KiB leží hlavička na bajtu
  4096 a tabulka 16 KiB zabírá 4 sektory. Hranice oddílů jsou v sektorech
  disku. Rozbor se špatnou velikostí sektoru by GPT nenašel a disk by byl
  jen pro čtení (test `wrong_sector_size_never_writable`).
- **Zarovnaný přístup** (začátek i konec na hranici sektoru disku) jde
  přímo. FS pracuje po blocích 4 KiB, takže jeho čtení a zápisy jsou na
  disku 4 KiB vždy zarovnané.
- **Nezarovnané čtení** přečte celé sektory a vrátí jen požadovanou část.
- **Nezarovnaný zápis** přečte okrajové sektory, změní v nich požadovanou
  část a zapíše celé sektory (read-modify-write). Ostatní data v těchto
  sektorech zůstanou (test sousedních sektorů). Zápis jednoho sektoru disku
  je pro zařízení atomický, takže výpadek proudu uprostřed nepoškodí víc
  než ten sektor, který by se jinak zapisoval celý.
- **TRIM** se omezí na celé sektory disku uvnitř rozsahu; zbytek se
  přeskočí (TRIM je jen nápověda, data v částečně zasažených sektorech
  zůstanou).

Podporovaný sektor: mocnina dvou od 512 B do 32 KiB (velikost bufferu
ovladače). Prakticky 512 B a 4 KiB.

## Ovladač NVMe

Podle specifikace NVM Express Base 1.4. Ovladač je synchronní (jeden příkaz
najednou), proto stačí:

- reset řadiče (`CC.EN = 0`, čekání na `CSTS.RDY`), admin fronta,
  zapnutí (`CC`: stránka 4 KiB, položky 64/16 B),
- jedno přerušení MSI-X (vektor 0) pro admin i I/O frontu dokončených,
- Set Features „Number of Queues“ a jedna dvojice I/O front (16 položek),
- IDENTIFY řadiče (`MDTS`, `ONCS`, `VWC`, `NN`) a aktivních jmenných
  prostorů; každý jmenný prostor bez metadat je jeden disk (nejvýš 16);
  sektor podle formátu LBA (`LBADS`, typicky 512 B nebo 4 KiB, nejvýš
  `MDTS`). Seznam aktivních jmenných prostorů (IDENTIFY CNS 2) mají řadiče
  od NVMe 1.1; u NVMe 1.0 (např. virtuální NVMe starších verzí VMware)
  ovladač projde čísla 1..`NN` a neaktivní pozná podle nulové velikosti.

| požadavek `blockproto` | příkaz NVMe |
|---|---|
| `READ`, `WRITE` | Read 02h / Write 01h, data přes PRP (PRP1 + seznam PRP); rozdělí se podle `MDTS` |
| `FLUSH` | Flush 00h; bez volatilní mezipaměti (`VWC = 0`) není co ukládat a hlásí se `WRITE_THROUGH` |
| `DISCARD` | Dataset Management 09h s atributem Deallocate, až 256 úseků v jednom příkazu |
| `FEATURES` | `FLUSH`, `WRITE_THROUGH` (bez VWC), `DISCARD` (ONCS bit 2), `READ_ONLY` (write protect) |

Při ukončení ovladače (klienti zavřeli kanály) proběhne normální vypnutí
(`CC.SHN`), takže řadič uloží mezipaměť. Na každé cestě ukončení, i po
chybě při startu, ovladač řadič vypne (`CC.EN = 0`) dřív, než uvolní
fronty a buffery. Když řadič neodpoví do 10 s,
nebo vrátí položku s cizím číslem příkazu, ovladač ho vypne (`CC.EN = 0`,
přestane sahat do DMA paměti) a další požadavky vrací `IO_ERROR`.

## Ovladač AHCI

Podle specifikace Serial ATA AHCI 1.3.1:

1. převzetí řadiče od firmwaru (BIOS/OS handoff, `BOHC`), reset řadiče
   (`GHC.HR`) a režim AHCI (`GHC.AE`); řadič bez 64bitových adres
   (`CAP.S64A = 0`) dostane jen DMA paměť pod 4 GiB, port s pamětí nad
   4 GiB se vynechá (řadič by horní polovinu adresy zahodil a psal do
   cizí paměti),
2. přerušení MSI (řadiče AHCI obvykle nemají MSI-X); bez MSI ovladač
   obsluhuje řadič dotazováním,
3. pro každý implementovaný port (`PI`): zastavení, seznam příkazů, oblast
   pro přijaté FIS, roztočení (`SUD`); pak jedno společné čekání na linky
   (`DET = 3`, nejvýš 1 s, prázdný port se vzdá po 200 ms); porty bez
   linky dostanou COMRESET (`PxSCTL.DET`) a čeká se znovu, protože po
   resetu řadiče linka na některých čipsetech sama nenaskočí; pak
   připravený disk a podpis disku SATA (`0x101`; ATAPI se vynechá),
4. IDENTIFY DEVICE: kapacita LBA48, mezipaměť zápisu, FLUSH CACHE (EXT),
   TRIM (slovo 169), velikost logického sektoru (slova 106, 117-118;
   kapacita i počty sektorů v příkazech jsou v logických sektorech).

| požadavek `blockproto` | příkaz ATA |
|---|---|
| `READ`, `WRITE` | READ/WRITE DMA EXT (25h/35h), jedna položka PRDT |
| `FLUSH` | FLUSH CACHE EXT (EAh) nebo FLUSH CACHE (E7h); s vypnutou mezipamětí `WRITE_THROUGH` |
| `DISCARD` | DATA SET MANAGEMENT (06h) s TRIM, úseky po 8 B (LBA 48 b, počet 16 b) |

Disk se zapnutou mezipamětí, který neumí FLUSH, nehlásí žádnou záruku
trvanlivosti a FS se na něm připojí jen pro čtení. Po chybě příkazu (TFES)
ovladač port restartuje; port, který neodpoví do 20 s, zastaví. Disk,
který ovladač vynechá (neprojde IDENTIFY, neumí LBA48), má port zastavený,
než se uvolní jeho paměť. Při ukončení ovladače (i po chybě) se mezipaměť
zápisu disků uloží (FLUSH CACHE) a porty se zastaví.

## Hlášení disků a spuštění

`devmgr` ovladači pošle `pci <sběrnice> <zařízení> <funkce>` a `bar
<adresa> <velikost>` (NVMe BAR0, AHCI BAR5). Ovladač pro každý disk
zjistí, co klient uvidí, vytvoří kanál a pošle `block` s koncem kanálu,
nakonec `done`. `devmgr` zprávy `block` předá initu. Na ovladač čeká
nejvýš 150 s (reset řadiče NVMe smí podle `CAP.TO` trvat až 127,5 s,
roztočení disku SATA desítky sekund) a během čekání posílá initu každých
5 s `busy`, aby init (čeká na každou zprávu správce nejvýš 30 s) start
zařízení nevzdal.

Prostředky ovladačů (nejmenší práva):

| ovladač | přerušení | DMA |
|---|---|---|
| `nvme` | 2 | 1 MiB (fronty, seznam PRP, buffer 32 KiB) |
| `ahci` | 2 | 4 MiB (až 32 portů po nejvýš 64 KiB) |

## Testy a spuštění v QEMU

`cargo xtask test` přidá řadič NVMe a AHCI s testovacími disky
(`xtask/src/testdisks.rs`, GPT zapisuje stejný kód, jakým ji čte
ovladač):

| disk | řadič | klient vidí |
|---|---|---|
| `scratch` | virtio-blk | náš oddíl 1 MiB (testy virtio-blk) |
| `nvme-test` | NVMe | náš oddíl 2 MiB, ve 1. sektoru značka |
| `nvme4k-test` | NVMe se sektorem 4 KiB | GPT v sektorech 4 KiB, náš oddíl 4 MiB se značkou |
| `ahci-test` | AHCI port 0 | náš oddíl 3 MiB, ve 1. sektoru značka |
| `foreign` | AHCI port 1 | celý cizí disk GPT (oddíl Linux), jen čtení |

Sada `disky` ověří čtení, zápis (víc požadavků, poslední sektor oddílu,
nezarovnaný zápis bez změny sousedních sektorů), flush, TRIM (i částečný
sektor 4 KiB), meze oddílu, IOMMU a u cizího disku `READ_ONLY` pro zápis
i TRIM a nezměněná cizí data. Rozbor GPT testuje `cargo test -p diskguard`
(poškozená hlavička a tabulka, překryv, atribut jen pro čtení, MBR,
prázdný disk, sektor 4 KiB a 64 KiB, špatná velikost sektoru).
QEMU umí sektor 4 KiB jen u NVMe a virtio, ne u AHCI (IDE ho odmítne);
SATA 4Kn je proto ověřené jen společným kódem serveru a rozboru GPT.

Datový disk na jiném řadiči (dostane GPT s naším oddílem):

```bash
cargo xtask run --data-disk nvme
```

```bash
cargo xtask test --data-disk ahci
```

Celý FS na disku se sektorem 4 KiB:

```bash
cargo xtask test --data-disk nvme4k
```

Vlastní disk přes NVMe (disk bez našeho oddílu bude jen pro čtení):

```bash
cargo xtask run -- -drive if=none,id=nv,format=raw,file=disk.img -device nvme,drive=nv,serial=nv0
```

## VMware a skutečný počítač

- **VMware:** řadič disku NVMe nebo SATA (AHCI). Datový disk z QEMU
  (`OS_KEEP_DATA=1 cargo xtask run --data-disk nvme`) převede
  `qemu-img convert -O vmdk target/data-gpt-<pid>.img data.vmdk`.
- **Skutečný počítač:** náš oddíl vytvoří nástroj na oddíly s typem
  `C123ADA0-51BA-48C5-A536-C2CE6B2B3581` (např. `gdisk`, kód typu
  zadaný jako GUID), nebo s libovolným typem a jménem `MagnumOpus`.
  Formátování oddílu ze systému zatím není (TODO); obraz FS z nástroje
  [osfs](osfs.md) se na oddíl nahraje mimo systém.
- Ostatní disky počítače systém vidí, ale jen pro čtení (viz tabulka výše).
