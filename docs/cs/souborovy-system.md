# Souborový systém

Vlastní souborový systém (pracovně „OSFS“, formát v3). Kód je v knihovně
`libs/fs` (`no_std`, stejná na hostiteli i v systému), službu nad diskem
poskytuje `user/fsd`, protokol mezi klientem a službou definuje
`libs/fsproto`.

Cíle návrhu: **nikdy neztratit konzistenci** (ani při výpadku proudu nebo
lhajícím disku), **odhalit každé poškození dat**, **být rychlý** (souvislé
zápisy, levný `fsync`) a **nezabírat místo zbytečně** (režie metadat ~0,4 %).

> Popis platí pro formát v3. Co se změnilo proti v2 a proč, shrnují
> [Verze formátu](fs-verze.md) a rámečky „Změna proti v2“ u jednotlivých
> kapitol; starý popis je v [archivu v2](fs-v2.md).
>
> Formát v3 se zavádí po fázích podle [návrhu FS v3](navrh-fs-v3.md). Hotová
> je fáze 1 (formát, datasety a klony, handly, oprávnění, protokol v2) a
> **fáze 2 - historie**: žurnál změn, koš, vracení operací, verze souborů,
> automatické snapshoty, commity s logickým ID, větve a merge, „zkusit
> změny bokem“, rozšířené atributy a expirace - popis v samostatné kapitole
> [Historie FS](fs-historie.md) - a **fáze 3 - data**: komprese, okamžité
> kopie, deduplikace, automatické díry, metadata 2x se samoléčením, data
> 2x, scrub na pozadí a předalokace - kapitola [Data FS](fs-data.md).
> Svazek formátu v2 se nepřipojí
> (`NOT_SUPPORTED`); data se zatím nepřevádějí, disky z nástroje `xtask` se
> vytvářejí znovu.

## Co formát obsahuje a co ovladač používá

Formát v3 má místo i pro věci, které ovladač zatím neumí. Aby nebylo
sporné, co funguje:

| Položka | Ve formátu | Ovladač dnes |
|---|---|---|
| UUID svazku, příznaky vlastností | ano | **používá**; definované bity `INCOMPAT_TIME_SECS` ([časy](#časy-objektů)), `INCOMPAT_PTR_COPY`, `INCOMPAT_DATA` ([data](fs-data.md#na-disku)), `RO_COMPAT_ACL`, `RO_COMPAT_HISTORY` ([historie](fs-historie.md#na-disku)) |
| číslo zařízení v `BlockPtr`, `device_index/count` v superbloku | ano | **jen jedno zařízení**: zařízení musí být 0 a počet 1, jinak `CORRUPTED`; víc disků (zrcadlo, RAID) až po fázi 5 |
| druhá kopie v `BlockPtr` (metadata 2x) | ano | **používá** ([samoléčení](fs-data.md#metadata-2x-a-samoléčení)) |
| příznaky `BlockPtr` (komprese, šifrování, sdílení, cloud) | vyhrazené | **nepoužívá** pro uzly (uzel s příznakem = `NOT_SUPPORTED`); komprese a sdílení dat jsou v extentu |
| příznaky extentu (nezapsaný, sdílený, kopie, komprese) | ano | **používá** ([extent](fs-data.md#extent)) |
| příznak objektu `INODE_REMOTE` | vyhrazený | **nepoužívá** (fáze 6) |
| oprávnění (`acl`), vlastník | ano | **používá** ([Oprávnění](#oprávnění-a-identita-relací)) |
| rozšířené atributy (`XATTR`), počet v záznamu objektu | ano | **používá** ([atributy](fs-historie.md#rozšířené-atributy-a-expirace)) |
| žurnál změn, koš, metadata snapshotů, commity, větve, index expirace | ano | **používá** ([Historie FS](fs-historie.md)) |
| čas přístupu (`atime`) | ano | **jen nastavením** (`set_times`); čtení ho nemění |
| generace objektu, čas vytvoření | ano | **používá** (`Stat`, přehrání logu) |
| symbolické odkazy, jména do 1024 znaků, limit velikosti | ano | **používá** |
| datasety, klony s vazbou na původní snapshot | ano | **používá** ([Datasety](#datasety-a-klony)) |
| diff verzí, obnova jako kopie | ano | **používá** ([Rozdíl verzí](#rozdíl-verzí-a-obnova-jako-kopie)) |
| handly, sirotci, sdílecí režimy, zámky rozsahu | ano | **používá** ([Handly](#otevřené-soubory-handly)) |
| protokol v2 ve fsd (relace, handly, podmíněné uložení) | ano | **používá** ([Protokol v2](#protokol-v2-relace-handly-podmíněné-uložení)) |

## Co nabízí

| Vlastnost | Stav | Kde v dokumentu |
|---|---|---|
| Copy-on-write: nic živého se nepřepisuje na místě | hotovo | [Commit](#commit-krok-za-krokem) |
| Atomický commit, konzistence po výpadku kdykoli | hotovo | [Výpadek proudu](#co-se-stane-při-výpadku-proudu) |
| Merkleův strom: 128bitový součet v každém ukazateli | hotovo | [Kontrolní součty](#kontrolní-součty-a-merkleův-strom) |
| Volba součtu XXH3-128 / BLAKE3 | hotovo | [Kontrolní součty](#kontrolní-součty-a-merkleův-strom) |
| Návrat o 1-3 generace, když je nejnovější stav poškozený | hotovo | [Návrat o generaci](#připojení-a-návrat-o-generaci) |
| Rychlý `fsync` přes intent log (jeden zápis + flush) | hotovo | [fsync a intent log](#fsync-a-intent-log) |
| Extenty, odložená alokace, souvislé zápisy | hotovo | [Zápis dat](#zápis-dat-odložená-alokace-a-extenty) |
| Alokační skupiny se space logy (bez bitmapy) | hotovo | [Správa místa](#správa-volného-místa) |
| TRIM/DISCARD pro SSD | hotovo | [TRIM](#trim) |
| Defragmentace na pozadí | hotovo | [Fragmentace](#fragmentace-a-defragmentace) |
| Okamžitá velikost adresáře (souhrny podstromu) | hotovo | [Souhrny](#okamžité-velikosti-adresářů) |
| Jména bez ohledu na velikost písmen (Unicode NFC) | hotovo | [Jména](#jména-souborů) |
| Jména až 1024 znaků, zákaz řídicích znaků směru textu | hotovo | [Jména](#jména-souborů) |
| Symbolické odkazy | hotovo (knihovna, fsd následuje ve jmenném prostoru relace) | [Symbolické odkazy](#symbolické-odkazy) |
| Soubory až 2^63-1 B, kontrolovaná aritmetika | hotovo | [Limity](#limity-formátu) |
| UUID svazku a příznaky vlastností (compat/ro/incompat) | hotovo | [Superblok](#superblok) |
| Snapshoty jen pro čtení a jejich mazání (deadlisty) | hotovo | [Snapshoty](#snapshoty) |
| Datasety (složky s vlastní historií), zapisovatelné klony | hotovo | [Datasety](#datasety-a-klony) |
| Kontrola konzistence (fsck) a ověření dat (scrub) | hotovo | [Kontrola](#kontrola-konzistence-a-scrub) |
| Žurnál změn s pořadovými čísly, sledování změn (`JOURNAL_WATCH`), pohled s pozicí | hotovo | [Historie: žurnál](fs-historie.md#žurnál-změn) |
| Koš, vracení operací, verze souborů, automatické snapshoty | hotovo | [Historie](fs-historie.md#koš) |
| Commity s logickým ID, větve, trojcestný merge, „zkusit změny bokem“ | hotovo | [Historie: commity](fs-historie.md#commity-a-logické-id) |
| Rozšířené atributy, expirace souborů | hotovo | [Historie: atributy](fs-historie.md#rozšířené-atributy-a-expirace) |
| Komprese po kusech (LZ4, Zstd; výchozí vypnuto) | hotovo | [Data: komprese](fs-data.md#komprese) |
| Okamžité kopie souborů, rozsahů a složek (sdílené bloky) | hotovo | [Data: okamžité kopie](fs-data.md#okamžité-kopie-reflink) |
| Deduplikace na pozadí a při zápisu | hotovo | [Data: deduplikace](fs-data.md#deduplikace) |
| Automatické díry, předalokace se zápisem na místo | hotovo | [Data: předalokace](fs-data.md#předalokace) |
| Metadata 2x se samoléčením, volitelně data 2x, scrub na pozadí | hotovo | [Data: samoléčení](fs-data.md#metadata-2x-a-samoléčení) |
| Vrstva pro git, indexy, vyhledávání | plán | [Verzování a git](#verzování-a-git) |

## Rychlý příklad

V programu v systému (přes službu fsd):

```rust
use fsproto::{FsClient, kind};

fn priklad(fs: &FsClient) -> libos::Result<()> {
    fs.create("/Dokumenty", kind::DIR)?;
    fs.create("/dokumenty/Poznámky.txt", kind::FILE)?; // jména bez ohledu na velikost písmen
    fs.write("/DOKUMENTY/poznámky.txt", 0, "Ahoj světe\n".as_bytes())?;
    fs.fsync()?; // od teď přežije výpadek proudu

    let st = fs.stat("/dokumenty")?;
    // Velikost celého podstromu je hned k dispozici, bez procházení:
    libos::println!("{} B v {} souborech", st.tree_bytes, st.tree_files);

    fs.snapshot("pred-upravou")?; // verze jen pro čtení
    fs.truncate("/dokumenty/poznámky.txt", 0)?;
    let stara = fs.snapshot_read("pred-upravou", "/dokumenty/poznámky.txt", 0, 100)?;
    assert_eq!(stara, "Ahoj světe\n".as_bytes());
    Ok(())
}
```

Přímo nad zařízením (nástroje na hostiteli, testy):

```rust
use fs::{Algo, FileDevice, FormatOptions, Fs, Kind};

let dev = FileDevice::open("data.img".as_ref())?;
let opts = FormatOptions { label: "data".into(), algo: Algo::Xxh3, ag_blocks: None, uuid: None };
let dev = Fs::format(dev, &opts)?;          // nový prázdný systém
let mut fs = Fs::mount(dev)?;               // připojení (+ přehrání intent logu)
fs.create("/vitejte.txt", Kind::File)?;
fs.write("/vitejte.txt", 0, b"Ahoj!\n")?;
fs.symlink("/uvod", "/vitejte.txt")?;      // symbolický odkaz (cíl je text)
assert_eq!(fs.readlink("/uvod")?, "/vitejte.txt");
fs.commit()?;                               // atomicky uloží vše
let report = fs.check(true)?;               // fsck + scrub
assert!(report.problems.is_empty());
```

## Pojmy

| Pojem | Význam |
|---|---|
| **blok** | 4 KiB (`BLOCK_SIZE`), jednotka alokace a kontrolního součtu. Na disku se sektory 512 B je blok = 8 sektorů. |
| **transakce (txg)** | Všechny změny od posledního commitu. Mají číslo `txg = generace posledního commitu + 1`. |
| **generace** | Číslo uloženého stavu (commitu). Roste o 1. |
| **birth** | Generace, ve které blok vznikl. Rozhoduje, kdy smí být blok uvolněn. |
| **uzel** | Uzel B+stromu metadat: 16 KiB = 4 souvislé bloky (`NODE_BLOCKS`). |
| **ukazatel (`BlockPtr`)** | (zařízení, číslo bloku, délka, birth, 128bitový kontrolní součet obsahu, příznaky). |
| **objekt (oid)** | Soubor, adresář nebo symbolický odkaz; číslo `oid`. Kořen má `oid` 1, metadata systému 0. |
| **extent** | Souvislý úsek dat souboru: fyzický začátek, délka (nejvýš 496 bloků ~ 2 MiB), součet každého bloku. |
| **alokační skupina (AG)** | Úsek disku (výchozí 1 GiB) s vlastním záznamem volného místa. |

## Rozložení na disku

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Rozlozeni disku">
<rect class="box" x="10" y="40" width="60" height="60" rx="6"/><text class="t" x="40" y="66" text-anchor="middle">blok 0</text><text class="m" x="40" y="84" text-anchor="middle">volný</text>
<rect class="acc" x="76" y="40" width="190" height="60" rx="6"/><text class="t b" x="171" y="64" text-anchor="middle">bloky 1-8: kruh superbloků</text><text class="m" x="171" y="84" text-anchor="middle">commit g → slot 1 + g mod 8</text>
<rect class="box" x="272" y="40" width="478" height="60" rx="6"/><text class="t b" x="511" y="64" text-anchor="middle">bloky 9 … konec: alokovaný prostor</text><text class="m" x="511" y="84" text-anchor="middle">uzly stromu, data, space logy, tabulka AG, intent log</text>
<rect class="ok" x="272" y="120" width="150" height="34" rx="6"/><text class="t" x="347" y="142" text-anchor="middle">metadata: od začátku</text>
<rect class="warn" x="430" y="120" width="320" height="34" rx="6"/><text class="t" x="590" y="142" text-anchor="middle">data: best-fit, od středu disku, zarovnaná na 1 MiB</text>
<text class="m" x="10" y="25">Všechna čísla little-endian. Pevné místo mají jen superbloky; vše ostatní najde strom ukazatelů.</text>
</svg>
<figcaption>Disk: pevné jsou jen bloky 0-8, zbytek přiděluje alokátor.</figcaption>
</figure>

### Superblok

> **Změna proti v2:** přibylo UUID svazku, příznaky vlastností (compat/ro_compat/incompat) a zařízení svazku; ukazatele mají 40 B. **Proč:** formát jde rozšiřovat bez nové verze a starší ovladač bezpečně pozná, co neumí. ([všechny změny](fs-verze.md#přechod-v2---v3))

Superblok má **vlastní** kontrolní součet XXH3-128 v posledních 16 bajtech
bloku (musí jít ověřit dřív, než je známý zvolený algoritmus). Platný je ten
s nejvyšší generací, jehož stav jde načíst.

| Offset | Velikost | Pole | Význam |
|---|---|---|---|
| 0 | 8 | magic | `OSFS\0\0v3` |
| 8 | 4 | version | 3 |
| 12 | 1 | algo | 1 = XXH3-128, 2 = BLAKE3 |
| 16 | 8 | generation | číslo commitu |
| 24 | 8 | block_count | velikost svazku v blocích |
| 32 | 8 | ag_blocks | velikost alokační skupiny (64 bloků až 2^20 skupin) |
| 40 | 16 | uuid | jednoznačné ID svazku |
| 56 | 8 | features_compat | vlastnosti, se kterými starší ovladač smí normálně pracovat |
| 64 | 8 | features_ro_compat | vlastnosti, se kterými starší ovladač smí jen číst |
| 72 | 8 | features_incompat | vlastnosti, bez jejichž znalosti se svazek nesmí připojit |
| 80 | 48 | root | `BlockPtr` kořene B+stromu |
| 128 | 48 | space | `BlockPtr` indexu tabulky AG |
| 176 | 8 | zil_block | první blok intent logu této epochy |
| 184 | 8 | zil_seed | semínko součtů intent logu |
| 192 | 8 | next_oid | další volné číslo objektu |
| 200 | 8 | snapshot_generation | generace posledního snapshotu |
| 208 | 8 | next_snapshot | číslo dalšího snapshotu (= seznam mrtvých bloků živé verze) |
| 216 | 8 | dead_seq | pořadí další položky deadlistu |
| 224 | 4 | unicode_version | verze tabulek Unicode pro jména |
| 228 | 2 | device_index | toto zařízení ve svazku (zatím 0) |
| 230 | 2 | device_count | počet zařízení svazku (zatím 1) |
| 232 | 32 | label | jméno svazku (UTF-8, doplněné nulami) |
| 4080 | 16 | csum | XXH3-128 bajtů 0..4080 |

**Příznaky vlastností** fungují jako v ext4: každý bit = jedna vlastnost
formátu (komprese, šifrování, ...). Ovladač, který bit z `ro_compat` nebo
`incompat` nezná, svazek odmítne s `NOT_SUPPORTED` - nikdy ho tak nepoškodí
tím, že by nerozuměl novým strukturám.

| Bit | Skupina | Jméno | Význam |
|---|---|---|---|
| 0 | incompat | `INCOMPAT_REMOTE` | vyhrazený: data jen v cloudu (fáze 6), ovladač ho zatím odmítne |
| 1 | incompat | `INCOMPAT_TIME_SECS` | časy jsou sekundy i64 + nanosekundy u32 ([časy](#časy-objektů)); nové svazky ho mají vždy |
| 2 | incompat | `INCOMPAT_PTR_COPY` | ukazatel 48 B s adresou druhé kopie ([data](fs-data.md#ukazatel-48-b-a-superblok)); **povinný** - svazek bez něj (ukazatel 40 B) se odmítne `NOT_SUPPORTED` |
| 3 | incompat | `INCOMPAT_DATA` | extenty fáze 3: komprese, sdílené bloky, nezapsané, s kopií ([data](fs-data.md#extent)) |
| 0 | ro_compat | `RO_COMPAT_ACL` | svazek má popisy oprávnění s počty odkazů ([oprávnění](#oprávnění-a-identita-relací)); starší ovladač jen čte |

**UUID** se při formátování bere z `FormatOptions::uuid`; nástroje na
hostiteli (`xtask`) předávají náhodné. Bez něj se odvodí ze jména, velikosti
a adresy na zásobníku (v `no_std` není zdroj náhody) - jednoznačnost pak není
zaručená. Superblok s `ag_blocks < 64`, více než 2^20 skupinami nebo nulovým
počtem zařízení je `CORRUPTED`.

## Kontrolní součty a Merkleův strom

Každý blok kromě superbloku a bloků intent logu ověřuje **součet uložený
v ukazateli rodiče**, ne v bloku samotném. Vzniká Merkleův strom se
superblokem na vrcholu:

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Merkleuv strom">
<defs><marker id="mk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="290" y="10" width="180" height="46" rx="6"/><text class="t b" x="380" y="30" text-anchor="middle">superblok</text><text class="m mono" x="380" y="47" text-anchor="middle">root = (blok, birth, csum)</text>
<path class="ln" d="M380 56 L380 84" marker-end="url(#mk1)"/>
<rect class="box" x="280" y="86" width="200" height="46" rx="6"/><text class="t b" x="380" y="106" text-anchor="middle">vnitřní uzel</text><text class="m mono" x="380" y="123" text-anchor="middle">[(klíč, BlockPtr)] × až 72</text>
<path class="ln" d="M330 132 L190 166" marker-end="url(#mk1)"/><path class="ln" d="M430 132 L570 166" marker-end="url(#mk1)"/>
<rect class="box" x="90" y="168" width="200" height="46" rx="6"/><text class="t b" x="190" y="188" text-anchor="middle">list</text><text class="m" x="190" y="205" text-anchor="middle">inode, položky adresáře</text>
<rect class="box" x="470" y="168" width="200" height="46" rx="6"/><text class="t b" x="570" y="188" text-anchor="middle">list</text><text class="m" x="570" y="205" text-anchor="middle">EXTENT: phys, birth, csum[n]</text>
<path class="ln" d="M520 214 L450 248" marker-end="url(#mk1)"/><path class="ln" d="M570 214 L570 248" marker-end="url(#mk1)"/><path class="ln" d="M620 214 L690 248" marker-end="url(#mk1)"/>
<rect class="ok" x="400" y="250" width="100" height="36" rx="6"/><text class="t" x="450" y="273" text-anchor="middle">data 4 KiB</text>
<rect class="ok" x="520" y="250" width="100" height="36" rx="6"/><text class="t" x="570" y="273" text-anchor="middle">data 4 KiB</text>
<rect class="ok" x="640" y="250" width="100" height="36" rx="6"/><text class="t" x="690" y="273" text-anchor="middle">data 4 KiB</text>
<text class="m" x="10" y="250">Každá šipka = ukazatel se součtem cíle.</text><text class="m" x="10" y="268">Změna kteréhokoli bajtu změní součty</text><text class="m" x="10" y="286">až ke kořeni.</text>
</svg>
<figcaption>Merkleův strom: superblok přes ukazatele se součty ověřuje celý svazek.</figcaption>
</figure>

Proč v rodiči: když disk ohlásí zápis, ale neprovede ho („ztracený zápis“),
zůstane na místě starý, sám o sobě platný blok. Součet uvnitř bloku by
prošel; součet v rodiči ne. Stejně se odhalí zápis na špatnou adresu.

### Algoritmy

Volí se při formátování (`FormatOptions::algo`), platí pro celý svazek.

| | XXH3-128 (`Algo::Xxh3`, výchozí) | BLAKE3-128 (`Algo::Blake3`) |
|---|---|---|
| Rychlost na x86 (jedno jádro) | ~30-60 GB/s (AVX2) | ~2-7 GB/s |
| Nezachycená náhodná chyba | 1 : 2^128 | 1 : 2^128 |
| Odolá úmyslné kolizi | ne | ano |
| Použití | běžné svazky | deduplikace, podepsané systémové snapshoty |

Proč ne CRC32C (formát v1): 32 bitů znamená 1 nezachycenou chybu ze
4 miliard; 10TB disk má 2,5 miliardy bloků. Pro srovnání: btrfs umí
crc32c/xxhash64/sha256/blake2b, ZFS fletcher4 a sha256/blake3, APFS
kontroluje jen metadata.

Hashe zatím poskytují převzaté knihovny `xxhash-rust` a `blake3` (čisté
Rust, `no_std`). Poznámka: `blake3` 1.8 hlásí varování o budoucí
nekompatibilitě s novějším Rustem - řešit aktualizací nebo vlastní
implementací.

### Ukazatel (`BlockPtr`, 48 B)

> **Změna proti v3 (fáze 2):** z 40 B na 48 B: adresa druhé kopie
> (metadata 2x). **Proč:** poškozený uzel se opraví z kopie bez ztráty dat
> (samoléčení); kopie leží jinde na disku. Starý tvar ovladač odmítne.
> ([Data FS](fs-data.md#metadata-2x-a-samoléčení))

> **Změna proti v2:** z 32 B na 40 B: číslo zařízení (16 bitů), délka v blocích a příznaky. **Proč:** uzel má 4 bloky a formát je připravený na víc disků a na kompresi, šifrování a sdílené bloky (poznají se přímo na ukazateli). **Ovladač zatím** umí jen jedno zařízení (číslo 0) a žádný příznak - viz [Co formát obsahuje](#co-formát-obsahuje-a-co-ovladač-používá). ([všechny změny](fs-verze.md#přechod-v2---v3))

| Offset | Velikost | Pole |
|---|---|---|
| 0 | 8 | adresa: číslo bloku (bity 0-47) \| zařízení svazku (bity 48-63) |
| 8 | 8 | birth (generace vzniku) |
| 16 | 16 | kontrolní součet obsahu (všech bloků, na které ukazuje) |
| 32 | 4 | délka v blocích (uzel stromu = 4) |
| 36 | 2 | příznaky: 1 = komprimováno, 2 = šifrováno, 4 = sdílené bloky, 8 = jen v cloudu (rezervované; uzel s příznakem se odmítne `NOT_SUPPORTED`) |
| 38 | 2 | rezerva (nuly) |
| 40 | 8 | adresa druhé kopie (tvar jako adresa; 0 = bez kopie); součet platí pro obě |

48 bitů čísla bloku = až 1 EiB na zařízení; číslo zařízení je ve formátu
připravené pro svazky přes víc disků, **ovladač zatím přijme jen zařízení 0**.
Ukazatel na uzel musí mít délku přesně 4, zařízení 0
a celý rozsah uvnitř svazku, jinak je uzel `CORRUPTED` (načte se dřív, než se ověří
součet).

## B+strom metadat

Všechna metadata leží v **jednom** copy-on-write B+stromu. Klíč je
`(oid u64, druh u8, offset u64)`, řadí se v tomto pořadí - vše o jednom
objektu tak leží u sebe (jedno čtení listu dá inode i malý obsah).
Uzel má 16 KiB: list pojme kolem 120 záznamů objektů a vnitřní uzel 255
potomků, takže strom se 100 miliony položek má jen 4 úrovně.

| Druh | Hodnota | Offset klíče | Hodnota položky |
|---|---|---|---|
| `INODE` | 1 | 0 | záznam objektu, 128 B (viz níže) |
| `DIRENT` | 2 | FNV-1a hash složeného jména | seznam položek se stejným hashem: (oid u64, druh u8, délka jména u16, jméno) |
| `INLINE` | 3 | 0 | celý obsah souboru do 1984 B (`INLINE_MAX`) |
| `EXTENT` | 4 | první logický blok | phys u64, počet u32 (1-496), příznaky u32 (1 = předalokovaný, rezervované; zatím musí být 0), birth u64, součty 16 B × počet |
| `SNAPSHOT` | 5 | číslo snapshotu (objekt 0) | `BlockPtr` kořene, generace u64, počet klonů u32, příznaky u32 (1 = skrytý), jméno |
| `DEADLIST` | 6 | seznam << 32 \| pořadí (objekt 0) | až 80 mrtvých úseků (start u64, délka u64, birth u64) |
| `SYMLINK` | 7 | 0 | cíl symbolického odkazu (UTF-8, 1-4096 B) |
| `ACL` | 10 | číslo popisu (objekt 0) | popis oprávnění s počtem odkazů ([oprávnění](#na-disku)) |
| `ACL_INDEX` | 11 | otisk popisu (objekt 0) | čísla popisů u64 se stejným otiskem |
| `PROPS` | 12 | 0 (objekt 0) | vlastnosti datasetu: příznaky u32 (1 = zákaz spouštění) |
| `ORPHAN` | 9 | oid sirotka (objekt 0) | prázdná - objekt bez jména, který je otevřený; smaže ho poslední `close` nebo připojení |
| `DATASET` | 8 | číslo datasetu (objekt 0, jen v kořenovém datasetu) | `DatasetRecord` 100 B: kořen, další oid, další snapshot, generace posl. snapshotu, pořadí deadlistu, původ klonu (dataset, snapshot, generace), příznaky |

### Záznam objektu (inode, 128 B)

> **Změna proti v2:** z 64 B na 128 B: čas vytvoření a přístupu, generace, oprávnění, vlastník, rozšířené atributy, příznaky. **Proč:** oid + generace je stabilní identita pro handly; pro oprávnění a atributy už nebude potřeba měnit formát. ([všechny změny](fs-verze.md#přechod-v2---v3))

| Offset | Pole | Význam |
|---|---|---|
| 0 | kind u8 | 1 = soubor, 2 = adresář, 3 = symbolický odkaz, 5 = uzel socketu `AF_UNIX` (bez dat) |
| 1 | flags u8 | příznaky objektu, rezervované: 1 = data jen v cloudu (zatím 0; nastavení synchronizace jsou atributy `sync.*`, ne bity) |
| 4 | acl u32 | číslo popisu oprávnění (0 = zdědit; přijde ve fázi 1d) |
| 8 | size | soubor: velikost v bajtech (nejvýš 2^63-1); odkaz: délka cíle |
| 16 | mtime | čas poslední změny obsahu: sekundy i64 od 1970 UTC |
| 24 | ctime | čas změny metadat: sekundy i64 |
| 32 | btime | čas vytvoření: sekundy i64 |
| 40 | atime | čas posledního přístupu: sekundy i64 (0 a 0 ns = nesleduje se) |
| 48 | parent | oid rodičovského adresáře (každý objekt má právě jednoho rodiče) |
| 56 | generation | generace (txg), ve které objekt vznikl |
| 64 | tree_bytes | adresář: součet velikostí souborů v celém podstromu (saturuje) |
| 72 | tree_files | adresář: počet souborů a odkazů v podstromu |
| 80 | tree_dirs | adresář: počet adresářů v podstromu (bez sebe) |
| 88 | owner | vlastník (0 = systém; přijde se správou uživatelů) |
| 96 | xattrs u32 | počet rozšířených atributů |
| 100 | - | rezerva u32 (nula) |
| 104 | version | verze obsahu (počítadlo změn; revize pro podmíněné uložení) |
| 112 | mtime_ns u32 | nanosekundy k `mtime` (0..10⁹) |
| 116 | ctime_ns u32 | nanosekundy k `ctime` |
| 120 | btime_ns u32 | nanosekundy k `btime` |
| 124 | atime_ns u32 | nanosekundy k `atime` |

#### Časy objektů

> **Změna formátu času (příznak `INCOMPAT_TIME_SECS`):** dřív byl každý čas
> u64 nanosekund od 1970 na offsetech 16-40, teď jsou tam sekundy i64 se
> znaménkem a nanosekundy jsou v dřívější rezervě 112-128. **Proč:** u64
> nanosekund neuměl data před rokem 1970 (staré archivy, NTFS počítá od
> roku 1601) a končil v roce 2262. Nový tvar má rozsah ±292 miliard let,
> stejný jako `timespec` v POSIX a `SystemTime` v Rustu, a vejde se do
> 128 B záznamu bez změny jeho velikosti. ([verze](fs-verze.md#časy-objektů-u64-ns---i64-s--u32-ns))

- **Nové svazky** mají bit `INCOMPAT_TIME_SECS`. Starší ovladač, který ho
  nezná, je odmítne (`NOT_SUPPORTED`), takže nikdy nepřečte sekundy jako
  nanosekundy.
- **Svazky ze starší verze** (bez bitu) tento ovladač připojí normálně
  a časy v nich čte i zapisuje dál ve starém tvaru u64 ns (čas před 1970
  se tam ořízne na 1970). Převod na nový tvar zatím není (bylo by nutné
  přepsat všechny záznamy objektů).
- **Nanosekundy ≥ 10⁹** = poškozený záznam (`CORRUPTED`); stejně tak v intent
  logu.
- **Zdroj času** je `Fs::set_time(Timestamp)`. Služba fsd bere reálný čas
  ([Reálný čas](cas.md)), nástroje na hostiteli čas hostitele.
- **Nastavení časů** `Fs::set_times(cesta, SetTimes)` a `set_times_h(handle, ...)`
  (handle s právem zápisu) mění `mtime`, `btime`, `atime` (`None` = nechat).
  `ctime` nastavit nejde: nastaví se na aktuální čas, protože zaznamenává, že
  se objekt měnil. Změna se zapíše do intent logu (záznam 9), takže přežije
  `fsync` a pád.

```rust
use fs::{SetTimes, Timestamp};

// Kopie ze starého archivu se zachovaným časem (1. 1. 1601 i před 1970).
fs.set_times("/archiv/dopis.txt", SetTimes {
    mtime: Some(Timestamp { secs: -11_644_473_600, nanos: 0 }),
    btime: Some(Timestamp { secs: -86_400, nanos: 500 }),
    atime: None,
})?;
```

**Generace**
spolu s `oid` určuje objekt jednoznačně: oid smazaného objektu se nepoužije
znovu, ale handly protokolu v2 se podle generace přesto ověří. Záznam
s neznámým druhem nebo velikostí nad 2^63-1 je `CORRUPTED`.

### Uzel stromu (16 KiB)

> **Změna proti v2:** z 4 KiB na 16 KiB a úroveň se ukládá a kontroluje. **Proč:** jméno do 4 KiB i extent s 496 součty se vejdou do položky, strom je mělčí a poškozený ukazatel nevytvoří smyčku. ([všechny změny](fs-verze.md#přechod-v2---v3))

Uzel zabírá 4 souvislé bloky (`NODE_BLOCKS`), alokuje se a zapisuje vcelku.

```text
0   magic "BT", úroveň u8 (0 = list, nejvýš 16), rezerva, počet položek u16
8   generace vzniku u64
32  položky:
      list (32 B):     klíč 24 B, offset hodnoty u16, délka u16, rezerva
      vnitřní (72 B):  klíč 24 B, BlockPtr potomka 48 B
    hodnoty listu leží od konce uzlu směrem dolů
```

Limity: hodnota nejvýš 8144 B (`MAX_VALUE`, zaručuje, že přeplněný list
jde vždy rozdělit), vnitřní uzel nejvýš 227 potomků. Úroveň je uložená
v každém uzlu a při každém sestupu se kontroluje (potomek musí mít úroveň
o 1 nižší) - poškozený ukazatel tak nemůže udělat ze stromu smyčku ani
podstrčit list místo vnitřního uzlu.

Proč 16 KiB: jméno do 4 KiB i extent s 496 součty se vejdou do jedné
položky, strom je o úroveň mělčí (méně čtení při hledání) a režie hlaviček
klesne. Cena: změna jedné položky přepíše 16 KiB místo 4 KiB, což copy-on-write
rozloží do souvislých zápisů na konci commitu.

- **Dělení listu:** při přidávání na konec (sekvenční zápis) jde nová
  položka sama doprava a levý list zůstane plný. Jinak se dělí na co
  nejvyrovnanější poloviny. Bez toho by postupné plnění nechávalo listy
  z poloviny prázdné (naměřeno: režie 0,85 % místo 0,43 %).
- **Mazání:** prázdné uzly se odstraní, kořen s jedním potomkem se nahradí
  potomkem, sousední podplněné listy (levý <= 1/4 uzlu, oba <= 1/2) se
  sloučí.

## Jména souborů

> **Změna proti v2:** z 255 B na 1024 znaků (4 KiB UTF-8), zakázané i řídicí znaky směru textu. **Proč:** dlouhá jména z jiných systémů a ochrana proti podvržené příponě. ([všechny změny](fs-verze.md#přechod-v2---v3))

- Uloží se tak, jak je uživatel zadal (v Unicode NFC) - velikost písmen
  zůstává.
- Porovnávají se **bez ohledu na velikost písmen** v celém systému: klíč
  = NFC(malá písmena(NFC(jméno))). `Foto.JPG`, `foto.jpg` i `fóto`
  zapsané rozloženě (`o` + čárka) jsou stejné jméno. Převod nezávisí na
  jazyku (žádná turecká pravidla pro `i`).
- Platné jméno: **1-1024 znaků Unicode a nejvýš 4096 bajtů UTF-8**
  (`NAME_MAX_CHARS`, `NAME_MAX`), měřeno na tvaru NFC, který se opravdu
  uloží. Zakázané je `.` a `..`, znak `/`, řídicí znaky (včetně NUL) a
  **řídicí znaky směru textu** U+202A-202E, U+2066-2069 a značky směru
  U+200E, U+200F, U+061C (`is_bidi_control`).
  Jinak `INVALID_ARGS`.
- Přejmenování jen na jinou velikost písmen (`/fotky` -> `/FOTKY`) je
  povolené.
- Verze tabulek Unicode je v superbloku (`unicode_version`).

Proč zákaz znaků směru textu: znak U+202E obrátí směr zobrazení zbytku
jména, takže soubor `faktura[U+202E]fdp.exe` se v průzkumníku ukáže jako
`fakturaexe.pdf` - klasický trik, jak podstrčit spustitelný soubor. Ostatní
znaky, které jiné systémy nesnesou (`:` `\` `*` `?` na Windows, délka nad
255 bajtů na Linuxu), FS povolí; ovladače pro Windows (WinFsp) a Linux
(FUSE) je při zobrazení přeloží.

```rust
use fs::{validate_name, is_bidi_control, NAME_MAX_CHARS};

assert!(validate_name("Dovolená 2026 - Itálie.jpg").is_ok());
assert!(validate_name(&"x".repeat(NAME_MAX_CHARS)).is_ok());
assert!(validate_name("faktura\u{202E}fdp.exe").is_err());
assert!(is_bidi_control('\u{2067}'));
```

Jména se stejným hashem sdílejí jednu položku `DIRENT`. Kdyby se do ní
dlouhá jména nevešla (nad 8144 B - prakticky jen při úmyslně vyrobené kolizi
FNV hashe), `create` vrátí `NO_RESOURCES`.

## Symbolické odkazy

> **Změna proti v2:** nový druh objektu. **Proč:** potřebují je git repozitáře a zdrojáky z Linuxu. ([všechny změny](fs-verze.md#přechod-v2---v3))

Symbolický odkaz je objekt druhu 3, jehož obsah je **cíl jako text**
(1-4096 bajtů UTF-8, bez NUL; `SYMLINK_MAX`). Cíl nemusí existovat a může
být relativní i absolutní. Leží v položce `SYMLINK`, velikost objektu =
délka cíle.

```rust
fs.symlink("/projekt/README", "docs/README.md")?;
assert_eq!(fs.readlink("/projekt/README")?, "docs/README.md");
assert_eq!(fs.stat("/projekt/README")?.kind, Kind::Symlink);
fs.rename("/projekt/README", "/projekt/CTIME")?; // přesun/mazání jako u souboru
fs.remove("/projekt/CTIME")?;
```

- Knihovna při procházení cesty odkazy **nenásleduje**: odkaz uprostřed
  cesty dá `NOT_A_DIRECTORY`, `read`/`write` odkazu `WRONG_TYPE`. Najde ho
  `find_link` (první odkaz na cestě, práva jako `walk`); následuje fsd.
- Odkaz se v souhrnech adresáře počítá jako soubor s velikostí 0.
- `symlink` se zapisuje do intent logu (záznam 7), takže přežije `fsync`
  i bez commitu. Formát se kvůli následování nemění (odkazy jsou ve v3 od
  začátku).
- `readlink` chce jen viditelnost odkazu (POSIX: průchod složkou).

### Následování ve fsd

**[ověřeno v QEMU]** x86_64 (test „symbolicke odkazy: nasledovani,
smycka, prava, jmenny prostor relace" v sadě `souborovy system`, krok
`link` testu `ctest` v sadě `cstd`; host testy `objects::find_link_on_path`).
fsd následuje odkazy při hledání cesty (`user/fsd/src/links.rs`):

- **Kde:** v prostředních složkách vždy, v poslední podle operace -
  `STAT`, `OPEN`, `LIST*`, `READ`, `WRITE`, `TRUNCATE`, `SET_MODE`, práva,
  `SOCKET_CONNECT`, `REALPATH` ano; `CREATE`, `REMOVE`, `RENAME`,
  `SYMLINK`, `READLINK`, `LSTAT`, `SOCKET_BIND` ne (působí na odkaz, ne na
  cíl). `OPEN` s příznakem `NOFOLLOW` na odkaz = `TOO_MANY_LINKS` (C
  `ELOOP`). Operace historie a datasetů odkazy nenásledují.
- **Jak:** relativní cíl od složky odkazu, absolutní od **kořene relace**;
  `..` po následování podle POSIX (od už vyřešené fyzické cesty; nad
  kořenem relace zůstane kořen). Víc než 40 odkazů = `TOO_MANY_LINKS`
  (smyčka).
- Cesta se řeší ve **jmenném prostoru relace** (připojené cesty, ne cesty
  ve FS): každý krok se znovu mapuje přes připojené cesty relace.
- Klient (`osrt::env::resolve`) `..` nechává fsd (dřív ho řešil
  lexikálně); složka před `..` musí existovat. `chdir` ukládá kanonickou
  cestu (`REALPATH`), `getcwd` je fyzická cesta jako v POSIX.
- `stat` přes odkaz vrací `st_dev`/`st_ino` cíle; jména s `:` (`c:`,
  `com1`) FS povoluje.

**Bezpečnost (výslovně):**

- Odkaz **nic neopravňuje**: práva se kontrolují na cíli i na každé
  složce cesty k němu; cíl, který relace nevidí, = `NOT_FOUND`.
- Cíl mimo jmenný prostor relace (absolutní cesta, která v relaci není
  připojená, nebo `..` nad připojenou cestou) = `NOT_FOUND`, **ne
  průchod** - odkaz nikdy nevede mimo capability relace. Tentýž odkaz
  může v jiné relaci vést jinam (aplikace s `/app` vidí jen svůj adresář).
- Vytvoření odkazu chce právo vytváření ve složce; `rename` a `unlink`
  působí na odkaz, cíl zůstane.
- Odkaz v cíli připojené cesty (mimo jmenný prostor relace) se
  nenásleduje (`NOT_FOUND`).
- Mód odkazu je `0777` jen jako zobrazení (`lstat`); práva má cíl.

## Soubory jako paměť

Rozhodnutí koordinátora (15. 11. 2026, otázky Q1-Q7 varianty A, okna podle
požadavků vlákna Wine): `mmap` souboru bez kopírování přes objekty paměti
(VMO), které drží fsd (`user/fsd/src/filevmo.rs`, `serve_map.rs`; klient
`libs/osrt/src/fmap.rs`, C `libs/cstd/src/mman.rs`).

**[ověřeno v QEMU]** x86_64 i aarch64 (kroky `mmap` a `fmap` testu `ctest`, sada
`cstd`: soukromé i sdílené mapování, koherence s `read`/`write` v obou
směrech i přes hranici oken, `ftruncate`, `msync` přes hranici oken,
`MAP_FIXED` do vlastní rezervace `PROT_NONE` (soukromé i sdílené), dva
procesy nad týmž souborem, trvalost po `munmap` a zavření fd).

- **Okna:** soubor se mapuje po oknech 16 MiB (`fsproto::v2::WINDOW`,
  zarovnaná). Okno je zvětšitelný VMO s kapacitou 16 MiB a velikostí podle
  souboru; **všechny procesy dostanou totéž okno** - sdílené mapování má
  všude tytéž rámce. Rozsah přes víc oken klient namapuje do jedné
  rezervace adres (`MAP_IN_RESERVATION`), takže je souvislý. Soubor může
  mít libovolnou velikost; limit platí na **namapovaný rozsah** (kvóta
  relace), ne na soubor. Okno je i jednotka pro budoucí pager (viz níže).
- **Handly:** jedna zpráva `MAP_VMO` na okno (mapování 1 GiB = 64
  zpráv). Soukromé a sdílené jen pro čtení drží handle okna jen během
  `mmap` (mapování pak drží objekt samo); **zapisovatelné sdílené** si
  handle každého okna ponechá do `munmap` kvůli obnově po pádu fsd
  (1 GiB = 64 handlů).
- **`MAP_PRIVATE`** = klon okna s kopií při zápisu (`VMO_CLONE`): snímek
  v okamžiku `mmap`, zdroj i klon se navzájem neovlivní, nic se nekopíruje
  předem. Stránky za koncem souboru zůstanou rezervací (přístup =
  výjimka).
- **`MAP_SHARED`** = přímo okno; se zápisem jen pro fd `O_RDWR` (jinak
  `EACCES`). Přístup za velikost souboru (v rámci okna) = výjimka za
  koncem objektu (`SIGBUS`), jako POSIX.
- **Zdroj pravdy:** dokud fsd drží záznam souboru, je obsah načtených oken
  v paměti, ne ve FS. `READ_H`, `WRITE_H`, `READ_VMO`, `SET_LEN`
  a velikost v `STAT_H` jdou přes záznam (i přes hranici oken), takže
  `read()` vidí zápis do mapování hned a mapování vidí `write()` hned.
  Nenačtená okna čtou a píšou přímo FS. Před každou jinou operací (cesta,
  `OPEN`, snapshot, kopie, verze, `COMMIT`, `FSYNC`...) fsd špinavé
  stránky zapíše zpět a po ní znovu načte okna souboru, který se změnil
  mimo záznam (verze objektu).
- **`ftruncate`** změní velikost oken: zvětšení = další stránky
  v existujících mapováních (nuly), zmenšení = přístup za konec `SIGBUS`,
  zbytek poslední stránky za koncem jsou nuly.
- **Zápis zpět:** každé okno je od načtení sledované syscallem
  `VMO_DIRTY` ([syscally](syscally.md#vmo_dirty)); špinavé stránky fsd
  zapíše přes interní handle (`Fs::pin_h` - drží objekt i po zavření fd
  a smazání jména, nepodléhá sdílecím režimům ani povinným zámkům, jako
  POSIX `mmap`). Kdy:
  - `msync(MS_SYNC)` (`SYNC_VMO` s `FSYNC`; rozsah přes víc oken i souborů
    zapíše všechny) a `fsync` klienta (operace `FSYNC` je cizí operace,
    viz výše) - **trvale**;
  - `munmap` celého mapování (`UNMAP_VMO`), zavření handlu souboru, konec
    relace;
  - před každou cizí operací a nejpozději za **30 s** (`filevmo::WRITEBACK`,
    hlavní smyčka fsd; řádek v logu fsd s počtem stránek, souborů, oken
    a bajtů);
  - `msync(MS_ASYNC)` je jen žádost: nic se neposílá, zápis zpět udělá
    fsd sám nejpozději za 30 s.

  Automatický commit FS (každých 5 s) mapovaná data nezahrnuje, dokud se
  nezapíšou zpět - po pádu systému se ztratí nejvýš 30 s zápisů do
  mapování (jako `dirty_expire` v Linuxu); jistotu dává `msync(MS_SYNC)`
  nebo `fsync`.
- **`munmap` části rozsahu** odmapuje jen tu část (jádro); okna zůstanou
  držená, dokud proces neuvolní celé mapování nebo neskončí relace (okno
  pokryté jiným mapováním téhož procesu se neuvolní). **`mremap` není**
  (cstd ho nemá; zvětšení mapování = nové `mmap`).
- **Cena prvního mapování:** fsd při prvním mapování okna načte celé okno
  z FS; další mapování téhož okna jen předá handle. Čísla v tabulce pod
  seznamem: čas jde skoro celý na čtení FS (`fs.read_h`), kopie do okna
  a zapnutí sledování jsou proti němu malé. **Příčina** (změřeno
  v ovladači a fsd): zařízení přečte 16 MiB za desítky ms, čas jde na
  přenos dat zprávami kanálu ovladač -> fsd (32 KiB na zprávu, kopie
  v jádře) - jádro v debug obrazu bylo bez optimalizace (teď
  `opt-level = 2`). Test „cteni
  velkeho souboru" četl data ještě z cache fsd, proto byl rychlejší.
  Ovladač virtio-blk navíc spojuje navazující čtení do příkazů po 1 MiB
  (`Disks::max_read`). Pager (plnění po stránkách při výpadku) zůstává
  pro velké soubory důležitý.
- **Cena zápisu:** první zápis do stránky po každém vyčištění stojí jeden
  výpadek (sledování bez dirty bitů PTE, stejně na x86_64 i aarch64).
  Každá cizí operace stojí jedno `VMO_DIRTY` na načtené okno, dokud je
  nějaký soubor namapovaný.
- **Paměť:** okna vytváří fsd, takže se počítají do **rozpočtu fsd** (vidí
  ho diagnostika paměti jobů), ne do jobu klienta - **omezení**: klient tím
  nezvětší svůj účet. Okno slíbí paměť jen podle velikosti souboru (ne
  kapacity; jádro platí 8 B na stránku kapacity = 32 KiB na okno);
  zvětšení souboru slíbí další stránky hned (`ENOMEM` při
  `ftruncate`/`write`, ne při výpadku). Sdílené mapování okno drží do
  `munmap`/konce relace; kvóta relace 1 GiB držených oken
  (`SESSION_QUOTA`, překročení = `ENOMEM`). Záznamy bez držení fsd
  zahazuje od nejstaršího nad 512 MiB oken (`CACHE_MAX`, po zápisu
  zpět). Stránky, do kterých klient zapíše v `MAP_PRIVATE`, platí job
  klienta (klon na jeho účet).
- **Smazání** souboru mapování neruší (interní handle drží sirotka).
- **Smazání datasetu a odpojení svazku** **[ověřeno v QEMU]** x86_64
  i aarch64 (test „smazani datasetu s namapovanym souborem: okno na 0"
  v sadě `historie FS`; rozhodnutí koordinátora B): před smazáním (cizí
  operace) se změny z mapování zapíšou zpět, smazání nic neblokuje (žádné
  `LOCKED`); záznamy souborů zaniklého datasetu fsd zmenší na 0 a interní
  handly zavře - klienti dostanou při přístupu výjimku za koncem
  (`SIGBUS`) místo starých dat. Při konci fsd (odpojení svazku) se vše
  zapíše zpět a všechna okna se zmenší na 0.
- **Mezera (výslovně): odvolání přístupu neplatí pro už otevřené handly
  ani mapování.** `REVOKE` (i odebrání grantu) mapování nemění, stejně
  jako už otevřené handly: relace má přístup k namapovanému oknu do
  `munmap`/konce relace. Okno je sdílené mezi procesy, jeho zmenšení by
  vzalo přístup i relacím, které právo mají dál. „Tvrdé odvolání"
  (zavření handlů i mapování odvolané relace) je samostatný návrh ve
  frontě koordinátora.
- **Riziko (výslovně):** sdílené mapování pro zápis dává klientovi
  zapisovatelnou paměť souboru - ostatní procesy se stejným souborem vidí
  jeho zápisy bez další kontroly FS (práva se ověřují jen při mapování),
  stejně jako POSIX. Klient s handlem okna pro zápis může zavolat i
  `VMO_DIRTY` a tím fsd „ukrást" špinavé stránky (zápisy jiných procesů do
  téhož okna pak nemusí dojít do souboru) - škodí jen souboru, do kterého
  smí zapisovat. Proudy souborů (`OPEN_STREAM`) čtou FS přímo a namapované
  okno nevidí, dokud se nezapíše zpět.

| Mapování okna 16 MiB (QEMU TCG, debug obraz, soubor po zápisu a `fsync`) | Čas |
|---|---|
| první mapování, x86_64 (`3a65dd78` + okna) | 2197 ms |
| z toho čtení FS po 1 MiB, x86_64 (`3a65dd78` + okna) | 130 ms/MiB |
| z toho kopie do okna, x86_64 (`3a65dd78` + okna) | 4 ms/MiB |
| z toho zapnutí sledování `VMO_DIRTY`, x86_64 (`3a65dd78` + okna) | 21 ms |
| další mapování téhož okna, x86_64 (`3a65dd78` + okna) | 4 ms |
| první / další mapování, aarch64 (`3a65dd78` + okna) | 2616 ms / 1 ms |
| pro srovnání: čtení FS v testu „cteni velkeho souboru" (`a313581a` + mapování) | 16 ms/MiB |
| rozpad, x86_64: zařízení (čekání virtio-blk) 16 MiB (`b2f54c3a` + měření) | 34 ms |
| rozpad, x86_64: odpovědi kanálem ovladač -> fsd 16 MiB (`b2f54c3a` + měření) | 1,7-3 s |
| první mapování, ovladač spojuje čtení po 1 MiB (`b2f54c3a` + virtio-blk) | 1851 ms |
| první mapování, jádro `opt-level = 2` (výchozí, [sestavení](sestaveni.md#ladění-přes-gdb)), x86_64 (`b2f54c3a` + virtio-blk + profil) | 122 ms |
| první mapování, jádro `opt-level = 2`, aarch64 (`b2f54c3a` + virtio-blk + profil) | 181 ms |

**Výpadek fsd (Q5)** **[ověřeno v QEMU]** x86_64 i aarch64 (test „zabij fsd
uprostred prace" v sadě `obnova fsd`: zápis do okna staré instance po pádu
se přenese do nové): okna nové instance jsou prázdná, ale staré okno žije
dál v mapování klienta i se svým sledováním zápisu. Zapisovatelné sdílené
mapování si drží handly oken a identitu souboru (`libs/osrt/src/fmap.rs`).
Když `msync` nebo `munmap` zjistí novou instanci (`SYNC_VMO` vrátí
`SERVICE_RESTARTED` nebo jiná generace spojení), klient soubor **znovu
otevře s právem zápisu** (`REOPEN`, práva ověří nová instance) a předá
staré okno operací `ADOPT_VMO`: fsd si přes `VMO_DIRTY` vezme stránky
změněné od posledního zápisu zpět staré instance a zapíše je jako zápis
toho klienta (nic za velikostí souboru, obsahu nevěří víc než zápisu
klienta). Pak se rozsah přemapuje na nová okna (mapování -> rezervace ->
nové okno; adresy zůstanou). Bez práva zápisu nebo když objekt zanikl
(soubor nebyl před pádem uložený) se změny zahodí a `msync` vrací
`SERVICE_RESTARTED` (C `EIO`). **Omezení:** zápisy jiných vláken procesu
mezi předáním a přemapováním se ztratí; do `msync`/`munmap` po pádu
nevidí změny ostatní procesy ani `read()`; sdílené mapování jen pro čtení
zůstane na starém okně (další změny souboru nevidí).

**[navrženo]** (další kroky):
- **Pager v jádře:** okno zůstane jednotkou, kterou fsd místo načtení
  celého při mapování plní líně po stránkách při výpadku; protokol
  klienta (`MAP_VMO` vrací handle okna) se nezmění.

## Limity formátu

> **Změna proti v2:** nová kapitola: soubor nejvýš 2^63-1 B, kontrolovaná aritmetika, saturující souhrny, extent 496 bloků. **Proč:** dřív mohly velikosti přetéct. ([všechny změny](fs-verze.md#přechod-v2---v3))

| Co | Limit | Konstanta |
|---|---|---|
| velikost souboru | 2^63-1 B (8 EiB) | `MAX_FILE_SIZE` |
| jméno | 1024 znaků / 4096 B UTF-8 | `NAME_MAX_CHARS`, `NAME_MAX` |
| cíl symbolického odkazu | 4096 B | `SYMLINK_MAX` |
| soubor uložený přímo ve stromu | 1984 B | `INLINE_MAX` |
| extent | 496 bloků (~2 MiB) | `MAX_EXTENT` |
| hodnota položky stromu | 8144 B | `node::MAX_VALUE` |
| hloubka stromu | 17 úrovní | `node::MAX_LEVEL` |
| zařízení svazku | 2^48 bloků (1 EiB) | 48 bitů adresy v `BlockPtr` |

Zápis nebo `truncate` za `MAX_FILE_SIZE` vrátí `OUT_OF_RANGE`. Díky limitu
se velikost i rozdíl dvou velikostí vždy vejdou do `i64`. Souhrny adresářů
**saturují** na 2^64-1: řídké soubory mohou mít dohromady logickou velikost
větší, než jde sečíst - saturovaný souhrn je jen informativní (data jsou
v pořádku).

## Okamžité velikosti adresářů

Každý adresář nese **souhrn celého podstromu**: součet velikostí souborů,
počet souborů a počet adresářů. Každá změna velikosti, vytvoření, smazání
i přesun ho aktualizuje u rodiče a všech předků (cena = hloubka cesty).
`stat` adresáře tak vrací velikost složky s tisíci souborů za
mikrosekundy (naměřeno 13,5 µs pro 20 000 souborů), bez procházení.

```rust
let st = fs.stat("/projekty")?;
println!("{} B, {} souborů, {} podadresářů", st.tree_bytes, st.tree_files, st.tree_dirs);
```

## Zápis dat: odložená alokace a extenty

> **Změna proti v2:** výsledek `write` znamená „přijato“, ne „trvalé“; přepsání nejdřív zapíše nové bloky a pak přepojí extenty; skládání bloků po celých kusech. **Proč:** chyba zápisu nesmí ztratit data ani nechat díru a klient musí vědět, co bylo přijato; zápis je 2× rychlejší. ([všechny změny](fs-verze.md#přechod-v2---v3))

1. `write` zapíše data do **bufferu v paměti** (nejvýš 16 MiB, pak se
   zapíší dřív). Částečně přepsaný blok se nejdřív načte.

   **Výsledek `write` = přijato, ne trvalé.** `Ok` = data jsou celá
   v bufferu (vidí je čtení, uloží je příští commit nebo `fsync`); chyba =
   z tohoto volání se nepřijalo nic. Trvanlivost potvrzuje až
   `fsync`/`commit`. Když odpis bufferu na disk selže, data zůstanou
   v bufferu a chyba se ohlásí při `fsync`/`commit`; teprve když buffer
   dosáhne 32 MiB (`DIRTY_HARD_LIMIT`) a nejde vyprázdnit, další `write`
   se odmítne.
2. Až při zápisu na disk (commit, `fsync`, plný buffer) dostanou po sobě
   jdoucí bloky **jeden souvislý úsek** - s cílem hned za předchozím
   extentem souboru (soubor se prodlužuje souvisle), jinak nejmenší dost
   velký volný úsek (best-fit). Velké úseky (≥ 1 MiB) se zarovnají na
   1 MiB.
3. Data se zapíší jedním požadavkem na úsek, součet každého bloku jde do
   položky `EXTENT`.

Soubor zapsaný po kouscích (např. 64 KiB po 64 KiB) tak skončí v jednom
souvislém úseku. Soubor, který do commitu zase zmizí, se na disk nezapíše
vůbec.

**Přepsání části extentu** ho rozdělí; přepsané fyzické bloky se uvolní
podle pravidel níže:

<figure class="diagram">
<svg viewBox="0 0 760 150" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Rozdeleni extentu">
<text class="m" x="10" y="20">před: extent logických bloků 0-9 → fyzické 500-509</text>
<rect class="box" x="10" y="30" width="740" height="34" rx="4"/><text class="t mono" x="380" y="52" text-anchor="middle">EXTENT@0: phys 500, 10 bloků</text>
<text class="m" x="10" y="92">po zápisu do bloků 3-5: tři extenty, bloky 503-505 se uvolní</text>
<rect class="box" x="10" y="102" width="220" height="34" rx="4"/><text class="t mono" x="120" y="124" text-anchor="middle">EXTENT@0: phys 500, 3</text>
<rect class="acc" x="236" y="102" width="220" height="34" rx="4"/><text class="t mono" x="346" y="124" text-anchor="middle">EXTENT@3: phys 812, 3 (nová)</text>
<rect class="box" x="462" y="102" width="288" height="34" rx="4"/><text class="t mono" x="606" y="124" text-anchor="middle">EXTENT@6: phys 506, 4</text>
</svg>
<figcaption>Copy-on-write na úrovni bloků: nová data jdou jinam, zbytky extentu zůstávají.</figcaption>
</figure>

**Malé soubory** (do 1984 B) leží přímo ve stromu (`INLINE`), bez
datového bloku. Při zvětšení nad limit se převedou na extenty a naopak.
Díry (nezapsané části) se čtou jako nuly a nezabírají místo.

### Pravidla uvolnění bloku

Blok s generací vzniku `birth` se při přepsání nebo smazání:

| Podmínka | Co se stane | Proč |
|---|---|---|
| `birth == txg`, nikdo na něj neodkazuje z intent logu a extent není sdílený | volný **hned** | žádný uložený stav na něj neukazuje |
| `birth > snapshot_generation` | volný po **odložení** (`REUSE_DELAY` = 4 commity) | předchozí uložené stavy na něj mohou ukazovat (návrat o generaci) |
| `birth ≤ snapshot_generation` | do **deadlistu** | drží ho snapshot; uvolní se se smazáním snapshotu |

Uvolní se vždy i druhá kopie (u uzlů a dat s kopií). Datový úsek, který je
v **tabulce sdílení** (okamžité kopie, deduplikace), se při definitivním
uvolnění jen odečte a uvolní až s posledním odkazem - viz [Tabulka
sdílení](fs-data.md#tabulka-sdílení).

## Commit krok za krokem

`commit` uloží všechny změny atomicky:

1. Zapíší se rozpracovaná data souborů (odložená alokace, viz výše).
2. Mrtvé bloky z paměti se zapíší do deadlistu živé verze.
3. Rozpracované uzly stromu se zapíší **od listů ke kořeni** (rodič potřebuje
   součty potomků) do jednoho souvislého úseku - metadata commitu jsou na
   disku jedním sekvenčním zápisem.
4. Bloky předchozí epochy intent logu se uvolní (odloženě) a alokuje se
   první blok nové epochy.
5. Zapíší se změny space logů a tabulky AG.
6. **flush** - vše výše je trvale na disku.
7. Nový superblok do slotu `1 + generace mod 8`.
8. **flush** - commit je hotový.
9. Bloky, kterým vypršela lhůta odložení, jsou volné a jdou do fronty
   TRIM (pokud je zapnutý).

Pokud commit selže uprostřed (chyba I/O), stav v paměti už neodpovídá
disku: instance se označí jako poškozená a další změny vrací `IO_ERROR`.
Je nutné svazek znovu připojit - na disku platí poslední dokončený commit.

## Co se stane při výpadku proudu

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Vypadek proudu pri commitu">
<defs><marker id="mk2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<path class="ln-acc" d="M20 60 L740 60" marker-end="url(#mk2)"/>
<rect class="box" x="20" y="35" width="200" height="50" rx="6"/><text class="t" x="120" y="58" text-anchor="middle">nová data, uzly, logy</text><text class="m" x="120" y="75" text-anchor="middle">do volných bloků</text>
<rect class="warn" x="235" y="35" width="80" height="50" rx="6"/><text class="t" x="275" y="65" text-anchor="middle">flush</text>
<rect class="acc" x="330" y="35" width="170" height="50" rx="6"/><text class="t" x="415" y="58" text-anchor="middle">superblok g+1</text><text class="m" x="415" y="75" text-anchor="middle">do jiného slotu</text>
<rect class="warn" x="515" y="35" width="80" height="50" rx="6"/><text class="t" x="555" y="65" text-anchor="middle">flush</text>
<rect class="ok" x="610" y="35" width="130" height="50" rx="6"/><text class="t" x="675" y="65" text-anchor="middle">hotovo</text>
<rect class="ok" x="20" y="110" width="480" height="40" rx="6"/><text class="t" x="260" y="135" text-anchor="middle">výpadek zde → po připojení stav g (nový stav se ztratí, nic se nerozbije)</text>
<rect class="ok" x="510" y="110" width="230" height="40" rx="6"/><text class="t" x="625" y="135" text-anchor="middle">výpadek zde → stav g+1</text>
<text class="m" x="20" y="180">Roztržený zápis superbloku odhalí jeho součet → použije se předchozí slot.</text>
<text class="m" x="20" y="198">Změny potvrzené fsync se v obou případech obnoví z intent logu.</text>
</svg>
<figcaption>Žádný okamžik výpadku nevede k poškozenému svazku.</figcaption>
</figure>

| Okamžik výpadku | Výsledek po připojení |
|---|---|
| kdykoli před dokončením zápisu superbloku | stav posledního commitu - jeho bloky nikdo nepřepsal |
| během zápisu superbloku | roztržený superblok neprojde součtem, platí předchozí |
| po druhém flush | nový stav |
| během `fsync` | stav před `fsync`, nebo po něm (podle toho, zda dávka logu stihla celá) |
| v libovolném okamžiku | změny potvrzené dokončeným `fsync` se přehrají z intent logu |

Tyto záruky ověřuje test, který simuluje výpadek po N-tém zápisu a z
nezapsaných bloků nechá přežít náhodnou podmnožinu v náhodném pořadí
(`tests/crash.rs`, desítky tisíc kombinací).

Předpoklad: zařízení po potvrzeném `flush` data opravdu má. Disky, které
o flush lžou, pokrývá [návrat o generaci](#připojení-a-návrat-o-generaci).

## Připojení a návrat o generaci

> **Změna proti v2:** nejdřív výběr generace podle hlavičky, pak rozhodnutí o kompatibilitě; nejistá obnova (nečitelný slot) jen pro čtení; návrat jen při poškození; zařízení bez flush jen pro čtení. **Proč:** dřív mohl starší ovladač nebo přechodná chyba vést k zápisu nad starší generací a ztrátě novějších dat. ([všechny změny](fs-verze.md#přechod-v2---v3))

Bloky uvolněné commitem `g` se znovu použijí až v transakci
`g + REUSE_DELAY` (4). Posledních 4 uložených stavů tak zůstává na disku
nedotčených. Kruh má 8 slotů superbloku.

Připojení (`Store::open`) má tři oddělené kroky:

1. **Výběr generace.** Ze všech slotů s platnou *hlavičkou* (podpis rodiny
   formátu `OSFS\0\0` a vlastní součet) se vezme nejvyšší generace.
   Nečitelný slot (chyba I/O) nebo slot se špatným součtem je jen neplatný
   slot, ostatní se zkusí dál.
2. **Umí ji tento ovladač otevřít?**

| Situace v nejnovější generaci | Výsledek |
|---|---|
| jiná verze formátu (i novější `v4`) | `NOT_SUPPORTED` |
| neznámý bit `incompat` | `NOT_SUPPORTED` |
| neznámý bit `ro_compat` | připojení **jen pro čtení**, intent log se přehraje jen v paměti, na disk se nezapíše nic |
| vše známé | normální připojení |

   Nikdy se tiše nepřipojí starší generace: jinak by starší ovladač dostal
   zapisovatelný přístup ke starému stavu a další commit by novější data
   zahodil.
3. **Obnova po poškození.** Když stav vybrané generace nejde načíst kvůli
   **poškození** (`CORRUPTED`: špatný součet kořene nebo metadat místa,
   nesmyslné hodnoty v superbloku), zkusí se starší - nejvýš o 3 generace
   zpět (starší už mohou být přepsané). `info().fallback` řekne, o kolik
   generací se systém vrátil, služba fsd to hlásí varováním. Jiná chyba
   (I/O, nedostatek paměti) se vrátí hned - přechodná chyba disku nesmí
   vést k návratu na starší stav.

**Nejistá obnova = jen pro čtení.** Commity jdou postupně a generace `g`
leží ve slotu `1 + g mod 8`, takže generace novější než nejnovější
přečtená (`newest`) by mohla ležet **jen ve slotu pro `newest + 1`** (a
`newest + 2` jen za ní). Projdou se proto sloty za nejnovějším: slot se
starší generací nebo prázdný = novější stav neexistuje; slot se špatným
součtem mohl být dokončený commit s později poškozeným superblokem, jde se
dál. Když je na této cestě slot nečitelný (chyba I/O), nejnovější stav nejde
bezpečně určit: svazek je **jen pro čtení** s `info().newest_uncertain`
(fsd vypíše varování) - zápis nad starší generací by po dalším commitu tu
novější přepsal. Nečitelný jiný slot (vadný sektor se starou generací)
nejnovější stav nezpochybňuje: svazek je zapisovatelný, jen
`info().unreadable_superblocks` > 0 a fsd vypíše varování (jinak by vadný
sektor udělal svazek navždy jen pro čtení).

**Volby připojení** (`Fs::mount_with`, `MountOptions`):

| Volba | Význam |
|---|---|
| `read_only` | vynutit jen pro čtení (kontrola nebo záchrana dat bez zápisu) |
| `discard_unreadable_log` | když intent log nejde přečíst celý, **vědomě zahodit** nečitelný zbytek a připojit zapisovatelně (`info().log_discarded`); změny po poslední čitelné dávce (i potvrzené `fsync`) se ztratí - jen když chyba čtení nezmizí |

Po návratu se sloty s novější (poškozenou) generací **před prvním commitem
vynulují** - jinak by po dalším připojení vyhrály a ukazovaly na bloky, které
mezitím dostal někdo jiný. Intent log se po návratu nepřehrává (patří
k novějšímu stavu) a hned proběhne commit s novou epochou logu.

### Zařízení bez záruky trvanlivosti

`BlockDevice::durability()` říká, jestli zařízení data opravdu uloží:

| Hodnota | Význam | Připojení |
|---|---|---|
| `Flush` | `flush` uloží vše z mezipaměti na médium | zapisovatelné |
| `WriteThrough` | zařízení nemá mezipaměť zápisu | zapisovatelné |
| `None` | žádná nebo neznámá záruka | **jen pro čtení** (`info().read_only`, změny vrací `READ_ONLY`) |

Ve fsd ji dává ovladač disku operací `FEATURES` blokového protokolu (bity
`FLUSH`, `WRITE_THROUGH`, `DISCARD`, `READ_ONLY`). virtio-blk bez vlastnosti
`VIRTIO_BLK_F_FLUSH` nehlásí žádnou záruku a `FLUSH` vrátí `NOT_SUPPORTED`
(dřív vracel úspěch, aniž by cokoli udělal - `fsync` by pak lhal). Starší
ovladač bez `FEATURES` = žádná záruka.

⚠️ **Omezení:** disk, který o flush lže (potvrdí ho, ale data drží
v mezipaměti), FS nepozná. Proti tomu je jen návrat o generaci a kontrolní
součty - poslední potvrzené změny se pak mohou ztratit.

## fsync a intent log

> **Změna proti v3 (fáze 1):** nové záznamy 14 `Actor` (kdo provedl
> následující operace), 15 `Trash`, 16 `Restore`, 17 `Purge`, 18
> `SetXattr`; sloučené záznamy žurnálu `CONTENT` dostanou číslo na konci
> dávky. **Proč:** žurnál změn musí po přehrání logu vyjít se stejnými
> čísly i uživateli a koš i atributy musí přežít pád. ([Historie FS](fs-historie.md#na-disku))

> **Změna proti v2:** čekající změny se mažou až po flush, bloky logu se alokují předem, chyba čtení logu není konec řetězu, nový záznam 7 (symlink) a generace u vytvoření. **Proč:** potvrzený `fsync` se nesmí ztratit ani nic nepotvrdit falešně. ([všechny změny](fs-verze.md#přechod-v2---v3))

Commit přepisuje mnoho bloků a dělá dva flush - pro každé uložení editoru
nebo transakci databáze je to drahé. Intent log (jako ZIL v ZFS nebo log
tree v btrfs) dělá změny trvalými jedním zápisem a jedním flush:

1. Od posledního commitu se v paměti vede seznam operací s adresáři
   (vytvoření, smazání, přejmenování, změna velikosti) a množina změněných
   bloků souborů.
2. `fsync` zapíše změněná data do nových bloků (odložená alokace) a zakóduje
   operace a mapování dat do **dávky**.
3. Dávka se zapíše do řetězu bloků logu, pak **jeden flush**.

<figure class="diagram">
<svg viewBox="0 0 760 140" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Retez intent logu">
<defs><marker id="mk3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="40" width="120" height="50" rx="6"/><text class="t" x="70" y="62" text-anchor="middle">superblok</text><text class="m" x="70" y="80" text-anchor="middle">zil_block, seed</text>
<path class="ln" d="M130 65 L170 65" marker-end="url(#mk3)"/>
<rect class="box" x="172" y="40" width="160" height="50" rx="6"/><text class="t mono" x="252" y="62" text-anchor="middle">pořadí 0</text><text class="m" x="252" y="80" text-anchor="middle">další = 731 (předem)</text>
<path class="ln" d="M332 65 L372 65" marker-end="url(#mk3)"/>
<rect class="box" x="374" y="40" width="160" height="50" rx="6"/><text class="t mono" x="454" y="62" text-anchor="middle">pořadí 1</text><text class="m" x="454" y="80" text-anchor="middle">další = 902</text>
<path class="ln" d="M534 65 L574 65" marker-end="url(#mk3)"/>
<rect class="err" x="576" y="40" width="170" height="50" rx="6"/><text class="t" x="661" y="62" text-anchor="middle">blok 902</text><text class="m" x="661" y="80" text-anchor="middle">špatný součet = konec</text>
<text class="m" x="10" y="120">Každý blok má součet XXH3-128 se semínkem (epocha + pořadí): starý obsah nikdy neprojde.</text>
</svg>
<figcaption>Řetěz logu: další blok je alokovaný předem, jeho číslo je v předchozím.</figcaption>
</figure>

Formát bloku logu:

```text
0   magic "ZL", 2 příznak (1 = poslední blok dávky)
8   pořadí u64, 16 další blok u64, 24 délka dat u32
32  součet XXH3-128 (pole součtu nulové) se semínkem zil_seed + pořadí
64  data dávky
```

Záznamy dávky (první bajt = druh): 1 vytvoření (rodič, oid, druh, čas,
jméno), 2 smazání (rodič, čas, jméno), 3 přejmenování (zdroj, cíl, čas,
jméno, nové jméno), 4 změna velikosti (oid, velikost, čas), 5 obsah malého
souboru (oid, velikost, mtime, data), 6 mapování dat (oid, velikost, mtime,
seznam extentů se součty), 7 symbolický odkaz, 8 značka datasetu,
9 nastavení časů (oid, maska u8: bit 0 mtime, 1 btime, 2 atime, čas změny,
přítomné časy), 10/11 vytvoření/odkaz s vlastníkem (vlastník u64, pak
záznam 1/7), 12 nastavení popisu oprávnění, 13 změna vlastníka. Čas v záznamu má 12 B (sekundy i64 + nanosekundy u32), na
svazku bez `INCOMPAT_TIME_SECS` 8 B (u64 ns).

**Při připojení:** log se čte od `zil_block`, dokud souhlasí magic,
pořadí a součet. Úplné dávky se přehrají nad stavem posledního commitu -
ale jen ty, jejichž data jsou celá na disku (součty bloků dat souhlasí;
jinak výpadek přišel před flush a dávka se zahodí). Hned potom proběhne
commit a začne nová epocha. `info().replayed` = počet přehraných operací.

**Chyba čtení logu není konec řetězu.** Za nečitelným blokem může být už
potvrzený `fsync`. Při chybě I/O (čtení bloku logu nebo dat dávky) se
přehraje jen to, co jde přečíst, svazek je **jen pro čtení**
(`info().log_unreadable`) a log zůstane na disku nedotčený pro další pokus.
Konec řetězu je jen blok se špatným magic, pořadím nebo součtem. Když
nečitelný blok trvá (vadný sektor), jde log vědomě zahodit volbou
`discard_unreadable_log` (viz [Připojení](#připojení-a-návrat-o-generaci)).

Bloky dat, na které odkazuje dávka logu, se do commitu znovu nepoužijí,
ani když vznikly ve stejné transakci (jinak by je pozdější `fsync`
přepsal). Když log naroste nad 2048 bloků, `fsync` místo něj udělá commit.

**Atomicita `fsync`:** seznam čekajících změn se smaže až po úspěšném flush.

- Chyba **před** zápisem prvního bloku logu (zápis dat, nedostatek místa
  pro bloky logu - ty se alokují předem) nic nerozbije: data zůstanou
  v bufferu a další `fsync` to zkusí znovu.
- Chyba **při** zápisu logu nebo flush = nejistý stav (část dávky může být
  na disku). Svazek přejde do stavu chyby: všechny další změny i `fsync`
  vrací `IO_ERROR`, takže nic pozdějšího nepotvrdí jako trvalé něco, co
  trvalé není. Obnova = znovu připojit (přehrají se jen celé dávky).

**Zápis dat do nových bloků** (`write_run`) nejdřív alokuje a zapíše nové
bloky a teprve potom přepojí extenty. Když alokace nebo zápis selže, stará
data zůstanou namapovaná (žádná díra po přepsání), nové bloky se hned vrátí
a nezapsaná data se vrátí do bufferu.

## Správa volného místa

### Alokační skupiny a space logy

Disk je rozdělený na alokační skupiny (výchozí 262 144 bloků = 1 GiB; u
obřích disků větší, aby skupin bylo nejvýš ~10 800). Pro každou skupinu:

- **v paměti:** volné úseky indexované podle pozice i podle velikosti (jako
  XFS) - best-fit hledání v čase O(log n), paměť roste jen s fragmentací,
- **na disku:** **space log** - připisovaný záznam alokací a uvolnění
  (jako space mapy v ZFS). Commit zapíše jen nové záznamy, cena je úměrná
  změnám, ne velikosti disku.

<figure class="diagram">
<svg viewBox="0 0 760 190" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Space log">
<defs><marker id="mk4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="120" height="44" rx="6"/><text class="t" x="70" y="47" text-anchor="middle">superblok.space</text>
<path class="ln" d="M130 42 L168 42" marker-end="url(#mk4)"/>
<rect class="box" x="170" y="20" width="150" height="44" rx="6"/><text class="t" x="245" y="40" text-anchor="middle">index</text><text class="m" x="245" y="56" text-anchor="middle">≤ 127 bloků tabulky</text>
<path class="ln" d="M320 42 L358 42" marker-end="url(#mk4)"/>
<rect class="box" x="360" y="20" width="170" height="44" rx="6"/><text class="t" x="445" y="40" text-anchor="middle">tabulka AG</text><text class="m" x="445" y="56" text-anchor="middle">≤ 85 skupin na blok</text>
<path class="ln" d="M445 64 L445 98" marker-end="url(#mk4)"/>
<rect class="ok" x="360" y="100" width="170" height="44" rx="6"/><text class="t" x="445" y="120" text-anchor="middle">hlava logu AG</text><text class="m" x="445" y="136" text-anchor="middle">nejnovější záznamy</text>
<path class="ln" d="M530 122 L568 122" marker-end="url(#mk4)"/>
<rect class="box" x="570" y="100" width="170" height="44" rx="6"/><text class="t" x="655" y="120" text-anchor="middle">starší blok logu</text><text class="m" x="655" y="136" text-anchor="middle">→ … → nejstarší</text>
<text class="m" x="10" y="170">Záznam 24 B: začátek u64, délka u32, alokace/uvolnění u8, txg u64; 168 záznamů na blok.</text>
</svg>
<figcaption>Volné místo: index → tabulka skupin → řetěz space logu každé skupiny.</figcaption>
</figure>

- **Commit:** neplný poslední blok logu se nahradí novým (copy-on-write),
  plný zůstane a nový se naváže. Alokace bloků pro samotné logy mění volné
  místo, které se zase musí zapsat - příprava proto opakuje alokace, dokud
  se stav neustálí, a pak vše zapíše.
- **Připojení:** logy se přehrají od nejstaršího záznamu. Uvolnění z
  posledních `REUSE_DELAY` transakcí jdou do odložených.
- **Zhuštění (condense):** když má log víc než dvojnásobek bloků proti
  zhuštěné podobě (+8), přepíše se jako seznam obsazených úseků a
  odložených uvolnění.

### Umístění (proti fragmentaci)

| Co | Strategie |
|---|---|
| uzly stromu, logy | first-fit od začátku disku - metadata pohromadě, commit jedním souvislým zápisem |
| data | přesně za předchozím extentem souboru; jinak nejmenší dost velký úsek (best-fit) od středu disku; jinak největší dostupný a zbytek zvlášť |
| velká data (≥ 1 MiB) | začátek zarovnaný na 1 MiB, pokud úsek dovolí |

### Rezerva a nedostatek místa

> **Změna proti v2:** rozpracovaný uzel se počítá jako 4 bloky, počítá se jen místo pro celé uzly, operace uvolňující místo smí do poloviny rezervy. **Proč:** commit se na rozdrobeném disku zacyklil nebo selhal a na plném disku nešel smazat snapshot. ([všechny změny](fs-verze.md#přechod-v2---v3))

Část místa (1/128 svazku, 24 bloků až 256 MiB) je rezervovaná pro commit
(metadata, logy). Běžný zápis, který by ji spotřeboval, vrátí `NO_SPACE`
**hned** - ne až při commitu. Pokud čekají bloky na odložené uvolnění,
systém nejdřív zkusí commity (až 5), aby se uvolnily.

Do kontroly se počítá jen místo, **kam se vejde celý uzel stromu** (úseky
aspoň 4 souvislých bloků, `Space::node_room`), a každý rozpracovaný uzel jako
4 bloky. Kdyby data zapsaná při commitu zabrala poslední takové úseky, commit
by neměl kam uložit strom a svazek by přešel do režimu jen pro čtení. Cena:
na silně rozdrobeném disku přijde `NO_SPACE` o pár bloků dřív, než je disk
opravdu plný. Alokace uzlů (`alloc_min`) úseky kratší než 4 bloky přeskakuje.

**Uvolnit místo jde i na plném svazku.** Operace, které místo uvolňují
(`remove`, `delete_snapshot`, zkrácení souboru), smí sáhnout do poloviny
rezervy commitu (`has_room_freeing`); druhá polovina zůstává pro commit.
Jinak by na plném svazku nešel smazat ani snapshot, který drží většinu
místa. Uvolněné bloky jsou použitelné po ochranné lhůtě (`REUSE_DELAY`
commitů).

## TRIM

SSD potřebuje vědět, které bloky jsou volné (jinak klesá výkon a roste
opotřebení). Po commitu, když bloku vyprší lhůta odložení, jde do fronty
TRIM; sousední úseky se sloučí a pošlou zařízení (`BlockDevice::discard`,
v systému operace `DISCARD` blokového protokolu → virtio-blk
`VIRTIO_BLK_T_DISCARD`). Chyba TRIM není fatální (je to jen nápověda).

- `set_discard(true)` - průběžný TRIM (fsd ho zapíná).
- `trim_all()` - TRIM všech volných bloků najednou (jako `fstrim`).

## Fragmentace a defragmentace

`fragments(cesta)` vrátí počet souvislých fyzických úseků souboru
(1 = nefragmentovaný). Soubor stojí za defragmentaci, když má víc úseků a
průměrný úsek je kratší než 4 MiB.

Defragmentace soubor načte do bufferu, jako by byl zapsaný znovu, a
odložená alokace mu najde souvislý úsek. Díky copy-on-write je to bezpečné -
do commitu platí stará verze. **Bloky sdílené se snapshoty se vynechávají**
(přepsáním by se zdvojily). Velké soubory se zpracují po oknech (8 MiB),
další okno navazuje fyzicky za předchozím.

- `defrag_file(cesta)` - jeden soubor, vrací počet přepsaných bloků.
- `defrag_step(budget)` - krok na pozadí: projde další soubory, dokud
  nepřepíše `budget` bloků; `done` = průchod dokončen. fsd ho volá
  v nečinnosti po commitu, ale jen když od posledního průchodu přišly
  změny.

## Datasety a klony

> **Změna proti v2:** nové. v2 měla jeden strom pro celý svazek. **Proč:**
> jednotkou historie (snapshoty, klony, později commity a trezory) je
> složka - projekt, domov, systém - ne celý disk.
> ([všechny změny](fs-verze.md#přechod-v2---v3))

**Dataset** je složka s vlastním stromem: vlastní čísla objektů, vlastní
snapshoty a deadlisty. Pro uživatele je to obyčejná složka; v adresáři je
to položka druhu `KIND_DATASET` (`Kind::Dataset`) a cesta do ní prostě
přejde. Kořen svazku je **kořenový dataset** (číslo 1, stav v superbloku);
ostatní mají záznam `DatasetRecord` ve stromu kořenového datasetu.

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Datasety a klony">
<defs><marker id="dk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="200" height="60" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">kořenový dataset (1)</text><text class="m" x="110" y="64" text-anchor="middle">záznamy datasetů</text>
<rect class="box" x="280" y="20" width="200" height="60" rx="6"/><text class="t b" x="380" y="45" text-anchor="middle">/projekt (2)</text><text class="m" x="380" y="64" text-anchor="middle">vlastní strom</text>
<rect class="ok" x="280" y="120" width="200" height="50" rx="6"/><text class="t b" x="380" y="141" text-anchor="middle">snapshot S</text><text class="m" x="380" y="158" text-anchor="middle">clones = 1</text>
<rect class="warn" x="550" y="120" width="200" height="60" rx="6"/><text class="t b" x="650" y="145" text-anchor="middle">/zkouska (3, klon)</text><text class="m" x="650" y="164" text-anchor="middle">origin = (2, S, gen S)</text>
<path class="ln" d="M210 50 L278 50" marker-end="url(#dk1)"/>
<path class="ln" d="M380 80 L380 118" marker-end="url(#dk1)"/>
<path class="ln" d="M548 150 L482 145" marker-end="url(#dk1)"/>
<text class="m" x="10" y="215">Klon sdílí bloky dosažitelné z kořene S (narozené do generace S); přepsání je dá do deadlistu klonu,</text>
<text class="m" x="10" y="233">uvolnit je smí až S. Smazání S s klonem S jen skryje; skutečně se smaže se zánikem posledního klonu.</text>
</svg>
<figcaption>Dataset, jeho snapshot a klon s vazbou na původní snapshot.</figcaption>
</figure>

```rust
let id = fs.create_dataset("/projekt")?;          // nová složka s vlastní historií
fs.write("/projekt/cfg.toml", 0, b"a = 1")?;      // běžné operace beze změny
fs.snapshot_of("/projekt", "pred-refaktorem")?;   // snapshot jen tohoto datasetu
fs.clone_snapshot("/projekt", "pred-refaktorem", "/zkouska")?; // zapisovatelný klon
fs.write("/zkouska/cfg.toml", 0, b"a = 2")?;      // originál se nezmění
fs.delete_dataset("/zkouska")?;                   // uvolní jen bloky klonu
```

| Operace | Chování |
|---|---|
| `create_dataset(cesta)` | nový prázdný dataset; hned commit |
| `snapshot_of(dataset, jméno)`, `snapshots_of`, `delete_snapshot_of`, `snapshot_read_of/stat_of/list_of` | jako u kořene, pro dataset s kořenem `dataset` (`"/"` = kořenový; staré funkce bez `_of` = kořenový) |
| `clone_snapshot(dataset, snapshot, cesta)` | zapisovatelný klon (okamžitý, sdílí bloky); hned commit |
| `delete_dataset(cesta)` | smaže dataset i jeho snapshoty a uvolní bloky, které držel jen on; `NOT_EMPTY` s vnořeným datasetem, `BAD_STATE`, když z jeho snapshotu existuje klon |
| `datasets()` | seznam (číslo, cesta v živé verzi, původ klonu) |
| `stat` | pole `dataset` = číslo datasetu, ve kterém objekt leží |

**Pravidla:**

- **Přesun mezi datasety** (`rename`) = `NOT_SUPPORTED` - jsou to jiné stromy,
  šlo by o kopii dat. Přejmenovat nebo přesunout dataset uvnitř rodiče jde.
- `remove` datasetu = `WRONG_TYPE` (maže se `delete_dataset`);
  `create(…, Kind::Dataset)` = `INVALID_ARGS`.
- **Pohled snapshotu** do jiného datasetu nepřechází (`NOT_SUPPORTED`):
  snapshot kořene neví, co se v datasetu mezitím uvolnilo.
- Souhrny velikostí adresáře se na hranici datasetu zastaví (dataset se do
  rodiče nezapočítá).
- Operace s datasety (vytvoření, klon, smazání) hned commitují, proto
  nejsou v intent logu. Běžné změny ve více datasetech zapíše `fsync`
  do jedné dávky; záznamy každého datasetu začínají značkou datasetu
  (záznam logu 8).

**Kdo drží bloky naživu (klon).** Klon vzniká ze snapshotu S: kořen =
kořen S, hranice vlastnictví (`snapshot_generation`) = generace S, záznam
`origin` = (dataset, S, generace S). Strom klonu vychází z kořene S, takže
blok narozený do generace S je v klonu právě blok dosažitelný z S - klon ho
jen sdílí. Jeho přepsání v klonu jde do deadlistu klonu, ale uvolnit ho smí
až S. Blok narozený po vzniku klonu vlastní klon. Snapshot S drží počet
klonů; jeho smazání při klonech S jen **skryje** (`SNAP_HIDDEN`, dál drží
bloky) a skutečné smazání proběhne se zánikem posledního klonu. Mazání
vlastních snapshotů klonu bere jako spodní hranici generaci S.

**Smazání datasetu** projde jeho uložený strom a uvolní uzly a data
narozené po hranici (u klonu generace S); podstromy narozené dřív přeskočí
(sdílené s S). Vše v jednom commitu - výpadek během mazání nechá stav před,
nebo po (test `power_loss_during_dataset_deletion`).

**Implementace:** kód operací pracuje vždy s aktivním datasetem
(`Store::root`, `next_oid`, buffer dat, čekající změny logu); ostatní
načtené datasety leží v paměti zvlášť a `activate` je prohodí. `enter`
projde cestu a nechá aktivní dataset cíle. Commit zapíše strom a záznam
každého změněného datasetu a nakonec kořenový dataset - vše v jedné
transakci.

⚠️ **Omezení první verze:** přísná kontrola dvojího použití datového bloku ve fsck platí jen pro kořenový
dataset (klony sdílejí data legitimně); defragmentace na pozadí prochází jen
aktivní dataset; služba fsd a protokol v1 datasety zatím jen zobrazí (druh 4
ve výpisu), vytvářet je půjde s protokolem v2 nebo nástrojem `osfs`.

### Rozdíl verzí a obnova jako kopie

```rust
// Co se v projektu změnilo od snapshotu v1 (proti živé verzi):
for c in fs.diff("/projekt", "v1", None)? {
    println!("{:?} {} {:?}", c.kind, c.path, c.old_path); // Modified /cfg.toml None
}
// Co změnil klon proti snapshotu, ze kterého vznikl („zkusit bokem“):
let zmeny = fs.diff_clone("/projekt-bokem")?;
// Jeden soubor ze snapshotu vedle aktuálního:
fs.restore_copy("/projekt", "v1", "/cfg.toml", "/projekt/cfg (v1).toml")?;
```

| Funkce | Co dělá |
|---|---|
| `diff(dataset, from, to)` | rozdíl snapshotu `from` a snapshotu `to` (nebo živé verze, `to = None`); seřazený podle cesty |
| `diff_clone(klon)` | rozdíl klonu proti jeho původnímu snapshotu |
| `restore_copy(dataset, snapshot, cesta, cíl)` | obnoví soubor, odkaz nebo celou složku ze snapshotu jako kopii `cíl` (nesmí existovat); vnořené datasety přeskočí |
| `snapshot_readlink_of` | cíl odkazu ve snapshotu |

Druhy změn (`ChangeKind`): `Added`, `Removed`, `Modified` (obsah souboru
nebo cíl odkazu; u adresáře se nehlásí - jeho změny jsou změny položek),
`Moved` (jiná cesta, tentýž objekt - `old_path`), `MovedModified`.
Protože oid je v datasetu stabilní, **přesun se pozná přesně**, ne
heuristikou podle obsahu jako v gitu.

**Jak to funguje:** pro každou verzi se načte souhrn: objekt -> (druh,
velikost, otisk obsahu) a objekt -> (rodič, jméno). Otisk obsahu je součet
XXH3 přes položky dat (extenty se součty bloků, malý soubor, cíl odkazu),
takže se data nečtou. Při diffu proti živé verzi se nejdřív zapíše buffer
dat (bez commitu), aby diff viděl i neuložené změny.

⚠️ **Omezení první verze:** diff prochází oba stromy celé (cena podle
velikosti datasetu, ne změny) - plán: přeskočit podstromy se stejným
ukazatelem. Obnova jako kopie data opravdu kopíruje; okamžitá kopie přes
sdílené bloky přijde s okamžitými kopiemi souborů (fáze 3).

## Otevřené soubory (handly)

> **Změna proti v2:** nové. v2 pracovala jen s cestami. **Proč:** otevřený
> soubor musí přežít přejmenování, smazání i atomické nahrazení a vrstva
> Windows potřebuje sdílecí režimy a zámky rozsahu.
> ([všechny změny](fs-verze.md#přechod-v2---v3))

Handle drží **objekt** (dataset, oid, generace), ne cestu.

```rust
use fs::{access, share};

let a = fs.open("/report", access::READ, share::ALL)?;   // A čte report
fs.create("/report.tmp", Kind::File)?;
fs.write("/report.tmp", 0, b"nova verze")?;
fs.rename_replace("/report.tmp", "/report")?;            // B atomicky nahradí
let stare = fs.read_h(a, 0, 100)?;                        // A čte dál původní obsah
let b = fs.open("/report", access::READ, share::ALL)?;   // nové open = nový objekt
fs.close(a)?;                                             // původní objekt zmizí
```

| Funkce | Co dělá |
|---|---|
| `open(cesta, přístup, sdílení)` | otevře soubor nebo adresář (odkaz se nenásleduje: `WRONG_TYPE`); přístup `access::READ/WRITE/DELETE`, sdílení `share::READ/WRITE/DELETE/ALL` |
| `read_h`, `write_h`, `set_len_h`, `stat_h`, `list_h` | operace přes handle (kontroluje přístup, generaci a povinné zámky) |
| `close(h)` | zavře; s posledním handlem zmizí sirotek a provede se smazání při zavření |
| `rename_replace(z, do)` | atomické nahrazení: existující cíl (soubor za soubor, prázdný adresář za adresář) zmizí v téže operaci |
| `set_delete_on_close(h, ano)` | jméno zmizí se zavřením posledního handlu (vyžaduje `access::DELETE`) |
| `lock_range(h, od, délka, výhradní, povinný)`, `unlock_range` | zámek rozsahu (délka 0 = do konce souboru) |

**Životnost:**

- **Sirotek:** smazání posledního jména otevřeného objektu nesmaže data -
  záznam dostane rodiče 0 a položku `ORPHAN` (`META_OID`, druh 9). Kdo ho má
  otevřený, čte a zapisuje dál; s posledním `close` zmizí. Do souhrnů
  velikostí adresářů se nepočítá.
- **Po pádu nebo restartu** handly neplatí (jsou jen v paměti) a připojení
  sirotky smaže. Zápisy do sirotka v intent logu se při přehrání přeskočí
  (objekt už nemá záznam).
- **Generace:** handle ověřuje generaci objektu; objekt, který mezitím
  zanikl, = `BAD_HANDLE`.

**Sdílecí režimy** (jako Windows `FILE_SHARE_*`): nové otevření musí mít
požadovaný přístup sdílený všemi otevřenými handly a samo musí sdílet
jejich přístupy, jinak `LOCKED`. **Smazání a přejmenování** otevřeného
objektu vyžaduje, aby všechny handly sdílely mazání. Operace přes cestu
(`write`, `truncate`) se chovají jako otevření s daným přístupem.

**Zámky rozsahu** (jako `LockFileEx`): sdílený nebo výhradní; **poradní**
hlídá jen jiné zámky, **povinný** navíc brání zápisu (výhradní i čtení)
jiných handlů a cest. Patří handlu a mizí s jeho zavřením.

**Zámky POSIX** (`fcntl` `F_SETLK`/`F_SETLKW`/`F_GETLK` a `flock`,
zpráva `POSIX_LOCK`) **[ověřeno v QEMU]** (sada `cstd`, test `ctest lock`:
dva procesy nad jedním souborem): poradní zámky rozsahů s vlastníkem
**relace** (proces; `flock`: handle). Drží je fsd v paměti
(`libs/fs/src/posix_lock.rs`, host testy `cargo test -p fs --features std
posix_lock`), na disk se nezapisují.

- Vlastní zámky se nikdy nebijí: nový zámek v rozsahu nahradí starý
  (změna druhu, rozdělení), sousední stejného druhu se sloučí; odemčení
  části rozsah rozdělí. `F_GETLK` vrátí první cizí zámek v konfliktu.
- Uvolnění: zavření posledního handlu relace na souboru (`flock`: zavření
  handlu) a konec relace (konec procesu); restart fsd zámky zahodí
  (klient s handlem, který zámek měl, dostane `SERVICE_RESTARTED`).
- **Čekání** (`F_SETLKW`) vede klient opakovaným zkoušením (od milisekundy
  do padesáti milisekund mezi pokusy): blokující dotaz by držel jediný
  kanál relace a zastavil ostatní souborové operace procesu. Pokus s
  ohlášeným čekáním (`SET_WAIT`) si fsd zapamatuje (kdo čeká na koho,
  platnost jedna sekunda bez obnovení) a uzavřený cyklus čekání hlásí
  jako uváznutí (`EDEADLK`).
- `flock` a `fcntl` se navzájem neovlivňují (jako v Linuxu).

⚠️ **Omezení:** handly jsou v paměti jedné instance knihovny (služba,
ovladač) a po restartu neplatí; trvalé handly zatím nejsou. Zámky rozsahu
(`LOCK`) nečekají - konflikt hned vrátí `LOCKED`; čekání mají zámky POSIX
(výše).

⚠️ **Omezení:** otevření relativně k handlu adresáře protokol nemá, operace
jsou podle cesty. Funkce `openat`, `unlinkat`, `mkdirat`, `fstatat`
v [cstd](cstd.md#funkce-pro-c-krok-7a) proto skládají cestu adresáře
zapamatovanou při jeho otevření se jménem: nechrání před přejmenováním
adresáře mezi otevřením a voláním (Linux je váže na inode adresáře).
Oprava až s otevřením relativně k handlu adresáře ve fsd **[navrženo]**.

## Snapshoty

> **Změna proti v3 (fáze 1):** snapshot má metadata (položka `SNAPMETA`):
> druh (ruční, automatický, commit, pohled, bokem), čas vzniku a pozici
> žurnálu; `SnapshotInfo` je vrací. Dočasné pohledy (`VIEW`) ve výpisu
> nejsou. **Proč:** automatické snapshoty se prořezávají podle času a jen
> své, vracení operací hledá „poslední snapshot před operací“ podle
> žurnálu. ([Historie FS](fs-historie.md))

Snapshot je uložený kořen stromu v okamžiku vytvoření - jen pro čtení,
vytvoření je okamžité (jeden commit), nezabírá místo, dokud se živá data
nezmění.

```rust
fs.snapshot("pred-aktualizaci")?;                    // commit + záznam
for s in fs.snapshots()? { println!("{} {}", s.id, s.name); }
let st = fs.snapshot_stat("pred-aktualizaci", "/etc/config.toml")?;
let data = fs.snapshot_read("pred-aktualizaci", "/etc/config.toml", 0, 4096)?;
fs.delete_snapshot("pred-aktualizaci")?;             // uvolní, co držel jen on
```

Jména snapshotů platí stejná pravidla jako pro jména souborů (včetně
nezávislosti na velikosti písmen).

### Deadlisty: mazání bez procházení stromů

Bloky narozené nejpozději v generaci posledního snapshotu živá verze při
přepsání neuvolní, ale zapíše do **deadlistu** (seznam `L` = bloky
„zabité“ mezi předchozím snapshotem a snapshotem `L`; živá verze má seznam
`next_snapshot`). Algoritmus je stejný jako v ZFS:

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Deadlisty">
<path class="ln" d="M20 60 L740 60"/>
<circle class="acc" cx="140" cy="60" r="10"/><text class="t b" x="140" y="35" text-anchor="middle">P</text><text class="m" x="140" y="90" text-anchor="middle">předchozí</text>
<circle class="err" cx="380" cy="60" r="10"/><text class="t b" x="380" y="35" text-anchor="middle">S (mažeme)</text>
<circle class="acc" cx="620" cy="60" r="10"/><text class="t b" x="620" y="35" text-anchor="middle">N nebo živá verze</text>
<text class="m" x="260" y="90" text-anchor="middle">deadlist S</text><text class="m" x="500" y="90" text-anchor="middle">deadlist N</text>
<text class="t" x="20" y="125">1. Bloky v deadlistu N narozené po P držel jen S → uvolnit.</text>
<text class="t" x="20" y="147">2. Zbytek N zůstává; celý deadlist S se přesune do N.</text>
</svg>
<figcaption>Smazání snapshotu S: stačí projít jeden seznam, žádné počítání referencí.</figcaption>
</figure>

## Verzování a git

> **Změna proti v3 (fáze 1):** verzované commity s logickým ID, větve jako
> klony, trojcestný merge, verze souborů a rozšířené atributy jsou hotové.
> **Proč:** fáze 2 návrhu. ([Historie FS](fs-historie.md))

**Hotovo:** snapshoty a datasety dávají okamžité verze, zapisovatelné klony
a rozdíl verzí; fáze 2 přidala **commity** (autor, zpráva, rodiče, logické
ID BLAKE3 nezávislé na rozmístění bloků), **větve**, **merge**, verze
souborů, vracení operací a žurnál změn - viz [Historie FS](fs-historie.md).
Kontrolní součty v ukazatelích tvoří Merkleův strom - stejný princip, na
kterém stojí git.

**Plán** (viz [architektura](architektura.md#obsahově-adresovaný-fs-a-nativní-verzování-návrh)):

- A/B systémové aktualizace nad klony,
- rozdíl dvou verzí bez procházení celého stromu (porovnání součtů podstromů),
- vrstva pro git (fáze 5): ID objektů gitu jako rozšířené atributy,
  import a export repozitáře, push/pull po síti,
- indexy pro vyhledávání (jméno, typ, čas, velikost).

## Hodnoty z disku a operace „vše, nebo nic“

> **Změna proti v2:** nová kapitola. **Proč:** chyba softwaru se správným součtem mohla poškodit alokátor (dvojí uvolnění) nebo zacyklit procházení. ([všechny změny](fs-verze.md#přechod-v2---v3))

Součet v ukazateli chrání před poškozením média, ale ne před chybou
softwaru, která zapíše nesmyslnou hodnotu se správným součtem. Proto se
hodnoty z disku kontrolují i při použití:

| Hodnota | Kontrola | Při chybě |
|---|---|---|
| extent | celý rozsah uvnitř svazku, offset nepřeteče | `CORRUPTED` (žádné čtení ani uvolnění mimo) |
| uvolňovaný úsek (extent, deadlist, uzel) | uvnitř svazku, celý obsazený a nečekající na odložené uvolnění (`Space::free`, index odložených úseků) | neuvolní se, `CORRUPTED` a stav chyby (i `fsync` nic nepotvrdí) - dvojí uvolnění by poškodilo alokátor |
| obsah malého souboru | délka přesně rovná velikosti v záznamu | `CORRUPTED` |
| řetěz rodičů (souhrny, přesun) | nejvýš 65 536 kroků (`MAX_DEPTH`) | `CORRUPTED` místo zacyklení |
| záznam ve space logu | rozsah bez přetečení | připojení selže (`CORRUPTED`) |

**Operace „vše, nebo nic“** (`Fs::guarded`): veřejné změnové operace
(`create`, `symlink`, `remove`, `rename`, zápis do bufferu, `truncate`,
mazání snapshotu, defragmentace) počítají změny stavu v paměti.

- Selhání **před** první změnou (špatné jméno, neexistuje, málo místa,
  položka adresáře by se nevešla - ta se připraví předem) nic neohrozí.
- Selhání **po** změně (např. chyba čtení uzlu uprostřed) by nechalo
  operaci napůl hotovou. Svazek přejde do stavu chyby: žádný commit ani
  `fsync` ji nepotvrdí, všechny další změny vrací `IO_ERROR`. Znovu
  připojit = stav posledního commitu + intent log.

Plné vrácení v paměti (bez stavu chyby) by potřebovalo kopii
rozpracovaného stromu před každou operací; přijde s transakcemi pro
aplikace (fáze 5).

## Kontrola konzistence a scrub

> **Změna proti v3 (fáze 2):** kontroluje i kopie uzlů a extentů,
> tabulku sdílení (blok použitý víckrát musí být sdílený, záznam bez
> použitých bloků = chyba) a index deduplikace; scrub čte obě kopie
> a opravuje. Navíc **scrub na pozadí** po krocích. **Proč:** každá změna
> formátu musí jít ověřit. ([Data FS](fs-data.md#scrub))

> **Změna proti v3 (fáze 1):** kontroluje i struktury historie: souvislou
> řadu žurnálu a jeho stav, metadata snapshotů, koš (záznam ↔ položka
> v koši), počet atributů, index expirace proti atributům, paměť logického
> hashe. **Proč:** každá změna formátu musí jít ověřit. ([Historie FS](fs-historie.md#na-disku))

> **Změna proti v2:** kontroluje uložený stav i při připojení jen pro čtení, pevná paměť (bitové mapy), omezený počet hlášení, kontrola druhů objektů a cílů odkazů. **Proč:** při nejisté obnově je potřeba zjistit stav bez zápisu a velký nebo hodně poškozený svazek nesmí kontrolu zahltit. ([všechny změny](fs-verze.md#přechod-v2---v3))

`check(scrub)` kontroluje **uložený stav** (poslední commit na disku):

- zapisovatelný svazek: nejdřív commit, takže uložený stav = živý,
- **jen pro čtení** (nejistá obnova, zařízení bez flush): nic se nezapíše;
  kontroluje se uložená generace a metadata místa načtená znovu z disku.
  Změny přehrané z logu jen v paměti zkontrolované nejsou.

Výsledek říká, co kontroloval: `generation` (číslo generace) a
`live_not_checked` (v paměti jsou nezkontrolované změny). Projde uložený
strom, všechny snapshoty, metadata volného místa a intent log:

- ověří součty všech uzlů (a se `scrub = true` i všech bloků dat - čte celý
  obsah),
- každý použitý blok musí být podle alokátoru obsazený,
- žádný obsazený blok nesmí být „ztracený“ (nikdo na něj neukazuje),
- živý strom nesmí použít jeden blok dvakrát,
- druh v položce adresáře musí sedět se záznamem objektu a symbolický odkaz
  musí mít cíl s délkou rovnou velikosti (a nic jiného cíl mít nesmí).

Vrací `CheckReport` s počty bloků (stromu, dat, metadat), seznamem
problémů (prázdný = v pořádku; nejvýš 1000 popisů, celkový počet je
v `problems_total`), zkontrolovanou generací a `live_not_checked`. Testy ho
volají po každém scénáři.

**Paměť kontroly:**

| Část | Velikost | Závisí na |
|---|---|---|
| 3 bitové mapy (použité, data, viděné uzly) | 3 bity na blok: 1 TiB svazek = 96 MiB | velikosti svazku |
| mapa objektů (druh, velikost) a seznam položek adresářů | ~40 B na objekt + ~16 B na položku: milion souborů ~ 56 MiB | **počtu souborů** živé verze |
| popisy problémů | nejvýš 1000 | - |

Poškozený svazek s miliony chyb kontrolu nezahltí; velký počet souborů ale
paměť zvětší (plán: kontrola objektů po částech podle rozsahu oid). Uzel
nebo extent mimo svazek je problém v hlášení, ne pád. **Poškozený záznam
objektu nebo položka adresáře** je problém v hlášení - dřív se chyba
dekódování přeskočila a kontrola hlásila čistý svazek, přestože `stat`
vracel `CORRUPTED`.

Podobně se **extenty zpracovávají po dávkách** (256): zkrácení nebo smazání
obřího souboru (1 TiB = ~540 000 extentů se součty, ~4 GiB) nikdy nenačte
všechny extenty do paměti najednou.

## Výkon a režie

Naměřeno testem `tests/perf.rs` (paměťové zařízení = čistá cena FS bez
disku, release, jedno vlákno):

| Měření | Výsledek |
|---|---|
| sekvenční zápis 512 MiB po 1 MiB + commit | ~1,4 GiB/s, **1 souvislý úsek** |
| sekvenční čtení | ~2,7-3,0 GiB/s |
| režie metadat dat (stromu, extentů, součtů) | **0,42 %** kapacity |
| 20 000 souborů po 4 KiB (vytvoření + zápis + commit) | ~100 000 souborů/s |
| `stat` adresáře s 20 000 soubory (velikost podstromu) | 10-13 µs |

Spuštění: `cargo test -p fs --features std --release -- --ignored --nocapture perf`.

**Kde je limit zápisu** (rozbor po krocích na 512 MiB, release):

| Krok | Rychlost |
|---|---|
| kopírování paměti (memcpy) | ~10 GiB/s |
| součet XXH3-128 po 4 KiB | ~19 GiB/s |
| buffer odložené alokace (4 KiB `Vec` v `BTreeMap`) | ~4,3 GiB/s |
| skládání bloků do souvislého bufferu | ~5,7 GiB/s (dřív po bajtech 1,7 GiB/s) |

Limit je tedy v implementaci, ne ve formátu: největší cenu má buffer po 4 KiB
blocích. Plán: buffer po velkých úsecích (jedna alokace na souvislý zápis)
a zápis rovnou z bufferu klienta. **V běžícím systému** je dnes limit jinde:
protokol fsproto v1 a blockproto kopírují data zprávou kanálu a jsou
synchronní (jeden požadavek najednou). To řeší protokol v2 se sdílenou
pamětí a frontami požadavků.

## Služba fsd

> **Změna proti v2:** chyba odeslání odpovědi službu neukončí, `CHECK` jen v režimu `debug`, hlášení důvodu režimu jen pro čtení, podpis `v3`. **Proč:** pád služby by nechal svazek bez závěrečného commitu. ([všechny změny](fs-verze.md#přechod-v2---v3))

`user/fsd` drží jeden připojený svazek a obsluhuje klienta přes kanál.

- **Start:** init při startu přečte bloky 1-8 všech disků a službu spustí
  nad prvním, který má podpis `OSFS\0\0v3`. Startovní zpráva: kanál
  k ovladači disku a kanál pro klienta (oba `kind USER`). První zpráva na
  bootstrap kanálu je konfigurace: slova oddělená mezerou, `debug` povolí
  testovací operace.
- **Zařízení:** blok FS = 8 sektorů; souvislé úseky jdou jedním voláním
  ovladače.
- **Commit** každých 5 s (`COMMIT_INTERVAL`); `fsync` klienta je trvalý
  okamžitě přes intent log.
- **V nečinnosti** po commitu defragmentuje po krocích 256 bloků.
- **TRIM** zapnutý.
- **Konec:** když klient zavře kanál, služba udělá commit a skončí.
- **Chyba odeslání odpovědi** (např. odpověď se nevejde do zprávy) službu
  neukončí: místo odpovědi pošle `BUFFER_TOO_SMALL`. Končí jen při
  zavřeném kanálu - jinak by svazek zůstal bez závěrečného commitu.
- **`CHECK` jen v režimu `debug`** (dlouhá operace nad celým svazkem; v1
  nemá ověření klienta, s protokolem v2 půjde podle oprávnění).

Hlášky služby popisuje [hlasky.md](hlasky.md#služba-fsd-a-souborový-systém).

## Protokol fsproto

> **Změna proti v2:** nové operace `LIST_PAGE` a `SNAP_LIST_PAGE` (výpis po stránkách), `CHECK` vrací generaci a celkový počet, druh 3 = symbolický odkaz. **Proč:** velký adresář se nevešel do jedné zprávy a služba kvůli tomu končila. ([všechny změny](fs-verze.md#přechod-v2---v3))

Zprávy po kanálu (verze 1, synchronní, cestami). Požadavek: `op` u32 a
parametry v pořadí; odpověď: `status` i32 (0 nebo kód chyby), rezerva u32,
výsledek. Čísla little-endian, řetězec = délka u16 + UTF-8, data = zbytek
zprávy. Jedno čtení/zápis přenese nejvýš 32 KiB (`MAX_IO`); `FsClient`
větší požadavky rozdělí.

| op | Název | Parametry | Výsledek |
|---|---|---|---|
| 1 | `INFO` | - | jméno svazku, velikost bloku, bloky, volné, odložené, generace, algoritmus, návrat, přehráno |
| 2 | `STAT` | cesta | `Stat` (oid, druh, velikost, mtime, ctime, btime, vlastník, práva, souhrny podstromu, masky `known` a `hidden` u16, mód POSIX u16); čas = sekundy i64 + nanosekundy u32 |
| 3 | `LIST` | cesta | počet u32 + (druh u8, oid u64, jméno) - seřazené podle jména; druh 1 soubor, 2 adresář, 3 symbolický odkaz; `BUFFER_TOO_SMALL`, když se výpis do zprávy nevejde |
| 4 | `READ` | cesta, offset u64, délka u32 | data (kratší na konci souboru) |
| 5 | `WRITE` | cesta, offset u64, data | - |
| 6 | `CREATE` | cesta, druh u8 (1 soubor, 2 adresář; jiný = `INVALID_ARGS`) | - |
| 7 | `REMOVE` | cesta | - |
| 8 | `RENAME` | odkud, kam | - |
| 9 | `TRUNCATE` | cesta, velikost u64 | - |
| 10 | `COMMIT` | - | - |
| 11 | `FSYNC` | - | - |
| 12 | `SNAPSHOT` | jméno | číslo snapshotu u64 |
| 13 | `SNAPSHOTS` | - | počet u32 + (id, generace, jméno) |
| 14 | `DELETE_SNAPSHOT` | jméno | - |
| 15 | `SNAP_STAT` | snapshot, cesta | `Stat` |
| 16 | `SNAP_LIST` | snapshot, cesta | jako `LIST` |
| 17 | `SNAP_READ` | snapshot, cesta, offset, délka | data |
| 18 | `DEFRAG` | cesta | přepsané bloky u64 |
| 19 | `FRAGMENTS` | cesta | počet úseků u64 |
| 20 | `CHECK` | scrub u8 | generace u64, celkem problémů u64, nezkontrolované změny v paměti u8, počet u32 + popisy (kolik se vejde); jen s `debug` |
| 21 | `LIST_PAGE` | cesta, kurzor (jméno poslední položky předchozí stránky, prázdný = od začátku), nejvýš u32 | další u8 (1 = něco zbylo) + stránka jako `LIST` (kolik se vejde do 60 KiB) |
| 22 | `SNAP_LIST_PAGE` | snapshot, cesta, kurzor, nejvýš u32 | jako `LIST_PAGE` |
| 87 | `LIST_STAT_PAGE` | cesta, kurzor, nejvýš u32 | další u8 + počet u32 + (jméno, druh u8, oid u64, velikost u64, mtime, práva u8, souhrny podstromu, masky `known` a `hidden` u16) - kolik se vejde do 60 KiB; práva `LIST` a `TRAVERSE` k adresáři |
| 100 | `SIMULATE_POWER_LOSS` | - | přehrané operace u64; jen s `debug`, jinak `ACCESS_DENIED` |

`FsClient::list` a `snapshot_list` čtou po stránkách - tím je vyřešený
**limit jedné zprávy** (64 KiB), ne paměť:

- **Kurzor je jméno** poslední položky (porovnání bez ohledu na velikost
  písmen, stejné pořadí jako výpis). Změna adresáře během výpisu: položka,
  která existuje celou dobu, přijde právě jednou; nová za kurzorem se
  objeví, před ním ne; smazaná zmizí. Nikdy zdvojení ani vynechání
  existující položky (stránkování podle pořadového čísla by je mělo).
- ⚠️ **Paměť:** služba sestaví celý výpis jednou a drží ho pro další
  stránky (zahodí ho každá jiná operace); klient výsledek drží celý. Obojí
  roste s velikostí adresáře (~100 B na položku: 1 milion položek ~
  100 MiB na každé straně). Výpis bez držení celého seznamu přinese
  protokol v2 (kurzor ve stromu, klient zpracovává stránky postupně).

**`LIST_STAT_PAGE`** (`FsClient::list_stat` -> `Vec<StatEntry>`) je výpis
i s metadaty pro `ls -l`: klient nemusí posílat `STAT` na každou položku
(tisíce zpráv u velkého adresáře). Kurzor a stránkování jako `LIST_PAGE`.
Položky jsou tytéž jako u `LIST` (jen ty, ke kterým má volající nějaké
právo); druh a oid jako ve `LIST` (u vstupu do datasetu druh 4 a číslo
datasetu), velikost a mtime jako `STAT` položky. Protože metadata jsou
`STAT` položek, operace vyžaduje kromě `LIST` i `TRAVERSE` k adresáři (bez
něj by `STAT` položek nešel; jinak `ACCESS_DENIED`). Funguje pro relace v1
i v2 (stejná obsluha).

#### Dostupnost hodnot: známé, skryté, nedostupné

`Stat` i `StatEntry` končí dvěma maskami u16 se stejnými bity
(`fsproto::avail`) **[implementováno]**. Testy na hostiteli:
`cargo test -p fsproto` (kodek), `cargo test -p fs --features std avail`.

| bit | konstanta | pole |
|---|---|---|
| 0 | `SIZE` | velikost (u adresáře `tree_bytes`) |
| 1 | `MTIME` | `mtime` |
| 2 | `CTIME` | `ctime` |
| 3 | `BTIME` | `btime` |
| 4 | `OWNER` | `owner` |
| 5 | `TREE` | `tree_files`, `tree_dirs` |
| 6 | `RIGHTS` | `rights` |

- **`known`** = hodnota platí. Prázdná složka s právem `MANAGE` je známé
  0 B / 0 souborů / 0 složek.
- **`hidden`** = hodnota existuje, ale volající ji nesmí vidět; pole je 0.
  Souhrny podstromu adresáře bez práva `MANAGE` jsou `SIZE | TREE`
  v `hidden` (dřív jen nuly, nešly odlišit od prázdné složky).
- **Ani jeden bit** = nedostupná: FS ji nevede, nebo k druhu objektu
  nepatří. `TREE` u souboru a odkazu; u hranice vnořeného datasetu v klonu
  velikost, časy, vlastník i souhrny (`known` je jen `RIGHTS`).
- Kodek odmítne zprávu s `known & hidden != 0` nebo s neznámým bitem
  (`INVALID_ARGS`).
- `StatEntry` nese jen bity svých polí (`SIZE`, `MTIME`, `RIGHTS`, `TREE`);
  ostatní bity fsd vynuluje.

Na drátě je to +4 B na konci `Stat` (`Stat::SIZE`) i každé položky
`LIST_STAT_PAGE` (`StatEntry::FIXED`). Protokol má kvůli tomu verzi 3
(`HELLO`); starší tvar zprávy se nepřečte. Verze 4 přidala na konec `Stat`
mód POSIX u16 (bit `MODE` masky, [Mód POSIX](#mód-posix)); `StatEntry` ho
nemá.

> **Změna proti verzi 2 protokolu:** masky `known` a `hidden`. **Proč:**
> nula znamenala „nesmíš vidět“, „prázdné“ i „FS to nevede“ a klient
> (tabulky v shellu) to nerozlišil; teď ukáže skrytou a nedostupnou hodnotu
> jinak než nulu.

Příklad zprávy `READ "/a.txt", offset 0, 100 B`:

```text
04 00 00 00   06 00 2f 61 2e 74 78 74   00 00 00 00 00 00 00 00   64 00 00 00
op = 4        cesta (délka 6, "/a.txt")  offset = 0                délka = 100
```

### Protokol v2: relace, handly, podmíněné uložení

> **Změna proti v1:** v1 pracuje jen s cestami a má jednoho klienta.
> **Proč:** soubor musí přežít přejmenování a smazání, víc programů
> (editor, agent, synchronizace) potřebuje vlastní relace a zápis nesmí
> tiše přepsat cizí změnu. Operace v1 dál platí.
> ([verze formátu a protokolu](fs-verze.md#protokol-fsproto-v1---v2))

```rust
use fsproto::v2::{access, share, open_flags};

let editor = fs.connect()?;                                  // vlastní relace
let f = editor.open("/config.toml", access::READ, share::ALL, 0)?;
let text = editor.read_h(f.handle, 0, 4096)?;
editor.close(f.handle)?;
// ... uživatel upravuje ...
match editor.save_if("/config.toml", Some(f.revision), b"a = 3") {
    Ok(rev) => {}                         // uloženo, nová revize
    Err(Error::Conflict) => {}            // mezitím změnil někdo jiný: nic se nezměnilo
    Err(e) => return Err(e),
}
```

| op | Název | Parametry | Výsledek |
|---|---|---|---|
| 30 | `HELLO` | - | verze u32 (3), max_io u32 |
| 31 | `CONNECT` | - | nový kanál relace (handle ve zprávě) |
| 32 | `OPEN` | cesta, přístup u8, sdílení u8, příznaky u8 (`CREATE` 1, `EXCL` 2, `TRUNCATE` 4, `DIR` 8) | handle u64, `Stat`, `Revision` |
| 33 | `CLOSE` | handle | - |
| 34 | `READ_H` | handle, offset u64, délka u32 (nejvýš `MAX_IO` = 32 KiB; větší čtení klient dělá po 1 MiB přes `READ_VMO`) | data |
| 35 | `WRITE_H` | handle, offset u64, data | **přijato u32** |
| 36 | `SET_LEN` | handle, velikost u64 | - |
| 37 | `STAT_H` | handle | `Stat`, `Revision` |
| 38 | `LIST_H` | handle, kurzor, nejvýš u32 | další u8 + položky |
| 39 | `LOCK` | handle, od u64, délka u64 (0 = do konce), příznaky u8 (`EXCLUSIVE` 1, `MANDATORY` 2) | - |
| 40 | `UNLOCK` | handle, od, délka | - |
| 41 | `DELETE_ON_CLOSE` | handle, zapnout u8 | - |
| 42 | `RENAME_REPLACE` | z, do | - |
| 43 | `REPLACE_BEGIN` | cesta, má revizi u8, `Revision` | handle dočasného souboru |
| 44 | `REPLACE_COMMIT` | handle, vynutit u8 | `Revision` nového obsahu, nebo `CONFLICT` |
| 45 | `REPLACE_ABORT` | handle | - |

`Revision` = dataset u64, oid u64, generace u64, verze obsahu u64.

- **Relace:** `CONNECT` vrátí nový kanál; každá relace má vlastní handly
  (handle cizí relace = `BAD_HANDLE`). Zavření kanálu zavře všechny
  handly relace: sirotci zmizí, zámky se uvolní, dočasné soubory
  podmíněného uložení se smažou. Počet relací omezuje jen paměť: smyčka fsd
  čeká přes port (`libos::Waiter`), ne přes `OBJECT_WAIT_MANY`. Zavření
  hlavního kanálu (ze startovní zprávy) službu ukončí.
- **Zápis = přijato:** odpověď `WRITE_H` říká, kolik bajtů služba přijala
  (jsou v bufferu, trvalé až po `FSYNC`). Zápis rozdělený do více zpráv je
  víc samostatných zápisů; `FsClient::write_h` po první chybě nepokračuje
  a vrátí počet přijatých bajtů před ní.
- **Podmíněné uložení:** `REPLACE_BEGIN` založí dočasný soubor
  `.~jméno.N.tmp` ve stejném adresáři (se smazáním při zavření);
  `WRITE_H` do něj zapíše nová data; `REPLACE_COMMIT` cíl atomicky nahradí
  jen při očekávané revizi (`má revizi = 0` = cíl nesmí existovat), jinak
  `CONFLICT` a nic se nezmění - dočasný soubor zůstává (uložit vedle,
  potvrdit s `vynutit`, nebo `REPLACE_ABORT`). `FsClient::save_if` to
  udělá najednou. Kdo měl původní soubor otevřený, čte dál původní obsah.

Operace historie (žurnál a jeho sledování, pohled, koš, vracení operací,
verze, nastavení datasetu, commity, větve, merge, „bokem“, atributy,
expirace, vytvoření a smazání datasetu, rozdíl verzí, dataset cesty a čtení
snapshotu datasetu) mají čísla **54-86** a **88-92** - tabulka je
v [Historii FS](fs-historie.md#protokol-operace-54-92). Výpis s metadaty
`LIST_STAT_PAGE` má číslo 87 (modul `fsproto::op`, tabulka výše).

⚠️ **Omezení:** handly platí do restartu služby; po pádu a přehrání intent
logu může mít soubor jinou revizi (verze obsahu se do logu nepíše) - klient
pak dostane `CONFLICT` (bezpečný směr). Data se zatím kopírují zprávou (32
KiB na zprávu); sdílená paměť a fronty požadavků přijdou později.

## Oprávnění a identita relací

> **Změna proti v2:** v2 oprávnění neměla (kdo měl kanál, směl vše).
> **Proč:** soubory uživatelů, sdílení mezi nimi a aplikace, které vidí
> jen to, co jim uživatel dal. Formát je od v3 připravený (vlastník a číslo
> popisu v záznamu objektu); teď se používá.
> ([všechny změny](fs-verze.md#přechod-v2---v3))

<figure class="diagram">
<svg viewBox="0 0 760 270" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Identita relaci a opravneni">
<defs><marker id="pm1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="180" height="70" rx="6"/><text class="t b" x="100" y="45" text-anchor="middle">init / správce sezení</text><text class="m" x="100" y="64" text-anchor="middle">řídicí kanál fsd</text><text class="m" x="100" y="80" text-anchor="middle">(systém)</text>
<path class="ln" d="M190 55 L268 55" marker-end="url(#pm1)"/><text class="m" x="229" y="47" text-anchor="middle">CONNECT_AS</text>
<rect class="box" x="270" y="15" width="220" height="80" rx="6"/><text class="t b" x="380" y="38" text-anchor="middle">relace aplikace</text><text class="m" x="380" y="56" text-anchor="middle">identita: Alice (1000)</text><text class="m" x="380" y="72" text-anchor="middle">/app -> /home/alice/apps/editor</text><text class="m" x="380" y="88" text-anchor="middle">/vybrane/foto.jpg (jen čtení)</text>
<path class="ln" d="M490 55 L568 55" marker-end="url(#pm1)"/><text class="m" x="529" y="47" text-anchor="middle">cesta FS</text>
<rect class="ok" x="570" y="20" width="180" height="70" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">libs/fs</text><text class="m" x="660" y="64" text-anchor="middle">vlastník, granty,</text><text class="m" x="660" y="80" text-anchor="middle">dědičnost, viditelnost</text>
<path class="ln" d="M100 90 L100 150 L268 150" marker-end="url(#pm1)"/><text class="m" x="185" y="142" text-anchor="middle">GRANT / REVOKE</text>
<rect class="box" x="270" y="125" width="220" height="50" rx="6"/><text class="t" x="380" y="147" text-anchor="middle">dialog Otevřít (powerbox)</text><text class="m" x="380" y="165" text-anchor="middle">přidá relaci připojenou cestu</text>
<text class="m" x="10" y="215">Mimo připojené cesty = NOT_FOUND, „..“ = INVALID_ARGS. Uvnitř rozhodují oprávnění FS podle identity relace;</text>
<text class="m" x="10" y="233">objekt bez jakéhokoli práva je neviditelný (NOT_FOUND), s právem, ale bez potřebného = ACCESS_DENIED.</text>
<text class="m" x="10" y="251">Klient si identitu nevymyslí: dostane jen kanál od toho, kdo relaci založil.</text>
</svg>
<figcaption>Identita a jmenný prostor relace razí init; oprávnění vyhodnocuje FS.</figcaption>
</figure>

### Model

- **Kdo se ptá** (`Principal`): uživatel a jeho skupiny. Uživatel 0 je
  systém: kontroly se na něj nevztahují (nástroje na hostiteli, ovladač pro
  Windows, interní operace). Identitu FS dostane od volajícího
  (`Fs::set_principal`), služba fsd podle relace.
- **Vlastník** objektu má všechna práva. Nový objekt patří tomu, kdo ho
  vytvořil. Vlastníka mění jen systém (`set_owner`).
- **Práva:** *Vypsat* `LIST`, *Projít* `TRAVERSE`, *Číst* `READ`, *Zapsat*
  `WRITE`, *Vytvořit* `CREATE`, *Smazat* `DELETE`, *Spustit* `EXECUTE`,
  *Spravovat* `MANAGE` (oprávnění, snapshoty, datasety). Úrovně:
  **Zobrazit** `VIEW` = vypsat + projít + číst, **Upravovat** `EDIT` =
  zobrazit + zapsat + vytvořit + smazat, **Spouštět** `RUN` = zobrazit +
  spustit.
- **Granty:** popis oprávnění (`Acl`) = seznam „kdo -> práva“, **jen
  povolení**. Grant platí pro objekt sám (`THIS`), dědí se do potomků
  (`INHERIT`), nebo obojí. Příjemce je uživatel, skupina nebo `EVERYONE`
  (každý kromě systému).
- **Dědičnost:** objekt má zděděné granty rodiče a k nim své vlastní.
  Příznak `PROTECTED` dědičnost přeruší - soukromá složka uvnitř sdílené
  bez potřeby zákazů.
- **Viditelnost:** položka, ke které volající nemá žádné právo, se ve
  výpisu neobjeví a `stat` na ni vrátí `NOT_FOUND` (ne `ACCESS_DENIED` -
  neprozradí se ani jméno). Každý adresář po cestě potřebuje `TRAVERSE`.
  Souhrny podstromu (`tree_*`) vidí jen vlastník a `MANAGE`.

| Operace | Potřebné právo |
|---|---|
| `stat`, `revision` | jakékoli (viditelnost) |
| `list` | `LIST` |
| `read`, `readlink`, `fragments` | `READ` |
| `write`, `truncate`, `set_times` | `WRITE` |
| `create`, `symlink`, `create_dataset` | `CREATE` k rodiči |
| `remove` | `DELETE` k objektu |
| `rename`, `rename_replace` | `DELETE` ke zdroji (i k nahrazovanému cíli) + `CREATE` k cílovému rodiči |
| `open` | podle přístupu: `READ`, `WRITE`, `DELETE`, `EXECUTE` |
| `set_acl`, `set_noexec`, `snapshot_of`, `delete_snapshot_of`, `diff`, `clone_snapshot` (zdroj) | `MANAGE` |
| `delete_dataset` | `MANAGE` + `DELETE` |
| `snapshots_of`, čtení ve snapshotu | `READ` ke kořeni datasetu, uvnitř oprávnění **ze snapshotu** |
| `set_owner`, `check`, defragmentace | jen systém |

**Handly:** práva se ověří při `open`, další operace přes handle už se
nekontrolují (capability). Výpis přes handle (`list_h`) skrývá stejně jako
`list`.

**Snapshoty** mají oprávnění z doby, kdy vznikly: soubor, který autor
později skryl, je ve starém snapshotu vidět tomu, kdo ho tehdy viděl
a má `READ` ke kořeni datasetu.

### Na disku

- **Popis oprávnění** je položka `ACL` (druh 10, objekt 0, offset = číslo
  popisu) v datasetu: příznaky u32, počet grantů u16, rezerva u16, **počet
  odkazů u64**, granty (komu u64, práva u8, příznaky u8). Objekty na něj
  ukazují číslem (`Inode::acl`, 0 = jen zdědit), takže tisíce souborů se
  stejnými právy stojí jeden záznam.
- **Index** `ACL_INDEX` (druh 11, offset = otisk popisu) najde stejný popis
  při nastavení: granty se před uložením seřadí a sloučí, takže stejná
  práva = stejný popis. Poslední odkaz popis i položku indexu smaže.
- Příznak **`RO_COMPAT_ACL`** (ro_compat bit 0) se nastaví s prvním popisem:
  starší ovladač smí svazek jen číst (při mazání by nesnižoval počty odkazů).
- **Vlastnosti datasetu** `PROPS` (druh 12, objekt 0): příznaky u32, bit 0
  = **zákaz spouštění** (`NOEXEC`). `open` s `EXECUTE` pak selže pro
  každého, i pro systém - datasety s daty (stažené soubory, dokumenty)
  nemohou obsahovat spustitelné programy.
- **Intent log:** 10/11 = vytvoření/odkaz s vlastníkem (jen když vlastník
  není systém - svazky bez uživatelů zůstanou čitelné starším ovladačem),
  12 nastavení popisu, 13 změna vlastníka. Přežijí `fsync` a pád.
- **fsck** kontroluje, že popisy existují, počty odkazů sedí a index je
  úplný.

```rust
use fs::{Acl, Grant, Principal, grant, rights, acl_flags};

fs.set_owner("/home/alice", 1000)?;                    // systém
fs.set_principal(Some(Principal::user(1000, &[])));     // dál jako Alice
fs.set_acl("/home/alice/sdilene", Some(Acl {
    flags: 0,
    grants: vec![Grant { who: 5000, rights: rights::EDIT, flags: grant::BOTH }], // tým
}))?;
fs.set_acl("/home/alice/sdilene/soukrome", Some(Acl {
    flags: acl_flags::PROTECTED, grants: vec![],        // soukromá složka uvnitř sdílené
}))?;
```

### Relace ve fsd: identita a připojené cesty

- **Řídicí relace** = hlavní kanál ze startovní zprávy (má ho init, později
  správce sezení). Je systém a vidí celý FS.
- **`CONNECT_AS`** (jen řídicí relace) založí relaci uživatele: identita
  (uživatel, skupiny) a **připojené cesty** `jméno v relaci -> cesta ve FS`
  (příznak jen pro čtení). Vrátí číslo relace a kanál, který jde předat
  aplikaci. **Klient si identitu nevymyslí** - identita patří kanálu.
- **Jmenný prostor relace:** cesta se přeloží podle nejdelšího připojení na
  hranici složky; mimo připojení `NOT_FOUND`, `..` a `.` = `INVALID_ARGS`
  (z připojení nejde utéct). Odkazy FS nenásleduje, takže ani jimi ne.
- **`GRANT` / `REVOKE`** (jen řídicí) přidá nebo odebere relaci připojenou
  cestu - dialog Otevřít/Uložit (powerbox): aplikace dostane jen to, co
  uživatel vybral; stejným mechanismem dostane důvěryhodná aplikace
  pracovní složky bez dotazu
  ([Pracovní složky](navrh-fs-v3.md#pracovní-složky-a-chráněné-oblasti)
  **[navrženo]**).
- `CONNECT` z relace uživatele založí další relaci se stejnou identitou
  a cestami (ne řídicí).
- Operace nad celým kořenovým datasetem (snapshoty v1, kontrola,
  defragmentace) smí jen relace s připojením `/ -> /`.

| op | Název | Parametry | Výsledek |
|---|---|---|---|
| 46 | `CONNECT_AS` | uživatel u64, skupiny (u32 + u64…), připojení (u32 + jméno, cíl, příznaky u8) | číslo relace u64 + kanál |
| 47 | `GRANT` | relace u64, jméno, cíl, příznaky u8 | - |
| 48 | `REVOKE` | relace u64, jméno | - |
| 49 | `PERMISSIONS` | cesta | vlastník u64, práva u8, má popis u8, [popis] |
| 50 | `SET_ACL` | cesta, má popis u8, [popis: příznaky u32, počet u32, (komu u64, práva u8, příznaky u8)…] | - |
| 51 | `SET_OWNER` | cesta, vlastník u64 (jen řídicí) | - |
| 52 | `SET_NOEXEC` | kořen datasetu, zapnout u8 | - |
| 53 | `NOEXEC` | kořen datasetu | zákaz u8 |
| 94 | `ACL_VIEW` | cesta | jako `PERMISSIONS` + zděděné granty: počet u32, (komu u64, práva u8, původ str)…; původ = nejbližší složka výš, jejíž popis grant dědí (cesta v relaci, prázdná = mimo připojené cesty), pořadí od kořene; s `PROTECTED` prázdné |

`Stat` má navíc vlastníka u64 a účinná práva volajícího u8. `OPEN` zná
přístup `EXECUTE` (8).

**V shellu:** příkaz [`acl`](prikazy-shellu.md#acl) je nad `ACL_VIEW`
a `SET_ACL` relace uživatele terminálu: výpis (vlastník, vaše úroveň,
dědění, vlastní i zděděné granty), přidání a odebrání grantu. Úrovně jsou
masky `rights`, vzestupně a stejně jako karta Oprávnění v Souborech:
Zobrazit = `VIEW`, Spouštět = `RUN`, Upravovat = `EDIT | EXECUTE`,
Spravovat = vše; jiná maska se ukáže jako Vlastní s výčtem práv. Popis se
zapisuje celý (přečíst, změnit, zapsat), právo Spravovat vynucuje fsd.
Jména uživatelů (přihlašovací) dodá init z kanálu služby účtů `names`
(`NAMES`, stejný zdroj jako karta Oprávnění) na dotaz powerproto
`Accounts` kanálem napájení relace; známé skupiny `everyone` a `users`
zná shell sám, ostatní skupiny zatím jen číslem. **[ověřeno v QEMU]**

### Obnova po pádu fsd

> **Změna proti v3 (fáze 3):** nová kapitola. `fsd` běží pod supervisorem
> initu a po pádu se spustí znovu; klienti s lístkem pokračují (společný
> protokol [Obnova služeb po pádu](obnova-sluzeb.md)). Nové: operace
> `REOPEN` (118), příznak `CONNECT` „řídicí relace“, operace disku `SYNC`
> (`blockproto`, 6), volba připojení `keep_orphans`. Formát na disku beze
> změny. **Proč:** pád služby nesmí znamenat restart celého systému ani
> ztrátu otevřených souborů.

**Adaptér** (`user/init/src/fsd.rs`): konec kanálu k ovladači disku drží
supervisor (`Kept`), každá instance dostane duplikát. Konfigurace na
bootstrap: `start`, v testovacím běhu `debug`, po pádu `restart <generace>`.
Instance potvrdí start zprávou `ok`. Nová instance po pádu:

1. pošle disku `SYNC` s jedinečným číslem a zahodí všechno před jeho
   ozvěnou (pozdní odpovědi staré instance - blokový protokol nemá čísla
   požadavků),
2. připojí svazek s `keep_orphans`: sirotky (smazané, ale otevřené
   soubory) 30 s neuklidí, klienti je stihnou znovu otevřít; pak uklidí
   jen ty, které nikdo neotevřel,
3. čísluje handly od `generace << 40`, takže číslo handlu staré instance
   nikdy neukáže na cizí soubor nové instance.

**Recept** relace (kódování `libs/wire`):

| Druh | Pole | Relace |
|---|---|---|
| 0 systém | - | další řídicí relace (`CONNECT` s příznakem `CONTROL`) - drží ji init |
| 1 uživatel | `u64` uživatel, `u32` počet + `u64` skupiny, `u32` počet + připojené cesty (jméno, cíl, příznaky) | `CONNECT_AS` z řídicí relace adaptéru |

**Zúžení** (pro dítě, `NARROW` lístku): seznam připojených cest (`u32`
počet + cesty). Každá musí ležet v některé původní (stejný cíl nebo pod
ním) a původní jen pro čtení musí zůstat jen pro čtení; uživatel a skupiny
zůstávají. Z receptu systému se nezužuje. Jinak `ACCESS_DENIED`.

**Knihovna** (`FsClient` přes `svcclient::Conn`, `FsClient::with_lease`):

- každá operace má druh: čtení, `stat`, výpisy, `info`, stav, historie
  ke čtení se po pádu zopakují samy; ostatní vrátí `SERVICE_RESTARTED`
  (`OPEN` bez `CREATE`/`TRUNCATE` se opakuje),
- **trvalé handly**: aplikace dostává číslo handlu knihovny (jedinečné
  v procesu), knihovna u něj drží identitu objektu (dataset, oid, generace
  z `Revision`), přístup a sdílení. Po pádu ho před další operací otevře
  znovu (`REOPEN`): práva se ověří znovu podle identity relace, cesta musí
  ležet v připojených cestách relace; objekt s jinou generací =
  `NOT_FOUND`. Sirotka (bez cesty) smí znovu otevřít jen vlastník,
- čtení přes handle (`READ_H`, `STAT_H`, `LIST_H`) se pak zopakuje; zápis
  a ostatní operace s handlem vrátí `SERVICE_RESTARTED` (stará instance je
  mohla provést),
- handle se **zámkem**: zámky zanikly se starou instancí - první operace
  vrátí `SERVICE_RESTARTED` (aplikace zámek získá znovu), další projdou;
  „smazat při zavření“ se nastaví znovu sám,
- handle **podmíněného uložení** (dočasný soubor) po pádu neplatí:
  `SERVICE_RESTARTED`, pak `BAD_HANDLE`; aplikace uloží znovu.

| Op | Jméno | Pole | Výsledek |
|---|---|---|---|
| 31 | `CONNECT` | příznaky u32 (1 = řídicí, jen z řídicí relace) | nová relace (handle) |
| 118 | `REOPEN` | dataset u64, oid u64, generace u64, přístup u8, sdílení u8 | handle u64, `Stat`, `Revision` (jako `OPEN`) |
| 119 | `POSIX_LOCK` | handle u64, příkaz u8 (`TEST` 0, `SET` 1, `SET_WAIT` 2, `UNLOCK` 3), třída u8 (`FCNTL` 0, `FLOCK` 1), výhradní u8, od u64, délka u64 (0 = do konce), číslo procesu u32 | výsledek u8: `DONE` 0, `CONFLICT` 1 + výhradní u8, od u64, délka u64, číslo procesu držitele u32 (0 = neznámé); `DEADLOCK` 2 |
| 120 | `OPEN_STREAM` | handle u64, režim u8 (`READ` 0, `WRITE` 1, `APPEND` 2), pozice u64 | kanál proudu bajtů (handle ve zprávě) |
| 121 | `SOCKET_BIND` | cesta + handle kanálu posluchače | - (existující jméno `ALREADY_EXISTS`) |
| 122 | `SOCKET_CONNECT` | cesta, číslo procesu u32 + handle konce spojení | - (`CONNECTION_REFUSED`, plná fronta `SHOULD_WAIT`) |
| 123 | `WHOAMI` | - | uživatel u64 (0 = systém), počet u32, skupiny u64 |
| 124 | `SET_MODE` | cesta, mód u16 (9 bitů), rozšířit u8 | - (`MANAGE`; [Mód POSIX](#mód-posix)) |
| 125 | `SYMLINK` | cesta, cíl str (1-4096 B, bez NUL) | - (právo vytváření ve složce; [Symbolické odkazy](#symbolické-odkazy)) |
| 126 | `READLINK` | cesta | cíl str (`WRONG_TYPE`, když není odkaz) |
| 127 | `LSTAT` | cesta | `Stat` odkazu samotného |
| 128 | `REALPATH` | cesta | kanonická cesta relace (odkazy, `.`, `..` vyřešené) |
| 129 | `READ_VMO` | handle u64, offset u64, délka u32 (nejvýš `VMO_IO` = 1 MiB) + handle VMO klienta (`WRITE`, `MAP`) | přečteno u32; data od začátku VMO |
| 130 | `MAP_VMO` | handle u64, okno u64, příznaky u8 (`map_flags`: `WRITE` 1, `SHARED` 2) | velikost souboru u64, dataset u64, oid u64 + handle okna (`READ`, `MAP`, se zápisem i `WRITE`; kapacita `WINDOW` = 16 MiB); kvóta relace `NO_MEMORY` ([Soubory jako paměť](#soubory-jako-paměť)) |
| 131 | `UNMAP_VMO` | dataset u64, oid u64, okno u64 | - (relace okno uvolní, soubor se zapíše zpět) |
| 132 | `SYNC_VMO` | dataset u64, oid u64, trvale u8 | - (zápis zpět, `trvale` = i `FSYNC`); relace bez okna souboru `SERVICE_RESTARTED` |
| 133 | `ADOPT_VMO` | handle u64 (se zápisem), okno u64 + handle VMO staré instance (`WRITE`) | zapsaných stránek u32 (obnova po pádu fsd, [Soubory jako paměť](#soubory-jako-paměť)) |
| 134-136 | `MEDIA_ADD`, `MEDIA_REMOVE`, `MEDIA_LIST` | vyměnitelná média v `/media` | viz [Vyměnitelná média](media.md#operace-fsd) **[ověřeno v QEMU]** |

**`OPEN_STREAM`** **[ověřeno v QEMU]** (sada `cstd`, krok `posix2`:
výstup potomka `posix_spawn` do souboru a 200 000 bajtů souboru na jeho
stdin): soubor jako proud bajtů pro stdin/stdout potomka (blok parametrů
procesu nese jen kanály proudu, [cstd](cstd.md#procesy-posix_spawn-a-waitpid)).
Proud smí jen to, co handle klienta (čtení / zápis); fsd si otevře
vlastní handle téhož objektu (`reopen`), takže proud přežije zavření
relace klienta. Obsluhuje ho hlavní smyčka fsd vedle relací
(`user/fsd/src/stream.rs`): čtení posílá kusy po 60 KiB, dokud se fronta
kanálu nezaplní, pak zkouší znovu po 2 ms (kanál nemá signál
„zapisovatelný“) a na konci souboru kanál zavře; zápis bere zprávy
kanálu a zapisuje je od pozice (`APPEND` na konec). Nejvýš 16 proudů
(`stream::MAX_STREAMS`); smyčka čeká přes port, počet relací omezuje jen
paměť. Restart fsd proudy ukončí.

**Velká čtení (`READ_VMO`)** **[ověřeno v QEMU]** x86_64 (test „cteni
velkeho souboru: po 32 KiB a pres VMO" v sadě `vykon FS`): `FsClient::read_h`
nad `MAX_IO` čte po 1 MiB do objektu paměti klienta. Klient ho má namapovaný
trvale pro čtení (jeden na relaci), fsd ho na dobu kopie namapuje pro
zápis - kopie přes `VMO_WRITE`/`VMO_READ` (buffer jádra) byla řádově
pomalejší. Závislosti fsd na cestě každého požadavku (`libos`, `wire`,
`svcclient`, alokátor) jsou i v debug obrazu optimalizované
(`[profile.dev.package.*]` v `Cargo.toml`; přestavba po změně `libos` delší
o 13 s, `fs`/`fsd`/`fsproto` byly optimalizované už dřív).

| Čtení 8 MiB (x86_64, QEMU TCG, debug obraz) | Čas |
|---|---|
| po 32 KiB, před změnou (`34604355`) | 6829 ms |
| po 32 KiB, optimalizované závislosti fsd (`a313581a` + profil) | 1186 ms |
| přes VMO po 1 MiB, mapování ve fsd i klientovi (`a313581a` + mapování) | 130 ms |

**Uzly socketů `AF_UNIX`** **[ověřeno v QEMU]** (sada `cstd`, krok `unix`
testu `ctest`): `SOCKET_BIND` vytvoří objekt druhu 5 (jméno, vlastník,
práva jako soubor; data nemá) a přiřadí mu posluchače - kanál, do kterého
fsd při `SOCKET_CONNECT` zapíše přihlášení klienta (uživatel a skupina
z identity relace, ověřené fsd; číslo procesu od klienta) a handle konce
spojení. `SOCKET_CONNECT` chce právo zápisu na uzel. Mapa uzel ->
posluchač je jen v paměti fsd (`user/fsd/src/serve_sock.rs`): konec
posluchače nebo restart fsd = `CONNECTION_REFUSED`, uzel zůstane jako na
Unixu (smaže ho `unlink`, `bind` na existující jméno = `ALREADY_EXISTS`).
Socket v jádře ani v FS není: spojení je dvojice kanálů mezi procesy, fsd
jen zprostředkuje první setkání ([cstd](cstd.md#sockety-af_unix)).
**`WHOAMI`** vrátí identitu relace (C `getuid`, `getgroups`).

Relace s lístkem dnes: init (řídicí), `fwd` (`/config`). Relace pro
`dnsd`, `ntpd` (úložiště certifikátů) a `revocd` zatím lístek nemají -
po pádu `fsd` dostanou `SERVICE_UNAVAILABLE` (zapojení patří vláknu sítě).
Test: sada `obnova fsd` (`user/init/src/tests/fs_recovery.rs`, klient
`user/fsrtest`).

### Mód POSIX

**[ověřeno v QEMU]** x86_64 i aarch64 (krok `mode` testu `ctest`, sada
`cstd`; host testy `tests/mode.rs` a `perm::posix_mode_follows_grants`
v `libs/fs`). Rozhodnutí T1A koordinátora (15. 11. 2026, varianta A):
devět bitů `rwxrwxrwx` je jen **pohled na granty** (`libs/fs/src/mode.rs`),
FS bity neukládá.

| Třída | Odkud | Soubor | Adresář |
|---|---|---|---|
| user | vlastník (má vždy všechna práva) | `rw-` (spuštění určuje manifest a zákaz spouštění datasetu, ne bit) | `rwx` |
| group | granty ostatním příjemcům kromě `EVERYONE` (skupiny i jiní uživatelé) | `r` = `READ`, `w` = `WRITE`, `x` = `EXECUTE` | `r` = `LIST`, `w` = `CREATE`, `x` = `TRAVERSE` |
| other | grant `EVERYONE` | dtto | dtto |

- **Při vytvoření** (`mkdir(path, mode)` = `CREATE` s módem,
  `open(O_CREAT, mode)` = `OPEN` s příznakem `MODE`; mód po `umask`) mód
  zděděné granty nového objektu **jen zúží**, nikdy nerozšíří - ve stejném
  požadavku jako vytvoření, takže nikdo jiný objekt širší nevidí. Třída bez
  bitů ztratí grant celý (i práva mimo mód).
- **`chmod`** (`SET_MODE` s `rozšířit` = 1) potřebuje právo `MANAGE`
  (vlastník ho má vždy; bez něj `EPERM`) a smí i rozšířit: přidat práva
  existujícím grantům třídy a pro other grant `EVERYONE` jen pro objekt -
  vědomé sdílení (změna popisu jde do žurnálu FS jako každé `SET_ACL`).
  Bity group bez příjemce grantu zůstanou 0 (FS nemá „skupinu souboru“).
- **Změna** = vlastní popis s `PROTECTED`: zděděné granty se zkopírují
  zúžené (potomci adresáře je dál dědí), granty jen pro potomky
  (`INHERIT` bez `THIS`) mód nemění. Beze změny účinku se popis nezakládá.
- Bity setuid, setgid a sticky se ignorují; `chmod` symlinku neexistuje.

### Profily

Profil je **nastavení systému, ne formát** (hranice z návrhu: co musí platit
pro každý zápis, je ve FS; nastavení a rozhodnutí mimo). Používá ho správce
sezení přes `fsproto::perm::Profile`:

| | Standardní (výchozí) | Přísný | Volný |
|---|---|---|---|
| relace aplikace (`app_mounts`) | `/app` + vybrané položky | `/app` + vybrané položky | celý domov + `/app` |
| zapamatovat výběr (`remember_picks`) | ano | ne (vždy dialog) | ano |
| zákaz spouštění na datech (`noexec_on_data`) | ano | ano | ne |
| relace uživatele (`user_mounts`) | `/ -> /` a `/media` ([vyměnitelná média](media.md)), rozhodují oprávnění | stejně | stejně |

```rust
use fsproto::perm::{Mount, Profile, mount_flags};

let p = Profile::Standard;
let picked = [Mount::new("/vybrane/foto.jpg", "/home/alice/foto.jpg", mount_flags::READ_ONLY)];
let (id, app) = fs.connect_as(1000, &[], &p.app_mounts("/home/alice", "editor", &picked))?;
// ... uživatel později vybere složku:
fs.grant(id, &Mount::new("/dokumenty", "/home/alice/Dokumenty", 0))?;
```

⚠️ **Omezení:** souhrny velikostí podstromu nejsou počítané pro každého
uživatele zvlášť - kdo nemá `MANAGE`, uvidí 0. Položka datasetu ve výpisu
se hodnotí podle oprávnění jeho kořene. Audit (záznam přístupů) a kvóty
přijdou ve fázi 4. Ovladač pro Windows (`osfs mount`) běží jako systém:
připojený obraz je pro uživatele Windows celý jeho, oprávnění se zachovají,
ale nevynucují.

## Chybové kódy

Význam kódů popisuje [chyby.md](chyby.md). Kdy je vrací souborový systém:

| Kód | Kdy |
|---|---|
| `NOT_FOUND` | cesta, rodič nebo snapshot neexistuje; při připojení: disk nemá superblok |
| `ALREADY_EXISTS` | `create`/`rename`/`snapshot` na existující jméno (bez ohledu na velikost písmen) |
| `NOT_A_DIRECTORY` | součást cesty nebo cíl `list` je soubor |
| `IS_A_DIRECTORY` | `read`/`write`/`truncate` adresáře |
| `NOT_EMPTY` | `remove` neprázdného adresáře |
| `INVALID_ARGS` | relativní cesta, neplatné jméno nebo cíl odkazu, přesun adresáře do vlastního podstromu, formátování zařízení menšího než 64 bloků |
| `OUT_OF_RANGE` | zápis nebo `truncate` za `MAX_FILE_SIZE` (2^63-1 B), offset + délka přeteče |
| `WRONG_TYPE` | `read`/`write`/`truncate` symbolického odkazu, `readlink` něčeho, co není odkaz |
| `NO_RESOURCES` | položka adresáře by s kolidujícími jmény přerostla `MAX_VALUE` |
| `NO_SPACE` | zápis nebo vytvoření by spotřebovalo rezervu commitu |
| `CORRUPTED` | nesouhlasí kontrolní součet bloku nebo struktura nedává smysl; u čtení dat = poškozený blok (ostatní soubory jdou číst dál) |
| `IO_ERROR` | chyba zařízení; po selhání commitu nebo zápisu logu ve `fsync` všechny další změny (nutné znovu připojit) |
| `NOT_SUPPORTED` | neznámá verze formátu nebo algoritmu, neznámý bit `incompat` v nejnovější generaci, uzel nebo extent s neznámým příznakem; neznámá operace protokolu |
| `ACCESS_DENIED` | `SIMULATE_POWER_LOSS` bez režimu `debug` |
| `READ_ONLY` | změna na svazku připojeném jen pro čtení (zařízení bez záruky trvanlivosti, neznámý bit `ro_compat`) |

## Testy

| Test | Co ověřuje |
|---|---|
| `tests/basic.rs` | soubory, adresáře, jména, souhrny, chyby, štěpení stromu, opakované použití místa, plný disk, snapshoty, odhalení poškození |
| `tests/features.rs` | souvislý zápis, fsync + přehrání, návrat o generaci, mazání snapshotů, defragmentace, TRIM, víc AG + BLAKE3, zhušťování logů |
| `tests/model.rs` | property test: náhodné sekvence (vytvoření, zápis, zkrácení, mazání, přesun, commit, fsync, „pád“, snapshoty) proti modelu v paměti; na konci fsck |
| `tests/handles.rs` | handly: čtenář po atomickém nahrazení čte původní objekt, smazaný otevřený soubor do zavření (i úklid po pádu a přehrání logu), sdílecí režimy, smazání při zavření, zámky rozsahu, handly přes datasety a přejmenování |
| `tests/versions.rs` | první ukázka: snapshot, klon, změny, diff (i přesun), obnova souboru a složky jako kopie, totéž s pádem uprostřed zápisu |
| `tests/datasets.rs` | datasety: vlastní stromy, pravidla, snapshoty, dva klony z jednoho snapshotu po jeho smazání, snapshoty klonu, fsync přes víc datasetů, výpadek během mazání, účtování místa |
| `tests/crash.rs` | výpadek během commitu i fsync s náhodnou podmnožinou nezapsaných bloků |
| `tests/mount.rs` (doplněno) | nečitelný nejnovější slot = jen pro čtení a nejnovější generace přežije; nečitelná dávka logu neztratí potvrzený fsync; `check` jen pro čtení bez zápisu |
| `tests/ondisk.rs` | poškozené hodnoty se správným součtem: extent mimo svazek, uvolnění volných bloků, délka malého souboru, cyklus rodičů a vrácení operace |
| `tests/errors.rs` | chyby zařízení: selhaný zápis dat neztratí data a jde zopakovat, přepsání nenechá díru, po selhaném flush nic dalšího nepotvrdí |
| `tests/mount.rs` | výběr superbloku: nečitelný slot, přechodná chyba bez návratu, neznámý `incompat` a novější verze = odmítnutí, neznámý `ro_compat` = jen pro čtení bez zápisu, nesmyslné hodnoty = návrat se hlášením, vynulování novějších slotů, zařízení bez záruky trvanlivosti |
| `tests/objects.rs` | formát v3: záznam objektu, jména do 1024 znaků, zákaz znaků směru textu, symbolické odkazy (i přes intent log), limit velikosti souboru a saturace souhrnů, generace objektu |
| `tests/journal.rs`, `tests/trash.rs`, `tests/history.rs`, `tests/commits.rs` | historie (fáze 2) - viz [Historie FS](fs-historie.md#testy) |
| `tests/perf.rs` | výkon a režie (spouští se ručně) |
| init `tests/fs.rs` | 11 testů v systému přes fsd a skutečný ovladač virtio-blk, včetně souboru zapsaného na hostiteli a simulace výpadku |
| init `tests/fs_hist.rs` | historie v systému přes fsd: žurnál a `JOURNAL_WATCH`, koš, commit, větev a merge, „bokem“, atributy, fsck |

Počet náhodných případů: `PROPTEST_CASES=150000 cargo test -p fs --features std --release`
(ověřeno bez chyby).

## Omezení a plán

- Jedna služba = jeden svazek a jeden klient; víc klientů a připojení
  více svazků přijde se službou jmenného prostoru.
- Protokol v1 kopíruje data zprávou a je synchronní; plán: sdílená paměť a
  fronty požadavků (asynchronní I/O podle návrhu).
- Práva a vlastníci souborů zatím nejsou (přijdou se správou uživatelů).
- Cache uzlů se při zaplnění (8192 uzlů) celá vyprázdní; plán: LRU.
- Výměna verze tabulek Unicode zatím nepřepočítá hashe jmen.
- Komprese (lz4/zstd) a šifrování: plán.
