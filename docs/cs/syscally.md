# Syscally

Rozhraní mezi procesem a jádrem. **Programy mají používat libos**
([libos.md](libos.md)) - čísla a přesné argumenty syscallů nejsou stabilní
rozhraní a mění se spolu s libos. Tento dokument popisuje, co jádro dělá,
aby šlo chování dohledat.

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
(příliš dlouhý text).

### `CLOCK_MONOTONIC`

`() -> ns`. Monotónní čas v nanosekundách od startu jádra (TSC
kalibrovaný proti HPET). Nemůže selhat.

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
opakovat a nesmí jít o zapisovaný konec sám. Předávané handly se z tabulky
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

`(size) -> handle`. Objekt paměti o velikosti `size` (zaokrouhleno na 4 KiB,
max. 4 GiB), vyplněný nulami. Stránky se zatím alokují hned. Práva:
`DUPLICATE`, `TRANSFER`, `WAIT`, `READ`, `WRITE`, `MAP`, `EXECUTE`. Chyby:
`INVALID_ARGS` (0 nebo příliš velký), `NO_MEMORY`, `NO_RESOURCES`.

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
jádro). Adresa, délka a offset musí být zarovnané na 4 KiB, rozsah v
`0x10000..0x8000_0000_0000`. Práva: proces `WRITE`; objekt `MAP` + `READ`,
případně `WRITE`/`EXECUTE` podle příznaků. Mapování drží objekt naživu i po
zavření handlu. Chyby: `INVALID_ARGS`, `OUT_OF_RANGE`, `BAD_STATE`
(`MAP_FIXED` přes existující mapování), `NO_RESOURCES` (není volné místo),
`NO_MEMORY`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `VM_UNMAP`

`(process, addr, len) -> 0`. Zruší mapování. Rozsah musí přesně pokrývat
celá mapování (částečné odmapování zatím není). Po návratu žádné CPU nemá
starý překlad v TLB. Vyžaduje `WRITE` na procesu. Chyby: `NOT_FOUND`,
`INVALID_ARGS`, `BAD_HANDLE`, `ACCESS_DENIED`.

## Procesy a vlákna

Spuštění programu (dělá ho libos `spawn`): `PROCESS_CREATE` ->
`VMO_CREATE`/`VMO_WRITE`/`VM_MAP` segmentů a zásobníku -> `CHANNEL_CREATE`
a zápis startovní zprávy -> `THREAD_CREATE` -> `PROCESS_START`.

### `PROCESS_CREATE`

`(name_ptr, name_len) -> handle`. Prázdný proces bez paměti a vláken. Jméno:
UTF-8, max. 64 B (pro logy). Práva: `DUPLICATE`, `TRANSFER`, `WAIT`,
`MANAGE`, `WRITE`, `READ`. Chyby: `INVALID_ARGS`, `NO_MEMORY`, `BAD_ADDRESS`.

### `PROCESS_START`

`(process, thread, entry, stack, bootstrap_handle) -> 0`. Spustí první
vlákno procesu na adrese `entry` se zásobníkem `stack`. Handle
`bootstrap_handle` (typicky konec kanálu se startovní zprávou) se přesune do
tabulky nového procesu a vlákno dostane jeho hodnotu v `rdi`. Vlákno musí
patřit procesu a proces nesmí být spuštěný. Vyžaduje `MANAGE` na procesu
i vláknu, `TRANSFER` na bootstrap handlu. Chyby: `BAD_STATE`,
`INVALID_ARGS`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `PROCESS_EXIT`

`(code)` - nevrací se. Ukončí aktuální proces s kódem `code`: všechna jeho
vlákna skončí (čekající s `CANCELED`), handly se zavřou, proces dostane
signál `TERMINATED`.

### `PROCESS_KILL`

`(process) -> 0`. Ukončí jiný proces s kódem -1 000 000. Vlákna, která
právě běží na jiném CPU, jádro přeruší. Vyžaduje `MANAGE`.

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
System V ABI. Vyžaduje `MANAGE`. Chyby: `BAD_STATE`.

### `THREAD_EXIT`

`()` - nevrací se. Ukončí aktuální vlákno. Konec posledního vlákna ukončí
proces (kód -1 000 002, pokud předtím nevolal `PROCESS_EXIT`).

### `THREAD_YIELD`

`() -> 0`. Přenechá CPU dalšímu připravenému vláknu stejné nebo vyšší třídy.
