# Windows vrstva: Wine nad naším spodním patrem

Návrh kroku 8 pořadí v [architektura.md](architektura.md) (plán „Horní patro
z Wine, spodní naše“): aplikace a hry pro Windows nad převzatým Wine.
**Stav: [navrženo]**, otázky rozhodnuté 15. 11. 2026 (kapitola
[Rozhodnutí](#rozhodnutí)); začíná etapa W1.

## Stav

| Část | Co | Stav |
|---|---|---|
| W0 | sestavení Wine 11 (DLL PE, nástroje), port unixové části nad `libc.so` | DLL jako PE (602 knihoven) přes `cargo xtask wine` ve WSL **[implementováno]**; unixová část **[navrženo]** (přehled chyb `--keep-going`) |
| W1 | konzolové `.exe` (`cmd`, `hostname`, vlastní test): spodní `ntdll`, TEB přes GS, SEH, paměť, soubory, `wineserver` jako proces | rozpracováno: GS v jádře a `ucontext_t` v `cstd` **[ověřeno v QEMU]**; záplata výjimek `signal_x86_64.c` se přeloží **[implementováno]**; zbytek `ntdll/unix` čeká na části POSIX v `cstd` **[navrženo]** |
| W2 | víc procesů jedné aplikace, registry, pojmenované objekty, `winecfg` v režimu bez oken | **[navrženo]** |
| W3 | okna: ovladač Wine nad kompozitorem (`guiproto`), GDI, `notepad`, `winecfg`, schránka, vstup | **[navrženo]** |
| W4 | zvuk nad `audiod`, jednoduchá hra (GDI: `winemine`, pak hra přes SDL nebo Direct3D 9) | **[navrženo]** |
| W5 | DXVK a VKD3D-Proton nad Vulkanem (Venus v QEMU, ovladače GPU na HW) | **[navrženo]** |
| W6 | 32bitové aplikace (nový WoW64) | **[navrženo]** |
| W7 | rychlá synchronizace (obdoba ntsync), výkon her | **[navrženo]** |

## Výchozí stav

Ověřeno v repozitáři (`main` + větev 7a-cxx):

- **Hotové předpoklady** z pořadí převzetí: Rust std, `cstd` (`libc.a`
  i `libc.so`, signály, `mmap`, vlákna, sockety, locale a široké znaky),
  správa paměti (`VM_RESERVE`, `MAP_FIXED` v rezervaci, `VM_PROTECT`,
  stránky na požádání, CoW), dynamické linkování (`ld.so`, `dlopen`),
  výjimky a kontext vláken (`THREAD_SUSPEND`, `THREAD_STATE`, kanály
  výjimek, doručení ve vlákně), běh C++, Vulkan přes Venus v QEMU.
- **TEB přes GS chybí:** jádro na x86_64 při vstupu přepíná GS na data CPU
  (`swapgs`) a GS programu je vždy 0 (`MSR_KERNEL_GS_BASE` = 0); program
  má jen bázi FS (TLS, `THREAD_SET_POINTER`). Windows na x86_64 čte TEB
  přes `gs:[0x30]` a Wine to vyžaduje.
- **W^X je přísné:** nikdy zápis a spuštění zároveň, ani s oprávněním JIT
  ([správa paměti](sprava-pameti.md)). Aplikace pro Windows běžně žádají
  `PAGE_EXECUTE_READWRITE` (JIT .NET, LuaJIT, ochrany her, starší
  překladače).
- **Segmenty:** GDT má jen 64bitový kód pro userspace (`USER_CODE`);
  32bitový kód (WoW64) potřebuje segment kódu v režimu compat.
- **Nástroje:** porty `configure` (FFmpeg) se staví v MSYS2; repozitář má
  i `tools/wsl` (sestavení Venus).

## Wine dnes: co je co

Wine 11.0 (13. 1. 2026) a jeho rozdělení, ze kterého návrh vychází:

<figure class="diagram">
<svg viewBox="0 0 780 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Části Wine a co z nich převezmeme">
<defs><marker id="wn1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="760" height="40" rx="6"/><text class="t b" x="390" y="35" text-anchor="middle">aplikace .exe a její DLL (PE)</text>
<rect class="ok" x="10" y="70" width="760" height="56" rx="6"/><text class="t b" x="390" y="93" text-anchor="middle">DLL Wine jako PE: kernel32, kernelbase, user32, gdi32, ole32, comctl32, ucrtbase, ...</text><text class="m" x="390" y="112" text-anchor="middle">převezmeme beze změny (LGPL), překládá je clang pro cíl Windows</text>
<rect class="ok" x="10" y="146" width="370" height="64" rx="6"/><text class="t b" x="195" y="170" text-anchor="middle">ntdll.dll (PE)</text><text class="m" x="195" y="189" text-anchor="middle">zavaděč PE (ldr), SEH, heap, Rtl*</text><text class="m" x="195" y="203" text-anchor="middle">převezmeme</text>
<rect class="acc" x="400" y="146" width="370" height="64" rx="6"/><text class="t b" x="585" y="170" text-anchor="middle">unixové knihovny (.so)</text><text class="m" x="585" y="189" text-anchor="middle">ntdll.so, win32u.so, winevulkan.so, ovladače</text><text class="m" x="585" y="203" text-anchor="middle">hranice se systémem: tady je naše práce</text>
<rect class="acc" x="400" y="230" width="370" height="40" rx="6"/><text class="t b" x="585" y="255" text-anchor="middle">wineserver: objekty, okna, fronty, registry</text>
<rect class="box" x="10" y="290" width="760" height="34" rx="6"/><text class="t b" x="390" y="312" text-anchor="middle">libos, libc.so a služby systému (fsd, compd, audiod, netd)</text>
<path class="ln" d="M390 50 L390 66" marker-end="url(#wn1)"/>
<path class="ln" d="M195 126 L195 142" marker-end="url(#wn1)"/>
<path class="ln" d="M380 178 L396 178" marker-end="url(#wn1)"/>
<path class="ln" d="M585 210 L585 226" marker-end="url(#wn1)"/>
<path class="ln" d="M585 270 L585 286" marker-end="url(#wn1)"/>
<path class="ln" d="M500 210 L500 286" marker-end="url(#wn1)"/>
</svg>
<figcaption>Horní část (DLL jako PE) převezmeme beze změny. Systémová volání NT končí v unixových knihovnách a ve wineserveru: to je hranice, kterou napojíme na náš systém.</figcaption>
</figure>

- **DLL jako PE:** od Wine 9 se skoro všechny DLL překládají jako skutečné
  Windows DLL. Volají systém přes `ntdll.dll` a ta přes dispečer volání
  (`__wine_syscall_dispatcher`) přejde do **unixové části**.
- **Unixová část** (`ntdll.so` a „unixlibs“ jako `win32u.so`,
  `winevulkan.so`, `winepulse.so`, `winewayland.so`): C nad POSIX, asi
  40 000 řádků jen `ntdll/unix` (paměť `virtual.c`, signály a výjimky
  `signal_x86_64.c`, vlákna, soubory, spojení se serverem). `win32u.so`
  obsahuje okna, zprávy a GDI (mluví s wineserverem a s ovladačem oken).
- **wineserver:** samostatný proces na prefix (sdílený stav procesů:
  handly, synchronizační objekty, okna a fronty zpráv, háky, registry).
  Protokol přes unixový socket s předáváním fd (`SCM_RIGHTS`), sdílená
  paměť pro fronty.
- **WoW64 (nový, ve Wine 11 plně podporovaný):** 32bitový kód běží
  v 64bitovém procesu; přechod do 64bitového kódu je dlouhý skok na
  64bitový segment kódu (`wow64cpu.dll`). Unixová část je jen 64bitová.
- **Proton 11** (Valve, stabilní od 7. 7. 2026) stojí na Wine 11.0:
  větev Wine se záplatami her, DXVK, VKD3D-Proton, dxvk-nvapi, Wine Mono.

## Otázky

### Q1 - verze Wine

- **A (doporučuji):** upstream **Wine 11.0.x** (stabilní větev, připnutý
  archiv z dl.winehq.org, SHA-256). DXVK a VKD3D-Proton jako samostatné
  porty (stejné verze jako v Protonu 11). Záplaty her z Protonu brát
  jednotlivě, až na ně narazí konkrétní hra.
- **B:** větev Wine z **Protonu 11** jako základ (víc oprav her, ale
  vlastní větev s rebázemi, horší dohledatelnost změn, upstream vývojová
  řada 11.x se mezitím vzdaluje).
- **C:** obojí (aplikace upstream, hry Proton) - dvě sestavení a dvě sady
  záplat našeho spodního patra.

### Q2 - unixová část `ntdll`

- **A (doporučuji):** **převzít `ntdll/unix` z Wine (C) nad `libc.so`**
  a vyměnit jen moduly závislé na OS za vlastní: paměť (`virtual.c` nad
  `VM_RESERVE`/`VM_MAP`/`VM_PROTECT` přes rozšíření libos), výjimky
  (`signal_x86_64.c` nad naším doručením ve vlákně a `Frame`), vlákna
  a TEB (nad osrt a novým GS), spojení se serverem (kanály místo socketu).
  Zbytek (soubory, sync přes server, registry, procesy) zůstane kódem Wine
  nad POSIX z `cstd`. Odhad: dny až týdny na první konzolové `.exe`.
- **B:** vlastní unixová část v Rustu (tabulka asi 500 funkcí `Nt*`) -
  čistší, ale měsíce práce a stálé dohánění Wine.
- **C:** celé unixové části Wine beze změny nad POSIX vrstvou (jako port
  na macOS) - nejméně práce na začátku, ale vyžaduje `SCM_RIGHTS`,
  `ptrace`, signály mezi procesy a pevné adresy, které podle
  architektury nechceme stavět jen kvůli Wine.

### Q3 - wineserver

- **A (doporučuji):** **wineserver z Wine (C) jako samostatný proces na
  prefix** (= na aplikaci), nad `libc.so`; transport protokolu místo
  socketu a `SCM_RIGHTS` přes kanály s předáváním handlů. Okna, fronty
  zpráv, háky a registry zůstanou přesně podle Wine (to je nejtěžší
  a nejdůležitější sémantika). Synchronizaci později přesunout mimo
  server (Q4).
- **B:** vlastní služba v Rustu s protokolem wineserveru - desítky tisíc
  řádků (`window.c`, `queue.c`, `hook.c`, `registry.c`...) znovu.
- **C:** vlastní služba jen pro objekty a synchronizaci (Rust nad
  `wait_many`, `Event`), okna a registry z wineserveru - dva servery
  s rozděleným stavem, složitější než A + Q4.

### Q4 - synchronizace (výkon her, etapa W7)

- **A (doporučuji):** napřed přes wineserver (jako Wine bez ntsync), pak
  **obdoba ntsync ve službě vrstvy**: objekty NT (událost, mutant,
  semafor) v Rustu nad futexem a `wait_many`, čekání bez kola přes server.
- **B:** esync/fsync jako Proton (sdílená paměť + futex v procesech
  aplikace) - rychlé, ale jen v rámci jedné aplikace a se slabší izolací.
- **C:** objekty NT v jádře (jako modul ntsync v Linuxu) - nejrychlejší,
  ale porušuje „čisté jádro“ (jádro by vědělo o sémantice Windows).

### Q5 - TEB přes GS (jádro)

- **A (doporučuji):** **báze GS programu pro každé vlákno v jádře**:
  `THREAD_SET_POINTER` dostane druhý registr (FS = TLS jako dnes, GS =
  „druhý ukazatel vlákna“, obecně pojmenovaný, ne „TEB“), jádro ho při
  přepnutí vlákna zapíše do `MSR_KERNEL_GS_BASE` (vstup do jádra `swapgs`
  zůstává). Kontext (`THREAD_STATE`) bude GS číst i zapisovat. aarch64:
  Windows ARM64 drží TEB v `x18`, to je běžný registr a kontext ho už
  nese; syscall pro GS vrátí na aarch64 `NOT_SUPPORTED` a pravidlo dvou
  architektur výslovně zapíšeme jako omezení.
- **B:** zapnout `CR4.FSGSBASE` (program mění FS/GS sám instrukcí
  `wrgsbase`): rychlejší přepínání, ale vstup do jádra musí řešit GS
  nastavené programem (paranoidní vstup jako Linux) a FS přestane být
  pod kontrolou jádra.
- **C:** TEB jinde než v GS (patchovat Wine a binárky) - nejde, aplikace
  čtou `gs:` přímo.

### Q6 - paměť a `PAGE_EXECUTE_READWRITE`

- **A:** zachovat přísné W^X i pro Windows: `RWX` emulovat přepínáním
  (stránka RX, zápis = výpadek, vrstva ji přepne na RW, spuštění = výpadek
  zpět na RX). Bezpečné, ale JIT .NET a ochrany her budou pomalé
  (výpadek na každé střídání) a kód, který zapisuje do právě běžící
  stránky, nepůjde.
- **B (doporučuji):** **RWX jen pro aplikace s novým oprávněním
  v manifestu** (`wx_compat`, uživatel ho vidí při instalaci jako „smí
  měnit vlastní kód“), ostatní a nativní programy dál přísné W^X; audit
  jako u JIT. Změna invariantu správy paměti, proto výslovně.
- **C:** RWX pro všechny procesy Windows vrstvy (nejjednodušší, nejslabší).

### Q7 - soubory, `C:\` a velikost písmen

- **A (doporučuji):** prefix aplikace je **dataset fsd s vlastností
  „bez rozlišení velikosti písmen“** (casefold při vyhledávání v adresáři,
  jako NTFS nebo ext4 casefold); `C:\` = kořen prefixu, ostatní disky jen
  výslovně povolené složky (Pracovní složky). Nový rys FS, ne jen vrstvy.
- **B:** jako Wine na Linuxu: hledání bez rozlišení velikosti písmen
  v `ntdll` procházením adresáře (pomalé u velkých adresářů, souběh při
  vytváření).
- **C:** A, ale napřed B (rychlý start W1, dataset casefold v W2).

### Q8 - okna a grafika

- **A (doporučuji):** **`win32u.so` z Wine beze změny + vlastní ovladač
  oken** (unixová knihovna jako `winewayland`, v C nad rozhraním C
  kompozitoru podobně jako `libs/mosdl`): každé okno nejvyšší úrovně =
  okno kompozitoru, podřízená okna kreslí `win32u` (GDI softwarově, dibdrv)
  do bufferu okna; vstup, kurzor, schránka a DPI přes `guiproto`.
- **B:** napřed jen „virtuální plocha“ (jedno okno kompozitoru s celou
  plochou Windows uvnitř) - jednodušší start, horší integrace (Alt+Tab,
  okna mimo plochu).
- **C:** A, ale virtuální plocha jako volba pro hry na celou obrazovku.

Vulkan (`winevulkan.so`) poběží nad naším Vulkan loaderem (Venus v QEMU),
OpenGL (`wined3d` přes GL) nebudeme dělat - Direct3D jen přes DXVK
a VKD3D-Proton.

### Q9 - zvuk

- **A (doporučuji):** vlastní ovladač zvuku Wine (unixová knihovna jako
  `winepulse`, rozhraní `mmdevapi`) nad `audioproto`, relace s právy
  aplikace a s povolením nahrávání jako u SDL.
- **B:** zvuk přes port SDL (`winesdl`) - jedna vrstva navíc.

### Q10 - 32bitové aplikace (WoW64)

- **A (doporučuji, etapa W6):** nový WoW64 z Wine 11. Jádro: **32bitový
  segment kódu ring 3** v GDT (compat mode, obecná schopnost x86_64, ne
  „pro Windows“), výjimky a přerušení z režimu compat (návrat `iretq`
  s jiným CS, kontext s CS), `syscall` jen z 64bitového kódu (WoW64 ho
  volá po přepnutí). Paměť: rezervace v dolních 4 GiB.
- **B:** bez 32bitových aplikací (spousta starších her a instalátorů
  nepůjde).
- **C:** 32bitový kód emulovat (box64/FEX) i na x86_64 - zbytečně pomalé.

### Q11 - aarch64

- **A (doporučuji):** Windows vrstva jen na x86_64; na aarch64 výslovně
  **omezení** (pravidlo dvou architektur: nové veřejné API, např. GS, je
  definované i pro aarch64 s `NOT_SUPPORTED`). Aplikace ARM64 a ARM64EC
  s emulací x86 (FEX, jako Proton 11) jako pozdější krok.
- **B:** aarch64 hned s nativními aplikacemi Windows ARM64 (`x18` = TEB)
  a bez emulace x86 - málo aplikací.

### Q12 - sestavení Wine

- **A (doporučuji):** Wine se staví ve **WSL** (Debian): `configure`
  a nástroje Wine (`winegcc`, `widl`, `wrc`, `winebuild`) chtějí Unix
  hostitele; DLL PE překládá clang (`--target=x86_64-windows-gnu`,
  lld), unixovou část clang proti sysrootu C (`clang-dyn.cfg`). xtask
  sestavení spustí přes `wsl` (repozitář už má `tools/wsl`).
- **B:** MSYS2 jako FFmpeg - Wine na MSYS2 jako hostiteli pro cizí Unix
  oficiálně nepodporuje, čekám hodně úprav `configure`.
- **C:** DLL PE ze sestavení WineHQ (archiv s binárkami), sami jen
  unixovou část - rychlé, ale verze PE a unixové části se musí přesně
  shodovat a bez sestavení PE neopravíme chybu v DLL.

## Rozhodnutí

Rozhodnutí uživatele a koordinátora (15. 11. 2026):

| Otázka | Rozhodnutí |
|---|---|
| Q1 | A: upstream Wine 11.0.x, DXVK a VKD3D-Proton jako porty, záplaty z Protonu jednotlivě; případné druhé sestavení z Protonu později |
| Q2 | A: `ntdll/unix` z Wine nad `libc.so`, vlastní moduly paměť, výjimky, vlákna, transport |
| Q3 | A: wineserver z Wine jako proces na prefix, protokol přes kanály |
| Q4 | A: napřed přes wineserver, pak obdoba ntsync ve službě vrstvy |
| Q5 | A: GS jako obecný „druhý ukazatel vlákna“ pro každé vlákno v jádře |
| Q6 | B: RWX jen s oprávněním `wx_compat` v manifestu (viditelné při instalaci, audit); změna invariantu W^X výslovně v [správě paměti](sprava-pameti.md) a `kernel/CLAUDE.md` |
| Q7 | C: napřed hledání bez rozlišení velikosti písmen v `ntdll`, dataset s casefold ve fsd v W2 (obecný rys FS, samostatný návrh) |
| Q8 | A: `win32u.so` z Wine + vlastní ovladač oken nad kompozitorem |
| Q9 | A: vlastní ovladač zvuku nad `audioproto` |
| Q10 | A: WoW64, 32bitový segment kódu v etapě W6 |
| Q11 | A: jen x86_64; aarch64 = zdokumentované omezení, `NOT_SUPPORTED` v API |
| Q12 | A: sestavení ve WSL přes `tools/wsl` |

## W1: spojení se serverem a procesy (otázky T1-T5)

**Stav: [navrženo]**, čeká na rozhodnutí (vlákno Wine, 15. 11. 2026). Rozbor
Wine 11 (`dlls/ntdll/unix/server.c`, `server/request.c`, `server/ptrace.c`):

- **Spojení:** klient se připojí k serveru přes socket `AF_UNIX` v adresáři
  prefixu (nebo zdědí socket v proměnné `WINESERVERSOCKET`, když proces
  spouští jiný proces Wine). Každé vlákno pošle serveru své roury pro
  požadavky a odpovědi; soubory, roury a mapování putují oběma směry jako
  fd přes `SCM_RIGHTS`. Server čeká na všechny fd v jedné smyčce `poll`.
- **Signály vláknům jiného procesu:** server pozastavuje vlákno signálem
  `SIGUSR1` (vlákno v obsluze pošle serveru kontext a čeká), ukončuje
  `SIGQUIT` (`tkill` na pid a tid).
- **Paměť jiného procesu:** `ReadProcessMemory` a `WriteProcessMemory`
  přes `ptrace` a `/proc/<pid>/mem` (ladění, některé dotazy serveru).
- **Start procesu:** zavaděč `wine` a `wine-preloader` rezervují rozsahy
  adres pro Windows (obrazy PE od 0x10000, `KUSER_SHARED_DATA` na
  0x7ffe0000) dřív, než dynamický zavaděč nahraje knihovny.

### T1 - předávání fd mezi procesy

- **A (doporučuji):** obecné **`AF_UNIX` v `cstd`/`osrt`**: `socketpair`,
  `bind`/`connect` na cestu (uzel socketu ve fsd nebo jmenná služba),
  `sendmsg`/`recvmsg` s `SCM_RIGHTS`, kde se fd předá jako duplikát handlu
  v kanálu (soubor = relace fsd, roura = konec kanálu, paměť = VMO). Kód
  Wine zůstane beze změn a stejný mechanismus potřebují další porty
  (X11, D-Bus, Wayland).
- **B:** záplata Wine: spojení a předávání přes vlastní funkce nad kanály
  v `server.c`, `request.c`, `fd.c`. Předávání otevřených souborů mezi
  procesy je ale potřeba i tak - jen ne ve tvaru POSIX.
- **C:** jen `socketpair` a zděděný socket (bez `bind`/`connect` na cestu):
  stačí, pokud každý proces aplikace spouští spouštěč nebo server.

### T2 - signál vláknu jiného procesu

- **A (doporučuji):** obecné **přerušení vlákna v jádře**:
  `THREAD_INTERRUPT(thread, code)` s právem handlu vlákna; cílové vlákno
  dostane při návratu do userspace synchronní doručení na obsluhu ve
  vlákně (druh `INTERRUPT` s kódem), i když je v blokujícím syscallu (ten
  vrátí `INTERRUPTED` jako `EINTR`). `cstd` na tom postaví `kill`,
  `pthread_kill` a `tgkill` mezi procesy (chybí i pro Ctrl+C terminálu
  a `SIGCHLD`). Server dostane handly vláken klientů při `init_thread`.
- **B:** bez jádra: server vlákno pozastaví (`THREAD_SUSPEND`) a přepíše
  jeho kontext tak, aby skočilo do obsluhy (jako vkládání přes `ptrace`).
  Křehké uprostřed syscallu a bez `EINTR`.
- **C:** pozastavení jen přes `THREAD_SUSPEND` a čtení kontextu serverem
  (`THREAD_STATE`) - Wine ale chce, aby kontext poslalo vlákno samo
  (záplata synchronizace pozastavení v serveru i v `ntdll`).

### T3 - paměť jiného procesu

- **A (doporučuji):** syscall `PROCESS_MEMORY(process, addr, buf, len,
  flags)` s právy handlu procesu `READ_MEMORY` / `WRITE_MEMORY` (debuggery,
  `ReadProcessMemory`), kopie přes jádro bez mapování; etapa W2.
- **B:** jen pro procesy téže aplikace přes sdílené VMO - nepokryje
  libovolné adresy.

### T4 - spuštění procesu a dědění fd

`posix_spawn` v `cstd` (vlákno plánovače) zatím plánuje jen fd 0-2. Wine
dává dítěti socket serveru dalším fd a proměnnou `WINESERVERSOCKET`.

- **A (doporučuji):** `posix_spawn` s akcemi `dup2`/`close` pro libovolná
  fd (fd bez `FD_CLOEXEC` se dědí jako duplikát handlu ve startovní
  zprávě), jako POSIX.
- **B:** jen zděděný socket serveru zvláštním štítkem startovní zprávy
  (záplata Wine v `loader.c`).

### T5 - rezervace adres pro Windows

Rozsahy pro obrazy PE (dolní 2 GiB, obvyklé báze 0x140000000), sdílená
data systému 0x7ffe0000 a zásobníky musí být volné dřív, než `ld.so`
nahraje knihovny na náhodné báze.

- **A (doporučuji):** `ld.so` dostane seznam rozsahů k rezervaci před
  nahráním knihoven (auxv `AT_MO_RESERVE` z manifestu nebo od spouštěče
  vrstvy) a předá je programu jako rezervace (`VM_RESERVE`), které Wine
  převezme (místo `wine-preloader`).
- **B:** báze knihoven a zásobníků vždy nad 4 GiB pro programy s příznakem
  v manifestu (jednodušší, ale pevné pravidlo v zavaděči).

## Co doplní systém

| Kde | Co | Etapa |
|---|---|---|
| jádro | GS programu pro každé vlákno, GS v kontextu (`THREAD_STATE`): `THREAD_SET_POINTER` se `SECOND` ([syscally](syscally.md#thread_set_pointer)) **[ověřeno v QEMU]** x86_64 i aarch64 (`NOT_SUPPORTED`) | W1 |
| jádro | 32bitový segment kódu ring 3, výjimky z režimu compat | W6 |
| správa paměti | oprávnění `wx_compat` (podle Q6) | W1 |
| fsd | dataset s casefold (podle Q7) | W2 |
| libos | spojení procesů aplikace se serverem (kanály s handly), spuštění procesu Windows (`CreateProcess` -> spuštění launcheru se stejným prefixem) | W1-W2 |
| kompozitor | víc oken jedné aplikace s vazbou vlastník-okno, popup bez rámu, poloha okna od aplikace (`SetWindowPos`) | W3 |

## Bezpečnost a izolace

- Každá aplikace má vlastní prefix (= virtuální `C:` a registry) a vlastní
  wineserver; procesy různých aplikací spolu stav nesdílí. Odinstalace =
  smazání prefixu.
- Aplikace dostane jen práva z manifestu (síť, Pracovní složky, zvuk,
  kamera) jako nativní aplikace; vrstva sama žádná práva navíc nemá.
- Kód Wine je LGPL-2.1 a běží v procesu aplikace a ve wineserveru (ne
  v jádře ani ve službách systému); povinnosti LGPL splníme jako u FFmpeg
  (zdroje a záplaty, sdílené knihovny vyměnitelné).
- ⚠️ **Anti-cheat** (EasyAntiCheat, BattlEye, Vanguard) na neznámém
  systému obvykle nepoběží - rozhoduje výrobce hry, ne technika; jaderné
  anti-cheaty nepůjdou nikdy.

## Etapy

| Etapa | Cíl | Hotovo, když |
|---|---|---|
| W0 | sestavení Wine 11 (PE i unixová část) v xtasku, licence v `THIRD-PARTY.md` | `cargo xtask build` vyrobí DLL a `.so` pro x86_64 |
| W1 | konzolové `.exe`: GS, SEH, paměť, soubory, wineserver | test: vlastní konzolový program a `cmd /c echo` z Wine v QEMU |
| W2 | procesy, registry, pojmenované objekty | `reg query`, dva procesy s pojmenovanou událostí |
| W3 | okna, GDI, vstup, schránka | `notepad` (psaní, uložení souboru), `winecfg` |
| W4 | zvuk, jednoduchá hra | `winemine` (GDI), tón přes `mmdevapi` |
| W5 | DXVK, VKD3D-Proton | hra Direct3D 11 nad Venus |
| W6 | WoW64 | 32bitový `notepad` a instalátor |
| W7 | synchronizace bez serveru, výkon | měření snímků hry proti W5 |

## Rizika

- **Rozsah:** Wine je obrovský a každá aplikace najde jinou díru; cíl je
  rozhraní napojit tak, aby opravy chodily z upstreamu, ne psát Win32.
- **Unixová část Wine předpokládá Linux/macOS** na mnoha místech
  (`/proc`, signály mezi procesy, pevné adresy pro ntdll a zásobníky):
  každé místo je buď v našem modulu (Q2 A), nebo potřebuje doplnit
  `cstd`.
- **Rychlost sestavení:** Wine se staví desítky minut; v xtasku jen při
  změně verze nebo záplat (jako porty C).
- **Grafika pro hry** závisí na Vulkanu na skutečném HW (krok 9); v QEMU
  jen Venus.
