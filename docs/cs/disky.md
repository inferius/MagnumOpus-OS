# Disky: NVMe, AHCI a ochrana cizích disků

Disky obsluhují tři ovladače v userspace. Všechny mluví s klientem
(souborový systém `fsd`, init, testy) stejným protokolem
[`blockproto`](souborovy-system.md):

| ovladač | zařízení | kde | spouští ho |
|---|---|---|---|
| `virtio-blk` | virtuální disk virtio 1.x | QEMU | `devmgr` podle ID zařízení virtio |
| `nvme` | řadič NVMe (PCI třída 01:08:02) | VMware, skutečné PC, QEMU `-device nvme` | `devmgr` podle třídy PCI |
| `ahci` | řadič SATA AHCI (PCI třída 01:06:01) | VMware (SATA), skutečné PC, QEMU `-device ahci` / q35 | `devmgr` podle třídy PCI |

Společná obsluha požadavků je v `blockproto::server`: kontrola rozsahu,
překlad na náš oddíl GPT a zákaz zápisu na cizí disk jsou na jednom místě
pro všechny tři ovladače. Ovladač dodá jen čtení, zápis, flush a TRIM
v sektorech zařízení (trait `Disks`).

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
| celý disk s **naším FS** bez tabulky oddílů (superblok `OSFS` v sektorech 8..72, sektor 0 bez podpisu MBR) | celý disk | povolen (datový disk v QEMU, starší rozložení) |
| GPT bez našeho oddílu | celý disk | **zakázán** (`READ_ONLY`) |
| náš oddíl je neplatný (mimo disk, mimo použitelnou oblast, **překrývá jiný oddíl**) | celý disk | **zakázán** |
| MBR (bez GPT), ochranný MBR s poškozenými hlavičkami GPT | celý disk | **zakázán** |
| prázdný nebo neznámý disk, chyba čtení | celý disk | **zakázán** |
| zařízení hlásí ochranu proti zápisu (NVMe write protect, virtio `RO`) | podle tabulky | **zakázán** |

Podrobnosti:

- **Ve výchozím stavu jsou cizí disky jen pro čtení.** Přepínač, který by
  zápis na cizí disk povolil, zatím neexistuje. Až bude potřeba (např.
  instalace na prázdný disk), musí jít o výslovné rozhodnutí uživatele pro
  jeden konkrétní disk, ne o globální volbu.
- GPT se ověřuje celá: podpis, CRC hlavičky i tabulky, poloha hlavičky,
  rozsah použitelné oblasti. Při poškozené primární hlavičce se použije
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
- Disky s logickým sektorem jiným než 512 B (NVMe naformátované na 4 KiB,
  SATA 4Kn) ovladače zatím vynechají a zalogují. Tabulka GPT na nich leží
  v jiných sektorech; bez úpravy by se rozbor mýlil.

## Ovladač NVMe

Podle specifikace NVM Express Base 1.4. Ovladač je synchronní (jeden příkaz
najednou), proto stačí:

- reset řadiče (`CC.EN = 0`, čekání na `CSTS.RDY`), admin fronta,
  zapnutí (`CC`: stránka 4 KiB, položky 64/16 B),
- jedno přerušení MSI-X (vektor 0) pro admin i I/O frontu dokončených,
- Set Features „Number of Queues“ a jedna dvojice I/O front (16 položek),
- IDENTIFY řadiče (`MDTS`, `ONCS`, `VWC`) a aktivních jmenných prostorů;
  každý jmenný prostor se sektorem 512 B bez metadat je jeden disk
  (nejvýš 16).

| požadavek `blockproto` | příkaz NVMe |
|---|---|
| `READ`, `WRITE` | Read 02h / Write 01h, data přes PRP (PRP1 + seznam PRP); rozdělí se podle `MDTS` |
| `FLUSH` | Flush 00h; bez volatilní mezipaměti (`VWC = 0`) není co ukládat a hlásí se `WRITE_THROUGH` |
| `DISCARD` | Dataset Management 09h s atributem Deallocate, až 256 úseků v jednom příkazu |
| `FEATURES` | `FLUSH`, `WRITE_THROUGH` (bez VWC), `DISCARD` (ONCS bit 2), `READ_ONLY` (write protect) |

Při ukončení ovladače (klienti zavřeli kanály) proběhne normální vypnutí
(`CC.SHN`), takže řadič uloží mezipaměť. Když řadič neodpoví do 10 s,
nebo vrátí položku s cizím číslem příkazu, ovladač ho vypne (`CC.EN = 0`,
přestane sahat do DMA paměti) a další požadavky vrací `IO_ERROR`.

## Ovladač AHCI

Podle specifikace Serial ATA AHCI 1.3.1:

1. převzetí řadiče od firmwaru (BIOS/OS handoff, `BOHC`), reset řadiče
   (`GHC.HR`) a režim AHCI (`GHC.AE`),
2. přerušení MSI (řadiče AHCI obvykle nemají MSI-X); bez MSI ovladač
   obsluhuje řadič dotazováním,
3. pro každý implementovaný port (`PI`): zastavení, seznam příkazů, oblast
   pro přijaté FIS, roztočení (`SUD`); pak jedno společné čekání na linky
   (`DET = 3`, nejvýš 1 s, prázdný port se vzdá po 200 ms), připravený
   disk, podpis disku SATA (`0x101`; ATAPI se vynechá),
4. IDENTIFY DEVICE: kapacita LBA48, mezipaměť zápisu, FLUSH CACHE (EXT),
   TRIM (slovo 169), velikost logického sektoru (slovo 106).

| požadavek `blockproto` | příkaz ATA |
|---|---|
| `READ`, `WRITE` | READ/WRITE DMA EXT (25h/35h), jedna položka PRDT |
| `FLUSH` | FLUSH CACHE EXT (EAh) nebo FLUSH CACHE (E7h); s vypnutou mezipamětí `WRITE_THROUGH` |
| `DISCARD` | DATA SET MANAGEMENT (06h) s TRIM, úseky po 8 B (LBA 48 b, počet 16 b) |

Disk se zapnutou mezipamětí, který neumí FLUSH, nehlásí žádnou záruku
trvanlivosti a FS se na něm připojí jen pro čtení. Po chybě příkazu (TFES)
ovladač port restartuje; port, který neodpoví do 20 s, zastaví.

## Hlášení disků a spuštění

`devmgr` ovladači pošle `pci <sběrnice> <zařízení> <funkce>` a `bar
<adresa> <velikost>` (NVMe BAR0, AHCI BAR5). Ovladač pro každý disk
zjistí, co klient uvidí, vytvoří kanál a pošle `block` s koncem kanálu,
nakonec `done`. `devmgr` čeká nejvýš 20 s a zprávy `block` předá initu.

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
| `ahci-test` | AHCI port 0 | náš oddíl 3 MiB, ve 1. sektoru značka |
| `foreign` | AHCI port 1 | celý cizí disk GPT (oddíl Linux), jen čtení |

Sada `disky` ověří čtení, zápis (víc požadavků, poslední sektor oddílu),
flush, TRIM, meze oddílu, IOMMU a u cizího disku `READ_ONLY` pro zápis
i TRIM a nezměněná cizí data. Rozbor GPT testuje `cargo test -p diskguard`
(poškozená hlavička a tabulka, překryv, atribut jen pro čtení, MBR,
prázdný disk).

Datový disk na jiném řadiči (dostane GPT s naším oddílem):

```bash
cargo xtask run --data-disk nvme
```

```bash
cargo xtask test --data-disk ahci
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
