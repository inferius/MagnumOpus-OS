# Graphics and windows (GUI)

Milestone 6c. Describes the `virtio-gpu` driver, the `compd` compositor,
the window protocol (`libs/guiproto`), the native UI library (`libs/ui`),
fonts (`libs/font`), drawing (`libs/gfx`), system dialogs and the action
API. The agreed decisions are in the [Architecture](architecture.md#decisions-for-milestones-6a-6d-agreed)
and in the [6c decisions](#6c-decisions-agreed) table below.

<figure class="diagram">
<svg viewBox="0 0 780 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="GUI parts">
<defs><marker id="g1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">application</text><text class="m" x="95" y="64" text-anchor="middle">libs/ui: widgets, text</text><text class="m" x="95" y="80" text-anchor="middle">draws into its buffers</text>
<rect class="box" x="10" y="120" width="170" height="70" rx="6"/><text class="t b" x="95" y="145" text-anchor="middle">terminal window</text><text class="m" x="95" y="164" text-anchor="middle">native 6b protocol</text><text class="m" x="95" y="180" text-anchor="middle">as window content</text>
<rect class="box" x="10" y="220" width="170" height="70" rx="6"/><text class="t b" x="95" y="245" text-anchor="middle">shell, agent</text><text class="m" x="95" y="264" text-anchor="middle">call app actions</text><text class="m" x="95" y="280" text-anchor="middle">(capability actions)</text>
<rect class="acc" x="280" y="60" width="200" height="200" rx="6"/><text class="t b" x="380" y="88" text-anchor="middle">compd (compositor)</text><text class="m" x="380" y="110" text-anchor="middle">layer tree of all windows</text><text class="m" x="380" y="128" text-anchor="middle">transactions, frame clock</text><text class="m" x="380" y="146" text-anchor="middle">decorations, trust badge</text><text class="m" x="380" y="164" text-anchor="middle">system dialogs</text><text class="m" x="380" y="182" text-anchor="middle">focus, input routing</text><text class="m" x="380" y="200" text-anchor="middle">accessibility tree</text><text class="m" x="380" y="218" text-anchor="middle">action routing</text>
<path class="ln" d="M180 55 L278 100" marker-end="url(#g1)"/><text class="m" x="190" y="62">window: layers, a11y</text>
<path class="ln" d="M180 155 L278 160" marker-end="url(#g1)"/>
<path class="ln" d="M180 255 L278 220" marker-end="url(#g1)"/>
<rect class="box" x="580" y="20" width="190" height="70" rx="6"/><text class="t b" x="675" y="45" text-anchor="middle">inputd</text><text class="m" x="675" y="64" text-anchor="middle">focus = compd</text><text class="m" x="675" y="80" text-anchor="middle">secure attention</text>
<rect class="box" x="580" y="140" width="190" height="70" rx="6"/><text class="t b" x="675" y="165" text-anchor="middle">virtio-gpu (driver)</text><text class="m" x="675" y="184" text-anchor="middle">displayproto: frame,</text><text class="m" x="675" y="200" text-anchor="middle">damaged rects, flush</text>
<rect class="box" x="580" y="260" width="190" height="70" rx="6"/><text class="t b" x="675" y="285" text-anchor="middle">init</text><text class="m" x="675" y="304" text-anchor="middle">app identity and</text><text class="m" x="675" y="320" text-anchor="middle">permissions, dialogs</text>
<path class="ln" d="M578 55 L482 100" marker-end="url(#g1)"/><text class="m" x="500" y="62">Record</text>
<path class="ln" d="M482 170 L578 175" marker-end="url(#g1)"/><text class="m" x="500" y="164">Present</text>
<path class="ln" d="M578 290 L482 240" marker-end="url(#g1)"/><text class="m" x="490" y="290">connect, dialog</text>
<rect class="ok" x="280" y="290" width="200" height="50" rx="6"/><text class="t b" x="380" y="312" text-anchor="middle">QEMU virtio-gpu</text><text class="m" x="380" y="329" text-anchor="middle">2D now, virgl 3D next</text>
<path class="ln" d="M675 212 L482 305" marker-end="url(#g1)"/>
</svg>
<figcaption>GUI parts and who talks to whom. An application never sees other windows or the screen.</figcaption>
</figure>

## 6c decisions (agreed)

| # | Question | Decision |
|---|---|---|
| 1 | rendering | **software compositing on virtio-gpu 2D** as the compatibility layer everywhere (x86_64 and aarch64); right after that **virgl 3D** as a second compositor backend (same layer tree) |
| 2 | buffers | **shared memory object (VMO)**: allocated by the application (its job pays), the compositor gets a handle with only `READ`, `MAP`, `WAIT`, `DUPLICATE`, `TRANSFER`; drawing without copies |
| 3 | frames | **compositor clock + transactions**: 60 Hz frame deadline, an application gets `Frame` only on request; transactions are atomic even across windows and applications (synchronized resize) |
| 4 | font | **own TrueType parser and rasterizer** (`libs/font`): cmap, glyf, hmtx, kern, area anti-aliasing, no hinting or shaping |

⚠️ **Risks (stated up front):**

- **virgl 3D in QEMU** needs a display with OpenGL (`-display sdl,gl=on`,
  `gtk,gl=on` or `egl-headless`). Tests run without a window, so 3D can
  only be tested with `egl-headless`; when the host lacks it, the 3D test
  is skipped and only 2D is verified. Venus (Vulkan) needs a Mesa port and
  is missing in QEMU for Windows, so it is a later step.
- **Without an IOMMU** `virtio-gpu` can DMA anywhere in memory (like the
  other drivers). With an IOMMU (`iommu_platform=on`) only into its buffers.
- **An application may change its buffer while it is being read.** The
  worst outcome is torn content of its own window; the compositor never
  trusts buffer contents (dimensions are checked at creation, reads are
  bounded by the VMO).
- **Software compositing** costs CPU: at 1280x800 and 60 Hz that is
  ~250 MB/s of writes. Therefore only damaged regions are composed and
  nothing is drawn when nothing changes.
- **Kernel console and firmware GOP (solved in the loader):** the firmware
  (edk2/OVMF) has the `VirtioGpuDxe` driver and turns virtio-gpu into a GOP
  as well. Its framebuffer is plain RAM pushed to the screen by virtio
  commands - after ExitBootServices nobody would refresh it and the kernel
  console would be blank until `compd` takes over. The loader therefore goes
  through all GOPs, reads the device vendor and type via
  `EFI_PCI_IO_PROTOCOL` and **takes virtio-gpu (1af4:1050) only when there
  is no other GOP** (then it warns in the log). The loader log lists all
  GOPs (`GOP na zarizeni PCI 1234:1111`, `GOP mimo PCI`) and the chosen
  framebuffer: on x86_64 the VGA card, on aarch64 ramfb.
- **Imitating a system dialog** in a fullscreen application cannot be
  ruled out by looks alone. The protection is that a permission dialog
  accepts only physical input (synthetic input does not count), shows the
  user's security phrase which applications do not know, and the secure
  attention shortcut always shows the real system screen.

## Start-up

```text
init
 ├─ devmgr ── finds PCI 1af4:1050 (virtio-gpu) ── starts virtio-gpu
 │            sends init the message `display` + channel (displayproto)
 ├─ inputd
 └─ compd ─── init sends it `display` (driver channel), `input`
              (a new focus channel from inputd), `secure` (shortcut channel)
              and for each application `connect <identity>` -> window channel
```

The compositor receives the input focus from `inputd` (command `focus`),
so only it sees the keyboard and mouse and it routes them to windows. It
also receives the secure attention shortcut (channel `secure`) and shows
the system screen for it.

## Drawing and buffers

A pixel is a `u32` with byte order B, G, R, A (virtio-gpu format
`B8G8R8A8_UNORM`, `0xAARRGGBB` in a register) with **premultiplied
alpha**. A buffer has a width, height and row step (stride) **in pixels**.

<figure class="diagram">
<svg viewBox="0 0 780 260" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Buffer life cycle">
<defs><marker id="g2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="160" height="60" rx="6"/><text class="t b" x="90" y="55" text-anchor="middle">app draws</text><text class="m" x="90" y="74" text-anchor="middle">into buffer A</text>
<rect class="acc" x="220" y="30" width="160" height="60" rx="6"/><text class="t b" x="300" y="55" text-anchor="middle">COMMIT</text><text class="m" x="300" y="74" text-anchor="middle">layer -> A, damage</text>
<rect class="box" x="430" y="30" width="160" height="60" rx="6"/><text class="t b" x="510" y="55" text-anchor="middle">compd reads A</text><text class="m" x="510" y="74" text-anchor="middle">while composing</text>
<rect class="ok" x="620" y="30" width="150" height="60" rx="6"/><text class="t b" x="695" y="55" text-anchor="middle">RELEASED A</text><text class="m" x="695" y="74" text-anchor="middle">once layer shows B</text>
<path class="ln" d="M170 60 L218 60" marker-end="url(#g2)"/>
<path class="ln" d="M380 60 L428 60" marker-end="url(#g2)"/>
<path class="ln" d="M590 60 L618 60" marker-end="url(#g2)"/>
<rect class="box" x="10" y="150" width="160" height="60" rx="6"/><text class="t b" x="90" y="175" text-anchor="middle">meanwhile draws</text><text class="m" x="90" y="194" text-anchor="middle">into buffer B</text>
<rect class="acc" x="220" y="150" width="160" height="60" rx="6"/><text class="t b" x="300" y="175" text-anchor="middle">COMMIT</text><text class="m" x="300" y="194" text-anchor="middle">layer -> B</text>
<path class="ln" d="M170 180 L218 180" marker-end="url(#g2)"/>
<path class="ln dash" d="M380 180 L690 92" marker-end="url(#g2)"/>
<text class="m" x="10" y="245">Double buffering: the app never waits for the compositor, it only must not reuse a buffer before RELEASED.</text>
</svg>
<figcaption>A buffer belongs to the application; the compositor only reads it between COMMIT and RELEASED.</figcaption>
</figure>

- The application creates a VMO and sends `BUFFER_CREATE` with the handle
  (only `READ`, `MAP` and the basic rights). The compositor maps it
  read-only (`stride * height * 4` bytes - a smaller VMO fails to map) and
  checks the dimensions (at most 16384 px).
- The buffer memory is paid by the application's job: an application
  cannot exhaust the compositor's memory.
- The compositor holds a buffer from the `COMMIT` that attached it to a
  layer until `RELEASED`, which it sends as soon as no layer shows the
  buffer (the layer got another buffer or was destroyed). The `ui` library
  therefore alternates between two buffers and never waits for the
  compositor.

## Layer tree

A window is a tree of **layers**. Applications draw the layer contents,
but **offset, opacity, clipping, scrolling and animation are done by the
compositor**: moving a list or highlighting a button needs no redraw by the
application and stays smooth even while the application is busy.

| Layer kind | Content |
|---|---|
| `GROUP` | children only; optionally clipped to its rectangle with a content offset (scroll) |
| `BUFFER` | a rectangle from an application buffer (the source rectangle is scaled to the target) |
| `SOLID` | a single-colour rectangle (backgrounds, separators, selection) |

Layer properties (`LAYER_SET`, each optional): rectangle in the parent's
coordinates, order among siblings, opacity 0-255, visibility, clipping of
children, content offset `(sx, sy)`, buffer and source rectangle, colour.

**Animation** (`ANIMATE`): a property (opacity, offset x/y, scroll x/y),
target value, duration in ms and curve (`linear`, `ease-out`). The
compositor computes it every frame itself; the application gets
`ANIMATION_DONE`.

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Layer tree and accessibility tree">
<defs><marker id="g3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="t b" x="20" y="24">layer tree</text>
<rect class="acc" x="20" y="36" width="160" height="40" rx="6"/><text class="t" x="100" y="61" text-anchor="middle">1 GROUP window</text>
<rect class="box" x="20" y="100" width="150" height="40" rx="6"/><text class="t" x="95" y="125" text-anchor="middle">2 SOLID background</text>
<rect class="box" x="190" y="100" width="170" height="40" rx="6"/><text class="t" x="275" y="125" text-anchor="middle">3 GROUP list (clip)</text>
<rect class="box" x="190" y="170" width="170" height="40" rx="6"/><text class="t" x="275" y="195" text-anchor="middle">4 BUFFER rows</text>
<rect class="box" x="20" y="170" width="150" height="40" rx="6"/><text class="t" x="95" y="195" text-anchor="middle">5 BUFFER button</text>
<path class="ln" d="M100 76 L95 98" marker-end="url(#g3)"/>
<path class="ln" d="M120 76 L270 98" marker-end="url(#g3)"/>
<path class="ln" d="M275 140 L275 168" marker-end="url(#g3)"/>
<path class="ln" d="M70 76 L70 168" marker-end="url(#g3)"/>
<text class="m" x="190" y="232">scroll = property of layer 3, no app redraw</text>
<text class="t b" x="440" y="24">accessibility tree</text>
<rect class="ok" x="440" y="36" width="190" height="40" rx="6"/><text class="t" x="535" y="61" text-anchor="middle">window "Notes"</text>
<rect class="ok" x="440" y="100" width="150" height="40" rx="6"/><text class="t" x="515" y="125" text-anchor="middle">list "Files"</text>
<rect class="ok" x="610" y="100" width="160" height="40" rx="6"/><text class="t" x="690" y="125" text-anchor="middle">button "Save"</text>
<rect class="ok" x="440" y="170" width="150" height="40" rx="6"/><text class="t" x="515" y="195" text-anchor="middle">item "a.txt" ✓</text>
<path class="ln" d="M515 76 L515 98" marker-end="url(#g3)"/>
<path class="ln" d="M560 76 L680 98" marker-end="url(#g3)"/>
<path class="ln" d="M515 140 L515 168" marker-end="url(#g3)"/>
<path class="ln-acc dash" d="M360 190 L438 190"/>
<path class="ln-acc dash" d="M170 190 Q400 260 690 142"/>
<text class="m" x="440" y="240">a node points to a layer + rectangle: the compositor</text>
<text class="m" x="440" y="256">knows where it is even without the app (reader, agent, test)</text>
</svg>
<figcaption>Two trees of one window. Layers decide what is visible; accessibility what it means.</figcaption>
</figure>

## Accessibility tree

The structure of a window is part of the protocol. A node (`A11Y_UPDATE`)
carries:

| Field | Meaning |
|---|---|
| `id`, `parent` | identity and parent (0 = window root) |
| `role` | `window`, `titlebar`, `button`, `label`, `text-input`, `checkbox`, `list`, `list-item`, `group`, `image`, `progress`, `terminal` |
| `name` | text for humans ("Save") |
| `value` | value (text field, progress percent) |
| `state` | bits: `focused`, `checked`, `disabled`, `selected`, `expanded`, `busy` |
| `bounds` | layer + rectangle in it |
| `actions` | bits: `activate`, `focus`, `set-value`, `toggle`, `scroll` |

The compositor keeps the tree and serves it to:

- **a screen reader and magnifier** (later) - via a capability, not the app,
- **tests and the agent**: "click the Save button" goes through a node,
  not coordinates; the compositor sends the application `A11Y_ACTION`,
- **decorations**: a custom title bar is as accessible as the standard one
  (node `titlebar`).

## Frames and transactions

Window state has two copies: **pending** (messages since the last
`COMMIT`) and **current** (the one being composed). `COMMIT` turns the
pending state into the current one **atomically**: half a change is never
visible. One transaction may span several messages (the channel message
limit is 64 KiB).

<figure class="diagram">
<svg viewBox="0 0 780 290" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Frame clock">
<defs><marker id="g4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<path class="ln" d="M20 60 L760 60"/><text class="m" x="20" y="45">compd</text>
<path class="ln" d="M20 160 L760 160"/><text class="m" x="20" y="145">application</text>
<path class="ln" d="M20 250 L760 250"/><text class="m" x="20" y="235">virtio-gpu</text>
<path class="ln-acc" d="M100 50 L100 70"/><text class="m" x="85" y="40">tick N</text>
<path class="ln-acc" d="M367 50 L367 70"/><text class="m" x="350" y="40">tick N+1</text>
<path class="ln-acc" d="M634 50 L634 70"/><text class="m" x="617" y="40">tick N+2</text>
<path class="ln" d="M104 64 L170 156" marker-end="url(#g4)"/><text class="m" x="120" y="100">FRAME seq, deadline</text>
<rect class="box" x="172" y="148" width="120" height="24" rx="4"/><text class="m" x="232" y="165" text-anchor="middle">draws, COMMIT</text>
<path class="ln" d="M292 156 L330 66" marker-end="url(#g4)"/>
<rect class="acc" x="370" y="48" width="90" height="24" rx="4"/><text class="m" x="415" y="65" text-anchor="middle">compose</text>
<path class="ln" d="M460 66 L520 244" marker-end="url(#g4)"/><text class="m" x="490" y="140">PRESENT damage</text>
<path class="ln" d="M560 244 L600 70" marker-end="url(#g4)"/><text class="m" x="585" y="200">PRESENTED</text>
<path class="ln" d="M604 64 L680 156" marker-end="url(#g4)"/><text class="m" x="650" y="100">PRESENTED txn</text>
<text class="m" x="20" y="282">Input is delivered immediately (not on the clock); composing waits only for the deadline, never for a slow app.</text>
</svg>
<figcaption>60 Hz clock: the app draws after FRAME, the compositor composes on the tick, the driver confirms the display.</figcaption>
</figure>

- **Clock:** period 16.67 ms. The compositor ticks only when something is
  waiting (damaged region, animation, frame request). Nothing changes =
  no wake-ups.
- **Frame request:** `COMMIT` with the `FRAME` flag says "tell me when to
  draw the next one". `FRAME { seq, deadline }` arrives at the start of the
  next period; an application that commits before `deadline` makes that
  frame.
- **Input priority:** the compositor first handles input (delivers it to a
  window), only then composes. Input is never batched to the clock.
- **A slow application** does not delay others: its last current state is
  composed.
- **`PRESENTED { txn, time }`** confirms the display (time from the driver).

### Synchronized resize

The compositor sends `CONFIGURE { window, w, h, serial }`. The application
redraws and acknowledges `serial` in `COMMIT`. The window frame
(decorations) and the new content appear **in the same frame**. When the
application does not acknowledge within 200 ms, the compositor shows the
old content in the new frame (clipped or padded with background), so the
window never "runs away" from its frame.

### Transactions across windows and applications

For changes that must appear together in different windows (a terminal
window with content from another process, splitting the screen between
two applications):

1. A coordinator (application or compositor) asks for a group:
   `SYNC_GROUP { count }` -> `SYNC_TOKEN { token }` (a 128-bit random number).
2. It hands the token to the other participants (over its own channel).
3. Each participant sends `COMMIT` with `token`. The compositor holds the
   current states of all of them until `count` commits arrive (or 200 ms
   pass - then it applies all of them and drops the group, so nothing gets
   stuck).

## Windows and decorations

`WINDOW_CREATE { id, mode, w, h, title }`. The frame, title bar and
buttons are drawn by the compositor according to the mode ([Custom window
look](architecture.md#custom-window-look-custom-title-bar)):

| Mode | Application draws | System keeps |
|---|---|---|
| `STANDARD` | content only | frame, title, badge, buttons |
| `CUSTOM_TITLE` | the title area too | badge at a fixed place, buttons (the app chooses side and colour), move and resize via declared regions |
| `FRAMELESS`, `FULLSCREEN` | everything | badge at the top edge on hover and always on a permission change (3 s); secure shortcut and dialogs always |

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Custom title bar mode">
<rect class="box" x="20" y="20" width="740" height="200" rx="8"/>
<rect class="acc" x="20" y="20" width="740" height="36" rx="8"/>
<rect class="ok" x="28" y="26" width="150" height="24" rx="5"/><text class="m" x="103" y="43" text-anchor="middle">badge: Notes · network</text>
<rect class="box" x="196" y="26" width="110" height="24" rx="5"/><text class="m" x="251" y="43" text-anchor="middle">tab 1</text>
<rect class="box" x="312" y="26" width="110" height="24" rx="5"/><text class="m" x="367" y="43" text-anchor="middle">tab 2</text>
<rect class="warn" x="430" y="24" width="230" height="28" rx="4"/><text class="m" x="545" y="43" text-anchor="middle">DRAG region (declared)</text>
<rect class="err" x="672" y="26" width="24" height="24" rx="4"/><rect class="err" x="700" y="26" width="24" height="24" rx="4"/><rect class="err" x="728" y="26" width="24" height="24" rx="4"/>
<text class="m" x="680" y="76">buttons: system</text>
<text class="m" x="200" y="76">tabs: application (custom title)</text>
<text class="t" x="390" y="140" text-anchor="middle">application content</text>
<text class="m" x="20" y="242">Moving the window works even with a hung app: the compositor knows the regions and does not ask the app.</text>
</svg>
<figcaption>Custom title bar: the app draws the tabs, the system the badge and buttons.</figcaption>
</figure>

**Declared regions** (`REGIONS`): a list of rectangles of kind `DRAG`
(moves the window), `NO_DRAG` (exception inside `DRAG`, e.g. a tab) and
`RESIZE` (edges). The compositor moves and resizes the window by them on
its own, without asking the application.

**Trust badge** shows the application's identity (the manifest name given
by init, not by the application) and what it may do right now (network,
folders, camera). The application cannot paint over it: the compositor
composes it above the window content.

**Per-client limits** (protecting the compositor from an application): a
buffer at most 64 MiB and all buffers 256 MiB, 256 buffers, 8192 layers,
32 windows, 4096 accessibility nodes per window, pending messages before
`COMMIT` at most 16 MiB, title 256 characters, 4 transaction groups.
Messages that do not fit into an application's channel queue are deferred
by the compositor (mouse motion and ping may be dropped, `RELEASED`,
`FRAME` and `CONFIGURE` never); a client that does not read 4096
deferred messages is disconnected.

**Unresponsive application:** the compositor sends `PING`; without `PONG`
within 3 s it dims the window and shows a warning. Moving, minimizing and
closing keep working.

## Input in windows

The compositor receives events from `inputd` ([Input](input.md)) - from the
focus channel and from the `secure` channel (in secure mode input goes
only there) - and routes them:

- **keyboard** (`Key`, `Text`) -> the focused window; a new window of
  another application gets the focus only when the user has not typed for
  at least 1.5 s (otherwise a background window could "take" a password
  being typed),
- **mouse** -> the window under the pointer (converted to window
  coordinates); a button press outside decoration areas activates the
  window and gives it focus,
- **wheel** -> the window under the pointer,
- **secure attention shortcut** (`secure`) -> the system screen,
- an event flagged `SYNTHETIC` (synthetic input) reaches a window with the
  flag; a system dialog **ignores** it,
- **a new input device** (`NewDevice`, BadUSB protection): the compositor
  shows a system dialog "Allow device?" and drops that device's events
  until decided - the device cannot allow itself (type Enter). The answer
  goes to `inputd` as `trust <id>` / `reject <id>`.

A window receives `INPUT` with `InputRecord`s (28 B): key (layout
independent code, action, character), inserted text, pointer in window
coordinates, pointer leave, button with position, wheel and focus; plus
modifiers, the synthetic flag and time. Motion and button release go to
the window that got the press, even when the pointer leaves it.

## System dialogs

A dialog (permission, password, action confirmation) is drawn by the
compositor in a **separate layer above the dimmed windows**. No
application may draw into that layer.

- Only a service with the `dialog` capability may ask for a dialog (init
  gives it to the permission broker), never an application itself.
- The dialog shows the identity of the requesting application, the
  question, the options and the user's **security phrase**.
- Keyboard and mouse go only to the dialog while it is shown; synthetic
  input is ignored.
- The result `ANSWER { id, choice }` or `ANSWER { id, text }` (password)
  goes only to the requester.

The `dialog` channel messages are binary (`guiproto::control`):

| Message | Fields |
|---|---|
| `Ask` | id, kind (`permission`, `confirm`, `password`), application, text, options (1-8) |
| `Cancel` | id |
| `Phrase` | security phrase (at most 64 characters) |
| `Answer` (reply) | id, option index, text (password) |
| `Cancelled` (reply) | id |

Keys in a dialog: Tab and arrows change the option, Enter confirms, Esc
selects the last option (deny). Mouse: click a button. For the first
300 ms after it appears a dialog ignores input - the second click of a
double click or a key typed into another window cannot confirm a dialog
that just popped up.

**Secure screen:** Ctrl+Alt+Delete (caught by `inputd`, not by
applications) shows the system screen over everything. Only physical input
closes it; the compositor then sends `inputd` `release` and focus returns.

## Action API

An application declares **actions** with typed parameters and permissions
in its manifest. The shell, file explorer, assistant and agent call them -
always through the compositor, which checks the arguments against the
manifest before the application receives them.

```toml
[[action]]
id = "note.create"
title = "New note"
needs = ["fs:write"]
confirm = false          # true = confirm with a system dialog

[[action.param]]
name = "title"
type = "text"            # text, int, bool, path, choice
required = true

[[action.param]]
name = "color"
type = "choice"
choices = ["yellow", "blue"]
```

| Message | Direction | Content |
|---|---|---|
| `List` | caller -> compd | reply `Actions`: actions of running apps (app id, declaration) |
| `Call` | caller -> compd | call id, application, action, arguments (typed values), automation flag |
| `ACTION_INVOKE` | compd -> application | checked arguments, call id, who calls |
| `ACTION_RESULT` | application -> compd | `ok` + value, or an error text |
| `Result` | compd -> caller | result with the caller's call id |

An action with `confirm = true`, or one called from a synthetic source
(agent), is confirmed with a system dialog before it is passed on. A
caller sees only actions, not windows.

## Compositor control commands

Init talks to `compd` over the bootstrap channel with text commands (as
with `inputd`); the reply is `ok`, `ok <data>` or `err <CODE>`.

| Command | Handles | Reply |
|---|---|---|
| `display` | graphics driver channel | `ok <w> <h>` |
| `input` | focus channel from `inputd` | `ok` |
| `secure` | secure shortcut channel from `inputd` | `ok` |
| `connect` + application identity (binary) | - | `ok` + window channel |
| `dialog` | - | `ok` + system dialog channel |
| `actions <caller> [automat]` | - | `ok` + action call channel; `automat` = an agent's channel, every call is confirmed with a dialog (the caller cannot bypass it with the message flag) |
| `inspect` | - | `ok` + inspection channel |
| `status` | - | `ok clients=1 windows=2 frames=.. presented=.. dialogs=0 secure=0` |

**Inspection** (`inspect`, for tests, later a screen reader and
magnifier): list of windows, the accessibility tree of a window in screen
coordinates, an action on a node, the colour of a screen pixel, and "wait
until settled" (everything composed and displayed).

## Window protocol (`libs/guiproto`)

A message = operation code `u32` + fields encoded with `libs/wire`
(little-endian, string = `u16` length + UTF-8). An unknown operation is
`NOT_SUPPORTED` (reply `ERROR`), bad data `INVALID_ARGS`; the compositor
drops the message and keeps the client connected (the number of reported
errors is limited to 64).

Application -> compositor:

| Op | Name | Fields |
|---|---|---|
| 1 | `HELLO` | version |
| 2 | `WINDOW_CREATE` | window, mode, w, h, title |
| 3 | `WINDOW_SET` | window, title, min size, button side and colour |
| 4 | `WINDOW_DESTROY` | window |
| 5 | `BUFFER_CREATE` | buffer, w, h, stride (in pixels) + VMO handle |
| 6 | `BUFFER_DESTROY` | buffer |
| 7 | `LAYER_CREATE` | layer, window, parent, kind |
| 8 | `LAYER_SET` | layer, property mask, values |
| 9 | `LAYER_DESTROY` | layer (with its children) |
| 10 | `DAMAGE` | buffer, rectangle |
| 11 | `ANIMATE` | layer, property, target, duration, curve |
| 12 | `A11Y_UPDATE` | nodes |
| 13 | `A11Y_REMOVE` | node ids |
| 14 | `REGIONS` | window, rectangles with kind |
| 15 | `COMMIT` | transaction, flags (`FRAME`), serial from `CONFIGURE`, group token |
| 16 | `SYNC_GROUP` | number of participants |
| 17 | `PONG` | serial |
| 18 | `ACTION_RESULT` | call, result |

Compositor -> application:

| Op | Name | Fields |
|---|---|---|
| 100 | `WELCOME` | version, screen w and h, scale in percent |
| 101 | `CONFIGURE` | window, w, h, state (focus, maximized), serial |
| 102 | `FRAME` | seq, deadline (ns), period |
| 103 | `PRESENTED` | transaction, time |
| 104 | `RELEASED` | buffer |
| 105 | `INPUT` | window, input records |
| 106 | `CLOSE` | window (the user clicked close) |
| 107 | `PING` | serial |
| 108 | `SYNC_TOKEN` | token |
| 109 | `ANIMATION_DONE` | layer |
| 110 | `A11Y_ACTION` | node, action, value |
| 111 | `ACTION_INVOKE` | call, action, arguments |
| 112 | `ERROR` | op, code |

Example (a window with one buffer):

```text
-> HELLO 1
<- WELCOME 1 1280 800 100
-> WINDOW_CREATE 1 STANDARD 400 300 "Demo"
-> BUFFER_CREATE 1 400 300 400         + VMO handle
-> LAYER_CREATE 1 window=1 parent=0 BUFFER
-> LAYER_SET 1 rect=0,0,400,300 buffer=1
-> A11Y_UPDATE [1 window "Demo"] [2 button "OK" layer 1 (150,250,100,30)]
-> COMMIT 1 FRAME
<- CONFIGURE 1 400 300 focused serial=1
<- PRESENTED 1
<- FRAME 1 deadline=...
```

## Display protocol (`guiproto::display`)

Compositor <-> graphics driver. The driver is simple: the compositor draws
the whole image into its VMO and says which rectangles changed.

| Op | Direction | Fields |
|---|---|---|
| `INFO` | compd -> driver | - |
| `MODES` | driver -> compd | outputs: w, h, enabled; capabilities (`3D`) |
| `ATTACH` | compd -> driver | output, w, h, stride (in bytes) + VMO handle (read-only) |
| `PRESENT` | compd -> driver | seq, rectangles |
| `PRESENTED` | driver -> compd | seq, time (ns) |

The driver copies the damaged rectangles into its DMA buffers (with an
IOMMU the only memory the device may access), sends
`TRANSFER_TO_HOST_2D` and `RESOURCE_FLUSH` and replies `PRESENTED` when
done.

## virtio-gpu driver

Initialization as for the other virtio devices ([Input](input.md#the-virtio-input-driver)):
features `VERSION_1`, `ACCESS_PLATFORM` (and `VIRGL` when offered), MSI-X
interrupts and queue 0 (control); the cursor queue is not used yet (the
compositor draws the cursor). 2D commands:

1. `GET_DISPLAY_INFO` - output sizes,
2. `RESOURCE_CREATE_2D` (format `B8G8R8X8_UNORM`, the output needs no alpha),
3. `RESOURCE_ATTACH_BACKING` - a list of DMA memory chunks (each at most
   4 MiB of contiguous memory, always whole rows),
4. `SET_SCANOUT`,
5. for every frame `TRANSFER_TO_HOST_2D` per rectangle + one
   `RESOURCE_FLUSH`.

`devmgr` starts the driver for PCI `1af4:1050` with a 40 MiB DMA memory
limit and reports the message `display` to init. In QEMU the device is
`virtio-gpu-pci,iommu_platform=on,xres=1280,yres=800` on both x86_64 and
aarch64.

## Native UI library (`libs/ui`)

One library for all applications, declarative in Rust:

```rust
impl App for Notes {
    type Msg = Msg;
    fn title(&self) -> String { "Notes".into() }
    fn view(&self) -> Node<Msg> {
        column([
            label("Notes").heading(),
            text_input(&self.title).placeholder("Title").on_change(Msg::Title),
            row([
                checkbox("Important", self.important).on_toggle(Msg::Important),
                spacer(),
                button("Save").on_click(Msg::Save),
            ]),
            scroll(list(self.items.clone(), self.selected).on_select(Msg::Select)).height(150),
        ])
    }
    fn update(&mut self, m: Msg) { /* change state */ }
}

ui::client::run(window_channel, Notes::default(), Mode::Standard, 420, 380)
```

From the description the library builds the elements, lays them out
(columns, rows, flexible `grow` elements and `spacer`), draws them into
buffers (`libs/gfx`, `libs/font`) and derives **both the layer tree and
the accessibility tree** - the application writes none of that by hand.
Input (keys, text, mouse, accessibility actions) becomes `Msg` messages.

| Element | Mouse | Keyboard | Accessibility |
|---|---|---|---|
| `label` | - | - | `label` |
| `button` | click | Enter, space | `button`, `activate` |
| `text_input` | click = cursor | text, Backspace, Delete, arrows, Home, End, Enter = `on_submit` | `text-input`, `set-value` |
| `checkbox` | click | space, Enter | `checkbox`, `toggle` |
| `list` | click an item | up and down arrows | `list` and `list-item` |
| `progress` | - | - | `progress` (value in percent) |
| `column`, `row`, `scroll`, `spacer` | wheel in `scroll` | - | `group` |

Tab moves the focus (Shift+Tab back). A **scroll area** (`scroll`) is a
layer of its own: the wheel changes only the layer property `scroll` and
the compositor moves the content; the application redraws nothing.

The `ui::client` runtime (feature `os`) keeps the window: double buffers
in VMOs, layers, the whole accessibility tree after every change,
`COMMIT` with the `CONFIGURE` acknowledgement, replies to `PING` and
actions. The lower layer `ui::client::Window` serves applications that
draw on their own (the terminal window).

## Fonts (`libs/font`)

Own TrueType (`.ttf`) reader: tables `head`, `hhea`, `maxp`, `cmap`
(formats 4 and 12), `loca`, `glyf` (simple and composite glyphs), `hmtx`,
`kern` (format 0). The rasterizer flattens quadratic curves into lines and
computes **pixel area coverage** (anti-aliasing without hinting). Glyphs
are cached per size (`gfx::text`).

A font file is untrusted input: every read checks bounds, a broken glyph
is an empty glyph, never a panic (a test overwrites random bytes of the
file). No shaping (Arabic, Indic scripts) and no hinting.

The system font is **Tuffy** (Thatcher Ulrich, public domain): it covers
Latin including Czech and is bundled in `libs/font/fonts/`.

## Terminal window

The 6b terminal becomes a window in the GUI on top of its native
protocol. The `gui-term` demo is the prepared window: a **custom title
bar** (the app draws the tab "Session 1" and declares a `DRAG` region with
a `NO_DRAG` exception for the tab; the badge and buttons are drawn by the
system), the content is a `term::screen` character grid (Noto Sans Mono,
the same as on the console), and the accessibility tree has a `terminal`
node with the visible text. A line can be entered from the keyboard, with
the accessibility action `set-value` or with the action `terminal.run`
(requires confirmation).

Connecting to the session service (`termd`, protocol `term::proto`) is
done by the terminal thread: instead of the local line editor it sends
`Run` and draws the output blocks into the same grid.

## 3D: Venus and virgl (status)

The system's target API is Vulkan, so in a virtual machine **Venus**
(Vulkan over virtio-gpu). Temporarily **virgl** (Gallium/host OpenGL) only
for compositor composition.

| What | Status |
|---|---|
| `cargo xtask run/test --gpu 2d\|gl\|venus` | device choice: `virtio-gpu-pci`, `virtio-gpu-gl-pci` (virgl), `virtio-gpu-gl-pci,venus=on,blob=on,hostmem=256M`; without a window the `egl-headless` display |
| driver | negotiates `VIRGL`, `RESOURCE_BLOB`, `CONTEXT_INIT` and logs the capsets, e.g. `virgl(v1) virgl2(v2)` |
| `vgpu::virgl` | own encoder of the virgl command stream (state objects, TGSI shaders, vertices, draws, scissor) and virtio-gpu 3D commands, host tests |
| GPU composition | pending: on this host (QEMU 11.1 for Windows) the virtual machine stalls after the kernel starts whenever OpenGL is on (`egl-headless`, `gtk,gl=on`, `sdl,gl=es`) |
| Venus | QEMU for Windows lacks it: "Render server support was not enabled in virglrenderer" |

⚠️ **Where to test Venus:** Linux with a physical GPU and QEMU +
virglrenderer with the render server (or crosvm). WSL2 only with Vulkan
translated to D3D12 (uncertain). Not VMware - its SVGA3D is a different
protocol. On real hardware Vulkan comes with the Mesa port (milestone 7).

## Status and deviations from the design

| What | Now | Why it is fine / when it changes |
|---|---|---|
| rendering | software compositing on virtio-gpu 2D | 3D (virgl) is the next step, see risks |
| QoS `interactive` for the compositor | the kernel has no thread class API yet | input priority is in the compositor loop; the scheduler adds QoS |
| group opacity | multiplied into children (no intermediate buffer) | overlapping children of a translucent group blend with each other too; the GPU backend will do it properly |
| vsync | `PRESENTED` time from the driver (end of `RESOURCE_FLUSH`) | virtio-gpu 2D does not report vsync |
| cursor | drawn by the compositor into the frame | hardware cursor (queue 1) comes with 3D |
| restoring windows after restart | none | in the design ([Graphics](architecture.md#graphics)) |
| compositor restart | the supervisor only watches it | windows belong to the compositor; after a crash the apps would have to reconnect |
| commit held in a group | applied together with the whole group, even if the client sent another plain commit meanwhile | a client in a group should wait for `PRESENTED`; strict per-client commit order comes with the GPU backend |
| display driver crash | the compositor keeps running without a picture (windows, dialogs, actions) | no driver restart yet |

## Tests

- **host** (part of `cargo xtask test`): `cargo test -p guiproto -p gfx
  -p font -p vgpu -p comp -p ui` - message encoding and robustness against
  broken messages, colours and blending, clipping, damaged regions, text,
  rasterizer (exact square coverage, caron above the letter, randomly
  corrupted font file), virtio-gpu commands, transactions and their
  atomicity, transaction groups (including the deadline), synchronized
  resize, input routing and focus, moving a window by its title without
  the app, custom title regions, badge above application content,
  frameless badge, panel, dialogs only from physical input, password,
  secure screen, new device (BadUSB), actions (checks, confirmation, app
  exit), inspection, compositor-side scrolling and animation, ping, UI
  layout and controls, text editing with diacritics. The compositor and UI
  tests save `target/comp-scene.png` and `target/ui-window.png` for a
  visual check.
- **QEMU** (`cargo xtask test`, x86_64 and aarch64, suite `gui` last):
  compositor on virtio-gpu, the `gui-demo` window, a click through the
  accessibility tree, screen pixel colours (app background, badge), a
  click with synthetic input (`inject`), manifest actions (bad argument
  rejected, `note.add`, an agent action waits for confirmation), a system
  dialog rejects a synthetic Enter and accepts physical keys from QEMU
  (QMP), the secure screen after Ctrl+Alt+Del from QEMU, the terminal
  window with a custom title bar.
