# Multimédia: FFmpeg a přehrávač

Návrh kroku 7 pořadí v [architektura.md](architektura.md): dekódování zvuku
a videa převzatou knihovnou FFmpeg v **izolovaném procesu** a nad ním
přehrávač. Celá stránka je **[navrženo]**, kód zatím neexistuje. Otevřené
otázky jsou v kapitole [Otázky k rozhodnutí](#otázky-k-rozhodnutí); dokud
na ně nepadne odpověď, nic se neimplementuje.

## Stav

| Část | Co | Stav |
|---|---|---|
| F1 | sestavení FFmpeg jako port C (LGPL, sdílené knihovny nad `libc.so`), test `ffprobe`-like programu v QEMU | **[navrženo]** |
| F2 | služba `mediad` + proces dekodéru `ffdec`, protokol `mediaproto`, zvuk do `audiod` | **[navrženo]** |
| F3 | video do sdílených bufferů (`bufdesc` NV12/I420/P010) s ploty, A/V synchronizace | **[navrženo]** |
| F4 | aplikace Přehrávač nad `libs/ui` (videovrstva, ovládání, titulky, mediální relace) | **[navrženo]** |
| F5 | hardwarová akcelerace dekódování (VA-API/Vulkan Video přes ovladač GPU) | **[navrženo]** |

## Proč izolovaný proces

Demuxery a dekodéry zpracovávají nedůvěryhodná data ze sítě a ze souborů;
FFmpeg má každý rok desítky oprav paměťových chyb. Proto:

- dekodér běží v **samostatném procesu bez práv**: žádný přístup k FS, síti,
  launchd ani k ovladačům; dostane jen kanál ke `mediad` a sdílené buffery,
- jeden proces dekodéru **na jednu relaci přehrávání** (otázka 1), pád nebo
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
  (otázka 2), alokuje nebo dostane pool bufferů (otázka 3), řídí přehrávání
  a skládá video do svého okna.
- **mediad** (služba, důvěryhodná) drží relace, čte vstup za aplikaci
  (z handlu souboru nebo proudu, ne podle cesty), spouští `ffdec` přes
  launchd s prázdným jmenným prostorem a předává mu jen bajty. Rozhoduje,
  které kodeky jsou povolené (otázka 5).
- **ffdec** je jediný proces, ve kterém běží kód FFmpeg: libavformat
  (demux), libavcodec (dekódování), libswresample (převod formátu vzorků),
  podle potřeby libswscale. Nemá žádné capability kromě kanálu k `mediad`,
  VMO bufferů a plotů.

### Protokol `mediaproto` [navrženo]

Zprávy přes kanál aplikace <-> `mediad`; `mediad` <-> `ffdec` používá
stejný protokol rozšířený o dodávání bajtů. Čísla operací a rozložení budou
v crate `libs/mediaproto` s host testy (jako `audioproto`).

| Operace | Kdo | Význam |
|---|---|---|
| `OPEN { zdroj, příznaky }` | aplikace | nová relace; zdroj = handle souboru / proudu (otázka 2) |
| `INFO -> { délka, proudy[] }` | aplikace | proudy: typ, kodek, rozměr, fps, kanály, frekvence, jazyk |
| `SELECT { video, audio, titulky }` | aplikace | výběr proudů (-1 = žádný) |
| `POOL { buffery[], ploty }` | aplikace | předá pool výstupních bufferů (VMO + `bufdesc`) |
| `AUDIO { kruh }` | aplikace | předá kruh proudu `audiod` (otázka 4) |
| `PLAY` / `PAUSE` / `SEEK { čas, přesně }` | aplikace | řízení; `SEEK` vrátí skutečný čas klíčového snímku |
| `FRAME { buffer, pts, trvání, plot }` | `mediad` -> aplikace | snímek je v bufferu, až plot dosáhne hodnoty |
| `RELEASE { buffer, plot }` | aplikace | buffer je volný, až compd signalizuje release plot |
| `CLOCK { pts zvuku, čas monotonní }` | `mediad` -> aplikace | hodiny pro A/V synchronizaci |
| `EVENT { konec, chyba, změna formátu }` | `mediad` -> aplikace | včetně pádu dekodéru (relace jde obnovit) |

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
  zatím nejsou na `main`**, jen ve větvích `inferius-dev/6a-3-render`
  a `inferius-dev/6a-4-okna-video`. F3 na ně čeká; F1 a F2 (zvuk) ne.
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
- titulky: vlastní vykreslování SRT/WebVTT přes `libs/font` (otázka 9),
- **mediální relace** pro systém (právě hraje, ovládání z panelu, pozastavení
  při odpojení sluchátek) podle [rozhrani-zarizeni.md](rozhrani-zarizeni.md).

## Kodeky

Pořadí podle licenčního rizika. Všechny dekodéry jsou softwarové v rámci
FFmpeg nebo jeho externích knihoven pod kompatibilními licencemi.

| Skupina | Kodeky / kontejnery | Zdroj | Výchozí stav |
|---|---|---|---|
| bez poplatků | AV1 (dav1d, BSD-2), VP8/VP9 (vestavěné ve FFmpeg), Opus, Vorbis, FLAC, Theora; WebM/Matroska, Ogg, MP4/ISOBMFF (kontejner), WAV | FFmpeg + dav1d | zapnuté |
| patenty vypršely | MP3, AC-3, MPEG-1/2 video | FFmpeg | zapnuté (otázka 5) |
| patentově zatížené | H.264, HEVC, AAC, E-AC-3 | FFmpeg | **rozhodne uživatel** (otázka 5) |
| nikdy | cokoli vyžadující `--enable-gpl`, `--enable-nonfree` (x264, x265, fdk-aac, libpostproc s GPL) | - | vypnuté natvrdo |

Dav1d je rychlejší než vestavěný dekodér AV1 ve FFmpeg a je BSD, proto ho
FFmpeg použije jako externí knihovnu (vlastní port C stejným mechanismem).
Kodéry (encode) nejsou v F1-F4 potřeba; nahrávání obrazovky je samostatný
pozdější krok.

**Upozornění:** patentové licence (H.264, HEVC, AAC) jsou právní otázka,
ne technická. FFmpeg je dekódovat umí bez GPL; distribuce binárky s nimi
v některých zemích může vyžadovat licenci (MPEG LA / Access Advance /
Via LA). Návrh je proto dělá **volitelnou součástí** sestavenou zvlášť
(`ffdec-ext`), kterou lze vynechat z obrazu.

## Hardwarová akcelerace (později, F5)

Dekódování na GPU (Intel Quick Sync, AMD VCN, NVIDIA NVDEC) přijde až po
ovladačích GPU ([gpu-amd-nvidia.md](gpu-amd-nvidia.md)). Tvar rozhraní už
teď: `ffdec` dostane od služby GPU **frontu video dekódování** (ne zařízení),
výstup jsou tytéž buffery `bufdesc` s ploty, takže přehrávač ani compd se
nemění. Aplikace capability k ovladači nikdy nedostane.

## Sestavení FFmpeg

### Port C ve xtasku

Obecný mechanismus portů C (tarball + SHA-256 + záplaty + CMake/Ninja/
autotools) navrhuje vlákno SDL (`navrh-sdl.md`, otázka 2 = C). FFmpeg ho
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
   ho najde a při chybějícím dá radu k instalaci (otázka 7). Na aarch64
   stačí clang jako assembler (GNU syntaxe `.S`, NEON), nic navíc.
4. **Záplaty**: `target-os` pro náš systém, detekce počtu CPU přes
   `sysconf(_SC_NPROCESSORS_ONLN)` (ve větvi cstd-posix), přepínače
   `--disable-network`, `--disable-autodetect`; FFmpeg nesmí sáhnout po
   `fork` (cstd ho nemá, jen `posix_spawn`).
5. **Out-of-tree a per-arch**: build do `target/cports/<arch>/ffmpeg/`,
   instalace hlaviček, `.so` a `.pc` do C sysrootu, otisk (verze + záplaty
   + přepínače) pro přeskočení opakovaného buildu.
6. **Závislosti mezi porty**: dav1d (Meson + Ninja!) před FFmpeg. Meson je
   další systém sestavení, který SDL mechanismus nepokrývá (otázka 8).

Licenční přepínače natvrdo: `--disable-gpl --disable-nonfree
--disable-version3` (LGPL 2.1+), `--enable-shared --disable-static`,
`--disable-programs` (vlastní `ffdec` místo `ffmpeg`), `--disable-doc`.
`xtask` po configure ověří v `config.h`, že `CONFIG_GPL 0` a
`CONFIG_NONFREE 0`, jinak build selže.

### Na čem stojí v systému

- `libc.so` (krok libc.so ve větvi `inferius-dev/libc-so`, čeká na
  sloučení) a dynamický zavaděč `ld.so`: FFmpeg je sada `.so`
  (`libavformat.so.61`, `libavcodec.so.61`, `libavutil.so.59`,
  `libswresample.so.5`, `libswscale.so.8`) linkovaná proti `libc.so`.
- `cstd` s POSIX vrstvou (větev `inferius-dev/cstd-posix`): pthreads
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
  (otázka 6),
- **zdroje**: přesný tarball (SHA-256), všechny záplaty a přepínače
  configure jsou v repozitáři (`third_party/ffmpeg/`); obraz systému nese
  soubor s odkazem na ně a text licence,
- **skript pro opětovné sestavení**: `cargo xtask cport ffmpeg` sestaví
  stejnou knihovnu z týchž zdrojů; popis v uživatelské příručce,
- `THIRD-PARTY.md` doplní řádek FFmpeg (LGPL-2.1-or-later) a dav1d
  (BSD-2-Clause) s verzí a hashem,
- `About` přehrávače uvede, že používá FFmpeg pod LGPL, s odkazem na zdroje.

## Etapy

1. **F1 port**: dav1d + FFmpeg přes mechanismus portů, test v QEMU:
   program nad `libavformat` vypíše proudy testovacího souboru WebM
   (Opus + VP9) a dekóduje 10 snímků s kontrolním součtem, obě architektury.
2. **F2 služba a zvuk**: `mediaproto`, `mediad`, `ffdec`; přehrání Opus/FLAC
   do `audiod`; test pádu dekodéru (relace hlásí chybu, `mediad` žije).
3. **F3 video**: buffery `bufdesc` s ploty (po sloučení 6a-3/6a-4),
   A/V synchronizace, měření latence a zahozených snímků.
4. **F4 přehrávač**: aplikace, widget videovrstvy v `libs/ui`, titulky,
   mediální relace.
5. **F5 HW akcelerace**: po ovladačích GPU.

## Otázky k rozhodnutí

Každá otázka má doporučení **A**.

1. **Izolace dekodéru.**
   A) jeden proces `ffdec` na relaci (silná izolace, ~několik MB paměti
   navíc na relaci); B) jeden sdílený proces pro všechny relace jednoho
   uživatele; C) dekodér v procesu aplikace (bez izolace).
2. **Jak se dostane vstup k dekodéru.**
   A) aplikace předá `mediad` handle souboru / proudu, `mediad` čte
   a posílá bajty do `ffdec` (dekodér nikdy nedostane handle);
   B) `ffdec` dostane handle souboru jen pro čtení přímo;
   C) aplikace posílá bajty sama. Síťové proudy (HTTP, HLS/DASH) ve všech
   variantách čte `mediad` přes `netd`, ne FFmpeg (`--disable-network`).
3. **Cesta videa.**
   A) pool bufferů alokuje aplikace, `ffdec` do něj dekóduje, aplikace
   commituje do svého okna (aplikace řídí zobrazení, titulky nad videem);
   B) aplikace deleguje vrstvu okna přímo na `mediad`, který commituje
   do compd sám (méně skoků, ale `mediad` potřebuje přístup k oknu);
   C) `ffdec` vrací RGB přes CPU obraz (jednoduché, pomalé).
4. **Zvukový kruh.**
   A) aplikace otevře proud v `audiod` (hlasitost a mediální relace patří
   aplikaci) a kruh předá přes `mediad` do `ffdec`;
   B) proud otevírá `mediad` jménem aplikace.
5. **Patentové kodeky (H.264, HEVC, AAC).**
   A) volitelná součást `ffdec-ext`, ve výchozím obrazu vypnutá, zapnutí
   jedním přepínačem xtasku (rozhodnutí o distribuci je na uživateli);
   B) zapnuté ve výchozím obrazu; C) vůbec nesestavovat. MP3, AC-3
   a MPEG-2 (patenty vypršely) zapnout vždy?
6. **Linkování FFmpeg.**
   A) sdílené `.so` nad `libc.so` (nahraditelnost podle LGPL zadarmo);
   B) statické `.a` + sada pro opětovné slinkování (objekty `ffdec`
   v obrazu/SDK); C) obojí.
7. **nasm na x86_64.**
   A) povinný (bez něj build selže s radou, jak nainstalovat; bez SIMD
   je dekódování 2-5x pomalejší); B) volitelný s `--disable-x86asm`
   a varováním.
8. **Hostitel pro configure/make/Meson.**
   A) Git Bash / MSYS2 na Windows (sh, make; Meson a Ninja přes pip/MSYS2),
   s kontrolou nástrojů v xtasku před buildem; B) WSL (`tools/wsl`);
   C) dav1d sestavit bez Mesonu vlastním skriptem v xtasku (žádný Meson).
9. **Titulky.**
   A) vlastní parser SRT/WebVTT a vykreslení přes `libs/font` (ASS jen
   jako prostý text); B) převzít libass (ISC) + FriBidi/HarfBuzz pro plné
   ASS.
10. **Pořadí etap.**
    A) F1 -> F2 (zvuk) -> F3 -> F4, F3 až po sloučení 6a-3/6a-4;
    B) nejdřív jednoduchý přehrávač s CPU obrazem (C ve 3), pak buffery.
