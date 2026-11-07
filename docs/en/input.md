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
 │            on x86 starts ps2 (the i8042 controller, not on PCI)
 │            sends init an `input` message + channel for every device
 └─ inputd ── init sends it `device` (driver channel) for every device,
              then `focus` (its focus channel) and `secure` (shortcut channel)
```

Boot log:

```text
virtio-input 00:05.0: 'QEMU Virtio Keyboard' (klavesnice)
virtio-input 00:06.0: 'QEMU Virtio Mouse' (mys)
ps2: PS/2 mys s koleckem a 5 tlacitky
ps2: klavesnice
init: 3 blokovych a 4 vstupnich zarizeni
inputd: bezim (rozlozeni us)
inputd: zarizeni 'QEMU Virtio Keyboard' pripojeno
init: sluzba vstupu bezi (4 zarizeni)
```

QEMU (`cargo xtask run` / `test`) gets `virtio-keyboard-pci` and
`virtio-mouse-pci` devices, both with DMA translated by the IOMMU
(`iommu_platform=on`) and bound to the graphics console
(`display=video0`), so input from the QEMU window goes to them. The q35
machine always has a PS/2 controller, and it receives QMP events sent
without a device, which the test uses.

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

## The PS/2 driver

PS/2 is not a dead technology. A laptop's internal keyboard is often
connected through the embedded controller (EC) as PS/2. On desktop PCs the
firmware pretends to be PS/2 even for a USB keyboard (SMM emulation) until
the system takes over the USB controller. The `user/ps2` driver runs the
i8042 controller: the keyboard on the first port, the mouse on the second.

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="PS/2 path">
<defs><marker id="pk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="70" rx="6"/><text class="t b" x="85" y="45" text-anchor="middle">i8042 controller</text><text class="m" x="85" y="64" text-anchor="middle">ports 0x60, 0x64</text><text class="m" x="85" y="80" text-anchor="middle">keyboard, mouse</text>
<rect class="acc" x="210" y="20" width="170" height="70" rx="6"/><text class="t b" x="295" y="45" text-anchor="middle">kernel</text><text class="m" x="295" y="64" text-anchor="middle">IOAPIC: IRQ 1, 12</text><text class="m" x="295" y="80" text-anchor="middle">IOPORT_READ / WRITE</text>
<rect class="box" x="430" y="20" width="150" height="70" rx="6"/><text class="t b" x="505" y="45" text-anchor="middle">ps2 (process)</text><text class="m" x="505" y="64" text-anchor="middle">i8042 library:</text><text class="m" x="505" y="80" text-anchor="middle">scan codes, packets</text>
<rect class="ok" x="620" y="20" width="130" height="70" rx="6"/><text class="t b" x="685" y="45" text-anchor="middle">inputd</text><text class="m" x="685" y="64" text-anchor="middle">two sources:</text><text class="m" x="685" y="80" text-anchor="middle">keyboard, mouse</text>
<path class="ln" d="M160 45 L208 45" marker-end="url(#pk1)"/><text class="m" x="168" y="38">IRQ</text>
<path class="ln" d="M380 45 L428 45" marker-end="url(#pk1)"/><text class="m" x="386" y="38">signal</text>
<path class="ln" d="M428 70 L382 70" marker-end="url(#pk1)"/><text class="m" x="386" y="86">read</text>
<path class="ln" d="M580 55 L618 55" marker-end="url(#pk1)"/><text class="m" x="584" y="48">IEVT</text>
<rect class="box" x="210" y="140" width="370" height="70" rx="6"/><text class="t b" x="395" y="163" text-anchor="middle">driver Resource (from devmgr)</text><text class="m" x="395" y="182" text-anchor="middle">only ports 0x60 and 0x64, lines 1 and 12, 2 interrupts, no DMA</text><text class="m" x="395" y="198" text-anchor="middle">any other port or line = ACCESS_DENIED</text>
<path class="ln" d="M505 92 L470 138" marker-end="url(#pk1)"/>
<text class="m" x="10" y="238">The driver reads the controller by syscall through its Resource. The kernel checks every port and line.</text>
</svg>
<figcaption>PS/2: a controller on the board, interrupts through the IOAPIC, a driver in userspace.</figcaption>
</figure>

For it the **kernel** offers two things PCI drivers do not need:

- **I/O ports** through the [`IOPORT_READ`/`IOPORT_WRITE`](syscalls.md#ioport_read)
  syscalls. Every access passes the Resource check. This is slower than
  `in`/`out` instructions in userspace, but enough for PS/2 (hundreds of
  bytes a second).
- **Legacy interrupt lines** through the IOAPIC
  ([`INTERRUPT_CREATE_LINE`](syscalls.md#interrupt_create_line)). ISA IRQ 1
  and 12 are mapped to GSIs using the MADT overrides. The kernel masks a
  level-triggered line until it is acknowledged. In the architecture
  interface this is the `LegacyIo` trait, and aarch64 returns
  `NOT_SUPPORTED`.

**Start-up:** on x86 `devmgr` always starts `ps2`, because the controller is
not on PCI. It gives the driver a Resource with only ports 0x60 and 0x64
and lines 1 and 12. The driver reports what it found (`ps2 klavesnice mys`
for keyboard and mouse, or `ps2 nic` for nothing), and `devmgr` sends init
an `input` message only for the devices found.

**Initialisation** (controller interrupts off, polling):

1. The status port 0x64 reads `0xFF` = no controller, the driver exits.
2. Disable both ports, flush the buffer, configuration: no interrupts,
   **with translation** (the keyboard sends set 2, the controller converts
   it to set 1).
3. Controller self-test (`0xAA` -> `0x55`), detect the second port, port
   tests.
4. Keyboard: reset (`0xFF` -> `ACK`, `0xAA`), enable (`0xF4`).
5. Mouse: reset, then the sample-rate sequences 200, 100, 80 (IntelliMouse
   wheel, ID 3) and 200, 200, 80 (5 buttons, ID 4). A mouse that does not
   support them keeps reporting ID 0.
6. Enable interrupts only for the devices found.

**Operation:** the driver waits for the interrupts of both lines and after
each one reads everything the controller has. Status bit 5 (`AUX_DATA`)
tells mouse bytes from keyboard bytes. Because an edge can be lost, it also
checks the controller without an interrupt, at the latest after 500 ms.

**Scan codes** (`libs/i8042`, set 1):

| Bytes | Meaning | evdev code / `MSC_SCAN` |
|---|---|---|
| `k` / `k \| 0x80` | press / release | `k` (for 0x01-0x58 a set 1 code is the evdev code) |
| `E0 k` | extended key (arrows, right Ctrl/Alt, Delete, Windows) | from the table, `MSC_SCAN` = `0xE000 \| k` |
| `E0 2A`, `E0 AA`, `E0 36`, `E0 B6` | fake Shifts around extended keys | ignored |
| `E1 1D 45` / `E1 9D C5` | Pause (press / release) | `KEY_PAUSE`, `0xE11D45` |
| `FA FE EE 00 FF` | replies and errors, not keys | ignored |

**Mouse packets** (3 bytes, 4 with a wheel): buttons in byte 0, X and Y
motion as 9-bit signed numbers (Y up, flipped for evdev), overflow drops the
motion. A byte without sync bit 3 at the start of a packet is dropped, so
lost synchronisation recovers by itself. Wheel: positive PS/2 values mean
towards the user, `REL_WHEEL` is the opposite. Buttons 4 and 5 are
`BTN_SIDE` and `BTN_EXTRA`.

⚠️ **Risk on real hardware:** the driver detects the controller by probing
port 0x64 and does not read the 8042 flag from the ACPI FADT. On boards
without a controller a write to 0x64 is usually harmless, but on hardware
without legacy ports the firmware may redirect it elsewhere. Once the first
such machine shows up, the FADT flag (`IAPC_BOOT_ARCH` bit 1) should be
read first.

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

Buttons are codes 0x100-0x15F and 0x2C0-0x2E7 (`code::is_button`: mouse,
joystick, gamepad, tablet touch). They arrive as `Button`, do not go
through the layout and do not repeat. Every other `EV_KEY` code is a key.
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
<rect class="box" x="220" y="20" width="60" height="44" rx="4"/><text class="t" x="250" y="47" text-anchor="middle">src</text>
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
| 10 `DROPPED` | `Dropped { messages }` | number of messages the client missed | | | |
| 11 `NEW_DEVICE` | `NewDevice { source, state }` | source id | 0 pending, 1 trusted, 2 rejected | | |

- `flags`: `SYNTHETIC` (1) means the event comes from synthetic input.
- `mods`: modifiers after the event. `SHIFT` 1, `CTRL` 2, `ALT` 4,
  `ALTGR` 8, `META` 16, `CAPS_LOCK` 32, `NUM_LOCK` 64.
- `time`: monotonic time in ns since boot when the service received the
  frame.
- `source`: the id of the source (device or synthetic input) the event
  came from; 0 = an event of the service itself (focus, layout, shortcut).
  The `sources` command lists them. The compositor uses it, for example, to
  tell a tablet from a mouse.
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
- **Full queue:** the service never waits for a client. When a client
  falls behind and its queue is full, the message is dropped. The next
  delivered message starts with a `Dropped { messages }` record, and the
  client should, as after `Focus(false)`, treat every key and button as
  released, because a release may have been lost.

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
| `sources` | - | `ok` and lines `<id> <capabilities> <real\|inject\|pending\|rejected> <name>`, e.g. `3 1 real PS/2 klavesnice` |
| `policy verify-new-devices on\|off` | - | `ok`; the [new input device verification](usb.md#verifying-a-new-input-device-against-badusb) security option |
| `settled` | - | `ok`; devices attached up to now are trusted (init after boot) |

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

When nobody holds the secure channel, or its queue is full, the service
only swallows and logs the shortcut and does **not** enter secure mode.
Otherwise input would stay with a receiver that does not know about the
shortcut and would never release it. Windows works the same way (Ctrl+Alt+Del switches to the
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
| `cargo test -p i8042` | scan codes (E0, E1, fake Shifts, Print Screen), mouse packets (sync, overflow, wheel, 5 buttons) |
| `cargo xtask test`, suite `vstup` | the service in QEMU: devices, synthetic input, CZ, mouse, shortcut, repeat, non-mouse buttons, full queue (`Dropped`), invalid commands |

The test **"QEMU: klavesnice a mys az do sluzby"** (keyboard and mouse all
the way to the service) checks the whole path from the device. Init prints
the marker `init: test vstupu: cekam na udalosti z QEMU`. Xtask
(`xtask/src/qmp.rs`) spots it in the serial log and sends
`input-send-event` commands over QMP (`-qmp tcp:127.0.0.1:<port>`): `a`,
Shift+`1`, motion (10, −5), the left button, the wheel down and
Ctrl+Alt+Delete. The keyboard and the mouse are two devices, and their events keep no
order between each other. So once the test has the keys and the mouse
events, it prints a second marker,
`init: test vstupu: cekam na zkratku z QEMU`, and xtask sends
Ctrl+Alt+Delete only after it. The test checks the text `a!`, the total
motion, the wheel, that the shortcut reached only the secure channel, and
that focus came back after `release`. Xtask does the same in `cargo xtask run`, so
the test passes there too.

The test **"QEMU: PS/2 klavesnice a mys"** (PS/2 keyboard and mouse)
prints a third marker, `init: test vstupu: cekam na udalosti PS/2 z QEMU`.
Xtask then sends events **without** `device`, which QEMU delivers to PS/2:
`b`, Shift+`2`, Delete, motion (7, 3), the right button and the wheel up.
The test checks the text `b@`, the scan code `0xE053` for Delete, the total
motion, the wheel, and that every event came from the "PS/2 ..." sources
(the `source` field). The virtio test, in turn, checks that its events came
from the virtio sources.

## Limits of the first version

- **Key modifiers are tracked per device.** Shift on one keyboard does not
  change the character typed on another. Mouse events carry the modifiers
  of all keyboards combined (Ctrl+click works).
- **USB HID in the boot protocol only** (keyboard, mouse), see
  [USB](usb.md). A full report parser (multimedia, tablets) comes in
  phase 4.
- **PS/2 without hotplug:** a keyboard or mouse plugged in at run time is
  not found. The keyboard LEDs stay off for now.
- **Tablets and touch** (`EV_ABS`) are not supported yet.
- **The Caps Lock and Num Lock LEDs** stay off (virtio-input status
  queue).
- **Focus has a single owner.** The compositor (6c) will split it between
  windows.
- **Input methods** (Unicode input, IME) are not there yet. They will send
  `Text`.
