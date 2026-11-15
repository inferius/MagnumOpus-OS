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
| F1 | FFmpeg 9.0.2 a dav1d 1.5.4 jako porty C (LGPL, sdílené knihovny nad `libc.so`, dav1d v `libavcodec.so`), ověření licence v `config.h`, přepínač bez patentových kodeků | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `media ffmpeg`) |
| F2a | protokol `libs/mediaproto` (host testy), služba `mediad`, testovací dekodér `mediatest` (obraz NV12 do poolu s plotem, tón do kruhu `audiod`), zdroj VMO i kanálem, `SEEK`, pád a obnova dekodéru | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `media`) |
| F2b | proces `ffdec` (C, PIE nad `libc.so` a `libav*.so`): AV1 a VP9 bit po bitu podle MD5 testovacích vektorů, zvuk WAV kanálem zdroje do `audiod`, skok, poškozený vstup | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `media ffmpeg`) |
| F3 | video do sdílených bufferů (`bufdesc` NV12/I420/P010) s ploty, A/V synchronizace | **[navrženo]** |
| F4a | plocha videa v `libs/ui` (`node::video`, vrstva z bufferů s ploty), aplikace Přehrávač s vestavěnou ukázkou: přehrát/pozastavit, posuvník (skok), čas, A/V synchronizace podle zvuku | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `gui`, test přehrávače) |
| F4b | výběr souboru, titulky libass, hlasitost a výběr stop, celá obrazovka, mediální relace systému | **[navrženo]** |
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
na `READ_AT { posun u64, délka u32 }` (nejvýš 60 KiB, aby se odpověď vešla do zprávy kanálu 64 KiB) a `SIZE`. Dekodér
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
použít. Software dekodéry FFmpeg píší do vlastních rámců (I420, 10bitové
formáty); `ffdec` je zatím **kopíruje** do bufferu NV12 z poolu přes
libswscale (u stejného rozměru bezztrátové proložení chroma, ověřené MD5).
Dekódování **přímo do VMO z poolu** (`get_buffer2` s poolem I420 a
zarovnáním 64 B, bez kopie) je další krok spolu s F3.

Pozor na stav závislostí:

- **Ploty v jádře (6a-1) jsou na `main`** ([ploty.md](ploty.md)).
- **`bufdesc`, `BUFFER_CREATE_DESC` a body synchronizace oken (6a-3, 6a-4)
  jsou na `main`**; compd převádí YUV na CPU (`libs/yuv`) nebo ve
  `vkcomp`. `libs/ui` má plochu videa (`node::video`, F4a).
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

### Přehrávač (aplikace)

**Plocha videa v `libs/ui`** [ověřeno v QEMU]: uzel `video(id, w, h, alt)`
v rozvržení jen rezervuje místo (přirozená velikost = rozměr videa, menší
prostor ho zmenší se zachováním poměru stran, v přístupnosti role obrázku).
Obraz ukazuje klient (`ui::client::run_video` s `VideoSurface`: popisy
`bufdesc`, VMO, plot získání a uvolnění) ve vlastní vrstvě nad oknem
(`900 + id`): `LAYER_SET` s bufferem a obdélníkem, `SYNC_POINTS` a commit,
**bez překreslení okna**. Dokud `App::wants_frames`, klient tiká se snímky
kompozitoru (`FRAME` s termínem výměny) a ptá se `App::video_frame`, co
ukázat; `App::video_shown` vrátí hodnotu plotu uvolnění snímku. Po pádu
kompozitoru klient povrch vytvoří znovu při dalším snímku.

**Přehrávač** (`user/gui-player`) [ověřeno v QEMU]: zatím přehrává
vestavěnou ukázku `MTM1` (10 s, 640x360, tón) přes `mediad`
a `mediatest`. Pool 4 bufferů NV12, proud `audiod` (kruh předá
dekodéru); hodiny podle zvuku (`AUDIO_BASE` + přehrané rámce a latence
kruhu), bez zvukové karty monotónní čas. Na každý snímek kompozitoru
vybere `clock::decide` snímek pro čas výměny, pozdní zahodí, buffer vrátí
dekodéru až po plotu uvolnění. Ovládání: přehrát/pozastavit, posuvník
(skok na klíčový snímek), čas. Pád dekodéru obnoví (`REOPEN` + `SEEK`).
Test sady `gui`: prázdná plocha před přehráním, obraz a jeho pohyb po
„Přehrát“, postup času, zastavený obraz po pauze.

Zbývá (F4b) [navrženo]:

- výběr souboru (zdroj kanálem z aplikace, dokud fsd nemá úzký handle
  souboru), posuvník s náhledy klíčových snímků, hlasitost, výběr stop,
  celá obrazovka, klávesové zkratky a mediální klávesy,
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

Pro sestavení bez nich je **přepínač xtasku**
`XTASK_BEZ_PATENTOVYCH_KODEKU=1` (proměnná prostředí `cargo xtask build`
/ `test`): configure dostane `--disable-decoder=` pro h264, hevc, aac,
aac_fixed, aac_latm, eac3, vvc a `--disable-parser=` pro odpovídající
parsery, `xtask` po sestavení ověří v `config_components.h`, že jsou
vypnuté (jinak sestavení selže). `ffdec` je pak v `INFO` hlásí s příznakem
`available = 0` (aplikace dostane popis proudu, ne pád).

## Titulky

> **Zatím se nezobrazují.** HarfBuzz je napsaný v C++ a potřebuje
> standardní knihovnu C++ (hlavičky libc++); libass od verze 0.17 má
> HarfBuzz povinný. Běhové prostředí C++ (krok 7a: libc++, libunwind)
> v systému ještě není, proto se porty libass, HarfBuzz, FriBidi
> a FreeType staví až po něm (rozhodnutí koordinátora 2026-11-15).
> Do té doby přehrávač proudy titulků jen vypíše v `INFO` a nevybírá je.

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

## Sestavení FFmpeg [ověřeno v QEMU]

FFmpeg a dav1d staví obecný mechanismus portů C v xtasku
([Porty v C](porty-c.md)) - tarball z oficiálního místa s SHA-256, žádné
záplaty zdrojů:

- **dav1d 1.5.4** způsobem `Meson`: statická knihovna s `-fPIC`
  (`b_staticpic`), x86_64 s ručním SIMD v nasm, aarch64 NEON přes clang.
  FFmpeg ji najde přes `dav1d.pc` v předponě portů a vloží do
  `libavcodec.so`.
- **FFmpeg 9.0.2** způsobem `Configure` se sdílenými knihovnami:
  `--enable-cross-compile --target-os=none --arch=<arch> --cc=clang`
  (nástroje LLVM jménem), příznaky dynamického sysrootu C (`-fPIC`, link
  `-Wl,-Bdynamic`, `-Wl,-shared` pro knihovny, `crt1.o` a zavaděč jen pro
  zkušební programy configure), `--pkg-config=pkgconf`. Záplata
  `target-os` ani detekce CPU potřeba nebyla: `none` je obecný POSIX cíl
  a počet CPU dává `sysconf` z cstd.
- **Jen dekódování**: `--disable-encoders --disable-muxers
  --disable-filters --disable-avfilter --disable-avdevice
  --disable-devices --disable-hwaccels --disable-protocols
  --disable-network --disable-autodetect --disable-programs`; demuxery,
  parsery, dekodéry a bsf zůstávají (seznam `FFMPEG_ARGS` v
  `xtask/src/cports/mod.rs`).
- **Licence natvrdo**: `--disable-gpl --disable-version3
  --disable-nonfree`; xtask po sestavení ověří v `config.h`
  `CONFIG_GPL 0`, `CONFIG_NONFREE 0`, `CONFIG_VERSION3 0`
  a `CONFIG_SHARED 1`, jinak sestavení selže (host test
  `ffmpeg_config_check`).
- **Výstup**: `lib/libavutil.so.61`, `lib/libswresample.so.7`,
  `lib/libswscale.so.10`, `lib/libavcodec.so.63` (s dav1d),
  `lib/libavformat.so.63` v initfs (bez TEXTREL, BIND_NOW, verze symbolů
  `LIBAVCODEC_63` ...). Přidávají zhruba 20 MiB na architekturu, proto má
  obraz disku nově 256 MiB (`xtask/src/image.rs`).
- **Nástroje**: `sh`, `make`, nasm, Meson, Ninja a pkgconf z MSYS2
  (rozhodnutí 8 = A); chybějící = chyba s radou k instalaci
  ([Sestavení](sestaveni.md#nástroje)).

Na čem to stojí v systému:

- `libc.so` a zavaděč `ld.so` (knihovny z `/system/lib` přes libd),
- `cstd`: pthreads (FFmpeg dekóduje ve vláknech), `clock_gettime`,
  `sysconf`, nově `posix_memalign` / `aligned_alloc` / `memalign`
  (zarovnání pro SIMD) a makro `static_assert` (C11),
- `magnumopus/sys.h` (`libs/mosys` v `libc.so`): kanály, VMO, ploty
  a čekání pro `ffdec` v C,
- W^X: FFmpeg ani dav1d nepotřebují JIT,
- AVX/XSAVE: FFmpeg detekuje AVX2/AVX-512 za běhu ([cstd.md](cstd.md)).

### Proces `ffdec` [ověřeno v QEMU]

`user/ffdec` je program v C (PIE nad `libc.so` a `libav*.so`), který
mluví protokolem `mediaproto` přes bootstrap kanál (kódování zpráv
v `proto.c`, shoda s Rustem host testem `c_header_tests`). `mediad` ho
spouští jednou na relaci ve vlastním jobu (256 MiB uživatelské paměti)
**bez jakýchkoli handlů** kromě řídicího kanálu; dekodér volí podle
prvních bajtů zdroje (`MTM1` = `mediatest`, jinak `ffdec`).

- **Vstup**: libavformat s vlastním `AVIOContext` nad kanálem zdroje
  (`READ_AT` po 60 KiB, `SIZE`); žádné soubory, síť ani protokoly FFmpeg.
- **Video**: libavcodec (vlákna podle počtu CPU) -> libswscale do NV12
  bufferu z poolu -> plot snímku -> `FRAME`.
- **Zvuk**: libswresample na s16 stereo ve frekvenci kruhu -> přímo do
  kruhu `audiod`; `AUDIO_BASE` s pozicí a pts prvního vzorku po startu
  a skoku.
- **Tempo**: dekóduje dopředu, dokud má volný buffer a místo v kruhu.
- **Skok**: `avformat_seek_file` dozadu na klíčový snímek; přesný skok
  zahodí snímky před cílem.

Sada `media ffmpeg` (`user/init/src/tests/media_ffmpeg.rs`): popis proudů
AV1 a VP9, **obraz bit po bitu** (I420 složené zpět z NV12 proti MD5
každého snímku z testovacích vektorů AOM a libvpx), skok na začátek po
konci, zvuk WAV kanálem zdroje do `audiod`, náhodné a useknuté vstupy
(`mediad` žije).

## Povinnosti LGPL

LGPL 2.1+ dovoluje FFmpeg použít v uzavřeném systému, pokud uživatel může
knihovnu nahradit vlastní upravenou verzí:

- FFmpeg je **dynamicky linkovaný** (`.so` nad `libc.so`), proces `ffdec`
  je jen tenká vrstva; nahrazení knihovny = nahrazení souborů
  v `/system/lib` (rozhodnutí 6),
- **zdroje**: připnutý tarball (SHA-256, podpis klíčem FFmpeg ověřen při
  připnutí), přepínače configure v `xtask/src/cports/mod.rs`
  (`FFMPEG_ARGS`), záplaty žádné,
- **skript pro opětovné sestavení**: `cargo xtask build` sestaví stejné
  knihovny z týchž zdrojů (popis v [Porty v C](porty-c.md)),
- `THIRD-PARTY.md` má řádky FFmpeg (LGPL-2.1-or-later) a dav1d
  (BSD-2-Clause) s verzí a otiskem; libass (ISC), HarfBuzz (MIT), FriBidi
  (LGPL-2.1-or-later) a FreeType (FTL) přibudou s titulky,
- FriBidi bude pod LGPL stejně jako FFmpeg: sdílená `.so`, zdroje
  a skript pro opětovné sestavení stejně,
- `About` přehrávače uvede, že používá FFmpeg pod LGPL, s odkazem na zdroje
  (s výběrem souboru v F4b).

## Etapy

1. **F1 port** (hotovo): dav1d + FFmpeg přes mechanismus portů; test
   v QEMU dekóduje testovací vektory AV1 a VP9 přes `ffdec` bit po bitu
   podle MD5, obě architektury. FreeType, FriBidi, HarfBuzz a libass až po
   kroku 7a (běhové prostředí C++).
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
