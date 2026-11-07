# Kódy chyb

Jádro i userspace používají jeden společný typ chyb (`abi::Error`,
`libs/abi/src/error.rs`). Každá chyba má:

- **stabilní číselný kód** (vždy záporný) - syscall ho vrací v `rax`,
  objevuje se v logu a nikdy se nemění ani nerecykluje,
- **název** (např. `BAD_HANDLE`) - stejný v logu, v kódu i zde,
- **krátký popis** (`Error::description()`) pro hlášení.

Úspěšný syscall vrací v `rax` nezáporné číslo, neúspěšný záporný kód
chyby. libos ho převádí na `Result<_, Error>`.

Test `cargo test -p abi` (spouští ho i `cargo xtask test`) ověřuje, že je
v tomto dokumentu i v anglické verzi popsaná každá chyba.

| Kód | Název | Stručně |
|---:|---|---|
| -1 | `BAD_HANDLE` | handle neexistuje nebo už byl zavřen |
| -2 | `ACCESS_DENIED` | handle nemá potřebná práva |
| -3 | `WRONG_TYPE` | objekt je jiného typu |
| -4 | `TIMED_OUT` | vypršel čas čekání |
| -5 | `NO_MEMORY` | nedostatek paměti |
| -6 | `NO_RESOURCES` | vyčerpaný limit |
| -7 | `INVALID_ARGS` | neplatný argument |
| -8 | `NOT_FOUND` | požadovaná věc neexistuje |
| -9 | `NOT_SUPPORTED` | operace není podporovaná |
| -10 | `CORRUPTED` | poškozená nebo neplatná data |
| -11 | `BAD_ADDRESS` | neplatný ukazatel do paměti procesu |
| -12 | `SHOULD_WAIT` | operace by musela čekat |
| -13 | `PEER_CLOSED` | druhá strana kanálu je zavřená |
| -14 | `BUFFER_TOO_SMALL` | buffer je příliš malý |
| -15 | `OUT_OF_RANGE` | offset nebo rozsah mimo objekt |
| -16 | `BAD_STATE` | objekt je ve stavu, kdy operace nedává smysl |
| -17 | `BAD_SYSCALL` | neznámý syscall |
| -18 | `CANCELED` | operace přerušena ukončováním |
| -19 | `ALREADY_EXISTS` | položka už existuje |
| -20 | `NOT_A_DIRECTORY` | část cesty není adresář |
| -21 | `IS_A_DIRECTORY` | je to adresář |
| -22 | `NOT_EMPTY` | adresář není prázdný |
| -23 | `NO_SPACE` | na zařízení není místo |
| -24 | `IO_ERROR` | chyba čtení nebo zápisu zařízení |
| -25 | `READ_ONLY` | jen pro čtení |
| -26 | `LOCKED` | objekt je otevřený nebo zamčený jiným handlem |
| -27 | `CONFLICT` | objekt se mezitím změnil |
| -28 | `JOURNAL_GAP` | požadované záznamy žurnálu už nejsou uložené |

## Podrobně

### `BAD_HANDLE` (-1)

**Význam:** hodnota handlu v tabulce procesu neodpovídá žádnému otevřenému
handlu.

**Kdy vzniká:** handle byl zavřen (`HANDLE_CLOSE`), předán jinému procesu
kanálem (předáním z tabulky zmizí), nebo jde o nesmyslnou hodnotu (0 není
nikdy platný handle). Každý slot tabulky má generaci, takže starý handle
nezačne po znovupoužití slotu omylem ukazovat na jiný objekt - místo toho
vrací tuto chybu.

**Co s tím:** v programu jde téměř vždy o chybu (použití po zavření,
dvojité zavření). Zkontrolovat, kdo handle zavírá nebo předává.

### `ACCESS_DENIED` (-2)

**Význam:** handle existuje, ale nemá právo, které operace vyžaduje.

**Kdy vzniká:** např. `OBJECT_SIGNAL` bez práva `SIGNAL`, `VM_MAP` objektu
bez práva `MAP`, `CHANNEL_WRITE` s handlem bez `TRANSFER`, nebo
`HANDLE_DUPLICATE`, který by práva přidával (práva lze jen ubírat).
Požadovaná práva uvádí u každého syscallu [syscally.md](syscally.md).

**Co s tím:** je to vlastnost bezpečnostního modelu, ne chyba jádra.
Proces dostal handle s omezenými právy záměrně; potřebuje-li víc, musí mu
je dát ten, kdo handle vydal.

### `WRONG_TYPE` (-3)

**Význam:** objekt za handlem je jiného typu, než operace očekává.

**Kdy vzniká:** např. `CHANNEL_READ` s handlem eventu, `VM_MAP` s handlem,
který není proces nebo objekt paměti.

**Co s tím:** chyba v programu - zaměněné handly.

### `TIMED_OUT` (-4)

**Význam:** čekání nebylo splněno do zadaného termínu.

**Kdy vzniká:** `OBJECT_WAIT_ONE` a `OBJECT_WAIT_MANY` s jiným termínem
než `DEADLINE_INFINITE`. `OBJECT_WAIT_MANY` s nulovým počtem objektů je
uspání do termínu a končí vždy touto chybou (libos `time::sleep` ji
ignoruje).

**Co s tím:** běžný stav, ne chyba. Jádro ji neloguje.

### `NO_MEMORY` (-5)

**Význam:** jádru došla fyzická paměť pro požadovanou operaci.

**Kdy vzniká:** vytvoření objektu paměti (`VMO_CREATE` alokuje stránky
hned), tabulky stránek při `VM_MAP`, zásobník jádra pro nové vlákno, nový
proces.

**Co s tím:** uvolnit paměť (zavřít nepotřebné objekty, ukončit procesy).
Opakovaný výskyt znamená únik paměti nebo příliš malou RAM.

### `NO_RESOURCES` (-6)

**Význam:** vyčerpal se jiný limit než paměť.

**Kdy vzniká:** plná tabulka handlů procesu (max. 1 048 575 handlů), plná
fronta kanálu (`CHANNEL_MAX_QUEUED` = 1024 zpráv; zápis pak vrací
`SHOULD_WAIT`), nedostatek volného virtuálního prostoru procesu při
`VM_MAP` bez `MAP_FIXED`.

**Co s tím:** zavírat nepotřebné handly, číst zprávy z kanálu, mapovat
menší úseky.

### `INVALID_ARGS` (-7)

**Význam:** argument je mimo povolený rozsah nebo je kombinace argumentů
zakázaná.

**Kdy vzniká:** např. `VM_MAP` s příznaky zápis i spuštění zároveň (W^X),
bez `MAP_READ`, s nezarovnanou adresou nebo délkou; `OBJECT_SIGNAL` s jiným
bitem než `SIGNALED`; jméno procesu delší než 64 B nebo ne-UTF-8; poslání
konce kanálu sebou samým; stejný handle dvakrát v jedné zprávě; `VMO_CREATE`
s velikostí 0 nebo nad 4 GiB.

**Co s tím:** chyba v programu - opravit argumenty podle
[syscally.md](syscally.md).

### `NOT_FOUND` (-8)

**Význam:** požadovaná věc neexistuje.

**Kdy vzniká:** `VM_UNMAP` rozsahu, který není přesně tvořen celými
mapováními (částečné odmapování zatím není podporované); v jádře chybějící
ACPI tabulka MADT nebo program `init` v initfs.

**Co s tím:** u `VM_UNMAP` odmapovat přesně adresu a délku, které vrátilo
`VM_MAP`.

### `NOT_SUPPORTED` (-9)

**Význam:** operace je platná, ale tato verze systému ji neumí.

**Kdy vzniká:** zatím ji žádný syscall nevrací; je vyhrazená pro
rozšíření (nepodporovaný hardware, volba, kterou jádro nezná).

**Co s tím:** použít jiný postup nebo počkat na podporu.

### `CORRUPTED` (-10)

**Význam:** data jsou poškozená nebo nemají očekávaný formát.

**Kdy vzniká:** neplatná ACPI tabulka (špatný checksum nebo délka),
poškozený initfs, program `init` není platný ELF; v libos neplatný ELF při
`spawn`.

**Co s tím:** u firmware tabulek jádro tabulku vynechá a pokračuje
(viz [hlasky.md](hlasky.md)); u souborů opravit nebo znovu sestavit obraz.

### `BAD_ADDRESS` (-11)

**Význam:** ukazatel, který proces předal jádru, nemíří do platné paměti
procesu s potřebnými právy.

**Kdy vzniká:** buffer syscallu leží (i jen zčásti) v nenamapované paměti,
mimo spodní polovinu adresního prostoru, nebo jde o zápis do mapování jen
pro čtení (např. do kódu programu). Jádro celý rozsah ověří předem, takže se
nic neprovede napůl.

**Co s tím:** chyba v programu - neplatný ukazatel nebo délka bufferu.

### `SHOULD_WAIT` (-12)

**Význam:** operace by teď musela čekat; neblokující volání ji proto
nedokončilo.

**Kdy vzniká:** `CHANNEL_READ` z prázdné fronty, `CHANNEL_WRITE` do plné
fronty.

**Co s tím:** počkat na signál (`READABLE` u čtení) a zkusit znovu. libos
`Channel::read` to dělá sám. Jádro tuto chybu neloguje.

### `PEER_CLOSED` (-13)

**Význam:** druhý konec kanálu už neexistuje.

**Kdy vzniká:** `CHANNEL_WRITE`, když druhý konec zavřel poslední handle;
`CHANNEL_READ`, když je fronta prázdná a druhý konec zavřený (zprávy, které
ve frontě už jsou, jde dočíst). Konec kanálu zároveň dostane signál
`PEER_CLOSED`.

**Co s tím:** běžný konec komunikace - druhá strana skončila. Např. `echo`
při ní končí s kódem 0.

### `BUFFER_TOO_SMALL` (-14)

**Význam:** výsledek se nevejde do bufferu.

**Kdy vzniká:** `CHANNEL_READ`, když je zpráva delší než buffer dat nebo
nese víc handlů, než je místa. Zpráva zůstane ve frontě a do
`out_actual_ptr` jádro zapíše potřebné velikosti.

**Co s tím:** zvětšit buffery podle vrácených velikostí a číst znovu (libos
to dělá automaticky).

### `OUT_OF_RANGE` (-15)

**Význam:** offset nebo rozsah leží (i zčásti) mimo objekt.

**Kdy vzniká:** `VMO_READ` / `VMO_WRITE` přes konec objektu paměti,
`VM_MAP`, kde `vmo_offset + len` přesahuje velikost objektu.

**Co s tím:** chyba v programu - zkontrolovat velikost objektu.

### `BAD_STATE` (-16)

**Význam:** objekt je ve stavu, ve kterém operace nedává smysl.

**Kdy vzniká:** `PROCESS_EXIT_CODE` u procesu, který ještě běží;
`PROCESS_START` u již spuštěného procesu; `THREAD_START` u spuštěného
vlákna nebo v procesu, který ještě nebyl spuštěn; `VM_MAP` s `MAP_FIXED`
na místo, kde už něco je; vytvoření vlákna v končícím procesu.

**Co s tím:** dodržet pořadí operací (popsané u syscallů); u exit kódu nejdřív
počkat na signál `TERMINATED`.

### `BAD_SYSCALL` (-17)

**Význam:** číslo syscallu jádro nezná.

**Kdy vzniká:** program volá jádro přímo s neplatným číslem, nebo program
sestavený pro jinou verzi rozhraní. Vrací ji i libos, pokud jádro vrátí
neznámý záporný kód.

**Co s tím:** používat libos (čísla syscallů nejsou stabilní rozhraní).

### `CANCELED` (-18)

**Význam:** operace byla přerušena, protože vlákno nebo proces končí.

**Kdy vzniká:** vlákno čeká (např. `OBJECT_WAIT_ONE`) a jeho proces je
ukončen (`PROCESS_KILL`, `PROCESS_EXIT` z jiného vlákna, pád jiného
vlákna). Čekání skončí touto chybou a vlákno skončí při návratu do
userspace - program ji tedy obvykle ani neuvidí.

**Co s tím:** nic, proces končí.

### `ALREADY_EXISTS` (-19)

**Význam:** položka se zadaným jménem už existuje.

**Kdy vzniká:** vytvoření souboru nebo adresáře, přejmenování na jméno,
které už v cílovém adresáři je, vytvoření snapshotu se stejným jménem.
Jména se porovnávají bez ohledu na velikost písmen a po normalizaci
Unicode, takže `Foto.JPG` koliduje s `foto.jpg`.

**Co s tím:** zvolit jiné jméno, nebo existující položku nejdřív odstranit.

### `NOT_A_DIRECTORY` (-20)

**Význam:** část cesty není adresář.

**Kdy vzniká:** cesta `/a/b/c`, kde `a` nebo `b` je soubor; výpis
adresáře u souboru.

**Co s tím:** opravit cestu.

### `IS_A_DIRECTORY` (-21)

**Význam:** operace pro soubor dostala adresář.

**Kdy vzniká:** čtení nebo zápis dat adresáře.

**Co s tím:** opravit cestu (adresář se čte výpisem).

### `NOT_EMPTY` (-22)

**Význam:** adresář není prázdný.

**Kdy vzniká:** odstranění adresáře, který obsahuje položky (mazání je
záměrně nerekurzivní).

**Co s tím:** nejdřív odstranit obsah.

### `NO_SPACE` (-23)

**Význam:** na zařízení došlo místo.

**Kdy vzniká:** zápis dat nebo změna metadat, když nejsou volné bloky.
Kvůli copy-on-write potřebuje i přepsání existujících dat volné bloky;
bloky patřící snapshotům se neuvolňují.

**Co s tím:** odstranit nepotřebná data, případně snapshoty (až to bude
podporované).

### `IO_ERROR` (-24)

**Význam:** zařízení nahlásilo chybu čtení nebo zápisu.

**Kdy vzniká:** chyba ovladače nebo média; v nástrojích chyba souboru
s obrazem.

**Co s tím:** zkontrolovat zařízení; data, která neprošla commitem, se
mohla ztratit, uložený stav zůstává konzistentní.

### `READ_ONLY` (-25)

**Význam:** pokus o zápis do objektu jen pro čtení.

**Kdy vzniká:** zápis do snapshotu.

**Co s tím:** zapisovat do živého souborového systému.

### `LOCKED` (-26)

**Význam:** objekt je otevřený v nekompatibilním sdílecím režimu, nebo je
rozsah souboru zamčený jiným handlem.

**Kdy vzniká:** otevření souboru, který jiný handle otevřel bez sdílení
požadovaného přístupu (`FILE_SHARE_*` ve vrstvě Windows); smazání nebo
přejmenování otevřeného souboru bez sdílení mazání; zápis do rozsahu
s povinným zámkem; zámek rozsahu, který se překrývá s cizím.

**Co s tím:** počkat, až druhý handle soubor zavře nebo zámek uvolní,
případně otevřít se slučitelným režimem.

### `CONFLICT` (-27)

**Význam:** podmíněná operace se neprovedla, protože se objekt od zadané
revize změnil (nebo cíl, který neměl existovat, existuje).

**Kdy vzniká:** podmíněné uložení (`rename_replace_if`, `REPLACE_COMMIT`
v protokolu v2), když soubor mezitím změnil někdo jiný - jiný editor,
agent, synchronizace.

**Co s tím:** nic se nezměnilo a nová data zůstala klientovi; nabídnout
uložení vedle, porovnání verzí, nebo vědomé přepsání (operace s novou
revizí).

### `JOURNAL_GAP` (-28)

**Význam:** odběratel žurnálu změn žádá záznamy od pořadového čísla, které
je starší než nejstarší uložený záznam - mezitím je zahodila retence
(počet záznamů nebo stáří, nastavení datasetu).

**Kdy vzniká:** `journal_read` / `JOURNAL_READ` a `JOURNAL_WATCH`, když se
indexer, synchronizace nebo průzkumník dlouho neozval. Nikdy se záznamy
tiše nevynechají.

**Co s tím:** načíst nový výchozí stav: `VIEW` (dočasný snapshot a číslo
posledního záznamu, který obsahuje), projít ho a navázat odběrem od čísla
+ 1. Častější odběr nebo delší retence zabrání opakování.

## Návratové kódy procesů

Proces končí kódem z `PROCESS_EXIT` (0 = úspěch, jiné hodnoty určuje
program). Když proces ukončí jádro, použije kód z vyhrazeného rozsahu
(`libs/abi/src/process.rs`):

| Kód | Význam |
|---:|---|
| 0 | úspěch |
| -1 000 000 | ukončen jiným procesem (`PROCESS_KILL`) |
| -1 000 001 | panika v programu (panic handler libos) |
| -1 000 002 | poslední vlákno skončilo bez `PROCESS_EXIT` |
| -1 000 100 - vektor | výjimka CPU v programu (pád); např. page fault (vektor 14) = -1 000 114, dělení nulou (0) = -1 000 100, neplatná instrukce (6) = -1 000 106, general protection (13) = -1 000 113 |

Při pádu jádro zaloguje vektor, chybový kód, adresu instrukce a u page
faultu i adresu přístupu (viz [hlasky.md](hlasky.md)).
