# Verze formátu souborového systému

Přehled všech verzí formátu na disku, co se mezi nimi změnilo a **proč**.
Aktuální formát podrobně popisuje [souborovy-system.md](souborovy-system.md);
každá jeho kapitola, která se proti předchozí verzi změnila, má na začátku
rámeček **„Změna proti v2“**. Plán dalších změn je v [návrhu FS
v3](navrh-fs-v3.md).

| Verze | Podpis | Stav | Popis |
|---|---|---|---|
| v1 | - | interní prototyp, nikdy nevydaný (není v historii gitu) | - |
| v2 | `OSFS\0\0v2` | nahrazená (do commitu `ff2bc6b`) | [archiv v2](fs-v2.md) |
| **v3** | `OSFS\0\0v3` | **aktuální**, zavádí se po fázích | [souborovy-system.md](souborovy-system.md) |

**Kompatibilita mezi verzemi:** ovladač připojí jen svou verzi; jiná verze
(starší i novější) = `NOT_SUPPORTED`. Převod v2 -> v3 zatím neexistuje
(disky z `xtask` se vytvářejí znovu). Od v3 se menší změny přidávají
**příznaky vlastností** (compat / ro_compat / incompat) bez nové verze -
viz [Připojení](souborovy-system.md#připojení-a-návrat-o-generaci).

## Přechod v1 -> v2

v1 byl první prototyp během vývoje; do repozitáře se dostala až v2. Doložená
změna:

| Oblast | v1 | v2 | Proč |
|---|---|---|---|
| Kontrolní součet | CRC32C (32 bitů) | **XXH3-128** (výchozí) nebo **BLAKE3** | 32 bitů = 1 nezachycená chyba ze 4 miliard; 10TB disk má 2,5 miliardy bloků. 128 bitů = 1 : 2^128. BLAKE3 navíc odolá úmyslné kolizi (deduplikace, podpisy). |

Co v2 přinesla (výchozí stav pro další přechod): Merkleův strom se
součtem v každém ukazateli, copy-on-write a atomický commit, intent log
pro rychlý `fsync`, extenty s odloženou alokací, alokační skupiny se space
logy (bez bitmapy), TRIM, defragmentaci, souhrny podstromu, jména bez
ohledu na velikost písmen, snapshoty s deadlisty, fsck a scrub.

## Přechod v2 -> v3

Návrh a důvody celé v3: [navrh-fs-v3.md](navrh-fs-v3.md). Hotové změny:

### Formát na disku

| Oblast | v2 | v3 | Proč |
|---|---|---|---|
| Uzel B+stromu | 4 KiB, úroveň jen 0/1 | **16 KiB** (4 bloky), úroveň uložená a kontrolovaná | jména do 4 KiB a extent s 496 součty se vejdou do položky, strom o úroveň mělčí; kontrola úrovně brání smyčce v poškozeném stromu |
| Ukazatel (`BlockPtr`) | 32 B: blok, birth, součet | **40 B**: + zařízení (16 b), délka, příznaky | uzel má víc bloků; svazek přes víc disků; komprese/šifrování/sdílení se pozná na ukazateli |
| Superblok | bez UUID a vlastností | **UUID, příznaky vlastností compat/ro_compat/incompat**, zařízení svazku | bezpečné rozšiřování formátu bez nové verze; poznání svazku |
| Záznam objektu | 64 B | **128 B**: čas vytvoření a přístupu, generace, oprávnění, vlastník, rozšířené atributy | stabilní identita (oid + generace) pro handly; místo pro oprávnění a atributy bez další změny formátu |
| Jména | 255 B | **1024 znaků / 4 KiB**, zakázané znaky směru textu | dlouhá jména z jiných systémů; ochrana proti podvržené příponě (`faktura[U+202E]fdp.exe`) |
| Typy objektů | soubor, adresář | + **symbolický odkaz** | git repozitáře a zdrojáky z Linuxu je potřebují |
| Velikost souboru | bez limitu (přetečení) | **2^63-1 B**, kontrolovaná aritmetika, saturující souhrny | velikosti a rozdíly vždy v `i64`; obří řídké soubory nepřetečou souhrny |
| Extent | nejvýš 120 bloků, rezerva u32 | **496 bloků**, příznaky u32 (vyhrazeno: předalokace) | větší uzel; méně položek na velký soubor |
| Intent log | záznamy 1-6 | + 7 symlink, generace u vytvoření, 8 značka datasetu, 9 nastavení časů | symlink přežije `fsync`; generace po přehrání zůstane původní; jedna dávka pro víc datasetů; nastavené časy přežijí pád |
| Oprávnění | žádná | **vlastník, granty s dědičností (jen povolení), viditelnost, sdílené popisy** (`ACL`, `RO_COMPAT_ACL`), zákaz spouštění na datasetu; ve fsd identita relace od initu a připojené cesty | sdílení mezi uživateli, aplikace jen s tím, co dostaly ([oprávnění](souborovy-system.md#oprávnění-a-identita-relací)) |
| Časy | u64 ns od 1970 | **sekundy i64 + nanosekundy u32** (příznak `INCOMPAT_TIME_SECS`) | data před rokem 1970 a bez hranice 2262 - viz níže |
| Sirotci | - | položka `ORPHAN` (druh 9) pro otevřený objekt bez jména | otevřený soubor přežije smazání i atomické nahrazení; po pádu úklid při připojení |
| Datasety | jeden strom pro svazek | **datasety** (vlastní stromy, záznam `DATASET` v kořenovém datasetu, položka adresáře druhu 4), snapshot s počtem klonů a příznakem „skrytý“ | historie po složkách (projekt, domov), zapisovatelné klony bez kopírování |

### Chování (bez změny formátu)

| Oblast | v2 | v3 | Proč |
|---|---|---|---|
| Připojení | neznámá vlastnost/verze tiše přeskočena | **výběr generace podle hlavičky, pak rozhodnutí**: jiná verze/incompat = odmítnout, ro_compat = jen pro čtení | starší ovladač nesmí zapisovat do starého stavu a přepsat novější data |
| Nečitelný slot / log | slot = konec, log = konec řetězu | nečitelný slot pro `newest + 1` = **jen pro čtení**, jiný slot = jen varování; nečitelný log = jen pro čtení a log zachovaný, nebo vědomě zahodit (`discard_unreadable_log`) | nejistá obnova nesmí přepsat potvrzený commit ani `fsync`; vadný sektor nesmí udělat svazek navždy jen pro čtení |
| Návrat o generaci | při jakékoli chybě | jen při `CORRUPTED`, novější sloty se vynulují | přechodná chyba disku nesmí vést ke ztrátě novějších dat |
| Zařízení bez flush | zapisovatelné | **jen pro čtení** (`Durability`) | `fsync` by lhal |
| `fsync` | čekající změny smazané před zápisem | smazané až po flush, chyba při zápisu logu = stav chyby | nic se nesmí potvrdit jako trvalé, co trvalé není |
| Přepsání dat | nejdřív uvolnit, pak zapsat | nejdřív zapsat nové bloky, pak přepojit | chyba zápisu nesmí nechat díru ani ztratit data |
| `write` | chyba i při přijatých datech | `Ok` = přijato do bufferu, chyba = nic | klient musí vědět, co bylo přijato |
| Místo | rozpracovaný uzel = 1 blok | uzel = 4 bloky, jen úseky pro celé uzly, rezerva pro uvolňující operace | commit se nesmí zacyklit ani selhat na rozdrobeném disku; snapshot musí jít smazat i na plném disku |
| Hodnoty z disku | brané na víru | kontrola extentů, uvolnění, délek, cyklů rodičů; operace vše, nebo nic | chyba softwaru se správným součtem nesmí poškodit alokátor |
| fsck | mapa blok -> kdo, commit vždy | bitové mapy (pevná paměť), funguje i jen pro čtení | 1 TiB svazek = 96 MiB; kontrola při nejisté obnově bez zápisu |
| Zápis dat | skládání po bajtech | po celých blocích | 2x rychlejší zápis (0,7 -> 1,4 GiB/s) |

### Fáze 2: historie (`RO_COMPAT_HISTORY`)

Změna uvnitř v3, poznaná příznakem `RO_COMPAT_HISTORY` (bit 1 ro_compat).
Podrobně: [Historie FS](fs-historie.md#na-disku).

| Oblast | Fáze 1 | Fáze 2 | Proč |
|---|---|---|---|
| položky stromu | 1-12 | + 13 `JOURNAL`, 14 `JSTATE`, 15 `SNAPMETA`, 16 `SETTINGS`, 17 `TRASH`, 18 `XATTR`, 19 `EXPIRY`, 20 `LHASH`, 21 `BRANCH`, 22 `HEAD`, 23 `DSLINK` | žurnál změn, metadata snapshotů a commitů, nastavení datasetu, koš, atributy, index expirace, paměť logického hashe, větve, zpětný odkaz datasetu (rychlá cesta) |
| pevné objekty | 0 META, 1 kořen | + **2 koš** (`TRASH_OID`, rodič 0) | smazání do koše je přesun, žádná kopie |
| počet atributů v záznamu objektu | vždy 0 | **počet položek `XATTR`** (fsck kontroluje) | atributy existují |
| intent log | záznamy 1-13 | + 14 `Actor`, 15 `Trash`, 16 `Restore`, 17 `Purge`, 18 `SetXattr` | stejná čísla a uživatelé žurnálu po přehrání; koš a atributy přežijí pád |
| chyby | do -27 | + `JOURNAL_GAP` (-28) | starší záznamy zahodila retence - nikdy tiché vynechání |
| protokol v2 | op 30-53 | + **54-86** (`fsproto::hist`) | historie pro průzkumník, indexer, sync, agenty; vytvoření a smazání datasetu |

**Starší svazek** (bez bitu) ovladač při prvním zapisovatelném připojení
povýší: nastaví bit a odteď vede žurnál (začíná číslem 1); snapshoty z doby
před fází 2 nemají metadata (ruční, čas 1970). Starší ovladač svazek s
bitem jen čte - jinak by při zápisu nevedl žurnál ani indexy.

**Chování (bez změny formátu):** `remove` zůstává trvalé, do koše
přesouvá jen `trash`; každá změnová operace počítá s místem pro uzly
žurnálu (`JOURNAL_ROOM`), jinak by commit na plném disku nemusel mít kam
uložit strom; změny, které commit sám přidává (žurnál), se nepočítají jako
rozpracovaná operace.

### Zbývá ve v3

Fáze 3-6 (data, ochrana, ostatní, cloud) - pořadí v [návrhu](navrh-fs-v3.md#pořadí-implementace).

## Časy objektů: u64 ns -> i64 s + u32 ns

Změna uvnitř v3, poznaná příznakem `INCOMPAT_TIME_SECS` (bit 1 incompat).

| | Dřív | Teď |
|---|---|---|
| uložení | u64 nanosekund od 1970 (offsety 16-40) | sekundy i64 (16-40) + nanosekundy u32 (112-128, dřív rezerva) |
| rozsah | 1970 až 2262 | ±292 miliard let (i 1601 z NTFS, data před 1970) |
| intent log | čas 8 B | čas 12 B, nový záznam 9 (nastavení časů) |
| protokol `Stat` | mtime, ctime u64 ns | mtime, ctime, **btime**, každý sekundy i64 + nanosekundy u32 |
| API | `set_time(u64)` | `set_time(Timestamp)`, nové `set_times` a `set_times_h` |

**Proč:** při kopii ze starých archivů a z NTFS se časy před rokem 1970
ztrácely a hranice roku 2262 je zbytečná. Varianty: i128 nanosekund (jedno
číslo, ale 16 B na čas - nevešlo by se do 128 B záznamu, MSVC nemá
`__int128`, převody potřebují 128bitové dělení) a 100 ns od 1601 jako NTFS
(rozsah do roku 30828, horší rozlišení). Zvolená varianta je stejná jako
`timespec` a `SystemTime` a vejde se do dnešního záznamu.

**Starší svazky** (bez příznaku) ovladač čte i zapisuje dál ve starém tvaru;
starší ovladač nové svazky odmítne. Převod starého svazku na nový tvar zatím
není.

## Protokol fsproto v1 -> v2

| Oblast | v1 | v2 | Proč |
|---|---|---|---|
| adresování | cesty | **handly** (dataset, oid, generace) | soubor přežije přejmenování i smazání; operace nevyhodnocuje cestu znovu |
| klienti | jeden | **relace** přes `CONNECT`, vlastní handly | editor, agent i synchronizace pracují zároveň a izolovaně |
| zápis | výsledek bez počtu | **počet přijatých bajtů** | klient ví, co služba přijala, i u zápisu ve více zprávách |
| uložení | přepis | **podmíněné uložení** s revizí (`CONFLICT`) | žádné tiché přepsání cizí změny |
| zámky a sdílení | - | zámky rozsahu, sdílecí režimy, smazání při zavření | vrstva Windows a databáze |
| výpis | stránky podle kurzoru | i nad handlem (`LIST_H`) | stejné záruky při změně adresáře |

Operace v1 zůstávají (kompatibilita); nový klient se ptá `HELLO`.
