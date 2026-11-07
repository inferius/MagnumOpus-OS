# Vstup: klávesnice a myš

Vstup má tři části: **ovladač zařízení** (`virtio-input`), **službu vstupu**
(`inputd`) a **vlastníka fokusu**, který události dostává (zatím init, v 6b
terminál, v 6c kompozitor). Všechny spolu mluví zprávami po kanálech
a formát zpráv je v knihovně `inputproto`. Rozložení klávesnice je
v knihovně `keymap` a používá ho jen `inputd`, takže je pro celý systém
na jednom místě.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cesta udalosti vstupu">
<defs><marker id="in1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="160" height="56" rx="6"/><text class="t b" x="90" y="43" text-anchor="middle">virtio-keyboard</text><text class="m" x="90" y="62" text-anchor="middle">QEMU / hardware</text>
<rect class="box" x="10" y="110" width="160" height="56" rx="6"/><text class="t b" x="90" y="133" text-anchor="middle">virtio-mouse</text><text class="m" x="90" y="152" text-anchor="middle">QEMU / hardware</text>
<rect class="box" x="10" y="200" width="160" height="56" rx="6"/><text class="t b" x="90" y="223" text-anchor="middle">umělý vstup</text><text class="m" x="90" y="242" text-anchor="middle">capability inject</text>
<rect class="box" x="210" y="20" width="150" height="56" rx="6"/><text class="t b" x="285" y="43" text-anchor="middle">virtio-input</text><text class="m" x="285" y="62" text-anchor="middle">ovladač (proces)</text>
<rect class="box" x="210" y="110" width="150" height="56" rx="6"/><text class="t b" x="285" y="133" text-anchor="middle">virtio-input</text><text class="m" x="285" y="152" text-anchor="middle">ovladač (proces)</text>
<path class="ln" d="M170 48 L208 48" marker-end="url(#in1)"/>
<path class="ln" d="M170 138 L208 138" marker-end="url(#in1)"/>
<path class="ln" d="M360 48 L418 120" marker-end="url(#in1)"/>
<path class="ln" d="M360 138 L418 138" marker-end="url(#in1)"/>
<path class="ln" d="M170 228 L418 158" marker-end="url(#in1)"/>
<text class="m" x="372" y="96">IDEV / IEVT</text>
<rect class="acc" x="420" y="95" width="150" height="90" rx="6"/><text class="t b" x="495" y="120" text-anchor="middle">inputd</text><text class="m" x="495" y="139" text-anchor="middle">rozložení (keymap)</text><text class="m" x="495" y="155" text-anchor="middle">opakování kláves</text><text class="m" x="495" y="171" text-anchor="middle">bezpečná zkratka</text>
<rect class="ok" x="610" y="60" width="140" height="56" rx="6"/><text class="t b" x="680" y="83" text-anchor="middle">vlastník fokusu</text><text class="m" x="680" y="102" text-anchor="middle">init, terminál, GUI</text>
<rect class="box" x="610" y="170" width="140" height="56" rx="6"/><text class="t b" x="680" y="193" text-anchor="middle">kanál secure</text><text class="m" x="680" y="212" text-anchor="middle">systém (odemčení)</text>
<path class="ln" d="M570 125 L608 92" marker-end="url(#in1)"/>
<path class="ln" d="M570 160 L608 195" marker-end="url(#in1)"/>
<text class="m" x="580" y="76">Record</text>
<text class="m" x="560" y="246">Ctrl+Alt+Delete</text>
<text class="m" x="10" y="300">Aplikace nikdy nečte vstup globálně: události dostane jen vlastník fokusu, bezpečnou zkratku jen systém.</text>
</svg>
<figcaption>Cesta události: zařízení, ovladač, služba, vlastník fokusu.</figcaption>
</figure>

Proč takhle:

- **Aplikace nečte zařízení.** Keylogger tak nemá kde poslouchat. Služba
  pošle událost jen vlastníkovi fokusu (konzole, později kompozitor) a ten
  ji rozdělí oknům.
- **Dva proudy.** Událost `Key` nese fyzickou klávesu. Její kód nezávisí na
  rozložení, takže sedí hrám a zkratkám. Událost `Text` nese vložený text
  podle rozložení a mrtvých kláves. Terminál a textová pole berou `Text`,
  hra `Key`. Metody zadávání (čínština, emoji, diktování) budou později
  posílat také `Text`, aplikace se tím nemění.
- **Bezpečná zkratka** se zachytí ve službě dřív, než cokoli dostane
  aplikace. Aplikace ji nemůže zachytit ani podvrhnout.

## Spuštění

```text
init
 ├─ devmgr ── najde PCI 1af4:1052 (virtio-input) ── spustí virtio-input
 │            initu pošle zprávu `input` + kanál k ovladači
 └─ inputd ── init mu pošle `device` (kanál ovladače) pro každé zařízení,
              pak `focus` (svůj kanál fokusu) a `secure` (kanál zkratky)
```

Log při startu:

```text
virtio-input 00:05.0: 'QEMU Virtio Keyboard' (klavesnice)
virtio-input 00:06.0: 'QEMU Virtio Mouse' (mys)
init: 3 blokovych a 2 vstupnich zarizeni
inputd: bezim (rozlozeni us)
inputd: zarizeni 'QEMU Virtio Keyboard' pripojeno
init: sluzba vstupu bezi (2 zarizeni)
```

V QEMU (`cargo xtask run` / `test`) jsou zařízení `virtio-keyboard-pci`
a `virtio-mouse-pci`, obě s překladem DMA přes IOMMU (`iommu_platform=on`).

## Ovladač virtio-input

Ovladač (`user/virtio-input`) je stejný pro klávesnici, myš i tablet:

1. Inicializace virtio 1.x jako u disku (`libs/virtio`). Vlastnosti jsou
   jen `VERSION_1` a `ACCESS_PLATFORM`, přerušení přes MSI-X.
2. Z konfigurace zařízení přečte jméno (`ID_NAME`) a podporované události
   (`EV_BITS`). Klávesa `A` znamená klávesnici, relativní osy myš.
3. Frontu 0 (události) naplní až 64 buffery po 8 B (`Virtqueue::post`).
   Zařízení je plní a vrací v libovolném pořadí.
4. Pošle službě představení `IDEV` a pak po každém přerušení všechny
   hotové události jednou zprávou `IEVT`. Formát `virtio_input_event` je
   už evdev (typ, kód, hodnota), takže ho ovladač jen přenese.
5. Když služba nestíhá (plná fronta kanálu), ovladač zprávu zahodí
   a zařízení neblokuje. Zavře-li služba kanál, ovladač zařízení resetuje
   a skončí.

Frontu 1 (stav, kontrolky Caps Lock a Num Lock) ovladač zatím nepoužívá.

## Protokol zařízení (`inputproto::device`)

Zdroj vstupu (ovladač nebo umělý vstup) posílá službě zprávy začínající
značkou:

| Značka | Obsah (little-endian) |
|---|---|
| `IDEV` | verze u16 (1), schopnosti u16 (`KEYBOARD` 1, `POINTER` 2), jméno UTF-8 (nejvýš 128 B) |
| `IEVT` | události po 8 B: typ u16, kód u16, hodnota i32 (nejvýš 1024 v jedné zprávě) |

Události mají tvar evdev a kódy se berou z Linuxu (`inputproto::code`):

| Typ | Kód | Hodnota |
|---|---|---|
| `EV_KEY` (1) | klávesa (`KEY_A` = 30, ...) nebo tlačítko (`BTN_LEFT` = 0x110, ...) | 1 stisk, 0 uvolnění (2 = opakování od zařízení se ignoruje) |
| `EV_REL` (2) | `REL_X`, `REL_Y`, `REL_WHEEL` (kladně nahoru), `REL_HWHEEL` | posun |
| `EV_MSC` (4) | `MSC_SCAN` | surový scan kód příští klávesy (PS/2, USB HID) |
| `EV_SYN` (0) | `SYN_REPORT` | konec snímku |

Služba skládá události do **snímků**. Pohyb v jednom snímku sečte do
jedné události `Motion`, kolečko do `Scroll`. Snímek uzavře `SYN_REPORT`
i konec zprávy.

Příklad: umělý vstup napíše `A` (Shift + a):

```rust
use inputproto::code::*;
use inputproto::device::{self, RawEvent};

let ev = [
    RawEvent::key(KEY_LEFTSHIFT, true), RawEvent::syn(),
    RawEvent::key(KEY_A, true),         RawEvent::syn(),
    RawEvent::key(KEY_A, false),        RawEvent::syn(),
    RawEvent::key(KEY_LEFTSHIFT, false), RawEvent::syn(),
];
inject.write(&device::encode_events(&ev), Vec::new())?;
```

## Události pro vlastníka fokusu (`inputproto::event`)

Jedna zpráva je jeden snímek a skládá se ze záznamů po 32 B:

<figure class="diagram">
<svg viewBox="0 0 760 120" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Zaznam udalosti 32 B">
<rect class="acc" x="10" y="20" width="70" height="44" rx="4"/><text class="t" x="45" y="47" text-anchor="middle">kind</text>
<rect class="box" x="80" y="20" width="70" height="44" rx="4"/><text class="t" x="115" y="47" text-anchor="middle">flags</text>
<rect class="box" x="150" y="20" width="70" height="44" rx="4"/><text class="t" x="185" y="47" text-anchor="middle">mods</text>
<rect class="box" x="220" y="20" width="60" height="44" rx="4"/><text class="m" x="250" y="47" text-anchor="middle">0</text>
<rect class="box" x="280" y="20" width="160" height="44" rx="4"/><text class="t" x="360" y="47" text-anchor="middle">time u64 (ns)</text>
<rect class="ok" x="440" y="20" width="77" height="44" rx="4"/><text class="t" x="478" y="47" text-anchor="middle">a</text>
<rect class="ok" x="517" y="20" width="77" height="44" rx="4"/><text class="t" x="555" y="47" text-anchor="middle">b</text>
<rect class="ok" x="594" y="20" width="77" height="44" rx="4"/><text class="t" x="632" y="47" text-anchor="middle">c</text>
<rect class="ok" x="671" y="20" width="77" height="44" rx="4"/><text class="t" x="709" y="47" text-anchor="middle">d</text>
<text class="m" x="10" y="86">0      2       4       6     8                         16      20      24      28     32</text>
<text class="m" x="10" y="108">Pole a-d jsou u32 a jejich význam závisí na druhu (tabulka níže).</text>
</svg>
<figcaption>Záznam události: hlavička 16 B, data 16 B.</figcaption>
</figure>

| Druh (`kind`) | Událost | a | b | c | d |
|---|---|---|---|---|---|
| 1 `KEY` | `Key { code, scancode, action, ch }` | kód klávesy | scan kód | akce 0/1/2 | znak (0 = žádný) |
| 2 `TEXT` | `Text(char)` | znak Unicode | | | |
| 3 `MOTION` | `Motion { dx, dy }` | dx i32 | dy i32 (kladně dolů) | | |
| 4 `BUTTON` | `Button { button, action }` | kód tlačítka | | akce 0/1 | |
| 5 `SCROLL` | `Scroll { dx, dy }` | dx i32 (doprava) | dy i32 (nahoru) | | |
| 6 `SECURE_ATTENTION` | `SecureAttention` | | | | |
| 7 `FOCUS` | `Focus(bool)` | 1 získán, 0 ztracen | | | |
| 8 `INJECTION` | `Injection { active }` | počet zdrojů umělého vstupu | | | |
| 9 `LAYOUT` | `Layout(id)` | 0 `us`, 1 `cz` | | | |

- `flags`: `SYNTHETIC` (1) = událost pochází z umělého vstupu.
- `mods`: modifikátory po události. `SHIFT` 1, `CTRL` 2, `ALT` 4,
  `ALTGR` 8, `META` 16, `CAPS_LOCK` 32, `NUM_LOCK` 64.
- `time`: monotónní čas v ns od startu, kdy služba snímek přijala.
- Neznámý druh klient přeskočí (`Event::Unknown`). Nový druh události tak
  není nekompatibilní změna.

Pravidla:

- `Key.ch` je znak, který klávesa dává v aktuálním rozložení se Shiftem,
  AltGr a Caps Lockem, a **počítá se i při držení Ctrl**. Ctrl+Z se tak
  pozná v každém rozložení (v CZ je `z` na klávese `KEY_Y`).
- `Text` přijde jen pro tisknutelné znaky včetně mezery. Enter, Tab,
  Backspace a šipky jsou jen `Key`. Při drženém Ctrl, Alt nebo Meta se
  `Text` neposílá, AltGr text píše.
- Pořadí v jedné zprávě: `Key` a hned za ní její `Text` (u nesložitelné
  mrtvé klávesy dva znaky).
- `Focus(false)` znamená, že klient má všechny klávesy považovat za
  uvolněné (přišla bezpečná zkratka nebo jiný vlastník fokusu).
- `Injection { active > 0 }`: vstup ovládá někdo jiný (automatizace,
  vzdálená plocha, agent). Klient to má viditelně ukázat.

Příklad: čtení událostí ve vlastníkovi fokusu:

```rust
use inputproto::{Event, event};

loop {
    let msg = focus.read(None)?;
    for r in event::decode(&msg.data).unwrap_or_default() {
        match r.event {
            Event::Text(c) => line.push(c),
            Event::Key { code: KEY_ENTER, action: Action::Press, .. } => submit(&line),
            Event::Key { ch: Some('c'), action: Action::Press, .. }
                if r.mods & mods::CTRL != 0 => interrupt(),
            Event::Focus(false) => release_all(),
            _ => {}
        }
    }
}
```

## Řídicí příkazy (`inputproto::control`)

Rodič (init) řídí `inputd` textovými příkazy po bootstrap kanálu. Na každý
příkaz přijde jedna odpověď: `ok`, `ok <data>` nebo `err <KÓD>`, kde kód
je jméno z [chyb](chyby.md), např. `err NOT_FOUND`.

| Příkaz | Handly | Odpověď |
|---|---|---|
| `device` | kanál ke zdroji | `ok`; nad 32 zdrojů `err NO_RESOURCES` |
| `focus` | kanál nového vlastníka fokusu | `ok`; předchozí dostane `Focus(false)`, nový `Focus`, `Layout`, `Injection` |
| `secure` | kanál pro bezpečnou zkratku | `ok` |
| `inject` | - | `ok` + kanál nového zdroje umělého vstupu |
| `layout us` / `layout cz` | - | `ok`; neznámé `err NOT_FOUND` |
| `status` | - | `ok devices=2 keyboards=1 pointers=1 layout=us injecting=0 dropped=0 secure=0` |

Neznámý příkaz vrátí `err NOT_SUPPORTED`, chybějící handle
`err INVALID_ARGS`.

**Umělý vstup je capability.** Kanál z `inject` smí vkládat vstup a nikdo
jiný to nedokáže. Init ho dá jen tomu, komu to systém dovolí. Události
z něj mají `SYNTHETIC` a dokud nějaký takový kanál existuje, vlastník
fokusu i kanál `secure` mají `Injection { active > 0 }`.

## Bezpečná zkratka

**Ctrl+Alt+Delete** (Alt i AltGr) na **fyzické** klávesnici:

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Bezpecny rezim">
<defs><marker id="in2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="20" y="50" width="220" height="80" rx="6"/><text class="t b" x="130" y="78" text-anchor="middle">běžný režim</text><text class="m" x="130" y="98" text-anchor="middle">události jdou vlastníkovi</text><text class="m" x="130" y="114" text-anchor="middle">fokusu</text>
<rect class="acc" x="520" y="50" width="220" height="80" rx="6"/><text class="t b" x="630" y="78" text-anchor="middle">bezpečný režim</text><text class="m" x="630" y="98" text-anchor="middle">všechny události jdou</text><text class="m" x="630" y="114" text-anchor="middle">na kanál secure</text>
<path class="ln" d="M240 70 L518 70" marker-end="url(#in2)"/>
<text class="m" x="380" y="60" text-anchor="middle">Ctrl+Alt+Delete (fyzická klávesnice)</text>
<text class="m" x="380" y="88" text-anchor="middle">secure: SecureAttention, fokus: Focus(false)</text>
<path class="ln" d="M518 115 L242 115" marker-end="url(#in2)"/>
<text class="m" x="380" y="135" text-anchor="middle">secure pošle release (nebo zavře kanál)</text>
<text class="m" x="380" y="151" text-anchor="middle">fokus: Focus(true)</text>
<text class="m" x="20" y="185">Umělý vstup zkratku nevyvolá: Delete z něj projde jako obyčejná klávesa s příznakem SYNTHETIC.</text>
</svg>
<figcaption>Bezpečná zkratka přepne vstup na systém, dokud ho systém nevrátí.</figcaption>
</figure>

1. Stisk a uvolnění Delete služba pohltí. Vlastník fokusu je nikdy
   nedostane, Ctrl a Alt předtím dostal normálně.
2. Kanál `secure` dostane `SecureAttention` a služba přejde do
   **bezpečného režimu**: vlastník fokusu dostane `Focus(false)` a všechny
   další události (i uvolnění Ctrl a Alt) jdou jen na kanál `secure`.
3. Systém ukáže svůj dialog (odemčení, potvrzení oprávnění, správce úloh)
   a pak pošle na kanálu `secure` zprávu `release`. Vlastník fokusu
   dostane `Focus(true)`.

Když nikdo kanál `secure` nemá, služba zkratku jen pohltí a zaloguje.
Takhle to dělá Windows (Ctrl+Alt+Del přepne na zabezpečenou plochu).
U nás je navíc zaručené, že zkratku nevyvolá ani umělý vstup, protože se
pozná jen na zdroji, který není `inject`.

## Rozložení (`keymap`)

| Id | Jméno | Popis |
|---|---|---|
| 0 | `us` | americké QWERTY; pravý Alt je Alt |
| 1 | `cz` | české QWERTZ jako „Czech“ ve Windows; pravý Alt je AltGr |

Rozložení přepíná příkaz `layout`. Služba pak pošle `Layout(id)`.

**České rozložení:**

| Klávesa (US) | Základ | Shift | AltGr |
|---|---|---|---|
| `` ` `` | `;` | `°` | |
| 1 - 0 | `+ ě š č ř ž ý á í é` | `1 - 0` | 1 `~`, 3 `^`, 7 `` ` `` |
| `-` | `=` | `%` | |
| `=` | mrtvá `´` | mrtvá `ˇ` | |
| `Y` | `z` | `Z` | |
| `Z` | `y` | `Y` | |
| `[` `]` | `ú` `)` | `/` `(` | `÷` `×` |
| `;` `'` | `ů` `§` | `"` `!` | `$` `ß` |
| `\` | mrtvá `¨` | `'` | `¤` |
| `,` `.` `/` | `, . -` | `? : _` | `< > *` |
| Q W E | | | `\` `\|` `€` |
| F G | | | `[` `]` |
| X C V B N | | | `# & @ { }` |

Úrovně AltGr píšou `~`, `^` a `` ` `` přímo, ne jako mrtvé klávesy
(ve Windows jsou mrtvé). V terminálu se tak píšou bez mezery navíc.

**Mrtvé klávesy:** čárka (á é í ó ú ý ĺ ŕ ć ń ś ź), háček
(č ď ě ľ ň ř š ť ž), přehláska (ä ë ï ö ü ÿ), vše i velkými písmeny.
Znaménko + mezera napíše znaménko samotné, dvakrát stejné znaménko také.
Nesložitelná kombinace napíše obojí (`´` + `x` = `´x`). Ctrl, Alt
a klávesa bez znaku (Enter, šipka) rozepsanou mrtvou klávesu zruší.

**Caps Lock** prohodí velikost písmen (Shift pak píše malá). Na české
horní řadě píše velká písmena (`Ě`) a se Shiftem číslice. Na znaky, které
nejsou písmena, nepůsobí.

**Num Lock:** číslice a desetinná klávesa numerické klávesnice píšou jen
se zapnutým Num Lockem (v `cz` desetinná čárka). Operace `/ * - +` píšou
vždy.

**Opakování:** drženou klávesu (ne modifikátor) opakuje služba sama, po
500 ms a pak 30× za sekundu, stejně pro všechna zařízení. Opakuje se
`Key` s akcí `Repeat` a jeho `Text`. Opakování od zařízení se ignoruje.

## Testy

| Kde | Co |
|---|---|
| `cargo test -p inputproto` | formát zpráv: všechny druhy záznamů, neplatná data, stav |
| `cargo test -p keymap` | US, CZ, AltGr, mrtvé klávesy, Caps Lock, Num Lock, opakování |
| `cargo xtask test`, sada `vstup` | služba v QEMU: zařízení, umělý vstup, CZ, myš, zkratka, opakování, neplatné příkazy |

Test **„QEMU: klávesnice a myš až do služby“** prověří celou cestu ze
zařízení. Init vypíše značku `init: test vstupu: cekam na udalosti z QEMU`.
Xtask (`xtask/src/qmp.rs`) ji najde v logu sériové linky a přes QMP
(`-qmp tcp:127.0.0.1:<port>`) pošle příkazy `input-send-event`: `a`,
Shift+`1`, pohyb (10, −5), levé tlačítko, kolečko dolů a Ctrl+Alt+Delete.
Test ověří text `a!`, součet pohybu, kolečko, bezpečnou zkratku jen na
kanálu `secure` a návrat fokusu po `release`. Totéž dělá xtask
v `cargo xtask run`, takže test projde i tam.

## Omezení první verze

- **Modifikátory kláves se počítají pro každé zařízení zvlášť.** Shift na
  jedné klávesnici neovlivní znak z druhé. Události myši nesou modifikátory
  všech klávesnic dohromady (Ctrl+klik funguje).
- **Žádné PS/2 ani USB HID.** Návrh je pro první verzi nevyžaduje
  a protokol zařízení na ně je připravený (`MSC_SCAN` nese scan kód).
- **Tablet a dotyk** (`EV_ABS`) zatím ne.
- **Kontrolky** Caps Lock a Num Lock nesvítí (fronta stavu virtio-input).
- **Fokus má jeden vlastník.** Rozdělení oknům udělá kompozitor (6c).
- **Metody zadávání** (Unicode vstup, IME) zatím ne. Budou posílat `Text`.
