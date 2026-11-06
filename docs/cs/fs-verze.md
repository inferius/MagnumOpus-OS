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
| Intent log | záznamy 1-6 | + 7 symlink, generace u vytvoření, 8 značka datasetu | symlink přežije `fsync`; generace po přehrání zůstane původní; jedna dávka pro víc datasetů |
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

### Zbývá ve v3

Datasety, verzování (klony, diff, obnova), vrstva handlů a ovladač pro
Windows, oprávnění, protokol v2 - pořadí v [návrhu](navrh-fs-v3.md#pořadí-implementace).
