# SDL

Port knihovny **SDL 3** (Simple DirectMedia Layer, licence zlib) pro
programy v C: okno, kreslení, klávesnice, myš, vlákna a časovače. SDL je
krok 6 pořadí převzetí software; staví na [knihovně C](cstd.md) a na
[mechanismu portů](porty-c.md). Hry a přehrávače v C (a později FFmpeg
`ffplay`) tak běží v okně kompozitoru beze změn svého kódu.

## Stav

| Část | Co | Stav |
|---|---|---|
| sestavení | SDL 3.4.20 (připnutá verze), CMake + Ninja, statická `libSDL3.a` v předponě portů | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `sdl`) |
| `libs/mosdl` | rozhraní C `mosdl_*` nad oknem z `libs/ui`, součást `libc.a` | **[ověřeno v QEMU]** x86_64 i aarch64; překlad událostí host testy |
| ovladač videa `magnumopus` | jedno okno, softwarový framebuffer, klávesnice (kódy evdev), text, myš, kolečko, fokus, změna velikosti, čekání na události s probuzením | **[ověřeno v QEMU]** `testsprite` (pixely), `checkkeys` (umělý vstup) |
| zvuk, schránka, kurzor, relativní myš, HiDPI, celá obrazovka, YUV bez kopie | etapa 6-2 | **[navrženo]** |
| ukázkové hry SDL (`snake`, `woodeneye-008`), test se snímkem obrazovky | etapa 6-3 | **[navrženo]** |
| gamepady | samostatný krok s vlastním návrhem (rozhodnutí Q6 B) | **[navrženo]** |

## Jak to funguje

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Program SDL v okně kompozitoru">
<defs><marker id="sd1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="240" height="60" rx="6"/><text class="t b" x="130" y="34" text-anchor="middle">program v C</text><text class="m" x="130" y="52" text-anchor="middle">testsprite, hra, přehrávač</text>
<rect class="acc" x="10" y="100" width="240" height="60" rx="6"/><text class="t b" x="130" y="124" text-anchor="middle">libSDL3.a</text><text class="m" x="130" y="142" text-anchor="middle">ovladač videa magnumopus</text>
<rect class="acc" x="300" y="100" width="220" height="60" rx="6"/><text class="t b" x="410" y="124" text-anchor="middle">mosdl (v libc.a)</text><text class="m" x="410" y="142" text-anchor="middle">mosdl_* nad ui::client::Window</text>
<rect class="ok" x="570" y="100" width="200" height="60" rx="6"/><text class="t b" x="670" y="124" text-anchor="middle">compd</text><text class="m" x="670" y="142" text-anchor="middle">okno, buffery, vstup</text>
<rect class="box" x="300" y="190" width="220" height="50" rx="6"/><text class="t b" x="410" y="212" text-anchor="middle">osrt</text><text class="m" x="410" y="228" text-anchor="middle">handly okna ze startu</text>
<path class="ln" d="M130 70 L130 96" marker-end="url(#sd1)"/>
<path class="ln" d="M250 130 L296 130" marker-end="url(#sd1)"/>
<path class="ln" d="M520 130 L566 130" marker-end="url(#sd1)"/>
<path class="ln" d="M410 186 L410 164" marker-end="url(#sd1)"/>
</svg>
<figcaption>Program volá SDL; ovladač magnumopus volá mosdl v libc.a, který mluví s kompozitorem okenním protokolem. Kanál okna dostane proces při startu.</figcaption>
</figure>

- **Ovladač videa** (`third_party/c-ports/sdl3/overlay/src/video/magnumopus/`)
  je náš kód v C uvnitř SDL. Volá jen funkce `mosdl_*` z hlavičky `mosdl.h`
  v sysrootu C. Záplata `0001-video-magnumopus.patch` ho v SDL zaregistruje
  a přidá makro `SDL_PLATFORM_MAGNUMOPUS`.
- **`libs/mosdl`** je v Rustu a je součástí `libc.a`. Dvě statické knihovny
  v Rustu v jednom programu by měly každá vlastní kopii std (alokátor, TLS,
  vlákna). Program bez SDL z `mosdl` nic nepřilinkuje (`--gc-sections`).
- **Okno**: `mosdl` drží `ui::client::Window` (stejná vrstva jako okno
  terminálu), takže okno SDL umí totéž co ostatní okna: dvojité buffery,
  `CONFIGURE` při změně velikosti, obnovu po pádu kompozitoru (SDL dostane
  `SDL_EVENT_WINDOW_EXPOSED` a překreslí).
- **Spuštění**: aplikace s oknem dostane ve startovní zprávě netagované
  handly 0 = kanál okna od kompozitoru, 1 = lístek k obnově. Runtime `osrt`
  je drží pro `mosdl` (`take_handles`), ostatní handly (stdout, FS) jsou
  tagované jako u každé aplikace se std.

### Rozhraní `mosdl.h`

| Funkce | Co |
|---|---|
| `mosdl_available` | má proces kanál okna? (jinak SDL zvolí jiný ovladač) |
| `mosdl_window_open`, `_close`, `_set_title`, `_set_size` | okno (jedno na proces) |
| `mosdl_window_present` | snímek B G R A (`XRGB8888`) do bufferu okna |
| `mosdl_next_event` | další událost s časovým limitem |
| `mosdl_wake` | probudí čekání z jiného vlákna (`SDL_WaitEvent`) |
| `mosdl_display_info` | obrazovka, pracovní plocha, škála, obnovovací frekvence |

Funkce vracejí 0 nebo záporný kód chyby jádra ([chyby](chyby.md)). Rozložení
`mosdl_event` a konstanty hlídá host test proti hlavičce.

### Vstup

| Událost okna | SDL |
|---|---|
| klávesa (kód evdev) | `SDL_EVENT_KEY_DOWN`/`UP`, scancode tabulkou Linuxu (`SDL_GetScancodeFromTable`) |
| Caps Lock, Num Lock | podle stavu systému (`SDL_ToggleModState`) |
| text | `SDL_EVENT_TEXT_INPUT` (jen při zapnutém textovém vstupu, jako jinde) |
| pohyb, tlačítko, kolečko | `SDL_EVENT_MOUSE_*` (tlačítka evdev `BTN_LEFT` ... `BTN_EXTRA` = levé ... X2) |
| fokus, opuštění okna | fokus klávesnice a myši SDL |
| `CONFIGURE`, zavření | `SDL_EVENT_WINDOW_RESIZED`, `SDL_EVENT_WINDOW_CLOSE_REQUESTED` |

Čas události (ns od startu systému) se převede na čas SDL.

## Omezení etapy 6-1

- **Jedno okno na proces.** Druhé okno vrátí chybu SDL „Unsupported“.
  Více oken přes jeden kanál přijde s okenním protokolem pro víc oken
  aplikace.
- **Snímek se kopíruje celý** při `SDL_UpdateWindowSurface` /
  `SDL_RenderPresent` (buffery okna jsou dvojité, výřez by nechal starší
  obsah). 1920 × 1080 = 8 MiB na snímek; video bez kopie (`BUFFER_CREATE_DESC`)
  je etapa 6-2.
- **Softwarový renderer.** Vulkan pro SDL (render i GPU API) až s ovladači
  GPU pro aplikace ([render](render.md)).
- **Bez zvuku a schránky** (ovladač zvuku SDL `dummy`), kurzoru, relativní
  myši, HiDPI a celé obrazovky: etapa 6-2.
- **Bez gamepadů** (rozhodnutí Q6 B), dialogů souborů (Q7 A: SDL vrátí
  nepodporováno), lišty (`SDL_TRAY`) a `SDL_OpenURL` (bude přes službu
  „otevři cestu“).
- **Priorita vláken** jen `SCHED_OTHER` (plánovač zatím aplikacím reálný čas
  nedává): `SDL_SetCurrentThreadPriority` s časově kritickou prioritou
  vrátí chybu, SDL pokračuje.
- **Statická knihovna** (rozhodnutí Q8 A). `libSDL3.so` až s `libc.so`.

## Sestavení

Port `sdl3` v `xtask/src/cports/mod.rs` (způsob `CMake`, [Porty v C](porty-c.md)).
Hlavní volby: `SDL_STATIC=ON`, `SDL_SHARED=OFF`, `SDL_UNIX_CONSOLE_BUILD=ON`
(okna dává náš ovladač, ne X11 ani Wayland), vypnuté X11, Wayland, KMSDRM,
OpenGL, Vulkan, GPU, ALSA, PulseAudio, PipeWire, D-Bus, HIDAPI, joysticky,
kamera, dialogy, lišta. Knihovna a hlavičky jdou do
`target/c-ports/<arch>/prefix`, testy SDL `testsprite` a `checkkeys` do
initfs i s manifesty a ikonou.

Co SDL potřebovalo v knihovně C a doplnilo se v tomto kroku
([cstd](cstd.md#sdl-krok-6)): `environ`, `fenv.h`, `nl_langinfo`, `sched_param`
a `pthread_*schedparam`, `SIGVTALRM`/`SIGPROF`, hlavičky `wchar.h` (zatím
jen typy) a `inttypes.h`, prázdné `libm.a`, `libpthread.a`, `librt.a`.

## Testy

| Test | Kde | Co ověřuje |
|---|---|---|
| `cargo test -p mosdl` | hostitel | překlad událostí, kopie snímku (oříznutí, doplnění, nepřátelské rozměry), shoda s `mosdl.h` |
| `sdl: testsprite ...` | QEMU, sada `sdl` | okno programu SDL v kompozitoru; deterministický snímek (`--iterations 0`): červený bod v rohu, zelená horní, modrá levá a červená dolní čára na obrazovce |
| `sdl: checkkeys ...` | QEMU, sada `sdl` | umělý vstup přes inputd a kompozitor: klávesa A = scancode 4, text „a“, pravé tlačítko myši, dvakrát Esc ukončí program kódem 0 |

```text
cargo xtask test --only sdl
cargo xtask test --arch aarch64 --only sdl
```

## Rozhodnutí (2026-11-14)

| Otázka | Rozhodnutí |
|---|---|
| verze | SDL 3.4.x, připnutá verze (3.4.20) |
| sestavení | obecný mechanismus portů v xtasku (archiv, SHA-256, záplaty, CMake/Ninja i configure) |
| rozhraní | `libs/mosdl` v Rustu s rozhraním C nad `libs/ui`, `audioproto`, `clipproto` |
| kreslení | softwarový renderer (6-1), YUV bez kopie přes `BUFFER_CREATE_DESC` (6-2), Vulkan později |
| kurzor a myš | `SET_CURSOR` a `POINTER_LOCK` v okenním protokolu (6-2) |
| gamepady | samostatný krok |
| dialogy souborů | nepodporované |
| knihovna | statická `libSDL3.a` |
| důkaz | ukázky SDL `snake`, `woodeneye-008` a testy `testsprite`, `checkkeys`, `testaudio` |
| záznam zvuku | `[audio] capture = true` v manifestu a dotaz při prvním použití (6-2) |
