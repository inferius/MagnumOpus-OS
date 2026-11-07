# Data souborového systému (fáze 3)

Fáze 3 formátu v3 („data“, viz [návrh FS v3](navrh-fs-v3.md#pořadí-implementace))
přidává: **kompresi** po kusech, **okamžité kopie** (sdílené bloky),
**deduplikaci** na pozadí i při zápisu, **automatické díry**, **metadata
2x se samoléčením**, volitelně **data 2x**, **scrub na pozadí**
a **předalokaci**. Obecný formát popisuje [souborovy-system.md](souborovy-system.md),
změny formátu shrnuje [Verze formátu](fs-verze.md#fáze-3-data-incompat_ptr_copy-incompat_data).

> **Změna proti v3 (fáze 2):** nová kapitola. Ukazatel má 48 B (adresa
> druhé kopie), extent nese příznaky (nezapsaný, sdílený, kopie)
> a kompresi, deadlist adresu kopie; nové položky stromu 24-29, příznaky
> `INCOMPAT_PTR_COPY` a `INCOMPAT_DATA`, operace protokolu 110-117.
> **Proč:** odolnost (kopie, samoléčení, scrub) a úspora místa (kopie bez
> kopírování, deduplikace, díry) musí platit pro každý zápis a přežít
> pád, takže patří do FS; kdy scrubovat a deduplikovat, rozhoduje služba.

| Co | Vrstva | Kód |
|---|---|---|
| zápis po kusech, komprese, díry, kopie dat, zápis na místo | invariant (FS) | `libs/fs/src/fs/blocks.rs`, `libs/fs/src/compress.rs` |
| tabulka sdílení (počty odkazů) | invariant (FS) | `libs/fs/src/fs/sharing.rs` |
| okamžité kopie | mechanismus (FS) | `libs/fs/src/fs/reflink.rs` |
| deduplikace (index, krok na pozadí, při zápisu) | mechanismus (FS) | `libs/fs/src/fs/dedup.rs` |
| předalokace, nastavení dat datasetu | mechanismus (FS) | `libs/fs/src/fs/dataopt.rs` |
| scrub | mechanismus (FS) | `libs/fs/src/fs/scrub.rs` |
| druhá kopie uzlů, čtení se samoléčením | invariant (FS) | `libs/fs/src/store.rs` |
| komprese, kopie, režim deduplikace | nastavení (data datasetu) | `datarec::DataSettings` |
| kdy defragmentovat, deduplikovat, scrubovat | rozhodnutí (služba) | `user/fsd/src/main.rs` |

```rust
use fs::{Compress, DataSettings, DedupMode, Fs, Kind, MemDevice};

let mut fs = Fs::mount(Fs::format(MemDevice::new(65536), &Default::default())?)?;
// Komprese je ve výchozím stavu vypnutá: zapnout pro dataset.
fs.set_data_settings("/", &DataSettings { compress: Some(Compress::Lz4), copies: 1, dedup: DedupMode::Background })?;
fs.create("/velky.log", Kind::File)?;
fs.write("/velky.log", 0, &data)?;
// Okamžitá kopie: sdílí bloky, hotová v čase úměrném počtu extentů.
fs.clone_path("/velky.log", "/kopie.log")?;
// Předalokace: 1 GiB souvislého místa, čte se jako nuly.
fs.create("/db", Kind::File)?;
fs.fallocate("/db", 0, 1 << 30, false)?;
// Scrub po krocích (služba ho volá mezi požadavky).
fs.scrub_start(now)?;
while !fs.scrub_step(256, now)? {}
```

Přes službu fsd totéž dělá `FsClient` (`fsproto::data`), viz
[Protokol](#protokol-operace-110-117).

## Výchozí stav a cena

Výchozí stav řídí pravidlo „co zpomaluje, je volba“:

| Funkce | Výchozí | Cena | Úspora |
|---|---|---|---|
| komprese (LZ4 / Zstd) | **vypnuto**, volba datasetu | LZ4 v jednom vlákně ~400-700 MB/s zápis, ~2-4 GB/s čtení; na NVMe zpomalí dlouhý zápis až ~6x, náhodné čtení 4 KiB rozbalí až 128 KiB | ~5-15 % na disku s médii, ~30-50 % systém, kód, databáze |
| automatické díry (nulové bloky) | **zapnuto** | porovnání bloku s nulou při zápisu (zanedbatelné) | soubory s nulami (obrazy, předpřipravené soubory) |
| okamžité kopie | vždy k dispozici | žádná (jen při kopii) | celá kopie |
| deduplikace na pozadí | **zapnuto** | čtení nových dat při nečinnosti; u XXH3 porovnání bajtů | duplicitní soubory, zálohy, kopie bez reflinku |
| deduplikace při zápisu | **vypnuto**, volba datasetu | hledání v indexu u každého kusu 128 KiB, extenty po 128 KiB | jako na pozadí, ale hned |
| metadata 2x | **vždy** | 2x místo pro uzly (~0,4 % -> ~0,8 % kapacity), 2x zápis uzlů | oprava poškozeného uzlu bez ztráty dat |
| data 2x | **vypnuto**, volba datasetu | 2x místo a zápis dat | oprava poškozeného bloku dat (jeden disk) |
| scrub | **jednou týdně** + na povel | čtení celého obsazeného místa v nečinnosti | najde a opraví skryté poškození dřív, než je potřeba |

Profily ([návrh](navrh-fs-v3.md#výchozí-stav-a-profily)) mění jen
deduplikaci při zápisu (Volný: volba); integrita (součty, metadata 2x)
se profily nemění.

## Na disku

### Ukazatel 48 B a superblok

Ukazatel (`BlockPtr`) přibírá na offsetu 40 **adresu druhé kopie**
(zařízení 16 bitů + blok 48 bitů, 0 = bez kopie); součet platí pro obě
kopie. Superblok se proto posunul (kořen 80-128, index místa 128-176,
další pole od 176, jméno svazku 232-264). Uzel stromu má o 8 B delší
položku vnitřního uzlu (72 B), takže vnitřní uzel nese nejvýš 227
potomků (dřív 255).

Příznaky vlastností: `INCOMPAT_PTR_COPY` (bit 2) a `INCOMPAT_DATA` (bit 3)
nastaví formátování. Svazek **bez** `INCOMPAT_PTR_COPY` má starý tvar
ukazatele (40 B) - ovladač ho odmítne (`NOT_SUPPORTED`), svazek je potřeba
přeformátovat (FS je ve vývoji, převod se nedělá).

### Extent

```text
0   fyzický začátek u64       8   logických bloků u32
12  příznaky u32              16  generace vzniku u64
24  [začátek druhé kopie u64]               jen s EXTENT_COPY
..  [fyzických bloků u32, bajtů dat u32]    jen komprimovaný
..  součty: nekomprimovaný = jeden na blok, komprimovaný = jeden za
    celá data na disku, nezapsaný (předalokovaný) = žádný
```

Příznaky: bit 0 `UNWRITTEN` (předalokovaný, čte se jako nuly), bit 1
`SHARED` (bloky mohou být v tabulce sdílení), bit 2 `COPY`, bity 8-15
algoritmus komprese (1 = LZ4, 2 = Zstd). Neznámý bit = `NOT_SUPPORTED`.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Podoby extentu">
<text class="t b" x="20" y="24">logické bloky souboru</text>
<rect class="box" x="20" y="36" width="200" height="40" rx="6"/><text class="t" x="120" y="54" text-anchor="middle">nekomprimovaný</text><text class="m" x="120" y="69" text-anchor="middle">součet každého bloku</text>
<rect class="acc" x="230" y="36" width="160" height="40" rx="6"/><text class="t" x="310" y="54" text-anchor="middle">komprimovaný kus</text><text class="m" x="310" y="69" text-anchor="middle">32 bloků -> N bloků</text>
<rect class="ok" x="400" y="36" width="160" height="40" rx="6"/><text class="t" x="480" y="54" text-anchor="middle">nezapsaný</text><text class="m" x="480" y="69" text-anchor="middle">čte nuly, bez součtů</text>
<rect class="warn" x="570" y="36" width="170" height="40" rx="6"/><text class="t" x="655" y="54" text-anchor="middle">sdílený (SHARED)</text><text class="m" x="655" y="69" text-anchor="middle">bloky jiného extentu</text>
<text class="t b" x="20" y="130">fyzické bloky</text>
<rect class="box" x="20" y="142" width="200" height="34" rx="6"/>
<rect class="acc" x="230" y="142" width="70" height="34" rx="6"/>
<rect class="ok" x="400" y="142" width="160" height="34" rx="6"/>
<rect class="box" x="600" y="142" width="140" height="34" rx="6"/><text class="m" x="670" y="163" text-anchor="middle">tabulka: 2 odkazy</text>
<path class="ln" d="M120 80 L120 138" marker-end="url(#fd1)"/>
<path class="ln" d="M310 80 L265 138" marker-end="url(#fd1)"/>
<path class="ln" d="M480 80 L480 138" marker-end="url(#fd1)"/>
<path class="ln" d="M655 80 L670 138" marker-end="url(#fd1)"/>
<path class="ln" d="M150 80 C 300 110 560 110 640 138" marker-end="url(#fd1)"/>
<defs><marker id="fd1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="m" x="20" y="210">Každý extent může mít navíc druhou kopii (EXTENT_COPY) stejné fyzické délky jinde na disku.</text>
</svg>
<figcaption>Podoby extentu: sdílený extent ukazuje na bloky, které počítá tabulka sdílení.</figcaption>
</figure>

**Generace vzniku je logická:** u sdílených bloků (kopie, deduplikace)
je to generace, kdy vznikl tento odkaz. Pravidla vlastnictví (snapshot,
deadlist, [uvolnění bloku](souborovy-system.md#pravidla-uvolnění-bloku))
tak platí pro každý odkaz zvlášť; fyzické uvolnění hlídá tabulka sdílení.

### Deadlist a položky stromu

Záznam deadlistu má 32 B: začátek, délka, generace, **začátek kopie**
(0 = bez kopie) - kopie se uvolní spolu s první.

| Položka | Kde | Offset | Hodnota |
|---|---|---|---|
| 24 `DATAOPT` | každý dataset | 0 | komprese u8, kopií u8, deduplikace u8 (`DataSettings`) |
| 25 `SHARE` | kořenový dataset | fyzický začátek | délka u32, odkazů u32, kopie u64, příznaky (1 = indexovaný), [hash 16 B] |
| 26 `DEDUP` | kořenový dataset | prvních 8 B hashe | seznam (hash 16 B, začátek u64) |
| 27 `SCRUB` | kořenový dataset | 0 | stav scrubu (`ScrubState`, 104 B) |
| 28 `SCRUBERR` | kořenový dataset | pořadí | text neopravitelné chyby |
| 29 `DEDUPPOS` | každý dataset | 0 | generace, do které je dataset deduplikovaný |

## Komprese

- **Kus = 32 logických bloků (128 KiB)** zarovnaných na logický offset.
  Kus se uloží komprimovaný, jen když ušetří aspoň 1/8 (aspoň jeden
  blok) - fotky, video a archivy zůstanou nekomprimované a čtení neplatí
  za rozbalení.
- **Součet nad daty na disku** (jeden za kus): scrub a deduplikace
  nerozbalují.
- **LZ4** (blokový formát, lz4_flex) a **Zstd** (rámec, ruzstd, úroveň
  ~zstd -1); obojí čistý Rust bez `std`. Číslo algoritmu je v extentu,
  takže lepší kodér nebo úroveň půjde doplnit bez změny formátu.
- **Kus je nedělitelný:** zápis do kteréhokoli bloku vezme celý kus do
  zápisu (rozbalí ho) a starý extent uvolní; zkrácení uprostřed kusu
  ponechané bloky zapíše znovu.
- **Čtení:** kus se přečte celý, ověří, rozbalí a drží v malé
  mezipaměti (8 kusů), takže sekvenční čtení po 4 KiB ho nerozbaluje
  znovu.
- Nastavení platí pro **nově zapsaná** data; stará zůstanou, jak jsou.
  Nový dataset zdědí nastavení rodiče.

> **Riziko (vědomě přijaté):** komprese běží v jednom vlákně uvnitř
> commitu. Na rychlém NVMe (3-7 GB/s) proto zpomalí dlouhý souvislý zápis
> až několikrát a náhodný přepis 4 KiB přepisuje celý kus 128 KiB - proto
> je ve výchozím stavu vypnutá. Paralelní komprese je v kapitole
> [Výkon](navrh-fs-v3.md#výkon).

## Automatické díry

Blok ze samých nul se při zápisu neuloží: v souboru vznikne díra (čte se
jako nuly) a dřívější data na tom místě se uvolní. S kompresí se nuly
uvnitř kusu komprimují s ním a díra vznikne jen z celého nulového kusu.
Do předalokovaného místa se nuly nezapisují (už se čtou jako nuly).

## Předalokace

`fallocate(cesta, posun, délka, zachovat_velikost)` vyplní díry v rozsahu
**nezapsanými extenty** (souvislé bloky, čtou se jako nuly, bez součtů).
Bloky, kde data už jsou, zůstanou. S kopiemi dat se alokuje i druhá
kopie.

- **Zápis na místo:** první zápis do nezapsaného extentu jde přímo do
  jeho bloků (žádná nová alokace, soubor zůstane souvislý), pokud extent
  nedrží snapshot ani sdílení. Pak se extent rozdělí na nezapsaný -
  zapsaný - nezapsaný; generace vzniku zůstává.
- **Pád před commitem:** uložený stav extent pořád vidí jako nezapsaný,
  takže čte nuly - nikdy staré cizí bloky.
- **fsync:** intent log zapíše i extenty zapsané na místo; přehrání pozná
  stejné bloky pod nezapsaným extentem a jen je označí jako zapsané
  (žádná nová alokace).
- Extent, který drží snapshot, se zapisuje klasicky copy-on-write
  (snapshot dál čte nuly).
- Komprese ani deduplikace se na předalokované soubory nepoužije.

## Okamžité kopie (reflink)

`clone_path(zdroj, cíl)` zkopíruje soubor, odkaz nebo celou složku,
`clone_range(zdroj, posun, cíl, posun, délka)` rozsah (posuny a délka po
4 KiB, délka smí končit na konci zdroje). Kopie ukazuje na stejné bloky:

- extent kopie dostane `SHARED` a generaci této transakce, tabulka
  sdílení přičte odkaz,
- nezapsaný extent = díra v kopii; komprimovaný kus se sdílí jen celý,
  část kusu se zkopíruje,
- zdroj se před kopií **commitne**: bloky z rozpracované transakce by se
  jinak směly uvolnit hned, i když na ně už ukazuje kopie,
- funguje v celém svazku, i mezi datasety; datasety uvnitř kopírované
  složky se přeskočí (mají vlastní historii); kopie složky do sebe
  = `INVALID_ARGS`,
- **obnova ze snapshotu** (`restore_copy`, verze souboru, merge) teď také
  sdílí bloky - je okamžitá a nezabere místo navíc.

Práva: číst zdroj (u složky vypsat a každý soubor číst), vytvořit, resp.
zapsat cíl.

## Tabulka sdílení

Záznam (`SHARE`) pokrývá souvislý fyzický úsek s počtem odkazů. Úsek bez
záznamu má jediného vlastníka (původní pravidla). Každý odkaz (extent nebo
záznam deadlistu) se jednou **definitivně** uvolní (odloženě nebo
z deadlistu); takové uvolnění datového úseku jde do fronty `unref`, kterou
commit zpracuje v kořenovém datasetu:

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Uvolneni sdileneho useku">
<defs><marker id="fd2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="50" width="150" height="50" rx="6"/><text class="t" x="95" y="72" text-anchor="middle">uvolnění odkazu</text><text class="m" x="95" y="88" text-anchor="middle">extent / deadlist</text>
<rect class="acc" x="210" y="50" width="140" height="50" rx="6"/><text class="t" x="280" y="72" text-anchor="middle">fronta unref</text><text class="m" x="280" y="88" text-anchor="middle">před commitem</text>
<rect class="box" x="390" y="20" width="160" height="44" rx="6"/><text class="t" x="470" y="40" text-anchor="middle">v tabulce</text><text class="m" x="470" y="56" text-anchor="middle">odkazů - 1</text>
<rect class="box" x="390" y="90" width="160" height="44" rx="6"/><text class="t" x="470" y="110" text-anchor="middle">mimo tabulku</text><text class="m" x="470" y="126" text-anchor="middle">jediný vlastník</text>
<rect class="ok" x="590" y="50" width="150" height="50" rx="6"/><text class="t" x="665" y="72" text-anchor="middle">uvolnit úsek</text><text class="m" x="665" y="88" text-anchor="middle">i s kopií, odloženě</text>
<path class="ln" d="M170 75 L205 75" marker-end="url(#fd2)"/>
<path class="ln" d="M350 70 L385 45" marker-end="url(#fd2)"/>
<path class="ln" d="M350 82 L385 108" marker-end="url(#fd2)"/>
<path class="ln" d="M550 42 L585 65" marker-end="url(#fd2)"/>
<text class="m" x="560" y="30">při 0</text>
<path class="ln" d="M550 112 L585 88" marker-end="url(#fd2)"/>
<text class="m" x="20" y="160">Dokud tabulka nemá žádný záznam, fronta se obchází (bez režie pro svazky bez sdílení).</text>
</svg>
<figcaption>Uvolnění odkazu na datový úsek: tabulka sníží počet, poslední odkaz úsek uvolní.</figcaption>
</figure>

- Záznamy se nepřekrývají a nejsou delší než extent (496 bloků).
  Částečné snížení záznam rozdělí; indexovaný záznam (deduplikace) při
  rozdělení ztratí položku indexu (kus už není celý).
- Sdílený extent se **nikdy neuvolní hned** (i když vznikl v této
  transakci) - fyzické bloky mohou patřit uloženému stavu.
- Všechno se mění v jedné transakci s commitem všech datasetů, takže pád
  nikdy nenechá počet odkazů a extenty v nesouladu.

## Deduplikace

Kus = 32 zarovnaných logických bloků jednoho extentu (fyzicky souvisle),
otisk:

- nekomprimovaný: součet (algoritmus svazku) z prefixu „P“ a součtů všech
  32 bloků - nic se nečte, součty už jsou v extentu,
- komprimovaný extent: součet z prefixu „C“, algoritmu, délek a součtu dat
  na disku (stejný vstup se stejně zkomprimuje).

Index (`DEDUP`) vede otisk -> záznam tabulky sdílení. Při shodě se u
**XXH3** data porovnají bajt po bajtu (ochrana proti záměrné kolizi),
u **BLAKE3** ne. Pak extent souboru ukáže na nalezený kus (`SHARED`)
a vlastní bloky se uvolní.

- **Na pozadí** (`dedup_step`, výchozí): průchod živými stromy jen
  v podstromech narozených po posledním dokončeném průchodu (`DEDUPPOS`) -
  cena úměrná novým datům. Soubory s rozpracovanými daty se přeskočí
  (přijdou na řadu příště). Služba ho volá v nečinnosti po defragmentaci.
- **Při zápisu** (`DedupMode::Inline`): kus se hledá před alokací; nový
  kus se hned zaindexuje a jeho extent dostane `SHARED`. `fsync` se
  sdíleným extentem z této transakce udělá commit (intent log počty
  odkazů nenese).
- Paměť: index je na disku, v paměti jen cache uzlů stromu.

> **Riziko:** deduplikují se jen data živé verze; bloky, které drží už
> jen snapshot, zůstanou. Smazání souboru, jehož bloky sdílí jiný
> soubor, místo neuvolní - volné místo se tak hůř odhaduje (`free_blocks`
> je pravda, velikosti souborů ne).

## Metadata 2x a samoléčení

Každý uzel stromu se zapíše **dvakrát**: druhá kopie se alokuje od konce
disku dolů (daleko od první, na disku s víc alokačními skupinami v jiné
skupině). Na rozdrobeném disku bez souvislého místa pro kopii se uzel
zapíše jen jednou (radši bez kopie než selhaný commit) - fsck to hlásí
jako `nodes_without_copy` (není to chyba).

<figure class="diagram">
<svg viewBox="0 0 760 160" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cteni se samolecenim">
<defs><marker id="fd3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="50" width="130" height="44" rx="6"/><text class="t" x="85" y="77" text-anchor="middle">čtení 1. kopie</text>
<rect class="ok" x="200" y="10" width="130" height="40" rx="6"/><text class="t" x="265" y="35" text-anchor="middle">součet sedí</text>
<rect class="box" x="200" y="95" width="130" height="44" rx="6"/><text class="t" x="265" y="114" text-anchor="middle">čtení 2. kopie</text><text class="m" x="265" y="130" text-anchor="middle">chyba / I/O</text>
<rect class="acc" x="390" y="95" width="160" height="44" rx="6"/><text class="t" x="470" y="114" text-anchor="middle">sedí: přepsat 1.</text><text class="m" x="470" y="130" text-anchor="middle">samoléčení</text>
<rect class="warn" x="590" y="95" width="150" height="44" rx="6"/><text class="t" x="665" y="114" text-anchor="middle">nesedí ani 2.</text><text class="m" x="665" y="130" text-anchor="middle">CORRUPTED</text>
<path class="ln" d="M150 65 L195 35" marker-end="url(#fd3)"/>
<path class="ln" d="M150 82 L195 112" marker-end="url(#fd3)"/>
<path class="ln" d="M330 117 L385 117" marker-end="url(#fd3)"/>
<path class="ln" d="M550 117 L585 117" marker-end="url(#fd3)"/>
</svg>
<figcaption>Čtení uzlu nebo dat s kopií: špatná první kopie se opraví z druhé.</figcaption>
</figure>

- Součet je v ukazateli a platí pro obě kopie, takže přepis špatné kopie
  na místě nic nerozbije (obsah je daný součtem).
- Při připojení (než je jasné, zda je svazek zapisovatelný) se opravy
  jen poznamenají a zapíšou, až připojení rozhodne; připojení jen pro
  čtení nezapíše nic.
- Poškozený nejnovější kořen se tak zahojí bez návratu o generaci; návrat
  ([Připojení](souborovy-system.md#připojení-a-návrat-o-generaci)) přijde
  až při poškození obou kopií.

**Data 2x** (`copies: 2` v nastavení datasetu): extent dostane druhou
kopii stejné délky (od konce disku). Čtení a scrub opraví špatný blok
z kopie, stejně jako u uzlů.

> **Riziko:** obě kopie leží na **jednom disku** - chrání proti vadným
> sektorům a tichému poškození, ne proti selhání celého disku (na to je
> zrcadlo, formát je připravený: číslo zařízení v ukazateli). **Space
> logy** (metadata volného místa) zatím 2x nejsou: poškozený space log
> se pozná součtem a připojení se vrátí o generaci; když nejde ani to,
> svazek nejde připojit. Obnova volného místa průchodem stromů je
> follow-up.

## Scrub

`scrub_start` / `scrub_stop` / `scrub_status` / `scrub_step`. Průchod:
datasety podle čísla, v každém snapshoty od nejstaršího a pak uložená
živá verze. U každého stromu se přeskočí podstromy a extenty narozené do
generace předchozího snapshotu - ty už ověřil průchod předchozím stromem
(klon přeskočí i vše do generace původu). Cena je úměrná obsazenému
místu, ne počtu verzí.

<figure class="diagram">
<svg viewBox="0 0 760 150" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Pruchod scrubu">
<defs><marker id="fd4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="40" width="150" height="50" rx="6"/><text class="t" x="95" y="62" text-anchor="middle">snapshot S1</text><text class="m" x="95" y="78" text-anchor="middle">vše</text>
<rect class="box" x="210" y="40" width="150" height="50" rx="6"/><text class="t" x="285" y="62" text-anchor="middle">snapshot S2</text><text class="m" x="285" y="78" text-anchor="middle">jen birth &gt; S1</text>
<rect class="box" x="400" y="40" width="150" height="50" rx="6"/><text class="t" x="475" y="62" text-anchor="middle">živá verze</text><text class="m" x="475" y="78" text-anchor="middle">jen birth &gt; S2</text>
<rect class="acc" x="590" y="40" width="150" height="50" rx="6"/><text class="t" x="665" y="62" text-anchor="middle">další dataset</text><text class="m" x="665" y="78" text-anchor="middle">podle čísla</text>
<path class="ln" d="M170 65 L205 65" marker-end="url(#fd4)"/>
<path class="ln" d="M360 65 L395 65" marker-end="url(#fd4)"/>
<path class="ln" d="M550 65 L585 65" marker-end="url(#fd4)"/>
<text class="m" x="20" y="125">U každého uzlu a extentu se čtou obě kopie; pozice (dataset, snapshot, klíč) je ve stromu a přežije restart.</text>
</svg>
<figcaption>Scrub ověří každý blok jednou, i když ho sdílí víc verzí.</figcaption>
</figure>

- U každého uzlu a extentu se přečtou **obě** kopie: špatná se přepíše
  dobrou. Když dobrá není žádná, zapíše se chyba s cestou souboru
  (`SCRUBERR`, nejvýš 64 záznamů za běh). Nezapsané extenty se nečtou.
- Krok ověřuje **poslední uložený stav** (kořeny podle superbloku), nic
  necommituje; pozice se uloží s příštím commitem. Změna stromu mezi
  kroky nevadí - krok začne nad aktuálním uloženým stavem od uloženého
  klíče.
- Služba fsd scrub spustí jednou týdně (od konce posledního dokončeného
  běhu) a dělá kroky po 1 MiB mezi požadavky klientů (požadavky mají
  přednost).

## Kontrola konzistence

fsck (`check`) navíc ověří:

- kopie uzlů leží ve svazku a nic jiného jejich bloky nepoužívá,
- extenty včetně kopií a komprimované délky leží ve svazku,
- blok použitý víckrát v živém stavu musí být v tabulce sdílení, sdílený
  extent živé verze musí mít záznam,
- záznamy tabulky se nepřekrývají a nikdo je nepoužívá „navíc“ (záznam
  bez použitých bloků = ztracený odkaz),
- index deduplikace odpovídá indexovaným záznamům tabulky (a naopak),
- se `scrub = true` přečte data všech extentů (komprimované rozbalí).

## Protokol (operace 110-117)

`fsproto::data` (87-92 patří vnořeným datasetům, 100 je `SIMULATE_POWER_LOSS`):

| Op | Jméno | Požadavek | Odpověď | Práva |
|---|---|---|---|---|
| 110 | `CLONE` | zdroj, cíl | sdílených bajtů u64 | číst zdroj, vytvořit cíl |
| 111 | `CLONE_RANGE` | zdroj, posun, cíl, posun, délka | - | číst zdroj, zapsat cíl |
| 112 | `FALLOCATE` | cesta, posun, délka, zachovat velikost u8 | - | zápis |
| 113 | `DATA_SETTINGS` | dataset | komprese, kopií, deduplikace | čtení |
| 114 | `SET_DATA_SETTINGS` | dataset, nastavení | - | `MANAGE` |
| 115 | `SCRUB_START` | - | - | `MANAGE` na kořeni svazku |
| 116 | `SCRUB_STOP` | - | - | `MANAGE` na kořeni svazku |
| 117 | `SCRUB_STATUS` | - | stav, časy, počty, chyby | kdokoli (stav disku, ne obsah) |

## Rizika a follow-upy

- komprese v jednom vlákně (viz výše) - paralelní commit je v plánu,
- space logy bez druhé kopie; obnova volného místa průchodem stromů,
- obě kopie na jednom disku (zrcadlo až s více disky),
- deduplikace na pozadí neprochází data, která drží jen snapshoty,
- deduplikace při zápisu dělí nekomprimované extenty po 128 KiB (víc
  položek extentů, ne fragmentace na disku),
- okamžitá kopie commituje zdroj (krátká pauza u velké rozpracované
  změny).
