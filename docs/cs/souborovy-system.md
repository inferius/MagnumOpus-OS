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
> je první část fáze 1: uzly 16 KiB, ukazatel, superblok a záznam objektu v3,
> jména do 1024 znaků, symbolické odkazy a limit velikosti souboru. Svazek
> formátu v2 se nepřipojí (`NOT_SUPPORTED`); data se zatím nepřevádějí,
> disky z nástroje `xtask` se vytvářejí znovu.

## Co formát obsahuje a co ovladač používá

Formát v3 má místo i pro věci, které ovladač zatím neumí. Aby nebylo
sporné, co funguje:

| Položka | Ve formátu | Ovladač dnes |
|---|---|---|
| UUID svazku, příznaky vlastností | ano | **používá** (žádný bit vlastnosti zatím není definovaný) |
| číslo zařízení v `BlockPtr`, `device_index/count` v superbloku | ano | **jen jedno zařízení**: zařízení musí být 0 a počet 1, jinak `CORRUPTED`; víc disků (zrcadlo, RAID) až po fázi 5 |
| příznaky `BlockPtr` (komprese, šifrování, sdílení, cloud) | vyhrazené | **nepoužívá**; uzel s příznakem = `NOT_SUPPORTED` |
| příznaky extentu (předalokace) | vyhrazené | **nepoužívá**; extent s příznakem = `NOT_SUPPORTED` |
| příznak objektu `INODE_REMOTE` | vyhrazený | **nepoužívá** (fáze 6) |
| oprávnění (`acl`), vlastník, čas přístupu, počet rozšířených atributů v záznamu objektu | ano | **zapisuje 0**, nečte (fáze 1d a později) |
| generace objektu, čas vytvoření | ano | **používá** (`Stat`, přehrání logu) |
| symbolické odkazy, jména do 1024 znaků, limit velikosti | ano | **používá** |
| datasety, klony, handly, protokol v2 | ne (přijde) | plán: [návrh v3](navrh-fs-v3.md#pořadí-implementace) |

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
| Symbolické odkazy | hotovo (knihovna; v protokolu jen výpis) | [Symbolické odkazy](#symbolické-odkazy) |
| Soubory až 2^63-1 B, kontrolovaná aritmetika | hotovo | [Limity](#limity-formátu) |
| UUID svazku a příznaky vlastností (compat/ro/incompat) | hotovo | [Superblok](#superblok) |
| Snapshoty jen pro čtení a jejich mazání (deadlisty) | hotovo | [Snapshoty](#snapshoty) |
| Kontrola konzistence (fsck) a ověření dat (scrub) | hotovo | [Kontrola](#kontrola-konzistence-a-scrub) |
| Zapisovatelné klony, nativní git, indexy, vyhledávání | plán | [Verzování a git](#verzování-a-git) |

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
| 80 | 40 | root | `BlockPtr` kořene B+stromu |
| 120 | 40 | space | `BlockPtr` indexu tabulky AG |
| 160 | 8 | zil_block | první blok intent logu této epochy |
| 168 | 8 | zil_seed | semínko součtů intent logu |
| 176 | 8 | next_oid | další volné číslo objektu |
| 184 | 8 | snapshot_generation | generace posledního snapshotu |
| 192 | 8 | next_snapshot | číslo dalšího snapshotu (= seznam mrtvých bloků živé verze) |
| 200 | 8 | dead_seq | pořadí další položky deadlistu |
| 208 | 4 | unicode_version | verze tabulek Unicode pro jména |
| 212 | 2 | device_index | toto zařízení ve svazku (zatím 0) |
| 214 | 2 | device_count | počet zařízení svazku (zatím 1) |
| 216 | 32 | label | jméno svazku (UTF-8, doplněné nulami) |
| 4080 | 16 | csum | XXH3-128 bajtů 0..4080 |

**Příznaky vlastností** fungují jako v ext4: každý bit = jedna vlastnost
formátu (komprese, šifrování, ...). Ovladač, který bit z `ro_compat` nebo
`incompat` nezná, svazek odmítne s `NOT_SUPPORTED` - nikdy ho tak nepoškodí
tím, že by nerozuměl novým strukturám. Zatím je definovaných nula bitů.

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

### Ukazatel (`BlockPtr`, 40 B)

> **Změna proti v2:** z 32 B na 40 B: číslo zařízení (16 bitů), délka v blocích a příznaky. **Proč:** uzel má 4 bloky a formát je připravený na víc disků a na kompresi, šifrování a sdílené bloky (poznají se přímo na ukazateli). **Ovladač zatím** umí jen jedno zařízení (číslo 0) a žádný příznak - viz [Co formát obsahuje](#co-formát-obsahuje-a-co-ovladač-používá). ([všechny změny](fs-verze.md#přechod-v2---v3))

| Offset | Velikost | Pole |
|---|---|---|
| 0 | 8 | adresa: číslo bloku (bity 0-47) \| zařízení svazku (bity 48-63) |
| 8 | 8 | birth (generace vzniku) |
| 16 | 16 | kontrolní součet obsahu (všech bloků, na které ukazuje) |
| 32 | 4 | délka v blocích (uzel stromu = 4) |
| 36 | 2 | příznaky: 1 = komprimováno, 2 = šifrováno, 4 = sdílené bloky, 8 = jen v cloudu (rezervované; uzel s příznakem se odmítne `NOT_SUPPORTED`) |
| 38 | 2 | rezerva (nuly) |

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
| `SNAPSHOT` | 5 | číslo snapshotu (objekt 0) | `BlockPtr` kořene, generace u64, jméno |
| `DEADLIST` | 6 | seznam << 32 \| pořadí (objekt 0) | až 80 mrtvých úseků (start u64, délka u64, birth u64) |
| `SYMLINK` | 7 | 0 | cíl symbolického odkazu (UTF-8, 1-4096 B) |

### Záznam objektu (inode, 128 B)

> **Změna proti v2:** z 64 B na 128 B: čas vytvoření a přístupu, generace, oprávnění, vlastník, rozšířené atributy, příznaky. **Proč:** oid + generace je stabilní identita pro handly; pro oprávnění a atributy už nebude potřeba měnit formát. ([všechny změny](fs-verze.md#přechod-v2---v3))

| Offset | Pole | Význam |
|---|---|---|
| 0 | kind u8 | 1 = soubor, 2 = adresář, 3 = symbolický odkaz |
| 1 | flags u8 | příznaky objektu, rezervované: 1 = data jen v cloudu (zatím 0; nastavení synchronizace jsou atributy `sync.*`, ne bity) |
| 4 | acl u32 | číslo popisu oprávnění (0 = zdědit; přijde ve fázi 1d) |
| 8 | size | soubor: velikost v bajtech (nejvýš 2^63-1); odkaz: délka cíle |
| 16 | mtime | čas poslední změny obsahu |
| 24 | ctime | čas změny metadat |
| 32 | btime | čas vytvoření |
| 40 | atime | čas posledního přístupu (0 = nesleduje se) |
| 48 | parent | oid rodičovského adresáře (každý objekt má právě jednoho rodiče) |
| 56 | generation | generace (txg), ve které objekt vznikl |
| 64 | tree_bytes | adresář: součet velikostí souborů v celém podstromu (saturuje) |
| 72 | tree_files | adresář: počet souborů a odkazů v podstromu |
| 80 | tree_dirs | adresář: počet adresářů v podstromu (bez sebe) |
| 88 | owner | vlastník (0 = systém; přijde se správou uživatelů) |
| 96 | xattrs u32 | počet rozšířených atributů |
| 100 | - | rezerva 28 B (nuly) |

Časy jsou v nanosekundách z `Fs::set_time` (služba fsd: od startu systému,
nástroj `xtask`: od roku 1970). **Generace**
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
      vnitřní (64 B):  klíč 24 B, BlockPtr potomka 40 B
    hodnoty listu leží od konce uzlu směrem dolů
```

Limity: hodnota nejvýš 8144 B (`MAX_VALUE`, zaručuje, že přeplněný list
jde vždy rozdělit), vnitřní uzel nejvýš 255 potomků. Úroveň je uložená
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
  cesty dá `NOT_A_DIRECTORY`, `read`/`write` odkazu `WRONG_TYPE`. Následování
  (s limitem hloubky proti smyčkám) bude dělat vrstva nad ním (VFS/fsd).
- Odkaz se v souhrnech adresáře počítá jako soubor s velikostí 0.
- `symlink` se zapisuje do intent logu (záznam 7), takže přežije `fsync`
  i bez commitu.
- Protokol fsproto v1 odkaz jen ukáže (`kind::SYMLINK` ve `STAT` a
  `LIST`); vytváření přes službu přijde s protokolem v2.

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
| `birth == txg` a nikdo na něj neodkazuje z intent logu | volný **hned** | žádný uložený stav na něj neukazuje |
| `birth > snapshot_generation` | volný po **odložení** (`REUSE_DELAY` = 4 commity) | předchozí uložené stavy na něj mohou ukazovat (návrat o generaci) |
| `birth ≤ snapshot_generation` | do **deadlistu** | drží ho snapshot; uvolní se se smazáním snapshotu |

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
přečtená (`newest`) by mohla ležet **jen ve slotu pro `newest + 1`**. Když
je nečitelný (chyba I/O, ne špatný součet) právě ten, nejnovější stav nejde
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
seznam extentů se součty).

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

## Snapshoty

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

**Hotovo:** snapshoty dávají okamžité verze celého svazku a čtení
libovolného souboru z libovolné verze. Kontrolní součty v ukazatelích tvoří
Merkleův strom - stejný princip, na kterém stojí git.

**Plán** (viz [architektura](architektura.md#obsahově-adresovaný-fs-a-nativní-verzování-návrh)):

- zapisovatelné klony snapshotů (větve) a A/B systémové aktualizace,
- rozdíl dvou verzí bez procházení celého stromu (porovnání součtů podstromů),
- s BLAKE3 obsahové adresování: deduplikace a mapování na objekty gitu
  (blob/tree) - git repozitář bez duplikace dat v `.git`,
- programový přístup k historii souboru a rozšířené atributy,
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
| 2 | `STAT` | cesta | `Stat` (oid, druh, velikost, mtime, ctime, souhrny podstromu) |
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

Příklad zprávy `READ "/a.txt", offset 0, 100 B`:

```text
04 00 00 00   06 00 2f 61 2e 74 78 74   00 00 00 00 00 00 00 00   64 00 00 00
op = 4        cesta (délka 6, "/a.txt")  offset = 0                délka = 100
```

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
| `tests/crash.rs` | výpadek během commitu i fsync s náhodnou podmnožinou nezapsaných bloků |
| `tests/mount.rs` (doplněno) | nečitelný nejnovější slot = jen pro čtení a nejnovější generace přežije; nečitelná dávka logu neztratí potvrzený fsync; `check` jen pro čtení bez zápisu |
| `tests/ondisk.rs` | poškozené hodnoty se správným součtem: extent mimo svazek, uvolnění volných bloků, délka malého souboru, cyklus rodičů a vrácení operace |
| `tests/errors.rs` | chyby zařízení: selhaný zápis dat neztratí data a jde zopakovat, přepsání nenechá díru, po selhaném flush nic dalšího nepotvrdí |
| `tests/mount.rs` | výběr superbloku: nečitelný slot, přechodná chyba bez návratu, neznámý `incompat` a novější verze = odmítnutí, neznámý `ro_compat` = jen pro čtení bez zápisu, nesmyslné hodnoty = návrat se hlášením, vynulování novějších slotů, zařízení bez záruky trvanlivosti |
| `tests/objects.rs` | formát v3: záznam objektu, jména do 1024 znaků, zákaz znaků směru textu, symbolické odkazy (i přes intent log), limit velikosti souboru a saturace souhrnů, generace objektu |
| `tests/perf.rs` | výkon a režie (spouští se ručně) |
| init `tests/fs.rs` | 11 testů v systému přes fsd a skutečný ovladač virtio-blk, včetně souboru zapsaného na hostiteli a simulace výpadku |

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
