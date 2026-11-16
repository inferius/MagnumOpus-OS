# Windows vrstva: Wine nad naším spodním patrem

Návrh kroku 8 pořadí v [architektura.md](architektura.md) (plán „Horní patro
z Wine, spodní naše“): aplikace a hry pro Windows nad převzatým Wine.
**Stav:** otázky rozhodnuté 15. 11. 2026 (kapitola
[Rozhodnutí](#rozhodnutí)); etapa W1 (konzolové programy) **[ověřeno
v QEMU]** x86_64, další etapy **[navrženo]** (tabulka níže).

## Stav

| Část | Co | Stav |
|---|---|---|
| W0 | sestavení Wine 11 (DLL PE, nástroje), port unixové části nad `libc.so` | celé Wine přes `cargo xtask wine` ve WSL **[implementováno]**: 602 DLL jako PE, 25 unixových `.so` nad `libc.so` (`ntdll.so` s `libunwind.so`), zavaděč `wine` a `wineserver`; `wine --version` v systému **[ověřeno v QEMU]** ([Wine na disku](#wine-na-disku-w1)) |
| W1 | konzolové `.exe` (`cmd`, `hostname`, vlastní test): spodní `ntdll`, TEB přes GS, SEH, paměť, soubory, `wineserver` jako proces | **[ověřeno v QEMU]** x86_64, release (sada `wine`): GS v jádře a `ucontext_t` v `cstd` **[ověřeno v QEMU]**; záplata výjimek `signal_x86_64.c` se přeloží **[implementováno]**; Wine na datovém disku jako aplikace (`/apps/org.winehq.wine`), zavaděč `wine` z disku nahraje `ntdll.so` ([DL-4c](dynamicke-linkovani.md#knihovny-z-disku-a-origin-dl-4c)) **[ověřeno v QEMU]**; nový prefix (`wineboot`), `cmd /c echo`, vlastní konzolový program `user/wintest` a `hostname` ([Wine na disku](#wine-na-disku-w1)); oprávnění `wx_compat` (RWX, Q6) zatím není potřeba **[navrženo]** |
| W2 | víc procesů jedné aplikace, registry, pojmenované objekty, `winecfg` v režimu bez oken | rozpracováno: další proces (`CreateProcess`, čekání, kód konce), pojmenované události a `Read`/`WriteProcessMemory` přes wineserver a [`PROCESS_MEMORY`](syscally.md#process_memory) **[ověřeno v QEMU]** x86_64, release ([Paměť jiného procesu](#paměť-jiného-procesu-w2)); sdílené sekce nad souborem mezi procesy, registr (`reg add`/`reg query` v dalším procesu) **[ověřeno v QEMU]** ([Sekce a registr](#sekce-a-registr-w2)); signály serveru klientům, `winecfg` **[navrženo]** |
| W3 | okna: ovladač Wine nad kompozitorem (`guiproto`), GDI, `notepad`, `winecfg`, schránka, vstup | **[navrženo]** |
| W4 | zvuk nad `audiod` (role `Audio`, `winemoaudio.drv`), schránka naše aplikace -> Wine, obnova oken po pádu kompozitoru, PuTTY (okno a konfigurace) | **[ověřeno v QEMU]** x86_64 ([W4](#zvuk-schránka-a-obnova-oken-w4)), obecné části (role `Audio`, `moaudio`) i aarch64; síť z programů Wine (W4b) **[navrženo]** |
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

## Wine na disku (W1)

**[ověřeno v QEMU]** x86_64 (sada `wine`).

- `cargo xtask wine` dá výsledek do `target/wine/x86_64/tree` v rozložení
  adresáře aplikace: `bin/wineserver`, `lib/wine/x86_64-unix` (unixové
  `.so` a zavaděč `wine`), `lib/wine/x86_64-windows` (DLL PE),
  `share/wine` (`nls`, `wine.inf`). Instaluje ho `make install`, bez
  ladicích informací (`llvm-strip`) a bez importních knihoven `.a`:
  145 MB, 871 souborů (před tím 454 MB jen DLL).
- `cargo xtask run|test --wine` dá strom na datový disk do
  `/apps/org.winehq.wine` (jako budoucí instalace balíčku do
  `/apps/<id>`). Disk má pak 512 MiB a vzniká jednou jako šablona
  (`target/wine/x86_64/datadisk-<klíč>.img`, klíč = obsah stromu); běh
  dostane kopii.
- Wine hledá své adresáře relativně k `ntdll.so`, prefix při sestavení
  není potřeba. Zavaděč `wine` najde `ntdll.so` přes `realpath(argv[0])`
  (`/proc/self/exe` u nás není): spouští se proto absolutní cestou
  (`launchd`), `ntdll.so` nahraje `dlopen` absolutní cesty
  ([DL-4c](dynamicke-linkovani.md#knihovny-z-disku-a-origin-dl-4c)).
- Test (`cargo xtask test --only wine --wine`): `wine --version` přes
  `launchd` vypíše `wine-11.0`. Sada `wine` je jen na vyžádání (plný běh
  ji bez `--wine` přeskočí).
- **První konzolový program** **[ověřeno v QEMU]** x86_64 (sada `wine`,
  `--release`): `wine cmd /c echo ahoj` založí nový prefix (`wineboot`:
  `services.exe`, `rundll32 setupapi` nad `wine.inf`, registr,
  `C:\windows\system32`, disky `dosdevices/c:` a `z:` jako symbolické
  odkazy) a `cmd.exe` vypíše `ahoj`, celkem asi 70 s v QEMU TCG. Cestou
  bylo potřeba:
  - záplaty Wine: `0003` wineserver bez `fork` (spustí sám sebe v popředí
    přes `posix_spawn`), `0004` paměť sekcí jako `memfd` (soubory fsd
    zatím nejdou `MAP_SHARED` se zápisem), `0005` nový proces Windows přes
    `posix_spawn` zavaděče (místo dvojího `fork`),
  - `[app] jit = true` v manifestu zavaděče (`wine.manifest.toml`; Wine
    přepíná obrazy PE RW -> RX, [brána JIT](sprava-pameti.md#wx-pečeť-a-oprávnění-jit)),
  - v `cstd`/`osrt`: `posix_spawn` normalizuje cestu, null fd, předání
    spojeného `AF_UNIX` se zprávami ve frontě, symbolické odkazy,
    `mmap(PROT_WRITE)` = i čtení (wineserver tak mapuje `KUSER_SHARED_DATA`;
    bez toho klientům stál čas).
- **Vlastní konzolový program** (`user/wintest`, PE přeložený nástroji
  Wine, na disku `/apps/org.magnumopus.wintest`): `printf`, argumenty
  (i s mezerou), prostředí (`getenv`, `GetEnvironmentVariableA`), soubor
  v textovém režimu (zápis, čtení, atributy, smazání v `%TEMP%`), vlákno
  (`CreateThread`, kód vlákna) a kód konce 42; s hotovým prefixem asi
  0,7 s. `wine hostname` vypíše jméno počítače.
- ⚠️ **Omezení:** Wine čeká na `wineboot` napevno 5 minut. V sestavení
  debug (QEMU TCG) se založení prefixu nestihne (čtení z fsd asi
  1,7 MB/s proti 36 MB/s v release, `wine --version` 609 ms proti 31 ms)
  a `cmd` pak skončí `c0000135`; test je proto pomalý a jen pro
  `--release`. Start procesu Wine v release asi 0,7 s, hlavně načítání
  DLL PE čtením souboru přes fsd po 32 KiB (soubor nejde namapovat).
- **Poznámky pro W2:** rozšířené atributy (`user.DOSATTRIB`) vracejí
  `ENOSYS`, Wine bez nich funguje (atributy DOS z jména/práv); bez ovladače
  oken (`nodrv`, „explorer failed to start“) a bez `RpcSs` - konzolovým
  programům nevadí.

## Paměť jiného procesu (W2)

**[ověřeno v QEMU]** x86_64 (sada `wine`, `wintest.exe procesy`).

- Wineserver klientské procesy nespouští, takže jejich handle nemá. Klient
  mu proto při připojení **svěří svou paměť sám**: `ntdll` vezme procesový
  fd (`mo_self_process_fd`, [cstd](cstd.md#procesový-fd)) a pošle ho
  serveru (`wine_server_send_fd` pod pevným číslem 1021 před
  `init_first_thread`); server ho uloží k procesu a `read_process_memory`
  / `write_process_memory` (`server/ptrace.c`) volají `mo_process_memory`
  místo `process_vm_readv`/`ptrace` (záplata `0006`).
- ⚠️ **Bezpečnost jako `ptrace` stejného uživatele:** fd nese jen práva
  paměti (žádné ukončení ani ladění), má `FD_CLOEXEC` a dostane ho jen
  server vlastní aplikace (prefixu). Server tak může číst a přepsat paměť
  každého svého klienta - stejně jako na Linuxu, kde wineserver a klienti
  běží pod jedním uživatelem.
- Test: `wintest.exe procesy` spustí sám sebe jako dítě (`CreateProcess`),
  domluví se s ním pojmenovanými událostmi (`Local\...`), přečte
  a přepíše mu proměnnou (`ReadProcessMemory`, `WriteProcessMemory`)
  a ověří jeho kód konce.

## Sekce a registr (W2)

**[ověřeno v QEMU]** x86_64 (sada `wine`, release).

- **Sdílená sekce nad souborem** (`wintest.exe sekce`): `CreateFileMapping`
  se jménem nad prázdným souborem (Wine ho zvětší na velikost sekce),
  pohled v rodiči i v dítěti (`OpenFileMapping`) = tytéž stránky, zápis
  dítěte rodič vidí; koherence pohledu s `ReadFile` jiným handlem i
  `WriteFile` do souboru oběma směry. Stojí na `MAP_SHARED` se zápisem
  nad okny souborů z fsd
  ([cstd](cstd.md)); anonymní sekce dál jako `memfd`
  (záplata `0004`).
- **Registr**: `wine reg add` a v dalším procesu `wine reg query` vidí
  hodnotu (registr drží wineserver). Server ho ukládá do `user.reg`
  a `system.reg` periodicky (30 s) a při svém konci; test po
  `wineboot -k` a `wineserver -w` najde klíč v `user.reg`.
- Ukládá se přes dočasný soubor a `rename` na existující `user.reg`:
  `rename` v `osrt` (tedy i `cstd` a `std`) cíl atomicky nahradí jako
  POSIX (požadavek fsd `RENAME_REPLACE`); dřív vracel `EEXIST`.
- Dřívější pád serveru při konci (assert v `free_async_queue`) se se
  signály serveru (záplata `0007`) už neobjevuje: server končí s kódem 0.
- Záplata `0003` předává při opětovném spuštění serveru všechny jeho volby
  (`-d`, `-p`), nejen `-f`.

## Signály serveru (W2)

**[ověřeno v QEMU]** jádro a `cstd` x86_64 i aarch64, napojení Wine
(záplata `0007`) x86_64 (sada `wine`, release: konec procesů prefixu
přes `wineboot -k` a čistý konec serveru).

Wineserver na Linuxu posílá klientům signály: `SIGUSR1` přes `tgkill`
(přerušení vlákna kvůli pozastavení, kontextu a APC jiného procesu),
`SIGQUIT` (násilný konec vlákna) a `SIGKILL` (ukončení procesu). U nás jde
`kill` z `cstd` jen na vlastní děti, proto klient serveru svěří jen to,
co server potřebuje (rozhodnutí A, nejmenší práva):

- **Řídicí fd procesu** (`mo_self_control_fd`): handle vlastního procesu
  jen s právem `KILL` (ukončit, `PROCESS_KILL`), k tomu `DUPLICATE`
  a `TRANSFER`, vždy `FD_CLOEXEC`. Server ho použije místo `kill(pid,
  SIGKILL)` (`process_sigkill`).
- **Fd vlákna** (`mo_self_thread_fd`): handle vlákna z
  [`THREAD_SELF`](syscally.md#thread_self) jen s právem `INTERRUPT`
  (`THREAD_INTERRUPT`), posílá ho každé vlákno v `init_thread`. Server
  místo `tgkill` volá `mo_thread_signal(fd, sig)`: jádro vlákno přeruší
  s kódem = číslo signálu a `cstd` ho doručí obsluze signálu jako na Linuxu
  (pozastavení a kontext řeší klient v obsluze `SIGUSR1` sám).
- Nic dalšího: server nedostane `MANAGE` (pozastavení, kontext, vlákna
  procesu) ani výčet vláken (`PROCESS_THREADS`).
- ⚠️ **Bezpečnost jako `kill`/`tgkill` stejného uživatele na Linuxu**,
  omezeno na server prefixu: fd dostane jen wineserver vlastní aplikace
  (záplata `0007`, pevná čísla fd 1022 a 1023 vedle procesového fd 1021).

## Okna programů Windows (W3)

**Stav:** rozhodnuto 15. 11. 2026 (koordinátor: 1A, 2A, 3A, 4A, 5A, 6C,
7A, 8A, 9A). ⚠️ **W3 je ověřené jen na x86_64**: Windows vrstva je jen
pro x86_64 (Q11 A); obecné části (role `Gui`, protokol okna verze 7,
limity na identitu) jsou ověřené na x86_64 i aarch64 v sadě `gui`.

Grafický ovladač Wine pro náš kompozitor (obdoba `winewayland.drv`) je
samostatný modul Wine `winemo.drv` (LGPL, jen ve složce Wine jako
záplata). Mluví s kompozitorem přes naši knihovnu `mowin` (C API v
`libc.a`, náš kód), ne přímo protokolem.

| Část | Stav |
|---|---|
| Relace okna pro procesy Wine: role `Gui` (zděděná zúžená relace se stejnou identitou, opt-out bez `MO_GUI`, odvolání celého stromu, limity na identitu) | **[ověřeno v QEMU]** x86_64 i aarch64 ([GUI](gui.md#relace-okna-pro-potomky-role-gui)) |
| Protokol okna verze 7: vlastník okna (dialog nad oknem), `POPOVER_DISMISS`, popover v mezích pracovní plochy, vyjednání verze v `HELLO` | **[implementováno]** (testy `libs/comp`) |
| Odmítnutá schránka v logu kompozitoru s identitou aplikace (bez obsahu) | **[implementováno]** (test `denied_clipboard_is_logged_with_identity`) |
| `libs/mowin`: víc oken a popupů na jednom spojení, snímky, události, kurzor, schránka; společný modul událostí s `mosdl` | **[implementováno]** (host testy; v QEMU s ovladačem Wine) |
| `winemo.drv` (`third_party/wine/winemo.drv`, záplata `0008`): okna, GDI do bufferu okna, vstup, kurzor, dekorace, popupy, schránka | **[implementováno]**; GDI okno a obě větve rámu **[ověřeno v QEMU]** x86_64 (sada `wine`: `wintest gdi`, `wintest ramy`) |
| Poznámkový blok (fyzický klik a psaní z QEMU, napsaný text je vidět, Ctrl+S, dialog Uložit jako jako vlastněné okno s fokusem, soubor na disku, Alt+F4, konec procesu), winecfg (dialog v rozumné velikosti) | **[ověřeno v QEMU]** x86_64 (sada `wine`, release) |
| Písma GDI přes FreeType 2.15.0 (`/system/lib/libfreetype.so`) | **[ověřeno v QEMU]** x86_64 (text v poznámkovém bloku, velikost dialogu winecfg); port FreeType x86_64 i aarch64 (sada `cstd porty`) |
| Klávesy: `WM_KEYDOWN` s VK i u písmen bez modifikátoru, znak jednou | **[ověřeno v QEMU]** x86_64 (`wintest klavesy`, klávesy `a` a `z` z QEMU) |
| Schránka Wine -> kompozitor -> jiná aplikace (Ctrl+A, Ctrl+C v poznámkovém bloku, vložení v `gui-probe`) | **[ověřeno v QEMU]** x86_64; opačný směr ve [W4](#zvuk-schránka-a-obnova-oken-w4) |
| Obnova oken Wine po pádu kompozitoru | ve [W4](#zvuk-schránka-a-obnova-oken-w4) |

Rozhodnutí:

- **Identita (2A):** všechny programy Windows jsou zatím jedna aplikace
  `org.winehq.wine` (sdílená oprávnění a schránka); identita podle `.exe`
  později.
- **DPI (4A):** zatím pevně 96 DPI (kompozitor má škálu 100 %); později
  DPI = 96 x škála / 100 podle výstupu, nevědomé aplikace zvětší
  virtualizace DPI ve Wine, buffery ve fyzických px.
- **Dekorace (6C):** okna s titulkem a běžným neklientským kreslením mají
  rám, titulek a tlačítka od systému (`Mode::Standard`, vzhled z tokenů jako
  ostatní okna); ovladač Wine pro ně vrací `GetWindowStyleMasks` s
  `WS_CAPTION` a rámem, win32u je pak z plochy okna vynechá a Wine je
  nekreslí. Okna bez titulku a okna, která si neklientskou oblast zrušila
  (`WM_NCCALCSIZE` vrátí klienta = celé okno), jsou `Frameless` a kreslí je
  Wine celá (odznak důvěry při najetí). Okno, které přepíše jen
  `WM_NCPAINT`, dostane systémový rám a jeho vlastní kreslení rámu není
  vidět (omezení).
- **Popupy (5A):** vlastněná `WS_POPUP` (nabídky, bubliny, rozbalený
  seznam) jsou vrstvy `POPOVER` okna vlastníka v jeho souřadnicích; stisk
  mimo ně je zavře (`POPOVER_DISMISS` -> `WM_CANCELMODE`). Nevlastněné
  popupy (úvodní obrazovka) jsou vlastní okna `Frameless`. Vlastník z jiného
  procesu není podporovaný.
- **Klávesnice:** každá klávesa jde jako `WM_KEYDOWN`/`WM_KEYUP` s VK
  a scan kódem sady 1 (kód evdev tabulkou v ovladači). Písmena a číslice
  mají VK podle znaku z rozložení systému (fyzická klávesa Y na QWERTZ dá
  `z` = `VK_Z`), ostatní klávesy VK z tabulek Wine (pozice US).
  **`WM_CHAR` jen jednou:** `TranslateMessage` se ptá ovladače
  (`ToUnicodeEx`) a ten vrátí znak, který kompozitor poslal s posledním
  stiskem té klávesy (rozložení systému); událost textu se stejným znakem
  se pak už nevkládá. Text bez stisku (mrtvá klávesa, složený znak) jde
  jako Unicode (`VK_PACKET`). S Ctrl/Alt a u řídicích kláves (Enter, Tab,
  Backspace, Esc) dávají znak tabulky Wine. Tabulky rozložení pro Wine
  (`KbdLayerDescriptor`, `MapVirtualKey` pro všechny jazyky) později.
  **Compose a IME přijdou později.**
- **Schránka (7A):** text oběma směry (`CF_UNICODETEXT` <-> UTF-8), číslo
  posledního fyzického stisku procesu; odmítnutí zapíše kompozitor do logu.
- **Polohy oken:** kompozitor polohu oken neurčuje od klienta (jako
  Wayland); `SetWindowPos` mění jen velikost (`WINDOW_SIZE`).
- **Písma:** GDI ve Wine kreslí text přes FreeType 2.15.0 (licence FTL,
  [port C](porty-c.md), `/system/lib/libfreetype.so`, `win32u` ho načte
  `dlopen`). Písma jsou vlastní písma Wine (`share/wine/fonts`, Tahoma,
  Courier a další; LGPL, zůstávají ve složce Wine); nástroj `sfnt2fon`
  z nich při sestavení dělá bitmapová písma `.fon` (FreeType hostitele ve
  WSL). Naše systémové písmo (`libs/font`) Wine zatím nepoužívá;
  náhrada písem (fontconfig a podobně) přijde později.

## Zvuk, schránka a obnova oken (W4)

**Stav:** rozhodnuto 16. 11. 2026 (koordinátor: 3D vyjmuto do W5, 5A, 6A,
7A, 8 ano, 9 PuTTY ve variantě B: okno a konfigurace, síť ve W4b). ⚠️ Ověřené jen na x86_64 (Windows vrstva je jen pro
x86_64, Q11 A); obecné části (role `Audio`, `moaudio`) i na aarch64
v sadě `sdl`.

| Část | Stav |
|---|---|
| Role `Audio` pro procesy Wine (zúžená relace se stejnou identitou, opt-out bez `MO_AUDIO`, odvolání celého stromu, limity na identitu; nahrávání jen přes dialog povolení mikrofonu) | **[ověřeno v QEMU]** x86_64 ([Zvuk](zvuk.md#relace-zvuku-pro-potomky-role-audio)) |
| `moaudio` (`libs/mowin/src/os/audio.rs`): společné jádro zvuku pro Wine i SDL (`mosdl_audio_*` jsou jen obálky) | **[ověřeno v QEMU]** x86_64 (sada `sdl`: zvuk SDL a role Audio) |
| `winemoaudio.drv` (`third_party/wine/winemoaudio.drv`, záplata `0009`): ovladač `mmdevapi` nad `moaudio`, výchozí seznam ovladačů `moaudio,pulse,...` | **[ověřeno v QEMU]** x86_64: tón z WAV (`PlaySound` -> `winmm` -> `mmdevapi` -> `audiod`, test `wintest ton`: přehrání trvá délku záznamu, `audiod` má během něj stream navíc) |
| Schránka naše aplikace -> Wine (`gui-probe kopiruj`, vložení v poznámkovém bloku, uložení a obsah souboru) | **[ověřeno v QEMU]** x86_64 (sada `wine`) |
| Obnova oken Wine po pádu kompozitoru (`mowin`: záznam stavu, nová relace přes lístek role Gui, vlastníci oken, popupy, fokus, `EXPOSE`) | **[ověřeno v QEMU]** x86_64 (test `wine: okna Wine po padu kompozitoru (W4)`, `wintest obnova`: vlastník, vlastněný dialog vystředěný nad ním s fokusem, popup dialogu, stejné pixely před pádem a po obnově, Wine okna překreslí, program běží dál) |
| PuTTY 0.85 (pinnutý zdroj, SHA-256 a podpis v `THIRD-PARTY.md`, překlad v `xtask wine` clangem nad SDK Wine): okno a dialog konfigurace | **[ověřeno v QEMU]** x86_64 (test `wine: PuTTY - dialog konfigurace (W4)`) |
| ⚠️ **Síť z programů Wine zatím ne** (`ws2_32`): Wine vytváří socket ve wineserveru a fd posílá klientovi přes `SCM_RIGHTS`, socket ale mezi procesy předat nejde (`osrt`, `inherit.rs`) a wineserver relaci sítě nemá. Spojení PuTTY (raw/telnet) a přenositelné sockety jsou etapa W4b | **[navrženo]** |

**Zvuk.** Ovladač `winemoaudio.drv` je samostatné DLL (mmdevapi načítá
`wine<jméno>.drv` a každé DLL má jednu tabulku unixových volání, proto ne
v `winemo.drv`). Data aplikace jdou do místního bufferu (jako
`wineoss.drv`); vlákno časovače `mmdevapi` je každou periodu přesune do
sdíleného kruhu `audiod` s předstihem tří period. **Zámek streamu drží
jen práci s pamětí** (`moaudio_put`, `moaudio_read` nečekají a službu
nevolají); volání služby (start, stop, hlasitost) dělá vlákno časovače až
po odemčení a zavření streamu až po konci vlákna. Blokující volání tak
nikdy nedrží zámek, na kterém čekají vlákna aplikace a `mmdevapi` (chyba,
kterou jsme řešili v SDL). Formáty: PCM 8, 16 a 32 bitů a float 32, 1-8
kanálů, 8-192 kHz (převod dělá `audiod`); formát mixu 48 kHz stereo
float; jen sdílený režim (`EXCLUSIVE` = `AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED`).
Hlasitost streamu je průměr kanálů (služba má jednu hlasitost na stream).
MIDI ani aux zařízení nejsou. Bez relace zvuku (proces bez role `Audio`)
je ovladač nedostupný a Wine běží bez zvuku. Otevření vstupu může čekat na
rozhodnutí uživatele v dialogu povolení mikrofonu (volání `Initialize`
aplikace do té doby stojí).

**Obnova oken.** `mowin` zaznamenává vše odeslané kompozitoru
(`guiproto::replay`). Když kanál zanikne, vezme si přes lístek role Gui
novou relaci (`osrt::gui::reconnect`, nejdéle 10 s, tři pokusy) a pošle
poslední potvrzený stav: okna, vlastníky (`WINDOW_OWNER`), buffery (handly
VMO drží `mowin`, kompozitor dostane kopie jen pro čtení), vrstvy obsahu
oken a popupů a titulky. Ukázaná okna hned commitne (jsou vidět se starým
obsahem), buffer na obrazovce označí jako držený novou instancí a ovladači
pošle `MOWIN_EV_EXPOSE` pro každé okno (Wine je překreslí). Dialog se pak
znovu vystředí nad vlastníkem a převezme fokus (pravidlo kompozitoru pro
vlastněné okno okna s fokusem). Popupy (vrstvy `POPOVER`) kreslí kompozitor
jen u okna s fokusem (nabídky aktivního okna); popup vlastníka, kterému
fokus vzal jeho dialog, se ukáže až s návratem fokusu. Bez lístku (proces bez role Gui) nebo po
odvolání GUI je ztráta spojení konec (`SERVICE_UNAVAILABLE`).

**PuTTY.** Překládá se na hostiteli (žádné nástroje navíc: CMake, Ninja
a LLVM jako u portů C): clang pro `x86_64-pc-windows-msvc` bez hlaviček
a knihoven hostitele, s hlavičkami Wine a importními knihovnami PE jako
`<dll>.lib` (`target/wine/<arch>/sdk`, exportuje `build_wine.sh`), link
`lld-link` s CRT Wine (`ucrtbase`, `mainCRTStartup`). Start s `WinMain`
a podporu TLS (`__declspec(thread)`) dává náš `xtask/src/wine/wincrt.c`;
prostředky (`.rc`) `llvm-rc`. `cargo xtask wine-putty` přeloží jen PuTTY
nad hotovým SDK.

## 3D: Vulkan, OpenGL a Direct3D (W5)

**Stav: [navrženo]**, rozhodnuto 16. 11. 2026 (koordinátor). Samostatná
etapa W5; ve W3 a W4 okna Wine kreslí jen GDI.

- **Vulkan pro programy v C (1A):** port Mesa (MIT) s ovladačem Venus
  a vlastním backendem rendereru nad render uzlem (`renderproto`:
  kontext s capsetem Venus, bloby, odeslání s ploty) a Khronos
  Vulkan-Loader (Apache-2.0) jako obecný port C v `/system/lib`
  (`libvulkan.so`, ICD podle JSON): používají ho nativní programy i Wine
  (`winevulkan` načítá `libvulkan.so.1`). Dnešní vlastní Venus v Rustu
  (`libs/venus`, [Venus](venus.md)) zůstává pro kompozitor.
- **Prezentace do okna (2A):** swapchain v ICD z obrázků v paměti
  `HOST_VISIBLE` (blob = VMO); kompozitor je dostane jako buffer `bufdesc`
  (`BUFFER_CREATE_DESC`, `LINEAR`, BGRA) s ploty `SYNC_POINTS` (získání =
  dokončení vykreslení, uvolnění = `RELEASED`) a čte je bez kopie.
  Protokol okna ani `bufdesc` se nemění; `mowin` dostane prezentaci bufferu
  s popisem a ploty.
- **OpenGL (3A):** Mesa zink (GL nad Vulkanem) ze stejného portu; WGL
  přes ovladač Wine.
- **Direct3D 9-11 (4A):** DXVK (zlib, PE DLL nad Vulkanem).
- ⚠️ **Omezení:** Venus jen s QEMU, které má virtio-gpu `venus=on`
  a renderer hostitele (virglrenderer). **Windows QEMU (výchozí) ani
  VMware 3D nemají.** Ve WSL je renderer softwarový lavapipe (llvmpipe):
  velmi pomalý (ukázky jako vkcube ano, hry ne). Zobrazení bez kopie
  (dmabuf) ve WSL nejde ověřit ([Venus](venus.md)). Testy 3D poběží jen
  s `--qemu-wsl --gpu venus`, jinde se přeskočí.

## W1: spojení se serverem a procesy (otázky T1-T5)

**Stav: [navrženo]**, rozhodnuto 15. 11. 2026 (koordinátor); T2
**[ověřeno v QEMU]**. Rozbor Wine 11 (`dlls/ntdll/unix/server.c`,
`server/request.c`, `server/ptrace.c`):

| Otázka | Rozhodnutí | Kdo |
|---|---|---|
| T1 fd mezi procesy | **A**: obecné `AF_UNIX` v `cstd`/`osrt`. Cesta socketu zůstává v modelu capability: `bind` vytvoří ve fsd uzel typu socket, `connect` potřebuje právo zápisu na uzel; `socketpair` a `SCM_RIGHTS` = předání handlu v kanálu. | vlákno plánovače (s `AF_UNIX` v `cstd`) |
| T2 signál vláknu jiného procesu | **A**: [`THREAD_INTERRUPT`](syscally.md#thread_interrupt) (syscall 76), právo `MANAGE` na handlu vlákna; obecné pro `kill`/`pthread_kill`, Ctrl+C a `SIGCHLD`. | vlákno Wine (jádro, `cstd` doručení) |
| T3 paměť jiného procesu | **A v W2**: [`PROCESS_MEMORY`](syscally.md#process_memory) (syscall 77), práva `READ_MEMORY`/`WRITE_MEMORY` na handlu procesu; jádro a libos **[ověřeno v QEMU]** x86_64 i aarch64, napojení wineserveru procesovým fd **[ověřeno v QEMU]** x86_64 ([Paměť jiného procesu](#paměť-jiného-procesu-w2)). | vlákno Wine, etapa W2 |
| T4 dědění fd | **A**: `posix_spawn` dědí libovolná fd bez `FD_CLOEXEC` jako duplikáty handlů ve startovní zprávě. | vlákno plánovače |
| T5 rezervace adres | **A**: `AT_MO_RESERVE`, `ld.so` rezervuje rozsahy před nahráním knihoven. Provedeno ještě dřív: rezervuje `spawn` před namapováním programu i `ld.so` ([dynamicke-linkovani.md](dynamicke-linkovani.md#rezervace-adres-před-zavedením-t5)), **[ověřeno v QEMU]**. | vlákno Wine |

**Riziko T3 (výslovně):** `WRITE_MEMORY` dává držiteli úplnou vládu nad
cílovým procesem (může přepsat kód, zásobník i klíče v paměti) a `READ_MEMORY`
čte všechna jeho tajemství. Obě práva proto dostane jen **tvůrce procesu**
(handle z `PROCESS_CREATE`/spuštění) nebo **debugger**, kterému je tvůrce
výslovně předá; nikdy je nedává služba jiné aplikaci a handle z výčtu
procesů je nemá. Wineserver je dostane jen pro procesy, které sám spustil
(nebo které se k němu připojí s handlem od spouštěče vrstvy).

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
