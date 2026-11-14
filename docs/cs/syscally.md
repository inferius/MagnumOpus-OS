# Syscally

Rozhraní mezi procesem a jádrem. **Programy mají používat libos**
([libos.md](libos.md)) - čísla a přesné argumenty syscallů nejsou stabilní
rozhraní a mění se spolu s libos. Tento dokument popisuje, co jádro dělá,
aby šlo chování dohledat.

## Stabilita ABI

Do verze 1.0 se rozhraní jádra může měnit (programy se překládají spolu se
systémem). Od 1.0 platí:

- **Programy volají jádro přes systémovou knihovnu** (libos, později libc a
  ntdll Windows vrstvy), ne přímo instrukcí `syscall`. Stabilní je rozhraní
  knihovny; čísla syscallů jsou interní věc knihovny a jádra (stejně jako
  ve Windows, macOS nebo Fuchsii).
- Čísla syscallů se **nikdy nemažou ani znovu nepoužijí**; změna chování =
  nový syscall (např. `..._V2`), starý zůstává.
- Struktury předávané jádru nesou délku nebo verzi, nová pole se přidávají
  jen na konec.
- Nové kódy chyb a signály nesmí změnit význam existujících.
- Startovní zpráva procesu nese verzi; jádro a knihovna podporují i starší.

## Konvence (x86_64)

- Instrukce `syscall`, číslo v `rax`, argumenty v `rdi`, `rsi`, `rdx`,
  `r10`, `r8`, `r9`.
- Výsledek v `rax`: nezáporné číslo = úspěch, záporné = kód chyby
  ([chyby.md](chyby.md)). Ostatní registry kromě `rcx` a `r11` (přepíše je
  instrukce) zůstanou zachované.
- Handle je 32bitová hodnota platná jen v tabulce volajícího procesu.
- Ukazatele míří do paměti volajícího procesu. Jádro celý rozsah ověří
  (namapováno, u výstupu i zapisovatelné), jinak vrátí `BAD_ADDRESS`
  a nic neprovede.
- Čas a termíny jsou v nanosekundách od startu (`CLOCK_MONOTONIC`).
  `DEADLINE_INFINITE` = `u64::MAX` = čekat bez limitu.
- Čísla a limity: `libs/abi/src/syscall.rs`. Práva: `libs/abi/src/object.rs`.

Neúspěšné syscally jádro loguje na úrovni Debug (kromě `SHOULD_WAIT`
a `TIMED_OUT`, které jsou běžné).

## Logování a čas

### `DEBUG_LOG`

`(ptr, len) -> 0`. Zapíše text (UTF-8, max. 1024 B, neplatné UTF-8 se
nahradí) do logu jádra s prefixem `[jméno id]` procesu. Do vzniku služby
logu je to jediný výstup programů. Chyby: `BAD_ADDRESS`, `INVALID_ARGS`
(příliš dlouhý text). Obálka `libos::sys::debug_log` (a `println!`) delší text
rozdělí na kusy po 1024 B, přednostně za koncem řádku
(`abi::syscall::debug_log_chunks`), takže se dlouhá zpráva neztratí celá.

### `CLOCK_MONOTONIC`

`() -> ns`. Monotónní čas v nanosekundách od startu jádra (TSC
kalibrovaný proti HPET). Nemůže selhat.

### `CLOCK_REALTIME`

`(out_ptr) -> 0`. Reálný čas („hodiny na zdi“) pro časy souborů
a zobrazení. Na `out_ptr` zapíše 16 B (little endian):

| Offset | Typ | Význam |
|---|---|---|
| 0 | i64 | sekundy od 1970-01-01 00:00:00 UTC (záporné = dřív) |
| 8 | u32 | nanosekundy uvnitř sekundy (0..10⁹) |
| 12 | u16 | zdroj: 1 UEFI, 2 CMOS RTC, 3 nastaveno za běhu, 4 synchronizováno podle sítě (`CLOCK_SET` s `NETWORK` nebo `CLOCK_ADJUST` s posunem) |
| 14 | u16 | příznaky: bit 0 = pásmo neuvedeno, čas brán jako UTC |

Jádro si při startu uloží reálný čas v okamžiku 0 monotónních hodin; výsledek
je ten počátek + `CLOCK_MONOTONIC`. Přesnost počátku je 1 s (rozlišení
firmwaru i RTC). Na rozdíl od `CLOCK_MONOTONIC` **není monotónní**: skočí
při nastavení hodin (`CLOCK_SET`). Pro měření a termíny
vždy `CLOCK_MONOTONIC`. Obal: `libos::time::wall()`, `wall_reading()`.
Chyby: `NOT_SUPPORTED` (jádro reálný čas nezná), `BAD_ADDRESS`.
Podrobnosti: [Reálný čas](cas.md).

### `CLOCK_SET`

`(resource, in_ptr, flags) -> 0 | 1`. Nastaví reálný čas. Vyžaduje handle
na Resource s právem `SET_CLOCK`. Má ho jen kořenový Resource a `init` ho
předá jen službě času `timed` (bez `MANAGE`). Na `in_ptr` je 16 B (little
endian, `abi::time::SetRequest`):

| Offset | Typ | Význam |
|---|---|---|
| 0 | i64 | nový čas: sekundy od 1970-01-01 00:00:00 UTC (roky 1970–9999, jinak `INVALID_ARGS`) |
| 8 | u32 | nanosekundy (0..10⁹, jinak `INVALID_ARGS`) |
| 12 | i32 | `rtc_offset`: sekundy přičtené k UTC při zápisu do hardwarových hodin (RTC v místním čase = posun pásma, jinak 0); nejvýš ±18 h, jinak `INVALID_ARGS` |

`flags`: bit 0 `WRITE_HARDWARE` = zapsat čas i do hardwarových hodin (x86:
CMOS RTC), bit 1 `NETWORK` = čas je ze synchronizace podle sítě (zdroj 4).
Ostatní bity = `INVALID_ARGS`. Vrací 1, když se hardwarové
hodiny zapsaly. Vrací 0 bez `WRITE_HARDWARE`, když hodiny chybí (aarch64,
ACPI hlásí, že RTC není) nebo když se rok nevejde do jejich rozsahu. S
`WRITE_HARDWARE` může volání spát až 1 s: jádro zapisuje do RTC v polovině
sekundy, aby hodiny tikaly na hranici sekund. Čas
systému se nastaví v každém případě. Poté `CLOCK_REALTIME` hlásí zdroj 3
(nastaveno za běhu; s `NETWORK` zdroj 4) a žádné příznaky. Termíny čekání se nemění, počítají se
z `CLOCK_MONOTONIC`. Obal: `libos::time::set`, `Resource::set_clock`.
Chyby: `ACCESS_DENIED` (chybí `SET_CLOCK`), `WRONG_TYPE`, `BAD_HANDLE`,
`INVALID_ARGS`, `BAD_ADDRESS`. Podrobnosti:
[Nastavení hodin](cas.md#nastavení-hodin-clock_set).

### `CLOCK_ADJUST`

`(resource, offset_ns i64, freq_ppb i64, flags, out_ptr) -> 0`. Plynulá
korekce reálného času bez skoku. Vyžaduje Resource s právem `SET_CLOCK`.
Příznaky (`abi::time::adjust`):

- `OFFSET` (1): začít dorovnávat posun `offset_ns`, nahradí zbytek
  předchozího. Dorovnává se rychlostí 500 ppm (`MAX_SLEW_PPB`), takže
  posun 1 s trvá 2000 s. Nejvýš 1 hodina (`MAX_OFFSET_NS`).
- `FREQUENCY` (2): trvalá oprava frekvence `freq_ppb`, nejvýš ±500 ppm
  (`MAX_FREQ_PPB`).
- bez příznaků: jen stav.

Na `out_ptr` (0 = nic) zapíše `abi::time::AdjustStatus` (24 B): zbývá
dorovnat (ns), frekvence (ppb), aktuální rychlost dorovnávání (ppb).
Reálný čas nikdy nejde pozpátku. S `OFFSET` hlásí `CLOCK_REALTIME`
zdroj 4 (synchronizováno podle sítě). `CLOCK_SET` dorovnávání zruší
a frekvenci nechá. Obal: `libos::time::adjust`. Chyby: `ACCESS_DENIED`
(chybí `SET_CLOCK`), `INVALID_ARGS` (meze, neznámé příznaky), `BAD_STATE`
(reálný čas není znám), `WRONG_TYPE`, `BAD_HANDLE`, `BAD_ADDRESS`.
Podrobnosti: [Plynulá korekce](cas.md#plynulá-korekce-clock_adjust).

## Handly

### `HANDLE_CLOSE`

`(handle) -> 0`. Zavře handle. Objekt zanikne, když na něj nevede žádný
handle ani interní odkaz (např. zavření posledního konce kanálu nastaví
druhému konci `PEER_CLOSED`). Chyby: `BAD_HANDLE`.

### `HANDLE_DUPLICATE`

`(handle, rights) -> handle`. Nový handle na stejný objekt. `rights` musí
být podmnožinou práv původního handlu; hodnota `u32::MAX` práva zachová.
Vyžaduje právo `DUPLICATE`. Chyby: `BAD_HANDLE`, `ACCESS_DENIED` (chybí
`DUPLICATE` nebo by se práva přidávala), `NO_RESOURCES`.

## Čekání a eventy

### `OBJECT_WAIT_ONE`

`(handle, mask, deadline, out_signals_ptr) -> 0`. Čeká, dokud signály
objektu neprotnou `mask`, nebo do termínu. Do `out_signals_ptr` (pokud není
0) zapíše aktuální signály objektu - i při `TIMED_OUT`. Vyžaduje `WAIT`.
Chyby: `BAD_HANDLE`, `ACCESS_DENIED`, `TIMED_OUT`, `CANCELED`,
`BAD_ADDRESS`.

### `OBJECT_WAIT_MANY`

`(items_ptr, count, deadline) -> index`. Čeká na více objektů najednou
(max. 64). `items` je pole `WaitItem { handle: u32, mask: u32, observed: u32 }`;
jádro vrátí index prvního splněného objektu a do `observed` všech položek
zapíše jejich aktuální signály. `count` = 0 je uspání do termínu (končí
`TIMED_OUT`). Vyžaduje `WAIT` u všech handlů. Chyby: jako `OBJECT_WAIT_ONE`
a `INVALID_ARGS` (count > 64).

### `OBJECT_SIGNAL`

`(handle, clear, set) -> 0`. Změní signály eventu: nejdřív smaže bity
`clear`, pak nastaví `set`. Proces smí měnit jen `SIGNALED`; ostatní signály
řídí jádro. Vyžaduje `SIGNAL`. Chyby: `BAD_HANDLE`, `ACCESS_DENIED`,
`WRONG_TYPE` (není event), `INVALID_ARGS` (jiný bit než `SIGNALED`).

### `EVENT_CREATE`

`() -> handle`. Nový event (nesignalizovaný) s právy `DUPLICATE`,
`TRANSFER`, `WAIT`, `SIGNAL`. Chyby: `NO_RESOURCES`.

## Porty

Port je fronta událostí pro čekání na **libovolně mnoho objektů** (služby
se stovkami kanálů, např. `netd`). `OBJECT_WAIT_MANY` je omezený na 64
objektů a při každém čekání registruje vlákno u všech. Port místo toho
drží **jednorázové registrace**: objekt, jehož signály protnou masku,
vloží do portu paket (klíč + signály) a registrace zanikne. Program
paket vyzvedne a po obsluze objekt zaregistruje znovu. Cena události
nezávisí na počtu hlídaných objektů.

- **Místo pro paket se rezervuje při registraci**, takže doručení
  (i z obsluhy přerušení) nealokuje a paket se neztratí. Každá registrace
  a nevyzvednutý paket stojí 64 B z rozpočtu paměti jádra jobu procesu,
  který port vytvořil ([Joby](#joby-rozpočty-prostředků)); pevný počet
  není. Strženo se vrátí vyzvednutím paketu, zánikem objektu nebo
  zavřením portu. Pakety leží v blocích po 4 KiB (žádná velká souvislá
  alokace), prázdný port vrátí přebytečné bloky.
- **Zaniklý objekt** (zavřený poslední handle) vrátí rezervaci svých
  nedoručených registrací. **Zavřený port**: objekty jeho registrace
  zahodí.
- Port má signál `READABLE`, dokud v něm je paket. Lze na něj tedy čekat
  i přes `OBJECT_WAIT_ONE`/`OBJECT_WAIT_MANY`.
- Pakety přicházejí v pořadí, v jakém nastaly.
- Obaly: `libos::Port` (`create`, `wait_async`, `wait`).

```rust
let port = libos::Port::create()?;
port.wait_async(ch.handle(), 1, Signals::READABLE)?;     // klíč 1
port.wait_async(ch.handle(), 2, Signals::PEER_CLOSED)?;  // klíč 2
loop {
    let p = port.wait(None)?;
    match p.key {
        1 => { /* přečíst zprávu */ port.wait_async(ch.handle(), 1, Signals::READABLE)?; }
        2 => break, // druhá strana zavřela
        _ => {}
    }
}
```

### `PORT_CREATE`

`() -> handle`. Nový prázdný port s právy `DUPLICATE`, `TRANSFER`, `WAIT`,
`READ`, `WRITE`. Chyby: `NO_RESOURCES` (rozpočet jobu).

### `OBJECT_WAIT_ASYNC`

`(handle, port, key, mask) -> 0`. Jednorázová registrace: až signály
objektu `handle` protnou `mask`, port dostane paket s `key` (u64, význam
určuje program). Když je podmínka splněná už teď, paket přijde hned.
Vyžaduje `WAIT` na objektu a `WRITE` na portu. Na port samotný registrovat
nejde. Chyby: `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE` (`port` není
port), `NOT_SUPPORTED` (objekt je port), `INVALID_ARGS` (prázdná maska),
`NO_RESOURCES` (vyčerpaný rozpočet jádra jobu), `NO_MEMORY`.

### `PORT_WAIT`

`(port, deadline, out_ptr) -> 0`. Vyjme nejstarší paket a zapíše ho na
`out_ptr` (16 B, `abi::object::PortPacket`): klíč u64, signály objektu
v okamžiku splnění u32, rezerva u32. Prázdný port čeká do termínu
(`deadline` = 0: jen zkusit). Vyžaduje `READ`. Chyby: `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`, `TIMED_OUT`, `CANCELED`, `BAD_ADDRESS`
(paket je pak ztracený).

## Kanály

Zpráva = data (max. 64 KiB) + handly (max. 64). Každý konec má vlastní
frontu příchozích zpráv (max. 1024). Signály konce: `READABLE`,
`PEER_CLOSED`.

### `CHANNEL_CREATE`

`(out_handles_ptr) -> 0`. Vytvoří propojenou dvojici konců a zapíše dva
handly (u32) do `out_handles_ptr`. Práva: `DUPLICATE`, `TRANSFER`, `WAIT`,
`READ`, `WRITE`. Chyby: `NO_RESOURCES`, `BAD_ADDRESS`.

### `CHANNEL_WRITE`

`(handle, data_ptr, data_len, handles_ptr, handles_count) -> 0`. Pošle
zprávu druhému konci. Předávané handly musí mít právo `TRANSFER`, nesmí se
opakovat a nesmí jít o zapisovaný konec ani o jeho protějšek (zpráva by
skončila ve frontě protějšku, který by tak držel sám sebe). Konec kanálu,
v jehož frontě už leží jiný konec kanálu, poslat nejde (`BAD_STATE`): jen
tak nikdy nevznikne cyklus konců, které drží jeden druhého ve frontách a
nikdy se neuvolní. Předávané handly se z tabulky
odesílatele **odeberou vždy, i když zápis selže** - handle tak nikdy nemá
odesílatel i příjemce zároveň. Vyžaduje `WRITE`. Chyby: `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`, `PEER_CLOSED`,
`SHOULD_WAIT` (plná fronta), `BAD_ADDRESS`.

### `CHANNEL_READ`

`(handle, data_ptr, data_cap, handles_ptr, handles_cap, out_actual_ptr) -> 0`.
Vyjme další zprávu: data zkopíruje do `data_ptr`, handly vloží do tabulky
procesu a jejich hodnoty zapíše do `handles_ptr`, do `out_actual_ptr` zapíše
dva u32 (délka dat, počet handlů). Neblokuje. Vyžaduje `READ`. Chyby:
`SHOULD_WAIT` (prázdná fronta), `PEER_CLOSED` (prázdná fronta a druhý konec
zavřený), `BUFFER_TOO_SMALL` (zpráva zůstane ve frontě, do `out_actual_ptr`
se zapíšou potřebné velikosti), `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`,
`BAD_ADDRESS`.

## Paměť

### `VMO_CREATE`

`(size, process) -> handle`. Objekt paměti o velikosti `size` (zaokrouhleno
na 4 KiB, max. 1 TiB), vyplněný nulami. Celá velikost se hned slíbí
(rozpočet uživatelské paměti jobu a globální strop paměti programů), rámce
vzniknou až při prvním zápisu ([sprava-pameti.md](sprava-pameti.md)).
Strhne se z rozpočtu uživatelské paměti jobu procesu `process`
(handle s právem `WRITE`; 0 = volající). Rodič tak vytváří paměť programu
(segmenty, zásobník) na účet potomka. Práva: `DUPLICATE`, `TRANSFER`,
`WAIT`, `READ`, `WRITE`, `MAP`, `EXECUTE`. Chyby: `INVALID_ARGS` (0 nebo
příliš velký), `NO_MEMORY`, `NO_RESOURCES` (rozpočet jobu), `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE` (`process`). `NO_MEMORY` znamená i to, že
se tolik paměti nedá slíbit (strop = volná RAM bez rezervy jádra a bez
už slíbených stránek).

### `VMO_CLONE`

`(vmo, offset, len, process) -> handle`. Klon `len` bajtů objektu od `offset`
s kopií při zápisu: nový objekt sdílí stránky zdroje, dokud do nich někdo
z nich nezapíše (první zápis si udělá kopii; zdroj i klon se navzájem
neovlivní). Klon slíbí celou svou velikost, zdroj stránku navíc za každý
nově sdílený rámec, takže kopie při zápisu nikdy nenarazí na nedostatek
paměti; strhne se z rozpočtu jobu procesu `process` (handle s `WRITE`;
0 = volající). Připnuté stránky (DMA, `VMO_PHYS_ADDR`) klon dostane jako
kopii hned. Zápisy zdroje souběžné s voláním klon může vidět. Práva:
zdroj `READ`; klon má plná práva objektu. Jen běžné objekty. Chyby:
`INVALID_ARGS` (nulová délka, nezarovnaný offset nebo délka),
`OUT_OF_RANGE`, `NOT_SUPPORTED` (souvislý, DMA nebo fyzický objekt),
`NO_MEMORY`, `NO_RESOURCES`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `VMO_SEAL`

`(vmo) -> 0`. Zapečetí objekt paměti: jeho obsah se už nikdy nezmění.
`VMO_WRITE`, mapování pro zápis a `VMO_PHYS_ADDR` pak vrátí
`ACCESS_DENIED`; mapování má nejvyšší práva bez zápisu. Klon
(`VMO_CLONE`) zapečetěného objektu je obyčejný objekt. Nevratné; opakovaná
pečeť nic nedělá. Bez oprávnění JIT jde spouštět jen zapečetěná paměť
(viz [sprava-pameti.md](sprava-pameti.md)). Vyžaduje `WRITE`. Chyby:
`BAD_STATE` (objekt má mapování, které smí zapisovat - i jen jako nejvyšší
právo), `NOT_SUPPORTED` (souvislý, DMA nebo fyzický objekt),
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `VMO_READ`

`(handle, offset, buf_ptr, len) -> 0`. Přečte data objektu (max. 1 MiB na
volání). Vyžaduje `READ`. Chyby: `OUT_OF_RANGE`, `BAD_ADDRESS`,
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`.

### `VMO_WRITE`

`(handle, offset, buf_ptr, len) -> 0`. Zapíše data do objektu (max. 1 MiB).
Vyžaduje `WRITE`. Chyby: jako `VMO_READ`.

### `VM_MAP`

`(process, vmo, vmo_offset, len, flags, addr) -> addr`. Namapuje `len`
bajtů objektu od `vmo_offset` do adresního prostoru procesu `process`
(může být vlastní i jiný, např. při spouštění programu). `flags`:
`MAP_READ` (povinné), `MAP_WRITE`, `MAP_EXECUTE` (nesmí být s `MAP_WRITE` -
W^X), `MAP_FIXED` (přesně na `addr`; jinak je `addr` nápověda a 0 = vybere
jádro). Adresa, délka a offset musí být zarovnané na velikost stránky
procesu (`VM_PAGE_SIZE`, dnes 4 KiB), rozsah v
`0x10000..0x7FFF_FFFF_F000` (`USER_MIN`..`USER_MAX`; poslední stránka
spodní poloviny zůstává vždy nenamapovaná). Práva: proces `WRITE`; objekt `MAP` + `READ`,
případně `WRITE`/`EXECUTE` podle příznaků. Mapování drží objekt naživu i po
zavření handlu. Chyby: `INVALID_ARGS`, `OUT_OF_RANGE`, `BAD_STATE`
(`MAP_FIXED` přes existující mapování; proces už skončil a jeho prostor je
zrušený; bez `MAP_IN_RESERVATION` přes rezervaci; s ním rozsah není celý
v rezervacích), `NO_RESOURCES` (není volné místo),
`NO_MEMORY`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `VM_UNMAP`

`(process, addr, len, flags) -> 0`. Zruší oblasti v rozsahu (mapování
i rezervace), i jejich části - oblast na hranici se rozdělí. Rozsah musí
být celý pokrytý oblastmi bez děr. `flags`: 0, nebo `UNMAP_DECOMMIT` =
mapování se změní na rezervaci (paměť zpět, adresy zůstanou zabrané;
`MEM_DECOMMIT` ve Windows), rezervace zůstanou. Po návratu žádné CPU nemá
starý překlad v TLB. Vyžaduje `WRITE` na procesu. Chyby: `NOT_FOUND`
(díra v rozsahu), `INVALID_ARGS`, `NO_RESOURCES` (rozpočet jádra na
rozdělení oblasti), `BAD_HANDLE`, `ACCESS_DENIED`.

### `VM_RESERVE`

`(process, addr, len, flags) -> addr`. Zabere rozsah adres bez paměti
(rezervace). `flags`: `MAP_FIXED` (přesně na `addr`, jinak volné místo
a `addr` je nápověda), `RESERVE_GUARD` (ochranná oblast: přístup ukončí
proces s hláškou o přetečení zásobníku). Stojí jen záznam oblasti z
rozpočtu jádra jobu (ne tabulky stránek ani paměť), takže jde rezervovat
i desítky GiB (Wine). Přístup do rezervace je chyba procesu. Mapovat do
ní jde jen `VM_MAP` s `MAP_FIXED | MAP_IN_RESERVATION`. Zruší se
`VM_UNMAP`. Vyžaduje `WRITE` na procesu. Chyby: `INVALID_ARGS`,
`BAD_STATE` (`MAP_FIXED` přes existující oblast; proces skončil),
`NO_RESOURCES`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `VM_PROTECT`

`(process, addr, len, prot) -> 0`. Změní práva mapování v rozsahu (i části;
oblast se rozdělí). `prot` je podmnožina `MAP_READ | MAP_WRITE |
MAP_EXECUTE`, 0 = žádný přístup. Nikdy zápis a spuštění zároveň (W^X)
a nikdy nad práva při `VM_MAP` (ta ověřila práva handlu objektu).
Namapované stránky se odmapují a výpadek je doplní s novými právy.
Vyžaduje `WRITE` na procesu. Chyby: `INVALID_ARGS` (neznámý bit, W+X,
zarovnání), `ACCESS_DENIED` (nad původní práva), `NOT_FOUND` (díra),
`BAD_STATE` (rezervace v rozsahu), `NO_RESOURCES`, `BAD_HANDLE`.

### `VM_QUERY`

`(process, addr, out_ptr) -> 0`. Zapíše `abi::vm::RegionInfo` (40 B) oblasti
na adrese `addr`: začátek, délka, druh (`region::FREE` / `RESERVED` /
`MAPPED`), aktuální a nejvyšší práva, atributy (`ATTR_GUARD`,
`ATTR_IN_RESERVATION`) a offset v objektu. U volného místa začátek a délka
mezery mezi sousedními oblastmi. Pro `VirtualQuery`, výpis paměti procesu
a ladění. Vyžaduje `READ` na procesu. Chyby: `INVALID_ARGS` (`addr` mimo
`USER_MIN..USER_MAX`), `BAD_ADDRESS`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `VM_PAGE_SIZE`

`(process) -> bajty`. Velikost stránky adresního prostoru procesu: mocnina
dvou od `MIN_PAGE_SIZE` (4 KiB) do `MAX_PAGE_SIZE` (64 KiB) z `abi::vm`.
Na tuto hodnotu musí být zarovnané adresy, délky a offsety pro `VM_MAP`
a `VM_UNMAP`. Není to konstanta ABI: dnes je všude 4 KiB, ale na aarch64
může jádro dát nativním procesům 16 KiB a procesům Windows vrstvy 4 KiB
(každý adresní prostor má vlastní granuli). Rodič se ptá na potomka při
nahrávání programu (`libos::vm::page_size_of`), program na sebe
(`libos::vm::page_size`, zjistí jednou a uloží). Stačí libovolný handle
procesu (žádná práva). Chyby: `BAD_HANDLE`, `WRONG_TYPE`.

## Procesy a vlákna

Spuštění programu (dělá ho libos `spawn`): `PROCESS_CREATE` ->
`VMO_CREATE`/`VMO_WRITE`/`VM_MAP` segmentů a zásobníku -> `CHANNEL_CREATE`
a zápis startovní zprávy -> `THREAD_CREATE` -> `PROCESS_START`.

### `PROCESS_CREATE`

`(job, name_ptr, name_len) -> handle`. Prázdný proces bez paměti a vláken
v jobu `job` (právo `MANAGE`); vše, co proces stojí, se strhává z rozpočtu
jobu. Jméno: UTF-8, max. 64 B (pro logy). Práva: `DUPLICATE`, `TRANSFER`,
`WAIT`, `MANAGE`, `WRITE`, `READ`. Chyby: `INVALID_ARGS`, `NO_MEMORY`,
`NO_RESOURCES` (rozpočet jobu), `BAD_ADDRESS`, `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`.

### `PROCESS_START`

`(process, thread, entry, stack, bootstrap_handle) -> 0`. Spustí první
vlákno procesu na adrese `entry` se zásobníkem `stack`. Handle
`bootstrap_handle` (typicky konec kanálu se startovní zprávou) se přesune do
tabulky nového procesu a vlákno dostane jeho hodnotu v `rdi`. Vlákno musí
patřit procesu a proces nesmí být spuštěný. Vyžaduje `MANAGE` na procesu
i vláknu, `TRANSFER` na bootstrap handlu. `entry` musí ležet v paměti
procesu (`USER_MIN` až `USER_MAX`), `stack` nejvýš na `USER_MAX`, jinak
`INVALID_ARGS`. Chyby: `BAD_STATE`, `INVALID_ARGS`, `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`.

### `PROCESS_EXIT`

`(code)` - nevrací se. Ukončí aktuální proces s kódem `code`: všechna jeho
vlákna skončí (čekající s `CANCELED`), adresní prostor se zruší a handly se
zavřou (paměť a rozpočty jobu zpět hned, ne až se zavřením handlu procesu),
potom proces dostane signál `TERMINATED`. Handle skončeného procesu drží jen
náhrobek (kód, čas CPU, jméno) - viz [jadro.md](jadro.md#procesy-a-vlákna).

### `PROCESS_KILL`

`(process) -> 0`. Ukončí jiný proces s kódem -1 000 000. Vlákna, která
právě běží na jiném CPU, jádro přeruší. Paměť a handly procesu se uvolní
jako u `PROCESS_EXIT`, jakmile poslední vlákno skončí. Vyžaduje `MANAGE`.

### `PROCESS_EXIT_CODE`

`(process, out_code_ptr) -> 0`. Zapíše návratový kód (i64) skončeného
procesu. Vyžaduje `READ`. Chyby: `BAD_STATE` (proces ještě běží - nejdřív
počkat na `TERMINATED`).

### `THREAD_CREATE`

`(process, name_ptr, name_len) -> handle`. Nové (nespuštěné) vlákno procesu.
Vyžaduje `MANAGE` na procesu. Chyby: `BAD_STATE` (proces končí),
`NO_MEMORY`, `INVALID_ARGS`.

### `THREAD_START`

`(thread, entry, stack, arg0, arg1) -> 0`. Spustí další vlákno již běžícího
procesu; `arg0`, `arg1` dostane v `rdi`, `rsi`. Zásobník musí být zarovnaný
jako po instrukci `call` (adresa = 8 mod 16), pokud vstupní funkce dodržuje
System V ABI. Pro `entry` a `stack` platí totéž co u `PROCESS_START`.
Vyžaduje `MANAGE`. Chyby: `BAD_STATE`, `INVALID_ARGS`.

### `THREAD_EXIT`

`()` - nevrací se. Ukončí aktuální vlákno. Konec posledního vlákna ukončí
proces (kód -1 000 002, pokud předtím nevolal `PROCESS_EXIT`).

### `THREAD_SUSPEND`

`(thread) -> token`. Pozastaví vlákno procesu. Vlákno se zastaví na hranici
návratu do userspace a dostane signál `SUSPENDED`; vlákno, které právě spí
v syscallu, se počítá jako pozastavené hned (jeho uživatelský kontext je
uložený) a po probuzení se zastaví před návratem. Běžící vlákno jádro
přeruší (IPI). Zavření tokenu vlákno obnoví; víc tokenů = vlákno stojí,
dokud žije aspoň jeden (když debugger zemře, vlákno se rozběhne samo).
Kill a `JOB_KILL` vždy vyhrají: pozastavené vlákno skončí. Vyžaduje
`MANAGE` na vlákně. Chyby: `WRONG_TYPE` (vlákno jádra), `BAD_STATE`
(vlákno skončilo), `NO_RESOURCES`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `THREAD_STATE`

`(thread, op, kind, ptr, len) -> 0`. Čtení (`thread::op::READ`, pravo
`READ`) nebo zápis (`WRITE`, pravo `WRITE`) kontextu pozastaveného vlákna.
`kind`: `thread::kind::GENERAL` - obecné registry, PC, SP, příznaky a báze
TLS (`abi::thread::X64General` 160 B, `A64General` 280 B); `FP` - FPU/SIMD
(x86_64 obraz FXSAVE 512 B, aarch64 V0-V31 + FPCR + FPSR 528 B); `len`
přesně podle architektury. Čtení počká (nejvýš desetinu sekundy), než pozastavené
vlákno opravdu stojí v jádře. Zápis jádro ověří a vlákno ho použije
těsně před návratem do userspace (čtení ho už vidí); u vlákna spícího
v syscallu přepíše i návratovou hodnotu (rax / x0). Ověření zápisu: PC, SP
a báze TLS v paměti procesu (na x86_64 kanonické, pod `USER_MAX`),
z příznaků jen uživatelské bity (x86_64 RFLAGS bez IOPL, IF vždy 1;
aarch64 jen NZCV), u FPU nulové rezervované bity (MXCSR, FPCR, FPSR).
Chyby: `INVALID_ARGS` (neznámý `op`/`kind`, špatná délka, neplatný
kontext), `BAD_STATE` (vlákno není pozastavené), `WRONG_TYPE`,
`BAD_ADDRESS`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `EXCEPTION_CHANNEL`

`(task, flags) -> channel`. Kanál výjimek procesu nebo vlákna (`task`,
právo `MANAGE`). `flags`: 0, `exception::channel::DEBUGGER` (jen proces:
kanál debuggeru, první a druhá šance), nebo `HANDLER_THREAD` (jen vlákno:
označí ho jako vlákno obsluhy - jeho výjimky přeskočí kanály vlákna
a procesu, kanál nevznikne, vrací 0). Na každé místo nejvýš jeden živý
kanál (zavřený se nahradí). Výjimka CPU v userspace, kterou jádro
nevyřeší samo (stránky na požádání), jde obsluhám v pořadí: debugger
(první šance), vlákno, proces, debugger (druhá šance); nikdo = konec
procesu jako dřív. Zpráva: `exception::Report` (56 B: druh, stupeň,
přístup, proces, vlákno, PC, adresa, kód architektury) a handly vlákna
(`READ`, `WRITE`, `MANAGE`) a tokenu výjimky. Vlákno mezitím stojí a jeho
kontext jde číst a zapsat (`THREAD_STATE`). Chyby: `INVALID_ARGS`
(příznak k jinému typu), `ALREADY_EXISTS`, `WRONG_TYPE`, `BAD_HANDLE`,
`ACCESS_DENIED`.

### `EXCEPTION_RESOLVE`

`(token, action) -> 0`. Rozhodnutí obsluhy (`exception::action`): `RESUME`
= vlákno pokračuje (s kontextem změněným přes `THREAD_STATE`), `NEXT` =
další obsluha v pořadí, `KILL` = konec procesu jako u neošetřené výjimky.
Zavření tokenu bez rozhodnutí = `NEXT` (i když obsluha zemře). Breakpoint:
na x86_64 je PC už za `int3`, na aarch64 na `brk` (obsluha ho posune o 4).
Chyby: `INVALID_ARGS` (neznámá akce), `BAD_STATE` (už rozhodnuto nebo
vlákno skončilo - kill vždy vyhraje), `WRONG_TYPE`, `BAD_HANDLE`.

### `EXCEPTION_HANDLER`

`(op, a0, a1) -> ...`. Doručení výjimky přímo ve vlákně (rychlá cesta pro
SEH ve Windows vrstvě a signály v `cstd`). Operace `exception::handler`:

- `SET_ENTRY(process, entry)`: vstup obsluhy procesu (0 = zrušit; právo
  `WRITE` na procesu). `entry` musí ležet ve spustitelném mapování - tedy
  zapečetěném nebo s oprávněním JIT (W^X), obsluha pravidla spustitelné
  paměti neobejde. Jinak `INVALID_ARGS`.
- `SET_STACK(base, size)`: alternativní zásobník aktuálního vlákna (jako
  `sigaltstack`, 0 = žádný; aspoň rámec + 256 B), nutný pro obsluhu
  přetečení zásobníku.
- `RETURN(frame, disposition)`: návrat z obsluhy. `RESUME` obnoví kontext
  z rámce (stejná kontrola jako `THREAD_STATE`: PC, SP v userspace,
  uživatelské příznaky, FPU bez rezervovaných bitů; báze TLS se
  neobnovuje) a vlákno pokračuje; `NEXT` obnoví kontext a výjimku nabídne
  dalším obsluhám (kanál vlákna, procesu, debugger, konec procesu) - druh
  a adresu výjimky bere jádro ze své kopie, ne z rámce. Úspěšný návrat se
  nevrací volajícímu. Chyby: `INVALID_ARGS` (neplatný rámec, neznámá
  akce), `BAD_STATE` (žádná obsluha neběží), `BAD_ADDRESS`.

Pořadí obsluh: debugger (první šance), obsluha ve vlákně, kanál vlákna,
kanál procesu, debugger (druhá šance). Obsluha ve vlákně dostane rámec
`exception::frame` (zarovnaný na 64 B: `Report`, obecné registry
a FPU/SIMD) na alternativním zásobníku (je-li nastavený a vlákno na něm
ještě není), jinak pod SP (x86_64 pod červenou zónou 128 B); x86_64:
`rdi` = rámec, `rsp % 16 == 8`, návratová adresa 0; aarch64: `x0` =
rámec, `x30` = 0. Když se rámec nevejde (ochranná oblast při přetečení),
obsluha se přeskočí a výjimka jde dál - nikdy rekurze. Vnoření (výjimka
v obsluze) nejvýš `handler::MAX_DEPTH` (8); další jde rovnou dál.

### `PROCESS_THREADS`

`(process, out_ptr, max) -> počet`. Zapíše na `out_ptr` až `max` (do 4096)
handlů (`u32`) živých vláken procesu (pro debugger: pozastavit všechna).
Práva handlů podle handlu procesu: `READ` = jen číst kontext, s `WRITE`
i zapsat a pozastavit (`MANAGE`). Vrací celkový počet vláken (i nad
`max`). Vyžaduje `READ`. Chyby: `INVALID_ARGS` (`max` nad 4096),
`BAD_ADDRESS`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

Kdo má handle procesu s `WRITE` (nebo vlákna s `WRITE`), má nad procesem
**plnou kontrolu**: stejně jako `VM_MAP` do něj může přepsat kód nebo
registry. Rodič je tak přirozeně debuggerem potomka; handle procesu
s `WRITE` se nikomu jinému nedává.

### `THREAD_YIELD`

`() -> 0`. Přenechá CPU dalšímu připravenému vláknu stejné nebo vyšší třídy.

### `THREAD_SET_POINTER`

`(value) -> 0`, číslo 56. Nastaví registr ukazatele vlákna aktuálního vlákna pro
TLS (x86_64 `FS_BASE`, aarch64 `TPIDR_EL0`). Jádro hodnotu ukládá ve
vlákně a nahraje ji při každém přepnutí; nové vlákno začíná s 0, takže
hodnota nikdy nepřeteče do jiného vlákna ani procesu (na aarch64 si
program `TPIDR_EL0` může zapsat i sám, jádro ho proto při přepnutí
ukládá). Chyby: `INVALID_ARGS` (`value` mimo userspace). V libos
`sys::thread_set_pointer`; používá ho runtime `std` ([Rust std](rust-std.md)).

## Futex

Čekání na změnu `u32` v paměti procesu, základ zámků a podmínkových
proměnných v userspace (`std::sync`, `libos::sync::Mutex`). Klíč je
virtuální adresa ve vlastním procesu; sdílení mezi procesy zatím není.
Kontrola hodnoty a zařazení do fronty proběhnou pod zámkem fronty
procesu, který bere i `WAKE`, takže se probuzení mezi kontrolou
a usnutím neztratí.

### `FUTEX`

`(op, addr, value, deadline) -> výsledek`. Číslo 55.

| `op` | `value` | význam |
|---|---|---|
| 0 `WAIT` | očekávaná hodnota | když `u32` na `addr` (zarovnané na 4 B) je rovné `value`, vlákno usne, dokud ho nevzbudí `WAKE` na stejné adrese nebo nevyprší `deadline` (ns od startu, `DEADLINE_INFINITE` = bez limitu); jiná hodnota = hned 0; plané probuzení je možné, volající podmínku ověří znovu; vrací 0 |
| 1 `WAKE` | nejvýš probuzených (`u32::MAX` = všechna) | probudí čekající vlákna na `addr` v pořadí příchodu, vrací počet probuzených; `deadline` se nepoužívá |

Chyby: `TIMED_OUT` (`WAIT`), `INVALID_ARGS` (neznámá operace,
nezarovnaná adresa, adresa mimo userspace), `INVALID_ADDRESS`
(nenamapovaná), `CANCELED` (ukončení procesu). V libos
`sys::futex_wait`, `sys::futex_wake`.

## Joby (rozpočty prostředků)

Každý proces patří do jednoho **jobu**. Joby tvoří strom: kořen dostane
init od jádra (bez limitu, se všemi právy) a rozděluje ho mezi služby.
Vše, co proces stojí, se strhává z rozpočtu jeho jobu **a zároveň ze
všech předků**, takže limit jobu platí pro celý jeho podstrom.

Dva rozpočty v bajtech (`abi::job::pool`):

| Rozpočet | Co se strhává |
|---|---|
| `KERNEL` (0) – paměť jádra | proces (struktura + kořenová tabulka stránek), objekty (event, kanál, port, objekt paměti, přerušení, prostředek, job), sloty handlů, registrace portů (64 B), zprávy ve frontách kanálů (strhne se odesílateli), mapování (záznam + horní odhad tabulek stránek), vlákna (struktura + 64 KiB zásobníku jádra; zásobník se vrátí hned po konci vlákna), seznamy stránek objektů paměti |
| `USER` (1) – uživatelská paměť | stránky objektů paměti (`VMO_CREATE`, `VMO_CREATE_CONTIGUOUS`, i paměť pro DMA) |

- Strženo se vrací automaticky se zánikem věci, která ho drží.
- Překročení rozpočtu = `NO_RESOURCES`. Nedostatek fyzické paměti
  systému je dál `NO_MEMORY` (rezerva jádra viz níže).
- Potomek dostane svůj job **bez práva `WRITE`**: smí v něm spouštět
  procesy a zakládat podřízené joby (`MANAGE`), ale limit si nezmění.
  I kdyby si v podřízeném jobu limit zvedl, platí limit předka.
- Limit pod aktuální spotřebou je povolený: nic se neodebírá, jen další
  stržení selžou.
- Souběh: dvě současná stržení těsně pod limitem mohou obě selhat; limit
  se ale nepřekročí nikdy.
- Obaly: `libos::job` (`Job::create`, `set_limit`, `info`, `current`),
  `libos::spawn_in` (spuštění programu v daném jobu; `spawn` = stejný job
  jako rodič). Startovní zpráva nese job jako `bootstrap::kind::JOB`.

```rust
use libos::job::{Job, pool};
let job = Job::create(&libos::job::current()?)?;
job.set_limit(pool::USER, 64 << 20)?;   // 64 MiB uživatelské paměti
job.set_limit(pool::KERNEL, 4 << 20)?;  // 4 MiB paměti jádra
let (proc, ch) = libos::spawn_in(&job, "sluzba", image, handles)?;
let info = job.info()?;                 // spotřeba, limity, zbývá
```

> **Změna proti předchozí verzi:** limity byly pevné počty na proces
> (65 536 handlů, 1024 vláken, 65 536 mapování, 65 536 registrací portů
> na port i na proces). Velký server (stovky tisíc spojení) by na ně
> narazil, a přitom nechránily to podstatné – paměť: 65 536 objektů
> paměti po 1 GiB by limit nepřekročilo. Rozpočty v bajtech chrání paměť
> přímo, nastavuje je rodič a dají se odstupňovat po stromu.

### `JOB_CREATE`

`(parent) -> handle`. Podřízený job bez vlastního limitu (platí limity
předků). `parent` potřebuje `MANAGE`. Objekt se strhne z rozpočtu jádra
`parent`. Práva: `DUPLICATE`, `TRANSFER`, `WAIT`, `READ`, `WRITE`,
`MANAGE`. Chyby: `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`,
`NO_RESOURCES`.

### `JOB_SET_LIMIT`

`(job, pool, bytes) -> 0`. Nastaví vlastní limit rozpočtu `pool`
(`KERNEL` = 0, `USER` = 1) v bajtech; `u64::MAX` (`job::UNLIMITED`) =
bez vlastního limitu. S `pool` = `policy::JIT` (2) nastaví politiku JIT:
hodnota = bity `jit::RUN` (procesy jobu smí spouštět paměť bez pečeti)
a `jit::GRANT` (job smí JIT udělit podřízeným); zapnout smí jen ten, jehož
rodičovský job udělovat smí, vypnout vždy (platí hned i pro potomky).
Nový job nemá nic, kořenový jen `GRANT`. Vyžaduje `WRITE`. Chyby: `INVALID_ARGS`
(neznámý `pool`, neznámý bit), `ACCESS_DENIED` (rodič JIT neudělí),
`BAD_HANDLE`, `WRONG_TYPE`.

### `JOB_INFO`

`(job, out_ptr) -> 0`. Zapíše `abi::job::JobInfo` (56 B, little endian):
pro paměť jádra a pak uživatelskou paměť vždy `used` (strženo z jobu
včetně potomků), `limit` (vlastní limit) a `available` (kolik ještě jde
strhnout – nejmenší volné místo přes job a všechny předky; `u64::MAX` =
nikde limit), nakonec `id` (číslo jobu, jedinečné do restartu).
Vyžaduje `READ`. Chyby: `BAD_HANDLE`, `ACCESS_DENIED`,
`WRONG_TYPE`, `BAD_ADDRESS`.

### `JOB_PROCESSES`

`(job, out_ptr, max) -> počet`. Procesy jobu a všech jeho potomků
(přehled úloh: i proces, který se sám neregistroval). Na `out_ptr` zapíše
nejvýš `max` (do 4096) záznamů `abi::job::ProcEntry` (64 B, little
endian): `pid`, `job` (číslo jobu procesu), `cpu_ns` (čas na CPU všech
vláken, přičítá scheduler při přepnutí), `threads` (živá vlákna), `flags`
(`proc_flags::FINISHED` = skončil; zbývá jen náhrobek bez paměti, dokud
někdo drží handle; `proc_flags::JIT_USED` = proces dostal spustitelnou
paměť bez pečeti - audit JIT), `name` (32 B UTF-8,
doplněno nulami). Vrátí celkový počet (i nad `max`). Vyžaduje `READ`.
Chyby: `INVALID_ARGS` (`max` > 4096), `BAD_HANDLE`, `ACCESS_DENIED`,
`WRONG_TYPE`, `BAD_ADDRESS`.

### `JOB_KILL`

`(job, code) -> 0`. **[implementováno]** Ukončí všechny procesy jobu
a všech jeho potomků s návratovým kódem `code` a job **uzavře**: nový
proces (`PROCESS_CREATE`) ani podjob (`JOB_CREATE`) v něm ani v jeho
potomcích už nevznikne (`BAD_STATE`). Vyžaduje `MANAGE`. Používá ho init
při odhlášení (job relace, viz [Uživatelské účty](ucty.md#relace)).

- **Volající uvnitř zabíjeného jobu** skončí jako ostatní: syscall se mu
  vrátí 0, jeho vlákna dostanou požadavek na ukončení a skončí při
  návratu do userspace (kód `code`).
- **Souběh se vznikem procesu nebo podjobu:** jádro nejdřív nastaví
  příznak jobu, pak projde registr procesů. Nový proces se nejdřív
  zaregistruje a pak zkontroluje příznak svého jobu a předků (obojí
  `SeqCst`): buď ho `JOB_KILL` v registru uvidí a ukončí, nebo ho jeho
  vlastní kontrola odmítne. Podjob se po vytvoření kontroluje stejně.
  Žádný proces tak neproklouzne; nespuštěný proces ukončený během
  zavádění neprojde `PROCESS_START` (`BAD_STATE`).
- Procesy skončí asynchronně (jako po `PROCESS_KILL`): na konec čekejte
  přes `TERMINATED` handlu procesu nebo `JOB_PROCESSES` (`FINISHED`).

Chyby: `BAD_HANDLE`, `ACCESS_DENIED` (bez `MANAGE`), `WRONG_TYPE`. V libos
`Job::kill`. Testy v sadě `joby`: `job: JOB_KILL ukonci podstrom a job
uzavre`, `job: JOB_KILL vlastniho jobu ukonci i volajiciho`, `job:
JOB_KILL behem spousteni nic neproklouzne`.

### `FB_CONTROL`

`(vmo, op, out_ptr) -> 0`. Framebuffer z firmware: `vmo` musí být právě
objekt z bootstrapu `kind::FRAMEBUFFER` (jiný objekt paměti =
`WRONG_TYPE`). Operace (`abi::syscall::fb_op`):

- `INFO` (0): zapíše `abi::syscall::FbInfo` (16 B: šířka, výška, pixelů
  na řádek, formát 1 = RGB, 2 = BGR; vše u32). Vyžaduje `READ`.
- `CLAIM` (1): **výslovné převzetí** - jádro přestane kreslit log do
  framebufferu, log dál jde na sériovou linku. Vyžaduje `WRITE`.
- Předání: když obrazovku drží jiný živý proces, `CLAIM` vrátí
  `BAD_STATE`. Nový vlastník ji dostane, až ji starý vrátí (`RELEASE`)
  nebo skončí (jádro ji pak vrátí samo). Předání řídí vlastník displeje
  (supervisor initu: záložní konzole po pádu GUI).
- `RELEASE` (2): vrácení - jádro obrazovku smaže a kreslí log dál (se
  startovní obrazovkou, `bootstrap::flags::SPLASH`, obraz nechá beze změny
  pro dalšího vlastníka, viz [Startovní obrazovka](start.md)). Smí jen vlastník (proces, který naposledy zavolal `CLAIM`), jinak `BAD_STATE`: pozdní vrácení předchozího vlastníka tak nesmaže obraz nového (konzole po pádu GUI).
  Vyžaduje `WRITE`. Konec procesu, který framebuffer převzal, ho vrátí
  sám.

Panika jádra kreslí do framebufferu vždy. Chyby: `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`, `BAD_ADDRESS`, `NOT_FOUND`
(bez framebufferu).

## Strukturální meze

Meze dané formátem dat nebo voláním, ne politikou:

| Mez | Hodnota | Po překročení | Proč |
|---|---|---|---|
| rezerva jádra: paměť pro programy (objekty paměti, zásobníky vláken, data zpráv, tabulky stránek, objekty a handly) se nealokuje, kdyby volné paměti zbylo méně | 1/32 RAM, nejméně 16 MiB | `NO_MEMORY` | jádro musí přežít vyčerpání paměti programem |
| handly v tabulce procesu | 1 048 575 | `NO_RESOURCES` | handle má 20 bitů indexu (zbytek je generace slotu) |
| velikost objektu paměti | 1 TiB | `INVALID_ARGS` | seznam stránek je dvouúrovňové pole |
| zprávy ve frontě jednoho konce kanálu | 1024 | `SHOULD_WAIT` | řízení toku (paměť hlídá rozpočet) |
| bajty dat ve frontě jednoho konce kanálu | 4 MiB | `SHOULD_WAIT` | řízení toku |
| jedna zpráva kanálu | 64 KiB, 64 handlů | `INVALID_ARGS` | kopie jedním voláním; větší data v objektu paměti |
| objekty v `OBJECT_WAIT_MANY` | 64 | `INVALID_ARGS` | pole na zásobníku jádra; na víc je port |
| registrace portů u jednoho objektu | ~100 000 | `NO_MEMORY` | seznam čekajících objektu je souvislé pole (na víc objektů žádná mez) |
| `VMO_READ` / `VMO_WRITE`, `RANDOM_GET` | 1 MiB, 4 KiB na volání | `INVALID_ARGS` | kopie přes buffer jádra; libos dělí na části |

Tabulky stránek, heap a zásobníky jádra smí čerpat i z rezervy, takže
vyčerpání paměti programem jádro neshodí.

## Hardware (ovladače)

Vyžadují handle na **Resource** s právem `MANAGE` (oprávnění k hardwaru).
Jádro vytvoří jediný kořenový Resource bez omezení a dá ho procesu `init`
(startovní zpráva, druh `ROOT_RESOURCE`); init ho předá správci zařízení.
Ten každému ovladači vytvoří **omezený** Resource (`RESOURCE_CREATE`): jen
pamětové BARy jeho zařízení a stránku jeho konfiguračního prostoru PCI,
nejvýš 8 přerušení a 4 MiB paměti pro DMA. Chyba v ovladači tak nemůže
přepsat registry jiného zařízení. S IOMMU navíc zařízení smí přes DMA jen
do paměti vytvořené přes svůj prostředek (viz [jadro.md](jadro.md#iommu-ochrana-dma));
bez IOMMU může zapsat kamkoli.

Kořenový Resource má navíc právo `SET_CLOCK` (nastavení hodin,
[`CLOCK_SET`](#clock_set)). Správce zařízení dostane kořen bez něj
a omezené prostředky ho nemají nikdy.

### Framebuffer z firmwaru

Není to syscall, ale druh handlu ve startovní zprávě initu
(`abi::bootstrap::kind::FRAMEBUFFER` = 7): fyzický objekt paměti
framebufferu GOP, práva `READ | WRITE | MAP`. Číst a psát jde jen přes
mapování (`VMO_READ`/`VMO_WRITE` = `NOT_SUPPORTED`, jako buffer syscallu
`BAD_ADDRESS`). Geometrii a převzetí obrazovky řídí [`FB_CONTROL`](#fb_control).

Objekt se mapuje **write-combining** (fyzický objekt s typem cache
`WRITE_COMBINING`, stejně jako u [`VMO_CREATE_PHYSICAL`](#vmo_create_physical);
x86_64: položka PAT 1 = WC, jádro ji nastaví na každém CPU; aarch64: Normal
Non-cacheable): zápisy se slučují,
čtení je pomalé. Jádro handle nepošle, když framebuffer překrývá RAM pro
alokátor, nezačíná na hranici stránky nebo je menší než jeho geometrie
(varování v logu, konzole jádra ho dál používá). Init ho předá vlastníkovi
obrazovky: bez virtio-gpu ovladači `fbdisplay` (viz
[Grafika](gui.md#displeje-virtio-gpu-a-fbdisplay)), jinak záchranné
konzoli `user/console`.

### `VMO_CREATE_CONTIGUOUS`

`(resource, size) -> handle`. Fyzicky souvislý objekt paměti pro DMA
(max. 4 MiB), vyplněný nulami. Do procesu se mapuje jako paměť pro DMA:
na koherentní platformě (x86_64, QEMU virt) normálně s cache, na
nekoherentní (např. Raspberry Pi 4) bez cache, aby zařízení i ovladač
viděli stejná data bez ruční údržby cache. Pořadí zápisů vůči zařízení
musí ovladač zajistit bariérami (`libos::barrier`). Jen z prostředku
s identitou zařízení (`RESOURCE_CREATE` s `device`): při konci ovladače
jádro zařízení zastaví a paměť pustí až potom (karanténa DMA, viz
[jadro.md](jadro.md#iommu-ochrana-dma)). Chyby: `INVALID_ARGS`
(0 nebo nad 4 MiB), `NO_MEMORY` (není souvislý blok), `ACCESS_DENIED`
(i prostředek bez identity zařízení), `WRONG_TYPE`.

### `VMO_CREATE_PHYSICAL`

`(resource, phys, size, cache) -> handle`. Okno na fyzický rozsah mimo RAM
(registry zařízení, BAR). Rozsah nesmí ležet v RAM ani paměti firmwaru
(jinak `ACCESS_DENIED`). `cache` (`abi::vm::cache`) určí typ cache všech
mapování objektu: `UNCACHED` (0, registry), `WRITE_COMBINING` (1, x86_64
PAT WC, aarch64 Normal Non-cacheable) nebo `CACHED` (2, paměť hostitele
namapovaná zařízením, např. sdílená paměť virtio-gpu). Přes
`VMO_READ`/`VMO_WRITE` ho číst nejde (`NOT_SUPPORTED`, čtení registrů má
vedlejší efekty) a jako buffer syscallu ho použít nejde (`BAD_ADDRESS`).
Omezený prostředek (`RESOURCE_CREATE`) smí jen své rozsahy. Držitel
prostředku ho může odvolat (`VMO_REVOKE`). Chyby: `INVALID_ARGS`
(nezarovnaná adresa, neznámý typ cache), `ACCESS_DENIED`.


**Paměť ACPI:** jedinou výjimkou z „mimo RAM“ je paměť firmwaru
s tabulkami ACPI (oblasti `AcpiReclaimable` a `AcpiNvs` z mapy paměti,
jádro je nikdy nedá alokátoru): rozsah, který v ní leží **celý**, dostane
objekt vždy s `CACHED` (stejné atributy jako přímá mapa jádra; `cache` se
ignoruje) - potřebuje ho služba ACPI (`acpid`, viz [ACPI](acpi.md)).
Rozsah, který jen zčásti zasahuje do RAM, je vždy `ACCESS_DENIED`.

### `VMO_REVOKE`

`(resource, vmo)`. Odvolá fyzický objekt paměti: zruší jeho mapování ve
všech procesech (i v těch, kterým byl handle předán) a další `VM_MAP`
skončí `ACCESS_DENIED`. Přístup přes staré mapování = výjimka stránky.
Ovladač tak může klientovi půjčit okno do paměti zařízení a bezpečně ho
vzít zpět (render uzel virtio-gpu po zničení blobu). Prostředek musí
rozsah objektu povolovat. Chyby: `NOT_SUPPORTED` (není fyzický objekt),
`ACCESS_DENIED`.

### `VMO_PHYS_ADDR`

`(resource, vmo, offset) -> phys`. Fyzická adresa bajtu `offset` objektu
paměti (pro programování DMA). U souvislého objektu platí pro celý rozsah,
u běžného jen v rámci jedné stránky. Běžnému objektu stránku vytvoří
a připne (adresa zůstane platná, dokud objekt žije). Chyby:
`OUT_OF_RANGE`, `ACCESS_DENIED`, `NO_MEMORY`.

### `INTERRUPT_CREATE`

`(resource, out_msi_ptr) -> handle`. Objekt přerušení s volným vektorem
CPU. Do `out_msi_ptr` zapíše zprávu MSI `{ address: u64, data: u32,
reserved: u32 }`, kterou ovladač zapíše do zařízení (tabulka MSI-X).
Když zařízení přerušení vyvolá, objekt dostane signál `SIGNALED`. Na
aarch64 (GICv3 ITS) musí být prostředek omezený na zařízení
(`RESOURCE_CREATE` s `device`), jinak `INVALID_ARGS`; na x86_64 to není
potřeba. Chyby: `NO_RESOURCES` (došly vektory), `INVALID_ARGS`,
`ACCESS_DENIED`, `BAD_ADDRESS`.

### `RESOURCE_CREATE`

`(resource, ranges_ptr, ranges_count, irq_max, dma_max, device) -> handle`. Užší
prostředek pro ovladač. `ranges_ptr` ukazuje na `ranges_count` (max. 16)
dvojic u64 (fyzický začátek, délka; obojí zarovnané na stránky, jinak
`INVALID_ARGS`). Nový prostředek smí vytvářet okna
(`VMO_CREATE_PHYSICAL`) jen uvnitř těchto rozsahů, nejvýš `irq_max` objektů
přerušení a nejvýš `dma_max` bajtů paměti pro DMA (`VMO_CREATE_CONTIGUOUS`,
počítá se skutečně alokovaný blok 2^n stránek; limity platí po celou dobu
života prostředku, po vyčerpání `NO_RESOURCES`, neúspěšný pokus nic
nespotřebuje). Kvóty se odečítají v celém řetězci rodičů: potomci dohromady
nikdy nepřekročí limit rodiče. Je-li rodič sám omezený, musí rozsahy ležet v
jeho rozsazích a limity nesmí být vyšší (jinak `ACCESS_DENIED`). Vyžaduje
`MANAGE`. Chyby: `INVALID_ARGS` (víc než 16 rozsahů, nulová délka,
přetečení), `ACCESS_DENIED`, `BAD_ADDRESS`, `BAD_HANDLE`, `WRONG_TYPE`.

`device` = zařízení, kterému prostředek patří: 0 = žádné, jinak
`1 << 32 | segment << 16 | sběrnice << 8 | zařízení << 3 | funkce`
(`abi::syscall::pci_device`). S IOMMU dostane zařízení doménu a paměť z
`VMO_CREATE_CONTIGUOUS` přes tento prostředek (a jeho potomky) se do ní
namapuje. Potomek zdědí zařízení rodiče a jiné zvolit nesmí
(`ACCESS_DENIED`). Bez IOMMU se `device` jen zaznamená.

**Porty a linky přerušení.** Horní dva bity začátku rozsahu určují jeho
druh (`abi::syscall::RANGE_KIND_MASK`): 0 = fyzická paměť (výše),
`RANGE_IO_PORTS` (`1 << 62`) = I/O porty od `začátek & 0xFFFF`, délka =
počet portů (konec nejvýš 0x10000), `RANGE_IRQ_LINES` (`2 << 62`) = pevné
linky přerušení od `začátek`, délka = počet (linky 0 až
`abi::syscall::IRQ_LINES_MAX - 1` = 1023: x86_64 ISA IRQ, aarch64 čísla
přerušení GIC SPI 32-1019). Porty
a linky nemusí být zarovnané. Potomek smí jen porty a linky rodiče. Kořenový
prostředek smí všechny. Jiný druh = `INVALID_ARGS`. V libos:
`hw::io_ports(první, počet)` a `hw::irq_lines(první, počet)`. Příklad
(ovladač PS/2):

```rust
let ranges = [io_ports(0x60, 1), io_ports(0x64, 1), irq_lines(1, 1), irq_lines(12, 1)];
let ps2 = root.restrict(&ranges, 2, 0)?;
```

### `IOPORT_READ`

`(resource, port, width) -> hodnota`. Přečte I/O port o šířce `width`
(1, 2 nebo 4 B). Jen na x86_64, jinde `NOT_SUPPORTED`. Prostředek musí
povolovat porty `port..port + width` (`RESOURCE_CREATE` s
`RANGE_IO_PORTS`), jinak `ACCESS_DENIED`. Je to syscall, ne instrukce
`in` v userspace: pomalejší, ale každý přístup projde kontrolou oprávnění
a stačí pro pomalá zařízení (PS/2). Chyby: `INVALID_ARGS` (šířka, port nad
0xFFFF), `ACCESS_DENIED`, `NOT_SUPPORTED`, `BAD_HANDLE`, `WRONG_TYPE`.
V libos `Resource::port_read8`.

### `IOPORT_WRITE`

`(resource, port, width, value) -> 0`. Zapíše I/O port. Podmínky a chyby
jako `IOPORT_READ`; navíc `INVALID_ARGS`, když `value` nevejde do šířky
(např. 0x100 pro 1 B). V libos `Resource::port_write8`.

### `INTERRUPT_CREATE_LINE`

`(resource, line, flags) -> handle`. Objekt přerušení na pevné lince.
x86_64: ISA IRQ 0-15 přes IOAPIC (PS/2 klávesnice 1, myš 12, SCI ACPI
obvykle 9); jádro převede linku na GSI podle přepisů z ACPI MADT
(polarita, spouštění) a nasměruje ji na volný vektor bootovacího CPU,
`flags` ignoruje. aarch64: SPI 32-1019 přes distributor GIC (zařízení na
desce bez MSI, např. Generic Event Device ACPI); spouštění určí `flags`
(`abi::syscall::line_flags::EDGE` = hrana, jinak úroveň; hodnota z `_CRS`
zařízení), přerušení jde na bootovací CPU. Neznámý bit ve `flags` =
`INVALID_ARGS`. Na přerušení objekt dostane `SIGNALED` jako
u `INTERRUPT_CREATE`; potvrzuje se `INTERRUPT_ACK`. Linku spouštěnou
**úrovní** jádro po přerušení zamaskuje a odmaskuje až potvrzením (jinak by
se přerušení opakovalo dokola). Linka spouštěná **hranou** se nemaskuje.
Ovladač má proto po každém přerušení obsloužit vše, co zařízení má, a pro
jistotu se na zařízení občas podívat i bez přerušení. Prostředek musí linku
povolovat (`RANGE_IRQ_LINES`) a počítá se do `irq_max`. Jedna linka může
mít jen jeden objekt (`ALREADY_EXISTS`). Po zavření posledního handlu se
linka zamaskuje a uvolní. Chyby: `INVALID_ARGS` (linka nad 15),
`ACCESS_DENIED`, `ALREADY_EXISTS`, `NO_RESOURCES`, `NOT_SUPPORTED`
(platforma bez IOAPIC), `NOT_FOUND` (linka mimo vstupy IOAPIC); na aarch64
`INVALID_ARGS` i pro číslo mimo SPI, které distributor má. V libos
`Resource::interrupt_line` a `Resource::interrupt_line_with` (s `flags`). Starší volání bez `flags` (libos posílá 0) se chovají stejně jako dřív.

### `INTERRUPT_ACK`

`(interrupt) -> 0`. Potvrdí přerušení (smaže `SIGNALED`). Ovladač má
potvrdit **před** obsluhou zařízení, aby se přerušení, které přijde během
obsluhy, neztratilo. Vyžaduje `WRITE`.

### `SYSTEM_POWER`

`(resource, op) -> nevrací se`. Vypnutí (`abi::syscall::power_op::OFF` = 0)
nebo restart (`RESET` = 1) počítače přes firmware. Vyžaduje Resource
s právem `POWER`: má ho jen kořenový prostředek (omezené prostředky
z `RESOURCE_CREATE` ho nemají nikdy) a init ho předá jen službě ACPI
(`acpid`) jako duplikát `BASIC | POWER` bez `MANAGE`. Jádro zapíše do logu,
který proces o vypnutí požádal.

| Architektura | `OFF` | `RESET` |
|---|---|---|
| aarch64 | PSCI `SYSTEM_OFF` | PSCI `SYSTEM_RESET` |
| x86_64 | `NOT_SUPPORTED` - vypnutí dělá `acpid` zápisem typu spánku S5 do registru PM1 (AML zná jen služba) | registr Reset Control 0xCF9, pak řadič klávesnice (0x64 = 0xFE) |

Normálně se volá až po čistém ukončení služeb (init, viz [ACPI](acpi.md)).
Při úspěchu se volání nevrátí. Chyby: `ACCESS_DENIED` (bez práva `POWER`),
`INVALID_ARGS` (neznámá operace; kontroluje se dřív, než se cokoli
vypne), `NOT_SUPPORTED` (firmware nemá PSCI, nebo x86 vypnutí), `BAD_HANDLE`,
`WRONG_TYPE`. V libos `Resource::system_power`.

### `SCHED_CONTROL`

`(op, ...) -> výsledek`. Plánování: třída QoS vláken, strop jobu,
Realtime rozpočet a údaje o CPU. Operace (`abi::sched::op`):

| Operace | Argumenty | Právo | Výsledek |
|---|---|---|---|
| `THREAD_SET` (0) | `thread, class, budget_us, period_us` | `MANAGE` na vlákně (0 = vlastní vlákno) | 0 |
| `PROCESS_SET` (1) | `process, class, budget_us, period_us` | `MANAGE` na procesu | 0 |
| `JOB_SET_MAX` (2) | `job, class` | `WRITE` na jobu | 0 |
| `THREAD_GET` (3) | `thread, out_ptr` | `READ` (0 = vlastní) | `QosInfo` (16 B) na `out_ptr` |
| `CPU_INFO` (4) | `cpu_index, out_ptr` | - | počet CPU; `CpuInfo` (64 B) na `out_ptr` |
| `CPU_HINT` (5) | `resource, cpu_index, capacity` | Resource s `POWER` | 0 |

Třída (`abi::sched::class`): `REALTIME` 0, `INTERACTIVE` 1, `NORMAL` 2,
`BACKGROUND` 3. Žádost nad strop jobu (včetně předků) = `ACCESS_DENIED`;
snížit třídu jde vždy. Realtime: `budget_us` a `period_us` (0 = výchozí
1 ms / 10 ms), perioda 500 µs až 1 s, rozpočet aspoň 50 µs a nejvýš
polovina periody; všechna Realtime vlákna dohromady nejvýš 50 % kapacity
CPU, jinak `NO_RESOURCES`. Strop jobu nejde nastavit nad strop rodiče
(`ACCESS_DENIED`). `CPU_HINT` s kapacitou 0 vrátí odhad z CPUID/MIDR.
Chyby navíc: `INVALID_ARGS` (neznámá operace, třída, index CPU, parametry
Realtime), `BAD_HANDLE`, `WRONG_TYPE`, `INVALID_ADDRESS`. Popis chování:
[Plánovač](planovac.md). V libos modul `sched`.

## Diagnostika

Syscally níže
vyžadují Resource s právem `DIAG`: má ho jen kořenový prostředek a init ho
předá jen službě logu (`logd`) jako duplikát `BASIC | DIAG` bez `MANAGE`
(prostředkem nejde sahat na hardware). Viz [Živé USB](zive-usb.md#diagnostika).

### `LOG_READ`

`(resource, cursor, buf_ptr, buf_len, out_start_ptr) -> bajty`. Čte log
jádra (v něm jsou i zprávy všech procesů z `DEBUG_LOG`) jako proud bajtů:
`cursor` je pozice v proudu (kolik bajtů kdy jádro zapsalo). Kruhový buffer
má 512 KiB; když starší data už přepsal, čte se od nejstarších dostupných
a skutečný začátek se zapíše na `out_start_ptr` (u64) - čtenář tak pozná
ztrátu; druhé u64 za ním je celkový počet řádků, které jádro přepsalo
dřív, než je čtenář přečetl („ztraceno N řádků“). Nová pozice = začátek
+ vrácená délka, 0 = nic nového. Nejvýš 64 KiB na volání. Chyby: `ACCESS_DENIED` (bez `DIAG`), `INVALID_ARGS`
(`buf_len` nad 64 KiB), `INVALID_ADDRESS`, `BAD_HANDLE`, `WRONG_TYPE`.
V libos `Resource::log_read`.

### `CRASH_RECORD`

`(resource, op, buf_ptr, buf_len) -> bajty`. Záznam pádu jádra z minulého
běhu (pstore, formát `libs/crashrec`). Loader vyhradí 64 KiB RAM na pevné
adrese (druh `Pstore` v mapě paměti); při panice nebo fatální výjimce do ní
jádro zapíše hlavičku a konec logu jádra a teplý restart ji nesmaže.

| `op` | význam |
|---|---|
| 0 `READ` | zkopíruje platný záznam (hlavička + text) do bufferu, vrátí délku; 0 = žádný; malý buffer = `BUFFER_TOO_SMALL` |
| 1 `CLEAR` | smaže záznam (služba logu ho uložila na disk) |
| 2 `TEST_PANIC` | jen testovací jádro (`qemu-test`): vyvolá paniku, jádro zapíše záznam a hned restartuje; jinak `NOT_SUPPORTED` |
| 3 `CAPACITY` | velikost oblasti (0 = loader ji nedostal) |

Chyby: `ACCESS_DENIED` (bez `DIAG`), `INVALID_ARGS` (neznámá operace),
`INVALID_ADDRESS`, `BAD_HANDLE`, `WRONG_TYPE`. V libos
`Resource::crash_record`.

### `LOG_CONTROL`

`(resource, op, buf_ptr, buf_len) -> bajty`. Úrovně logu jádra za běhu
(příkaz shellu `loglevel`, soubor `/system/log/urovne.conf`).

| `op` | buffer | význam |
|---|---|---|
| 0 `SET` | `modul=úroveň` | úroveň pro modul jádra (`sched`, `arch::x86_64::apic`; platí i pro podmoduly) nebo pro proces (jeho `DEBUG_LOG`); úroveň `error`, `warn`, `info`, `debug`, `trace` |
| 1 `CLEAR` | `modul` | zruší přepsání (platí globální úroveň) |
| 2 `LIST` | výstup | `modul=úroveň` po řádcích a `potlaceno=N` (opakované hlášky) |

Výchozí potlačené sekce (jen `info` a výš): `syscall`, `object::process`,
`smp::clock_sync`. Nezávisle na úrovních jádro z jednoho místa (soubor
a řádek, u `DEBUG_LOG` proces a text bez číslic) zapíše nejvýš 10 záznamů
za sekundu; další jen spočítá a s první zprávou v nové sekundě zapíše
„... předchozí hláška potlačena N×“. Nejvýš 32 přepsání, jméno nejvýš 40 B.
Chyby: `ACCESS_DENIED` (bez `DIAG`), `INVALID_ARGS` (neznámá úroveň nebo
operace, dlouhé jméno, plná tabulka), `BUFFER_TOO_SMALL`, `INVALID_ADDRESS`.
V libos `Resource::log_control`.

## Náhodná čísla

Jádro má kryptografický generátor (`kernel/src/random.rs`, crate
`libs/csprng`):

- **Zásobník entropie** míchá vstupy hashem BLAKE2s. Každý vstup má kredit
  = odhad skutečné náhody: RDSEED/RDRAND 32 bitů z 64, data od ovladače
  virtio-rng podle ovladače (4 bity na bajt), jitter čítače, čas firmwaru
  a časování přerušení 0.
- **Generátor je ChaCha20.** Reseed: klíč = BLAKE2s(starý klíč, zásobník),
  nejpozději po 10 s nebo když čeká aspoň 256 bitů kreditu. Po každém
  požadavku se klíč přepíše prvním blokem proudu (fast key erasure), takže
  pozdější únik stavu jádra neprozradí dřívější výstupy.
- **Nasazený** je generátor po reseedu se součtem kreditu aspoň 256 bitů.
  Před nasazením se kredit sčítá přes všechny reseedy (entropie zůstává
  v klíči), takže stačí i zdroj, který dodává po malých dávkách.
  V QEMU stačí RDRAND (`-cpu max`), na stroji bez něj ovladač virtio-rng.

⚠️ Bez hardwarového zdroje a bez virtio-rng generátor nasazený nebude
(výstup jen z času a jitteru) a jádro to zaloguje. Klíče (TLS) proto berte
přes `REQUIRE_SEEDED`.

### `RANDOM_GET`

`(buf_ptr, len, flags) -> 0 | 1`. Naplní buffer (max. `RANDOM_MAX` =
4096 B) náhodnými bajty. Vrací 1, když je generátor nasazený, 0, když
ještě ne (data jsou pak slabá). `flags`: bit 0 `REQUIRE_SEEDED` =
nenasazený generátor vrátí `SHOULD_WAIT` a nic nezapíše. Nepotřebuje
handle. Obaly: `libos::random::fill`, `fill_seeded`, `u64`. Chyby:
`INVALID_ARGS` (délka, neznámý příznak), `SHOULD_WAIT`, `BAD_ADDRESS`.

### `RANDOM_ADD`

`(resource, ptr, len, credit_bits) -> 0`. Přimíchá data (max. 4096 B) do
zásobníku entropie s kreditem `credit_bits` (jádro ho omezí na 8 bitů na
bajt). Vyžaduje Resource s právem `MANAGE` (ovladač zařízení). Přimíchání
nikdy nesníží kvalitu generátoru, ale **kredit věří volajícímu**: vadný
ovladač může generátor prohlásit za nasazený dřív, než skutečně je. Obal:
`libos::random::add`. Používá ho ovladač `virtio-rng`. Chyby: `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`, `BAD_ADDRESS`.
