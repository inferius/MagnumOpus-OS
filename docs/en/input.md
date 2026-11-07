# Input: keyboard and mouse

Input has three parts: the **device driver** (`virtio-input`), the **input
service** (`inputd`) and the **focus owner** that receives the events (init
for now, the terminal in 6b, the compositor in 6c). They all talk through
channel messages, and the message format is in the `inputproto` library.
The keyboard layout is in the `keymap` library, and only `inputd` uses it,
so the whole system has it in one place.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Input event path">
<defs><marker id="in1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="160" height="56" rx="6"/><text class="t b" x="90" y="43" text-anchor="middle">virtio-keyboard</text><text class="m" x="90" y="62" text-anchor="middle">QEMU / hardware</text>
<rect class="box" x="10" y="110" width="160" height="56" rx="6"/><text class="t b" x="90" y="133" text-anchor="middle">virtio-mouse</text><text class="m" x="90" y="152" text-anchor="middle">QEMU / hardware</text>
<rect class="box" x="10" y="200" width="160" height="56" rx="6"/><text class="t b" x="90" y="223" text-anchor="middle">synthetic input</text><text class="m" x="90" y="242" text-anchor="middle">inject capability</text>
<rect class="box" x="210" y="20" width="150" height="56" rx="6"/><text class="t b" x="285" y="43" text-anchor="middle">virtio-input</text><text class="m" x="285" y="62" text-anchor="middle">driver (process)</text>
<rect class="box" x="210" y="110" width="150" height="56" rx="6"/><text class="t b" x="285" y="133" text-anchor="middle">virtio-input</text><text class="m" x="285" y="152" text-anchor="middle">driver (process)</text>
<path class="ln" d="M170 48 L208 48" marker-end="url(#in1)"/>
<path class="ln" d="M170 138 L208 138" marker-end="url(#in1)"/>
<path class="ln" d="M360 48 L418 120" marker-end="url(#in1)"/>
<path class="ln" d="M360 138 L418 138" marker-end="url(#in1)"/>
<path class="ln" d="M170 228 L418 158" marker-end="url(#in1)"/>
<text class="m" x="372" y="96">IDEV / IEVT</text>
<rect class="acc" x="420" y="95" width="150" height="90" rx="6"/><text class="t b" x="495" y="120" text-anchor="middle">inputd</text><text class="m" x="495" y="139" text-anchor="middle">layout (keymap)</text><text class="m" x="495" y="155" text-anchor="middle">key repeat</text><text class="m" x="495" y="171" text-anchor="middle">secure attention</text>
<rect class="ok" x="610" y="60" width="140" height="56" rx="6"/><text class="t b" x="680" y="83" text-anchor="middle">focus owner</text><text class="m" x="680" y="102" text-anchor="middle">init, terminal, GUI</text>
<rect class="box" x="610" y="170" width="140" height="56" rx="6"/><text class="t b" x="680" y="193" text-anchor="middle">secure channel</text><text class="m" x="680" y="212" text-anchor="middle">system (unlock)</text>
<path class="ln" d="M570 125 L608 92" marker-end="url(#in1)"/>
<path class="ln" d="M570 160 L608 195" marker-end="url(#in1)"/>
<text class="m" x="580" y="76">Record</text>
<text class="m" x="560" y="246">Ctrl+Alt+Delete</text>
<text class="m" x="10" y="300">No application reads input globally: only the focus owner gets events, only the system gets the secure shortcut.</text>
</svg>
<figcaption>Event path: device, driver, service, focus owner.</figcaption>
</figure>

Why it works this way:

- **Applications do not read devices.** A keylogger has nothing to listen
  to. The service sends each event only to the focus owner (the console,
  later the compositor), and the focus owner hands it to its windows.
- **Two streams.** A `Key` event carries the physical key. Its code does
  not depend on the layout, which suits games and shortcuts. A `Text`
  event carries the inserted text after the layout and dead keys are
  applied. Terminals and text fields read `Text`, games read `Key`. Input
  methods (Chinese, emoji, dictation) will also send `Text` later, with no
  change for applications.
- **The secure attention shortcut** is caught in the service before any
  application sees anything. An application can neither intercept nor
  fake it.

## Start-up

```text
init
 ├─ devmgr ── finds PCI 1af4:1052 (virtio-input) ── starts virtio-input
 │            sends init an `input` message + channel to the driver
 └─ inputd ── init sends it `device` (driver channel) for every device,
              then `focus` (its focus channel) and `secure` (shortcut channel)
```

Boot log:

```text
virtio-input 00:05.0: 'QEMU Virtio Keyboard' (klavesnice)
virtio-input 00:06.0: 'QEMU Virtio Mouse' (mys)
init: 3 blokovych a 2 vstupnich zarizeni
inputd: bezim (rozlozeni us)
inputd: zarizeni 'QEMU Virtio Keyboard' pripojeno
init: sluzba vstupu bezi (2 zarizeni)
```

QEMU (`cargo xtask run` / `test`) gets `virtio-keyboard-pci` and
`virtio-mouse-pci` devices, both with DMA translated by the IOMMU
(`iommu_platform=on`).

## The virtio-input driver

The driver (`user/virtio-input`) is the same for keyboards, mice and
tablets:

1. virtio 1.x initialisation as for the disk (`libs/virtio`). It accepts
   only the `VERSION_1` and `ACCESS_PLATFORM` features, and interrupts go
   through MSI-X.
2. It reads the name (`ID_NAME`) and the supported events (`EV_BITS`) from
   the device configuration. A device with the `A` key is a keyboard, one
   with relative axes is a mouse.
3. It fills queue 0 (events) with up to 64 buffers of 8 bytes
   (`Virtqueue::post`). The device fills them and returns them in any
   order.
4. It sends the service an `IDEV` greeting. After each interrupt it sends
   all completed events in one `IEVT` message. `virtio_input_event` is
   already in evdev format (type, code, value), so the driver only copies
   it across.
5. When the service cannot keep up (the channel queue is full), the driver
   drops the message instead of blocking the device. When the service
   closes the channel, the driver resets the device and exits.

The driver does not use queue 1 yet (status, i.e. the Caps Lock and Num
Lock LEDs).

## Device protocol (`inputproto::device`)

An input source (a driver or synthetic input) sends the service messages
that start with a tag:

| Tag | Content (little-endian) |
|---|---|
| `IDEV` | version u16 (1), capabilities u16 (`KEYBOARD` 1, `POINTER` 2), UTF-8 name (at most 128 B) |
| `IEVT` | 8-byte events: type u16, code u16, value i32 (at most 1024 per message) |

Events use the evdev format, and the codes come from Linux
(`inputproto::code`):

| Type | Code | Value |
|---|---|---|
| `EV_KEY` (1) | key (`KEY_A` = 30, ...) or button (`BTN_LEFT` = 0x110, ...) | 1 press, 0 release (2 = device repeat, ignored) |
| `EV_REL` (2) | `REL_X`, `REL_Y`, `REL_WHEEL` (positive is up), `REL_HWHEEL` | delta |
| `EV_MSC` (4) | `MSC_SCAN` | raw scan code of the next key (PS/2, USB HID) |
| `EV_SYN` (0) | `SYN_REPORT` | end of frame |

The service groups events into **frames**. It sums the motion in one frame
into a single `Motion` event and the wheel into a single `Scroll`. A frame
ends at `SYN_REPORT` and at the end of a message.

Example: synthetic input types `A` (Shift + a):

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

## Events for the focus owner (`inputproto::event`)

One message is one frame, made of 32-byte records:

<figure class="diagram">
<svg viewBox="0 0 760 120" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="32-byte event record">
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
<text class="m" x="10" y="108">Fields a-d are u32; their meaning depends on the kind (table below).</text>
</svg>
<figcaption>Event record: 16-byte header, 16 bytes of data.</figcaption>
</figure>

| Kind | Event | a | b | c | d |
|---|---|---|---|---|---|
| 1 `KEY` | `Key { code, scancode, action, ch }` | key code | scan code | action 0/1/2 | character (0 = none) |
| 2 `TEXT` | `Text(char)` | Unicode character | | | |
| 3 `MOTION` | `Motion { dx, dy }` | dx i32 | dy i32 (positive is down) | | |
| 4 `BUTTON` | `Button { button, action }` | button code | | action 0/1 | |
| 5 `SCROLL` | `Scroll { dx, dy }` | dx i32 (right) | dy i32 (up) | | |
| 6 `SECURE_ATTENTION` | `SecureAttention` | | | | |
| 7 `FOCUS` | `Focus(bool)` | 1 gained, 0 lost | | | |
| 8 `INJECTION` | `Injection { active }` | number of synthetic input sources | | | |
| 9 `LAYOUT` | `Layout(id)` | 0 `us`, 1 `cz` | | | |

- `flags`: `SYNTHETIC` (1) means the event comes from synthetic input.
- `mods`: modifiers after the event. `SHIFT` 1, `CTRL` 2, `ALT` 4,
  `ALTGR` 8, `META` 16, `CAPS_LOCK` 32, `NUM_LOCK` 64.
- `time`: monotonic time in ns since boot when the service received the
  frame.
- A client skips kinds it does not know (`Event::Unknown`), so adding a new
  event kind is not an incompatible change.

Rules:

- `Key.ch` is the character the key gives in the current layout with the
  current Shift, AltGr and Caps Lock, and it **is computed even while Ctrl
  is held**. So Ctrl+Z is recognised in every layout (in CZ, `z` is on the
  `KEY_Y` key).
- `Text` comes only for printable characters, space included. Enter, Tab,
  Backspace and the arrows are `Key` only. No `Text` is sent while Ctrl,
  Alt or Meta is held; AltGr does type text.
- Order within one message: a `Key`, then its `Text` right after it (two
  characters for a dead key that does not compose).
- `Focus(false)` means the client should treat every key as released (the
  secure attention shortcut fired, or another focus owner took over).
- `Injection { active > 0 }`: someone else is controlling input
  (automation, remote desktop, an agent). The client should show this
  visibly.

Example: reading events in the focus owner:

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

## Control commands (`inputproto::control`)

The parent (init) controls `inputd` with text commands on the bootstrap
channel. Every command gets exactly one reply: `ok`, `ok <data>` or
`err <CODE>`, where the code is a name from the [error list](errors.md),
e.g. `err NOT_FOUND`.

| Command | Handles | Reply |
|---|---|---|
| `device` | channel to the source | `ok`; above 32 sources `err NO_RESOURCES` |
| `focus` | channel of the new focus owner | `ok`; the previous owner gets `Focus(false)`, the new one `Focus`, `Layout`, `Injection` |
| `secure` | channel for the secure attention shortcut | `ok` |
| `inject` | - | `ok` + channel of a new synthetic input source |
| `layout us` / `layout cz` | - | `ok`; unknown `err NOT_FOUND` |
| `status` | - | `ok devices=2 keyboards=1 pointers=1 layout=us injecting=0 dropped=0 secure=0` |

An unknown command returns `err NOT_SUPPORTED`, a missing handle
`err INVALID_ARGS`.

**Synthetic input is a capability.** Only the holder of a channel from
`inject` can inject input. Init hands one out only to whoever the system
allows. Its events carry `SYNTHETIC`, and while any such channel exists,
the focus owner and the secure channel see `Injection { active > 0 }`.

## Secure attention shortcut

**Ctrl+Alt+Delete** (Alt or AltGr) on a **physical** keyboard:

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Secure mode">
<defs><marker id="in2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="20" y="50" width="220" height="80" rx="6"/><text class="t b" x="130" y="78" text-anchor="middle">normal mode</text><text class="m" x="130" y="98" text-anchor="middle">events go to the</text><text class="m" x="130" y="114" text-anchor="middle">focus owner</text>
<rect class="acc" x="520" y="50" width="220" height="80" rx="6"/><text class="t b" x="630" y="78" text-anchor="middle">secure mode</text><text class="m" x="630" y="98" text-anchor="middle">all events go to the</text><text class="m" x="630" y="114" text-anchor="middle">secure channel</text>
<path class="ln" d="M240 70 L518 70" marker-end="url(#in2)"/>
<text class="m" x="380" y="60" text-anchor="middle">Ctrl+Alt+Delete (physical keyboard)</text>
<text class="m" x="380" y="88" text-anchor="middle">secure: SecureAttention, focus: Focus(false)</text>
<path class="ln" d="M518 115 L242 115" marker-end="url(#in2)"/>
<text class="m" x="380" y="135" text-anchor="middle">secure sends release (or closes the channel)</text>
<text class="m" x="380" y="151" text-anchor="middle">focus: Focus(true)</text>
<text class="m" x="20" y="185">Synthetic input cannot fire it: Delete from it passes through as an ordinary key flagged SYNTHETIC.</text>
</svg>
<figcaption>The shortcut hands input to the system until the system gives it back.</figcaption>
</figure>

1. The service swallows the press and release of Delete, so the focus
   owner never gets them. It got Ctrl and Alt as usual before that.
2. The secure channel gets `SecureAttention` and the service enters
   **secure mode**: the focus owner gets `Focus(false)`, and every further
   event (including the release of Ctrl and Alt) goes only to the secure
   channel.
3. The system shows its own dialog (unlock, permission prompt, task
   manager) and then sends `release` on the secure channel. The focus
   owner gets `Focus(true)`.

When nobody holds the secure channel, the service only swallows and logs
the shortcut. Windows works the same way (Ctrl+Alt+Del switches to the
secure desktop). On top of that, synthetic input is guaranteed never to
fire the shortcut, because it is only recognised on sources other than
`inject`.

## Layouts (`keymap`)

| Id | Name | Description |
|---|---|---|
| 0 | `us` | US QWERTY; right Alt is Alt |
| 1 | `cz` | Czech QWERTZ like "Czech" in Windows; right Alt is AltGr |

The `layout` command switches the layout. The service then sends
`Layout(id)`.

**Czech layout:**

| Key (US) | Base | Shift | AltGr |
|---|---|---|---|
| `` ` `` | `;` | `°` | |
| 1 - 0 | `+ ě š č ř ž ý á í é` | `1 - 0` | 1 `~`, 3 `^`, 7 `` ` `` |
| `-` | `=` | `%` | |
| `=` | dead `´` | dead `ˇ` | |
| `Y` | `z` | `Z` | |
| `Z` | `y` | `Y` | |
| `[` `]` | `ú` `)` | `/` `(` | `÷` `×` |
| `;` `'` | `ů` `§` | `"` `!` | `$` `ß` |
| `\` | dead `¨` | `'` | `¤` |
| `,` `.` `/` | `, . -` | `? : _` | `< > *` |
| Q W E | | | `\` `\|` `€` |
| F G | | | `[` `]` |
| X C V B N | | | `# & @ { }` |

The AltGr level types `~`, `^` and `` ` `` directly, not as dead keys (in
Windows they are dead), so a terminal user does not need an extra space.

**Dead keys:** acute (á é í ó ú ý ĺ ŕ ć ń ś ź), caron
(č ď ě ľ ň ř š ť ž), diaeresis (ä ë ï ö ü ÿ), all also in upper case.
Accent + space types the accent alone, and so does the same accent twice.
A combination that does not compose types both (`´` + `x` = `´x`). Ctrl,
Alt and keys without a character (Enter, arrows) cancel a pending dead
key.

**Caps Lock** swaps the case of letters (Shift then types lower case). On
the Czech top row it types capitals (`Ě`), and digits with Shift. It does
not affect characters that are not letters.

**Num Lock:** the keypad digits and the decimal key type only with Num Lock
on (a decimal comma in `cz`). The operators `/ * - +` always type.

**Repeat:** the service repeats a held key (not a modifier) itself, after
500 ms and then 30 times a second, the same for every device. It repeats
the `Key` with the `Repeat` action, together with its `Text`. Repeats from
the device are ignored.

## Tests

| Where | What |
|---|---|
| `cargo test -p inputproto` | message format: every record kind, invalid data, status |
| `cargo test -p keymap` | US, CZ, AltGr, dead keys, Caps Lock, Num Lock, repeat |
| `cargo xtask test`, suite `vstup` | the service in QEMU: devices, synthetic input, CZ, mouse, shortcut, repeat, invalid commands |

The test **"QEMU: klavesnice a mys az do sluzby"** (keyboard and mouse all
the way to the service) checks the whole path from the device. Init prints
the marker `init: test vstupu: cekam na udalosti z QEMU`. Xtask
(`xtask/src/qmp.rs`) spots it in the serial log and sends
`input-send-event` commands over QMP (`-qmp tcp:127.0.0.1:<port>`): `a`,
Shift+`1`, motion (10, −5), the left button, the wheel down and
Ctrl+Alt+Delete. The test checks the text `a!`, the total motion, the
wheel, that the shortcut reached only the secure channel, and that focus
came back after `release`. Xtask does the same in `cargo xtask run`, so
the test passes there too.

## Limits of the first version

- **Key modifiers are tracked per device.** Shift on one keyboard does not
  change the character typed on another. Mouse events carry the modifiers
  of all keyboards combined (Ctrl+click works).
- **No PS/2 and no USB HID.** The design does not need them for the first
  version, and the device protocol is ready for them (`MSC_SCAN` carries
  the scan code).
- **Tablets and touch** (`EV_ABS`) are not supported yet.
- **The Caps Lock and Num Lock LEDs** stay off (virtio-input status
  queue).
- **Focus has a single owner.** The compositor (6c) will split it between
  windows.
- **Input methods** (Unicode input, IME) are not there yet. They will send
  `Text`.
