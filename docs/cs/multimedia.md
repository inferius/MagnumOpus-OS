# Multimédia: FFmpeg a přehrávač

Návrh kroku 7 pořadí v [architektura.md](architektura.md): dekódování zvuku
a videa převzatou knihovnou FFmpeg v **izolovaném procesu** a nad ním
přehrávač. Stav jednotlivých částí je v tabulce níže; služba, protokol
a testovací dekodér už běží, FFmpeg a přehrávač zatím ne.
Rozhodnutí uživatele (2026-11-14) jsou v kapitole
[Rozhodnutí](#rozhodnutí). Části závislé na portech C (F1, sestavení
FFmpeg a knihoven titulků) čekají, až bude na `main` mechanismus portů C
z vlákna SDL; služba, protokol a přehrávač s testovacím dekodérem na něm
nezávisí.

## Stav

| Část | Co | Stav |
|---|---|---|
| F1 | sestavení FFmpeg jako port C (LGPL, sdílené knihovny nad `libc.so`), test `ffprobe`-like programu v QEMU | **[navrženo]** |
| F2a | protokol `libs/mediaproto` (host testy), služba `mediad`, testovací dekodér `mediatest` (obraz NV12 do poolu s plotem, tón do kruhu `audiod`), zdroj VMO i kanálem, `SEEK`, pád a obnova dekodéru | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `media`) |
| F2b | proces `ffdec` s FFmpeg místo `mediatest` (čeká na mechanismus portů C) | **[navrženo]** |
| F3 | video do sdílených bufferů (`bufdesc` NV12/I420/P010) s ploty, A/V synchronizace | **[navrženo]** |
| F4 | aplikace Přehrávač nad `libs/ui` (videovrstva, ovládání, titulky libass, mediální relace) | **[navrženo]** |
| F5 | hardwarová akcelerace dekódování (VA-API/Vulkan Video přes ovladač GPU) | **[navrženo]** |

## Proč izolovaný proces

Demuxery a dekodéry zpracovávají nedůvěryhodná data ze sítě a ze souborů;
FFmpeg má každý rok desítky oprav paměťových chyb. Proto:

- dekodér běží v **samostatném procesu bez práv**: žádný přístup k FS, síti,
  launchd ani k ovladačům; dostane jen kanál ke `mediad` a sdílené buffery,
- jeden proces dekodéru **na jednu relaci přehrávání** (rozhodnutí 1), pád nebo
  zneužití jedné relace nezasáhne jinou ani aplikaci,
- vstup jsou **jen bajty** (proud nebo bloky souboru), výstup jen
  dekódované snímky a vzorky ve sdílené paměti,
- limity jobu (paměť, CPU, počet vláken) z manifestu služby; po pádu
  `mediad` relaci ohlásí chybou a aplikace ji může otevřít znovu od
  posledního času (zotavení jako u GUI).

Kód pod LGPL tím zůstává v oddělené binárce i procesu; jádro, `libs/`,
služby a aplikace na něm nezávisí (pravidla v `THIRD-PARTY.md`).

## Architektura

<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cesta médií">
  <style>
    .b{fill:#f4f6fa;stroke:#4a5a78;stroke-width:1.5}
    .i{fill:#fdf1e6;stroke:#b4692a;stroke-width:1.5}
    .t{font:13px sans-serif;fill:#1d2433}
    .s{font:11px sans-serif;fill:#4a5a78}
    .a{stroke:#1d2433;stroke-width:1.3;fill:none;marker-end:url(#m)}
    .d{stroke:#b4692a;stroke-width:1.3;fill:none;stroke-dasharray:5 3;marker-end:url(#m)}
  </style>
  <defs><marker id="m" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0L10,5L0,10z" fill="#1d2433"/></marker></defs>
  <rect class="b" x="20" y="20" width="200" height="70" rx="6"/>
  <text class="t" x="35" y="45">Přehrávač (aplikace)</text>
  <text class="s" x="35" y="65">libs/ui, videovrstva,</text>
  <text class="s" x="35" y="80">ovládání, titulky</text>
  <rect class="b" x="290" y="20" width="180" height="70" rx="6"/>
  <text class="t" x="305" y="45">mediad (služba)</text>
  <text class="s" x="305" y="65">relace, čtení vstupu,</text>
  <text class="s" x="305" y="80">spouští dekodéry</text>
  <rect class="i" x="290" y="150" width="180" height="80" rx="6"/>
  <text class="t" x="305" y="175">ffdec (1 na relaci)</text>
  <text class="s" x="305" y="195">FFmpeg .so nad libc.so</text>
  <text class="s" x="305" y="210">bez FS, sítě, ovladačů</text>
  <rect class="b" x="560" y="20" width="180" height="60" rx="6"/>
  <text class="t" x="575" y="45">fsd / netd</text>
  <text class="s" x="575" y="65">zdroj bajtů</text>
  <rect class="b" x="560" y="150" width="180" height="60" rx="6"/>
  <text class="t" x="575" y="175">audiod</text>
  <text class="s" x="575" y="195">kruh vzorků + latence</text>
  <rect class="b" x="20" y="270" width="200" height="70" rx="6"/>
  <text class="t" x="35" y="295">compd</text>
  <text class="s" x="35" y="315">FRAME / PRESENTED,</text>
  <text class="s" x="35" y="330">videovrstva okna</text>
  <rect class="b" x="290" y="270" width="180" height="70" rx="6"/>
  <text class="t" x="305" y="295">sdílené buffery</text>
  <text class="s" x="305" y="315">bufdesc NV12/I420/P010</text>
  <text class="s" x="305" y="330">+ ploty acquire/release</text>
  <path class="a" d="M220,55 L288,55"/>
  <text class="s" x="225" y="48">mediaproto</text>
  <path class="a" d="M558,50 L472,50"/>
  <path class="a" d="M380,92 L380,148"/>
  <text class="s" x="386" y="125">bajty vstupu</text>
  <path class="d" d="M472,185 L558,180"/>
  <text class="s" x="478" y="172">vzorky</text>
  <path class="d" d="M380,232 L380,268"/>
  <text class="s" x="386" y="255">snímky</text>
  <path class="a" d="M288,305 L222,305"/>
  <path class="a" d="M120,92 L120,268"/>
  <text class="s" x="126" y="190">commit bufferu</text>
</svg>

Plné šipky jsou řízení a důvěryhodná data, čárkované výstup procesu
dekodéru (jen do sdílené paměti, kterou mu někdo předal).

### Role

- **Přehrávač** (aplikace) otevře relaci u `mediad`, předá mu vstup
  (rozhodnutí 2), alokuje nebo dostane pool bufferů (rozhodnutí 3), řídí přehrávání
  a skládá video do svého okna.
- **mediad** (služba, důvěryhodná, pod supervisorem initu) drží relace
  (jedno vlákno na relaci), čte vstup za aplikaci a spouští dekodér
  přímo (`spawn_with`) ve vlastním jobu s limity (64 MiB uživatelské,
  4 MiB jaderné paměti) **bez jakýchkoli handlů** kromě řídicího kanálu.
  Události dekodéru před přeposláním kontroluje (číslo bufferu v poolu,
  dekodér nesmí sám ohlásit svou ztrátu). Rozhoduje, které kodeky jsou
  povolené (rozhodnutí 5).
- **ffdec** je jediný proces, ve kterém běží kód FFmpeg: libavformat
  (demux), libavcodec (dekódování), libswresample (převod formátu vzorků),
  podle potřeby libswscale. Nemá žádné capability kromě kanálu k `mediad`,
  VMO bufferů a plotů.

### Protokol `mediaproto` [ověřeno v QEMU]

Crate `libs/mediaproto` (kódování `wire`, čtení neplatné zprávy vrací
`INVALID_ARGS`, nikdy paniku). **Jeden kanál klienta = jedna relace**;
další relace = další kanál (`CONNECT` na řídicím kanálu služby, který
drží init). `mediad` posílá dekodéru tytéž požadavky (navíc `PROBE`)
a jeho události přeposílá aplikaci.

Zprávy od služby začínají značkou u32: `0` = odpověď na poslední
požadavek (status i32 + data), jinak číslo události. Události chodí
kdykoli, i během čekání na odpověď (klient `MediaClient` je řadí do
fronty).

| Operace | Číslo | Parametry -> odpověď | Handly |
|---|---|---|---|
| `CONNECT` | 1 | -> nový kanál relace | odpověď: kanál |
| `OPEN` | 2 | druh zdroje u8 (`1` VMO, `2` kanál), velikost u64 -> `Info` | VMO jen pro čtení / kanál zdroje |
| `SELECT` | 3 | video, zvuk, titulky i32 (`-1` = žádný) | - |
| `POOL` | 4 | počet u8 + `BufDesc` každého bufferu | VMO bufferů, nakonec plot snímku |
| `AUDIO` | 5 | frekvence u32, kanály u8, vzorek u8, kapacita u32 | VMO kruhu, událost periody |
| `PLAY` / `PAUSE` | 6 / 7 | - | - |
| `SEEK` | 8 | čas us u64, přesně u8 -> skutečný čas u64 | - |
| `RELEASE` | 9 | buffer u8 | - |
| `CLOSE` | 10 | konec relace (dekodér skončí) | - |
| `REOPEN` | 11 | po ztrátě dekodéru: nový se stejným zdrojem, poolem, kruhem a výběrem -> `Info` | - |
| `PROBE` | 50 | jen `mediad` -> dekodér: velikost u64 -> `Info` | kanál zdroje |

| Událost | Číslo | Obsah |
|---|---|---|
| `FRAME` | 100 | buffer u8, pts us u64, trvání us u32, pořadí u64 (= hodnota plotu snímku) |
| `AUDIO_BASE` | 101 | pozice v kruhu u64, pts us u64 (kotva hodin zvuku po startu a `SEEK`) |
| `END` | 102 | konec všech vybraných proudů |
| `ERROR` | 103 | kód chyby i32, dekodér ztracen u8 (relace jde obnovit `REOPEN`) |

`Info` = délka us u64 a proudy (druh, kodek, rozměr, fps jako zlomek,
frekvence, kanály, jazyk, kodek dostupný). Titulky (`SUBTITLE_SIZE`,
`SUBTITLE`) přijdou s libass v F4 a zatím nejsou v protokolu.

**Zdroj bajtů** (`mediaproto::source`, odchylka od návrhu): fsd zatím
neumí předat jeden otevřený soubor jako přenositelný handle (handly
souborů platí jen v relaci fsd). Zdroj je proto buď **VMO jen pro čtení**
(malé soubory, testy), nebo **kanál zdroje**, na kterém aplikace odpovídá
na `READ_AT { posun u64, délka u32 }` (nejvýš 64 KiB) a `SIZE`. Dekodér
čte vždy přes svůj kanál zdroje od `mediad`, nikdy nedostane handle VMO
ani kanál aplikace. Úzký handle jednoho souboru ve fsd je samostatný
krok.

**Hodiny** (`mediaproto::clock`, host testy): čas zvuku = pts kotvy +
(přehrané rámce - latence * frekvence - pozice kotvy) / frekvence; bez
zvuku `WallClock`. Výběr snímku `decide`: snímek patří na výměnu, když
čas výměny padne do `[pts - trvání/2, pts + trvání)`; dřív = podržet,
později = zahodit, odchylka nad 200 ms = srovnat skokem.

### Testovací dekodér `mediatest` [ověřeno v QEMU]

Proces se stejným rozhraním jako budoucí `ffdec`, bez FFmpeg. Čte
testovací formát `MTM1` (`mediaproto::testmedia`: hlavička + záznam
každého snímku s pts, semínkem a příznakem klíčového snímku) přes kanál
zdroje, kreslí deterministický obraz NV12 (přechod jasu, pohyblivý pruh,
barva podle semínka) přímo do VMO z poolu, posune plot snímku a pošle
`FRAME`; tón s16 stereo píše do kruhu `audiod`. Pole `crash_at` v hlavičce
ho na daném snímku schválně ukončí (test obnovy relace). Dekóduje dopředu,
dokud má volné buffery; tempo určuje aplikace vracením bufferů
(`RELEASE`) a kruh zvuku.

Omezení testovacího dekodéru: jen NV12 a s16 stereo; po `SEEK` dohraje
zvuk, který už je v kruhu (kruh producent sám vyprázdnit neumí - nová
kotva `AUDIO_BASE` to aplikaci řekne).

Sada `media` (`user/init/src/tests/media.rs`) ověřuje: popis proudů,
obsah a pořadí snímků proti kontrolnímu součtu a plot, skok na klíčový
a přesný snímek, zdroj kanálem aplikace, zvuk do `audiod` s postupem
hodin zvuku a `END`, pád dekodéru (`ERROR` se ztrátou, `REOPEN`, `SEEK`,
snímek po obnově), neplatné požadavky a poškozený soubor.

### Video: buffery a ploty

Výstup videa jsou buffery popsané `bufdesc` (fourcc `NV12` / `YU12` (I420) /
`P010` pro 10 bit, roviny, rozteče, zarovnání). Dekodér zapíše snímek
a posune **acquire plot**; kompozitor snímek zobrazí, až plot nastane,
a po výměně posune **release plot**, teprve pak smí dekodér buffer znovu
použít. Software dekodéry FFmpeg píší do vlastních rámců: `ffdec` použije
`get_buffer2` tak, aby dav1d/libvpx/h264 dekódovaly **přímo do VMO
z poolu** (bez kopie) se zarovnáním, které FFmpeg vyžaduje (64 B rozteč
a okraje).

Pozor na stav závislostí:

- **Ploty v jádře (6a-1) jsou na `main`** ([ploty.md](ploty.md)).
- **`bufdesc`, `BUFFER_CREATE_DESC` a body synchronizace oken (6a-3, 6a-4)
  jsou na `main`**; compd převádí YUV na CPU (`libs/yuv`) nebo ve
  `vkcomp`. `libs/ui` zatím neumí vrstvu s cizím bufferem: přehrávač
  potřebuje widget videovrstvy (F4).
- Zobrazení YUV bez převodu na CPU potřebuje videovrstvu v compd
  ([rozhrani-zarizeni.md](rozhrani-zarizeni.md), Videorovina [navrženo]);
  do té doby compd převádí YUV -> RGB shaderem nebo na CPU (cena: jedna
  kopie každého snímku navíc, změří se v F3).

### Zvuk

Dekódované vzorky jdou do `audiod` přes jeho **sdílený kruh**; `audiod`
mixuje a převádí frekvenci. `ffdec` píše do kruhu přímo (libswresample na
formát proudu, typicky f32 / 48 kHz), aplikace ani `mediad` vzorky
nekopírují. Hlavička kruhu nese počet přehraných rámců a latenci výstupu,
což je zdroj hodin pro synchronizaci.

### A/V synchronizace

- **Hlavní hodiny jsou zvuk**: čas = pts posledního zapsaného vzorku -
  (zapsané - přehrané rámce + latence) / frekvence. Bez zvukové stopy jsou
  hodinami monotonní čas.
- Video se plánuje podle zpětné vazby compd: `FRAME { seq, deadline,
  period }` říká, kdy je další výměna, `PRESENTED { txn, time }` kdy snímek
  skutečně svítil. Přehrávač vybere snímek, jehož pts nejlépe padne na čas
  výměny, a commitne ho s cílovým časem.
- Odchylka nad ~1 snímek: zahodit snímek (pozadu) nebo podržet (napřed);
  zvuk se nikdy nenatahuje, jen při velkém skoku (> 200 ms) se resetuje kruh.
- Rozdílné takty (24 fps na 60 Hz): rytmus 3:2 vychází z deadlinů compd
  přirozeně; při VRR compd může periodu přizpůsobit (později).

### Přehrávač (aplikace) [navrženo]

Aplikace nad `libs/ui`. Potřebuje rozšíření `libs/ui`:

- **widget videovrstvy**: oblast okna, do které se commitují buffery
  z `mediad` přímo (ne přes CPU obraz); dnes `libs/ui` umí jen CPU obrázek,
- ovládání: přehrát/pauza, posuvník s náhledy klíčových snímků, hlasitost,
  výběr stop, celá obrazovka, klávesové zkratky a mediální klávesy,
- titulky: plné ASS/SSA, SRT a WebVTT přes libass (kapitola
  [Titulky](#titulky)); přehrávač jen skládá hotové bitmapy nad video,
- **mediální relace** pro systém (právě hraje, ovládání z panelu, pozastavení
  při odpojení sluchátek) podle [rozhrani-zarizeni.md](rozhrani-zarizeni.md).

## Kodeky

Pořadí podle licenčního rizika. Všechny dekodéry jsou softwarové v rámci
FFmpeg nebo jeho externích knihoven pod kompatibilními licencemi.

| Skupina | Kodeky / kontejnery | Zdroj | Výchozí stav |
|---|---|---|---|
| bez poplatků | AV1 (dav1d, BSD-2), VP8/VP9 (vestavěné ve FFmpeg), Opus, Vorbis, FLAC, Theora; WebM/Matroska, Ogg, MP4/ISOBMFF (kontejner), WAV | FFmpeg + dav1d | zapnuté |
| patenty vypršely | MP3, AC-3, MPEG-1/2 video | FFmpeg | vždy zapnuté |
| patentově zatížené | H.264, HEVC, AAC, E-AC-3 | jen nativní dekodéry FFmpeg (LGPL) | **zapnuté ve výchozím obrazu**; přepínač xtasku sestaví obraz bez nich |
| nikdy | cokoli vyžadující `--enable-gpl`, `--enable-nonfree` (x264, x265, fdk-aac, libpostproc s GPL) | - | vypnuté natvrdo |

Dav1d je rychlejší než vestavěný dekodér AV1 ve FFmpeg a je BSD, proto ho
FFmpeg použije jako externí knihovnu (vlastní port C stejným mechanismem).
Kodéry (encode) nejsou v F1-F4 potřeba; nahrávání obrazovky je samostatný
pozdější krok.

**Patentově zatížené kodeky (rozhodnutí 5 = B).** H.264, HEVC, AAC
a E-AC-3 jsou ve výchozím obrazu zapnuté, ale **jen nativní dekodéry
FFmpeg pod LGPL**; žádné knihovny pod GPL (x264, x265) ani nonfree
(fdk-aac). Kodéry těchto formátů se nesestavují.

> **Riziko (vědomě přijaté):** šíření systému s dekodéry H.264, HEVC
> a AAC může vést k **licenčním nárokům patentových poolů** (MPEG LA /
> Via LA pro H.264 a AAC, Access Advance a Via LA pro HEVC), hlavně
> v USA a dalších zemích, kde jsou softwarové patenty vymahatelné. Licence
> FFmpeg (LGPL) patenty nepokrývá. Kdo systém šíří, musí to posoudit
> sám; text je i v `THIRD-PARTY.md`.

Pro sestavení bez nich zůstává **přepínač xtasku** (pracovně
`--bez-patentovych-kodeku`): configure dostane `--disable-decoder=` pro
h264, hevc, aac, aac_latm, eac3 (a odpovídající parsery), `xtask` po
sestavení ověří v `config.h`, že jsou vypnuté, a `mediad` je pak hlásí
jako nedostupné (aplikace dostane `INFO` s kodekem a chybou "kodek
nedostupný", ne pád).

## Titulky

Rozhodnutí 9 = B: **libass** (ISC) s **HarfBuzz** (MIT, tvarování písma),
**FriBidi** (LGPL-2.1-or-later, obousměrný text) a **FreeType** (pod
licencí FTL, ne GPL - FreeType je duálně FTL/GPL-2 a my volíme FTL).

- **Proč FreeType, ne `libs/font`**: libass volá přímo API FreeType
  (`FT_Face`, `FT_Outline`, ...), nemá vrstvu pro jiný rasterizér.
  Náhrada by znamenala vlastní záplatu velké části libass nebo obálku
  API FreeType nad `libs/font`; obojí je dražší a křehčí než převzít
  FreeType. `libs/font` zůstává písmem systému, FreeType žije jen
  v procesu dekodéru.
- **FTL** vyžaduje zmínku v dokumentaci produktu ("Portions of this
  software are copyright The FreeType Project"); přidá se do `About`
  přehrávače a `THIRD-PARTY.md`.
- **Kde běží**: titulky ASS jsou nedůvěryhodná data (skripty s tisíci
  tagů, vložená písma), proto libass běží **v procesu `ffdec`** se stejnou
  izolací jako dekodér. Výstupem jsou bitmapy (`ASS_Image`: alfa maska +
  barva + pozice), které `ffdec` zapíše do sdíleného bufferu titulků
  (RGBA s předem vynásobenou alfou) s plotem; přehrávač je jen skládá
  nad video.
- **Písma**: `ffdec` nemá FS, takže fontconfig se nesestavuje
  (`--disable-fontconfig`). Písma dodá `mediad`: vložená písma
  z kontejneru (přílohy Matroska) a systémová písma, která si skript
  vyžádá, předá jako bajty (`ass_add_font`); výchozí písmo je písmo
  systému z `design/tokens.toml`.
- **Rozměr a čas**: přehrávač pošle velikost videovrstvy a poměr pixelů
  (`SUBTITLE_SIZE`), `ffdec` vykresluje k pts snímku, který právě dodává;
  změna velikosti okna = nové vykreslení.
- SRT a WebVTT jdou stejnou cestou (FFmpeg je převede na ASS), takže
  vykreslení je jedno.

Licence knihoven titulků: ISC, MIT a FTL jsou volné; FriBidi pod LGPL se
linkuje dynamicky jako FFmpeg (stejné povinnosti, kapitola
[Povinnosti LGPL](#povinnosti-lgpl)).

## Hardwarová akcelerace (později, F5)

Dekódování na GPU (Intel Quick Sync, AMD VCN, NVIDIA NVDEC) přijde až po
ovladačích GPU ([gpu-amd-nvidia.md](gpu-amd-nvidia.md)). Tvar rozhraní už
teď: `ffdec` dostane od služby GPU **frontu video dekódování** (ne zařízení),
výstup jsou tytéž buffery `bufdesc` s ploty, takže přehrávač ani compd se
nemění. Aplikace capability k ovladači nikdy nedostane.

## Sestavení FFmpeg

### Port C ve xtasku

Obecný mechanismus portů C (tarball + SHA-256 + záplaty + CMake/Ninja/
autotools) navrhuje vlákno SDL (`navrh-sdl.md`, jeho otázka 2 = C). FFmpeg ho
**použije, ne zdvojí**. Co od něj potřebuje navíc proti CMake/autotools:

1. **Vlastní `configure`** (shell skript, ne autoconf): běh pod `sh`
   (Git Bash / MSYS2 na Windows hostiteli), `make` (GNU make) pro build.
   Mechanismus musí umět "jiný druh" sestavení: příkaz configure s přepínači
   a pak `make -jN install`.
2. **Křížový překlad**: `--enable-cross-compile --target-os=none` (nebo
   záplata přidávající `magnumopus`), `--arch=x86_64|aarch64`,
   `--cc=clang --cfg=...` se `clang-dyn.cfg` ze C sysrootu, `--ar=llvm-ar`,
   `--nm=llvm-nm`, `--ranlib=llvm-ranlib`, `--pkg-config` mířící na `.pc`
   v sysrootu (pro dav1d).
3. **Assembler**: na x86_64 **nasm** (>= 2.13) pro ruční SIMD; mechanismus
   ho najde a při chybějícím dá radu k instalaci (rozhodnutí 7). Na aarch64
   stačí clang jako assembler (GNU syntaxe `.S`, NEON), nic navíc.
4. **Záplaty**: `target-os` pro náš systém, detekce počtu CPU přes
   `sysconf(_SC_NPROCESSORS_ONLN)` (cstd), přepínače
   `--disable-network`, `--disable-autodetect`; FFmpeg nesmí sáhnout po
   `fork` (cstd ho nemá, jen `posix_spawn`).
5. **Out-of-tree a per-arch**: build do `target/cports/<arch>/ffmpeg/`,
   instalace hlaviček, `.so` a `.pc` do C sysrootu, otisk (verze + záplaty
   + přepínače) pro přeskočení opakovaného buildu.
6. **Závislosti mezi porty**: dav1d (Meson + Ninja) před FFmpeg; pro
   titulky FreeType (CMake), FriBidi a HarfBuzz (Meson) a libass
   (autotools) před `ffdec`. Meson je další systém sestavení, který
   mechanismus SDL zatím nepokrývá; mechanismus ho musí umět (nebo
   alespoň "vlastní příkaz configure + build", jako pro FFmpeg).
7. **Kontrola nástrojů** (rozhodnutí 8 = A): xtask před buildem ověří
   `sh`, `make`, `nasm` (x86_64), `meson`, `ninja`, `pkg-config` z Git
   Bash / MSYS2 a při chybějícím nástroji skončí s přesnou radou, jak ho
   doinstalovat (bez tichého přeskočení portu).

Licenční přepínače natvrdo: `--disable-gpl --disable-nonfree
--disable-version3` (LGPL 2.1+), `--enable-shared --disable-static`,
`--disable-programs` (vlastní `ffdec` místo `ffmpeg`), `--disable-doc`.
`xtask` po configure ověří v `config.h`, že `CONFIG_GPL 0` a
`CONFIG_NONFREE 0`, jinak build selže.

### Na čem stojí v systému

- `libc.so` (sloučeno) a dynamický zavaděč `ld.so`: FFmpeg je sada `.so`
  (`libavformat.so.61`, `libavcodec.so.61`, `libavutil.so.59`,
  `libswresample.so.5`, `libswscale.so.8`) linkovaná proti `libc.so`.
- `cstd` s POSIX vrstvou (sloučeno): pthreads
  (FFmpeg dekóduje ve vláknech), `clock_gettime`, `sysconf`, `mmap` pro
  zarovnané alokace (`posix_memalign` stačí).
- W^X: FFmpeg ani dav1d nepotřebují JIT, ruční SIMD je předpřeložený.
- AVX/XSAVE: požadavek už popsaný v [cstd.md](cstd.md) (FFmpeg detekuje
  AVX2/AVX-512 za běhu přes `cpuid` + `xgetbv`, jádro musí ukládat stav).

## Povinnosti LGPL

LGPL 2.1+ dovoluje FFmpeg použít v uzavřeném systému, pokud uživatel může
knihovnu nahradit vlastní upravenou verzí. Návrh:

- FFmpeg je **dynamicky linkovaný** (`.so` nad `libc.so`), proces `ffdec`
  je jen tenká vrstva; nahrazení knihovny = nahrazení souborů v `/lib`
  (rozhodnutí 6),
- **zdroje**: přesný tarball (SHA-256), všechny záplaty a přepínače
  configure jsou v repozitáři (`third_party/ffmpeg/`); obraz systému nese
  soubor s odkazem na ně a text licence,
- **skript pro opětovné sestavení**: `cargo xtask cport ffmpeg` sestaví
  stejnou knihovnu z týchž zdrojů; popis v uživatelské příručce,
- `THIRD-PARTY.md` doplní řádky FFmpeg (LGPL-2.1-or-later), dav1d
  (BSD-2-Clause), libass (ISC), HarfBuzz (MIT), FriBidi
  (LGPL-2.1-or-later) a FreeType (FTL) s verzí a hashem,
- FriBidi je pod LGPL stejně jako FFmpeg: sdílená `.so`, zdroje
  a skript pro opětovné sestavení stejně,
- `About` přehrávače uvede, že používá FFmpeg pod LGPL, s odkazem na zdroje.

## Etapy

1. **F1 port**: dav1d + FFmpeg (a FreeType, FriBidi, HarfBuzz, libass)
   přes mechanismus portů, test v QEMU:
   program nad `libavformat` vypíše proudy testovacího souboru WebM
   (Opus + VP9) a dekóduje 10 snímků s kontrolním součtem, obě architektury.
2. **F2 služba a zvuk**: `mediaproto`, `mediad`, `ffdec`; přehrání Opus/FLAC
   do `audiod`; test pádu dekodéru (relace hlásí chybu, `mediad` žije).
   Protokol, služba a izolace se staví **nejdřív s testovacím dekodérem**
   (proces se stejným rozhraním jako `ffdec`, generuje tón a testovací
   obrazce bez FFmpeg), takže nečekají na porty C.
3. **F3 video**: buffery `bufdesc` s ploty (6a-3/6a-4 jsou na `main`),
   A/V synchronizace, měření latence a zahozených snímků.
4. **F4 přehrávač**: aplikace, widget videovrstvy v `libs/ui`, titulky,
   mediální relace.
5. **F5 HW akcelerace**: po ovladačích GPU.

## Rozhodnutí

Rozhodnuto uživatelem 2026-11-14 (původní otázky A/B/C, doporučení bylo
vždy A).

| # | Otázka | Rozhodnutí |
|---|---|---|
| 1 | izolace dekodéru | **A** jeden proces `ffdec` na relaci |
| 2 | vstup k dekodéru | **A** aplikace předá handle `mediad`, ten posílá bajty; síťové proudy čte `mediad` přes `netd`, FFmpeg má `--disable-network` |
| 3 | cesta videa | **A** pool bufferů alokuje aplikace, `ffdec` do něj dekóduje, aplikace commituje do okna |
| 4 | zvukový kruh | **A** proud v `audiod` otevírá aplikace a kruh předá přes `mediad` |
| 5 | patentové kodeky | **B** H.264, HEVC, AAC zapnuté ve výchozím obrazu (jen nativní LGPL dekodéry FFmpeg); MP3, AC-3, MPEG-2 vždy; přepínač xtasku pro obraz bez nich; riziko patentových poolů výslovně v docs a `THIRD-PARTY.md` |
| 6 | linkování | **A** sdílené `.so` nad `libc.so` |
| 7 | nasm | **A** povinný na x86_64 |
| 8 | hostitel | **A** Git Bash / MSYS2, kontrola nástrojů v xtasku |
| 9 | titulky | **B** libass + HarfBuzz + FriBidi + FreeType (FTL) hned, v procesu `ffdec` |
| 10 | pořadí | **A** F1 -> F2 -> F3 -> F4; části bez portů C (protokol, `mediad`, přehrávač s testovacím dekodérem) smí začít dřív |
