# Nástroj osfs: souborový systém na hostiteli a ve Windows

`osfs` (zdroj `tools/osfs`) pracuje s naším souborovým systémem
(`libs/fs`, formát popsaný v [souborovy-system.md](souborovy-system.md))
na hostitelském počítači:

- vytvoří a naformátuje **soubor s obrazem** svazku,
- zkontroluje ho (fsck, volitelně scrub),
- vypíše adresář, vypíše soubor, nahraje soubor,
- ve Windows obraz **připojí jako jednotku** (např. `Q:`) přes WinFsp,
  takže s ním jde pracovat v průzkumníku a v libovolném programu.

Nástroj používá jen veřejné API knihovny (`fs::Fs`); knihovnu nemění.

> ⚠️ **Jen na zkoušení, ne na ostrá data.** FS je mladý. Data, o která
> nechceš přijít, měj zálohovaná mimo něj.
>
> ⚠️ **Skutečné disky zatím ne.** Formátování disku je nevratné (smaže vše,
> co na něm je). `osfs` proto zatím odmítne vše, co vypadá jako disk nebo
> oddíl (`E:`, `\\.\PhysicalDrive1`, `\\?\Volume{...}`, `/dev/sdb`), a pracuje
> jen se soubory s obrazem. Disky přijdou, až budou hotové ochrany
> (odmítnout systémový disk a disk s připojenými oddíly, potvrzení ručním
> opsáním čísla disku) - viz [návrh v3](navrh-fs-v3.md#ovladač-pro-windows).

## Instalace a překlad

1. **WinFsp** (jen pro `osfs mount`): stáhnout z <https://winfsp.dev>
   a nainstalovat (potřebuje práva správce). `osfs` si knihovnu
   `winfsp-x64.dll` najde podle registru (`HKLM\SOFTWARE\WOW6432Node\WinFsp`,
   hodnota `InstallDir`), jinak v `C:\Program Files (x86)\WinFsp`.
2. Překlad:

```text
cargo build -p osfs --release
```

Výsledek je `target\release\osfs.exe`. Napojení na WinFsp je vlastní a malé
(`tools/osfs/src/mount/ffi.rs`, struktury ručně přepsané z hlaviček
`inc\winfsp\*.h`), bez generátoru vazeb a bez clang; DLL se načte až za
běhu, takže nástroj jde přeložit i bez WinFsp (selže jen `osfs mount`).

Testy (překlad jmen včetně vlastnostního testu):

```text
cargo test -p osfs
```

## Příkazy

| Příkaz | Co dělá |
|---|---|
| `osfs format <obraz> [--size N] [--label X] [--blake3]` | vytvoří **nový** soubor s obrazem (`N` např. `64M`, `1G`; výchozí 256 MiB) a naformátuje ho; existující soubor nepřepíše |
| `osfs check <obraz> [--scrub]` | kontrola konzistence **bez zápisu** (připojení jen pro čtení); `--scrub` ověří i kontrolní součty dat; vypíše i položky, které se ve Windows nezobrazí |
| `osfs ls <obraz> [cesta]` | výpis adresáře (výchozí `/`), u skrytých jmen poznámka `[ve Windows skryte]` |
| `osfs cat <obraz> <cesta>` | obsah souboru na standardní výstup |
| `osfs put <obraz> <soubor> <cesta>` | zkopíruje místní soubor do obrazu (existující přepíše) a uloží (commit) |
| `osfs mount <obraz> <X:>` | připojí obraz jako jednotku `X:`; běží, dokud se neodpojí |
| `osfs unmount <X:>` | odpojí jednotku z jiného okna (stejné jako Ctrl+C v okně s `mount`) |

**Jeden zapisovatel:** obraz může mít otevřený pro zápis jen jeden proces
`osfs` (`mount`, `put`); druhý skončí hláškou `obraz ma otevreny jiny proces
osfs`. Dva zapisovatelé by si navzájem přepisovali bloky a svazek by se
poškodil. Čtení (`check`, `ls`, `cat`) vedle běžícího `mount` projde, jen
upozorní, že vidí stav po posledním commitu (až o pár sekund starý). Zámek
je bajt daleko za koncem souboru zamčený přes `LockFileEx`, takže neblokuje
čtení a zápis obrazu, platí mezi všemi procesy a přihlášeními a uvolní se
i při pádu `osfs`.

Cesty v obrazu se píšou s `/` od kořene (`/dokumenty/a.txt`). Velikost
písmen se nerozlišuje (stejně jako ve Windows), jména se ukládají tak, jak
je zadáš.

### Příklad: od prázdného obrazu po jednotku v průzkumníku

```text
> osfs format D:\zkouska.img --size 1G --label Zkouska
D:\zkouska.img: naformatovano, 1024 MiB, kontrolni soucty Xxh3

> osfs put D:\zkouska.img C:\Windows\win.ini /win.ini
/win.ini: zapsano 92 B

> osfs ls D:\zkouska.img
file           92  win.ini

> osfs mount D:\zkouska.img Q:
D:\zkouska.img pripojeno jako Q: ('Zkouska'). Odpojeni: Ctrl+C.
POZOR: FS je mlady - jen na zkouseni, ne na ostra data.
```

Teď je v průzkumníku jednotka `Q:` (systém souborů `MagnumOpusFS`):
vytvářet, otevírat, upravovat, kopírovat, přejmenovávat i mazat soubory
a složky jde běžně. Odpojení: Ctrl+C v okně s `osfs mount`, nebo z jiného
okna

```text
> osfs unmount Q:
Q:: odpojeni vyzadano (osfs mount ulozi zmeny a skonci)
```

a pak kontrola:

```text
> osfs check D:\zkouska.img --scrub
svazek 'Zkouska': 262144 bloku po 4096 B, volnych 262127, generace 2, soucty Xxh3
generace 2: uzly stromu 4 bloku, data 0 bloku, metadata 4 bloku
v poradku
```

Pokud by připojení skončilo bez odpojení (pád, vypnutý proud), obraz
zůstane ve stavu po posledním commitu plus změny potvrzené `fsync` - viz
[ukládání](#ukládání-commit-a-fsync).

## Jak `osfs mount` funguje

```text
 program ve Windows (průzkumník, editor, ...)
        |  CreateFile / ReadFile / MoveFileEx / ...
        v
 jádro Windows -> ovladač WinFsp (winfsp-x64.sys)
        |  požadavky přes WinFsp DLL, z více vláken
        v
 osfs.exe: tools/osfs/src/mount/ops.rs
        |  1. jeden zámek kolem celého Fs
        |  2. překlad jména   \A\b<U+F03A>c  ->  /A/b:c
        |  3. volání fs::Fs   stat / read / write / create / rename / ...
        v
 libs/fs (Fs<FileDevice>)  ->  soubor s obrazem
```

- **Souběh:** WinFsp volá z více vláken, `Fs` je jednovláknový - na
  začátek je kolem něj **jeden zámek** (operace jdou po jedné).
- **Po cestách:** otevřený soubor si pamatuje jen svou cestu v našem FS.
  Vrstva handlů (smazaný, ale ještě otevřený soubor, sdílecí režimy) přijde
  později. Přejmenování přepíše cesty otevřených handlů pod přejmenovaným
  jménem.
- **Velikost písmen:** svazek se hlásí jako „nerozlišuje velikost písmen,
  zachovává ji“. Při otevření se WinFsp vrací skutečné jméno (normalizované,
  dohledané ve výpisu rodiče); bez toho by WinFsp bral jako jméno souboru
  zadanou cestu velkými písmeny a přejmenování `a.txt` -> `A.TXT` by
  považoval za „beze změny“.
- **Přejmenování s nahrazením** (`MoveFileEx` s `REPLACE_EXISTING`): cíl
  se nejdřív jen odsune na dočasné jméno `.osfs-nahrazeni-N` ve stejném
  adresáři, pak se přejmenuje zdroj a teprve potom se odsunutý cíl smaže.
  Když přejmenování selže, cíl se vrátí zpět - nic se neztratí. Vše probíhá
  pod jedním zámkem, takže commit ani `fsync` mezistav nezachytí. Handly,
  které byly otevřené na nahrazeném souboru, už vrací „soubor smazán“
  (jejich cesta by teď vedla na jiný objekt); obvykle to ale odmítne už
  WinFsp (otevřený cíl nejde nahradit).
- **Výpis adresáře** po částech: při prvním dotazu se pořídí snímek seznamu
  jmen, další části (WinFsp se ptá opakovaně, dokud se buffer plní)
  pokračují ve stejném snímku a metadata (`stat`) se čtou jen pro položky,
  které se do bufferu opravdu vejdou.
- **Mezipaměť skutečných jmen** (kvůli normalizovanému jménu) se mění po
  položkách: vytvoření přidá jedno jméno, smazání a přejmenování ho odeberou.
  Hromadné vytváření souborů v jednom adresáři je proto lineární (3000
  souborů asi za 1 s). Nejvýš 256 adresářů, pak se mezipaměť zahodí celá.
- **Sériové číslo svazku** je stálé, odvozené z UUID svazku (superblok):
  Windows podle něj poznávají svazek (zástupci, mezipaměť náhledů); nezmění
  se ani po přesunu obrazu. **Čas vytvoření svazku** = čas vytvoření
  kořene (`btime`, zapíše ho `osfs format`).
- **Časy:** náš čas (sekundy od 1970 se znaménkem + nanosekundy, umí
  i data před rokem 1970) -> FILETIME (100 ns od 1601; dřívější čas = 0).
  Čas vytvoření = `btime`, poslední zápis i přístup = `mtime`, změna =
  `ctime`. Nastavení času z Windows (`SetFileTime`, vlastnosti souboru) se
  uloží přes `Fs::set_times`.
- **ID souboru** (`IndexNumber`) = `oid` objektu.

### Ukládání: commit a fsync

| Událost | Co se stane |
|---|---|
| každých 5 s, když se něco změnilo | commit (atomické uložení všech změn) |
| `FlushFileBuffers` na souboru | `fsync` - změny do intent logu, přežijí výpadek |
| flush celé jednotky | commit |
| odpojení (Ctrl+C, `osfs unmount`, zavření okna) | commit, pak konec |

Při výpadku proudu nebo pádu `osfs` se ztratí nejvýš změny za posledních
pár sekund, které nikdo nepotvrdil `fsync`; obraz zůstane konzistentní
(copy-on-write).

## Překlad jmen

Náš FS povoluje skoro vše (zakázané je jen `/`, řídicí znaky, `.`, `..`
a znaky směru textu), Windows mnohem méně. Překlad je přesně podle tabulky
v [návrhu v3](navrh-fs-v3.md#ovladač-pro-windows) a je navržený tak, aby
**každé zobrazené jméno otevřelo právě ten objekt, ze kterého vzniklo**:

| Jméno v našem FS | Ve Windows | Příklad |
|---|---|---|
| platné i ve Windows | beze změny | `Příliš žluťoučký.txt` |
| obsahuje zakázaný znak `: \ * ? " < > \|` | znak -> U+F000 + kód znaku (jako WSL a Cygwin) | `a:b` -> `a<U+F03A>b` |
| už samo obsahuje znak z U+F000-F0FF | **skryté** (kolidovalo by s přeloženým jménem) | `x<U+F041>` |
| rezervované jméno, i s příponou nebo mezerou před ní | **skryté** | `CON`, `nul.txt`, `COM1.log`, `LPT9`, `NUL .txt` |
| končí tečkou nebo mezerou | **skryté** (Win32 je při otevření ořízne) | `zprava.`, `a ` |
| delší než 255 znaků UTF-16 | **skryté** | |
| symbolický odkaz | **skrytý** (první verze odkazy nezobrazuje) | |

Rezervovaná jména jsou `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`,
`LPT1`-`LPT9`; navíc proti tabulce v návrhu skrýváme i `COM¹ COM² COM³`
a `LPT¹ LPT² LPT³` (Windows 11 je bere jako rezervované) a `CONIN$`,
`CONOUT$` - skrýt víc je bezpečné. `COM0` a `COM10` rezervované nejsou.

Skryté položky dál existují, jen je Windows nevidí. **Vypíše je
`osfs check`** (a `osfs ls` je označí):

```text
ve Windows se nezobrazi (2):
  /CON.txt  - rezervovane jmeno Windows
  /odkaz  - symbolicky odkaz (zatim se nezobrazuje)
```

**Zápis z Windows:** znak z U+F000-F0FF odpovídající zakázanému znaku se
uloží jako ten původní znak (`a<U+F03A>b` -> `a:b`). Jméno odmítne
(`STATUS_OBJECT_NAME_INVALID`), když:

- obsahuje jiný znak z U+F000-F0FF (např. U+F041),
- by výsledek byl ve Windows skrytý (rezervované jméno, koncová tečka),
- ho odmítá náš FS (znaky směru textu - jinak by šlo
  `faktura[U+202E]fdp.exe` zobrazit jako `fakturaexe.pdf`).

Pravidlo, které z toho plyne a hlídá ho test: `from_windows(w) == n` právě
když `to_windows(n) == w`. Skryté jméno tedy z Windows nejde zadat vůbec
(ani oklikou přes U+F0xx nebo koncovou tečku), takže nejde omylem otevřít
nebo smazat jiný objekt, než jaký průzkumník ukázal.

**Akceptační (vlastnostní) test** `tools/osfs/tests/names.rs`: pro náhodná
jména se zákeřnostmi (zakázané znaky, U+F0xx, rezervovaná jména, koncové
tečky a mezery, rozložená diakritika, velikost písmen, dlouhá jména) vytvoří
objekty v FS v paměti a ověří, že každé zobrazené jméno se přeloží zpět na
cestu, která otevře právě objekt, ze kterého vzniklo, že žádná dvě jména se
nezobrazí stejně (ani bez ohledu na velikost písmen) a že skryté jméno se
z Windows nedá zadat.

## Omezení první verze

- Jen soubory s obrazem, ne disky (viz varování nahoře).
- Atributy (jen pro čtení, skrytý, ...) a ruční nastavení časů se
  **nepamatují** - API `fs` na to zatím nemá funkci; `SetBasicInfo` se
  přijme a vrátí skutečný stav.
- Žádná oprávnění (ACL): všichni mají plný přístup.
- Symbolické odkazy se nezobrazují; vytvořit soubor se stejným jménem
  selže (`jméno už existuje`).
- Bez pojmenovaných datových proudů (`soubor:proud`), rozšířených
  atributů a reparse pointů.
- Smazaný, ale ještě otevřený soubor (POSIXové mazání) zatím ne - až
  s vrstvou handlů.
- Přejmenování složky, ve které je otevřený soubor, odmítne už Windows
  (stejně jako u NTFS).
- Pravidla velikosti písmen se liší v detailech: náš FS porovnává po
  Unicode převodu na malá písmena a NFC, Windows podle vlastní tabulky
  velkých písmen. Dvojice jako `ı`/`i` (turecké bez tečky) jsou pro náš FS
  různá jména, pro Windows stejná; mezipaměť Windows je pak může zaměnit.
  Každé jméno ale v našem FS vede jen na svůj objekt.
- Volné místo ve vlastnostech jednotky se zpřesní až po commitu (odložená
  alokace).
- Panika v `osfs` = okamžité ukončení (profil `panic = "abort"`); data
  zůstanou ve stavu po posledním commitu.

## Datasety a verze

**Dataset** je složka s vlastní historií: vlastní snapshoty, klony,
rozdíly verzí a obnova (podrobně v [souborovy-system.md](souborovy-system.md#datasety-a-klony)).
Pro programy a průzkumník je to obyčejná složka.

| Příkaz | Co dělá |
|---|---|
| `osfs dataset create <obraz> <cesta>` | nový prázdný dataset (složka) |
| `osfs dataset list <obraz>` | datasety svazku: číslo, cesta, počet snapshotů, u klonu původ |
| `osfs dataset delete <obraz> <cesta>` | smaže dataset **i se snapshoty**; odmítne dataset s vnořeným datasetem a dataset, z jehož snapshotu je klon |
| `osfs snapshot <obraz> <dataset> <jméno>` | snapshot datasetu (stav jen pro čtení) |
| `osfs snapshots <obraz> [dataset]` | snapshoty jednoho nebo všech datasetů |
| `osfs clone <obraz> <dataset> <snapshot> <cesta>` | zapisovatelný klon snapshotu jako nový dataset (okamžitý, sdílí bloky) |
| `osfs diff <obraz> <dataset> <od> [do]` | změny mezi snapshoty, bez `[do]` proti živé verzi |
| `osfs diff <obraz> --clone <cesta>` | co změnil klon proti snapshotu, ze kterého vznikl |
| `osfs restore <obraz> <dataset> <snapshot> <cesta> <cíl>` | obnova souboru nebo složky **jako kopie** vedle originálu (`<cíl>` nesmí existovat) |

**Cesty:** dataset a `<cíl>` obnovy jsou cesty ve svazku (`/projekt`,
`/projekt/cfg (puvodni).toml`). Cesta ve snapshotu, zdroj obnovy a výpis
rozdílu jsou **uvnitř datasetu** (`/cfg.toml` = `/projekt/cfg.toml`).
Výpis rozdílu: `A` přidáno, `D` smazáno, `M` změněn obsah, `R` přesun
nebo přejmenování (`stará -> nová`), `RM` přesun i změna; složky končí `/`.

> ⚠️ **Obraz nesmí být připojený.** Tyto příkazy obraz otevírají
> výhradně: když ho má otevřený `osfs mount` (nebo jiný `osfs`), skončí
> hláškou `obraz je pripojeny nebo otevreny jinym procesem osfs` a nic
> nezmění. Dva zapisovatelé by svazek poškodili a čtení by vidělo starý
> stav. Řízení přes běžící `mount` přijde později; snapshot připojené
> jednotky už teď jde přes [rozšířené API](osfs-api.md): `osfs snapshot Q:
> /projekt jméno`.
>
> ⚠️ `osfs dataset delete` smaže i všechny snapshoty datasetu - **nevratné**.

### První ukázka: snapshot, klon, změny, diff, obnova

```text
> osfs format demo.img --size 64M
demo.img: naformatovano, 64 MiB, kontrolni soucty Xxh3
> osfs dataset create demo.img /projekt
/projekt: dataset vytvoren (cislo 2)
> osfs put demo.img cfg.toml /projekt/cfg.toml          (a = 1)
> osfs put demo.img main.rs /projekt/main.rs
> osfs snapshot demo.img /projekt pred-upravou
/projekt@pred-upravou: snapshot vytvoren (id 1)

  ... práce: změna cfg.toml (a = 2), nový soubor pomocnik.rs ...

> osfs diff demo.img /projekt pred-upravou
M  /cfg.toml
A  /pomocnik.rs

> osfs clone demo.img /projekt pred-upravou /zkouska
/zkouska: klon /projekt@pred-upravou (dataset 3), zapisovatelny, sdili bloky
> osfs put demo.img cfg2.toml /zkouska/main.rs           (zkouška bokem)
> osfs diff demo.img --clone /zkouska
M  /main.rs

> osfs dataset list demo.img
   1  /  (snapshotu: 0)
   2  /projekt  (snapshotu: 1)
   3  /zkouska  (snapshotu: 0)  klon z /projekt@pred-upravou

> osfs restore demo.img /projekt pred-upravou /cfg.toml "/projekt/cfg (puvodni).toml"
/projekt@pred-upravou:/cfg.toml -> /projekt/cfg (puvodni).toml: obnoveno 6 B (kopie, original beze zmeny)
> osfs cat demo.img "/projekt/cfg (puvodni).toml"
a = 1

> osfs dataset delete demo.img /projekt
osfs: /projekt: z jeho snapshotu existuje klon - nejdriv smaz klon (osfs dataset list ukaze, ktery)
> osfs dataset delete demo.img /zkouska
/zkouska: dataset smazan i se snapshoty
```

### Datasety po připojení (`osfs mount`)

- Dataset se ve Windows ukáže jako **obyčejná složka**; cesta do ní projde
  (otevírání, zápis, přejmenování uvnitř jako jinde). `osfs ls` ho
  označí `dset`.
- **Přesun mezi datasety** (např. z `Q:\` do `Q:\projekt`) jsou dva různé
  stromy: ovladač vrátí „jiné zařízení“ a Windows přesun udělá kopií
  a smazáním, stejně jako mezi dvěma disky (trvá podle velikosti dat).
- **Smazat dataset z Windows nejde** (složka vrátí „přístup odepřen“):
  smazala by se celá historie. Pozor: rekurzivní mazání (Shift+Delete
  složky, `rmdir /s`) přesto smaže **soubory uvnitř** - složka datasetu
  zůstane a obsah jde vrátit ze snapshotu (`osfs restore`).
- Snapshoty, klony, rozdíly a obnova zatím přes příkazy výše (s odpojeným
  obrazem) nebo snapshot přes rozšířené API.

## Rozšířené API pro aplikace

Aplikace (např. MagnumOpus Explorer) může z připojené jednotky číst i to,
co Win32 neumí: okamžité velikosti složek, skrytá jména, snapshoty, stav
svazku. Popis: [osfs-api.md](osfs-api.md). Z příkazové řádky:
`osfs info Q:`, `osfs ls Q: /`, `osfs cat Q: /CON.txt`,
`osfs snapshots Q: [dataset]`, `osfs snapshot Q: <dataset> <jméno>`,
`--at <snapshot>` (snapshot datasetu, ve kterém cesta leží).

## Ladění

`OSFS_DEBUG=1` vypisuje na chybový výstup operace, které mění jména
(vytvoření, smazání, přejmenování):

```text
> set OSFS_DEBUG=1
> osfs mount D:\zkouska.img Q:
osfs: create /Slozka/a.txt (volby 0x3400060)
osfs: rename /Slozka/a.txt -> /Slozka/A.TXT (nahradit: 0)
osfs: delete /Slozka/A.TXT
```

Časté potíže:

| Hláška | Příčina |
|---|---|
| `nejde nacist ...winfsp-x64.dll` | WinFsp není nainstalovaný |
| `pripojeni na Q: selhalo: 0x...` | písmeno už je obsazené |
| `Q: uz ho pripojuje jiny proces osfs mount` | jiné okno už `Q:` připojuje |
| `obraz ma otevreny jiny proces osfs` | obraz už je připojený (`mount`) nebo se do něj právě zapisuje (`put`) - nejdřív odpojit |
| `svazek je jen pro cteni` | nejistá obnova nebo nečitelný log - pusť `osfs check` |
| Windows po zasunutí disku nabízí „naformátovat“ | **neklikat** - Windows náš formát neznají (až budou podporované disky) |

## Co v API `fs` chybí (pro další verze)

- **Jméno položky podle cesty** (`stat` nevrací uložené jméno): ovladač
  ho kvůli normalizovanému jménu hledá ve výpisu rodiče (s mezipamětí
  jmen). První otevření v obřím adresáři načte celý jeho výpis.
- **Nastavení atributů** (příznak jen pro čtení, skrytý). Časy už jdou: ovladač
  ukládá čas vytvoření, zápisu a přístupu z `SetBasicInfo` přes `Fs::set_times`.
- **Informace „je co ukládat“** - commit se teď hlídá vlastním příznakem.
- **Zámek zařízení** v knihovně (nebo aspoň v `FileDevice`) - teď ho drží
  `osfs` sám.
- **Atomické nahrazení při přejmenování** - teď odsunutí cíle, přejmenování
  a smazání odsunutého pod jedním zámkem.
- **Operace přes handle/oid** místo cest (vrstva handlů).
