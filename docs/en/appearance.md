# Appearance and controls

A design of how MagnumOpus-OS looks and how it is operated: the design system
(colors, typeface, grid, icons, motion), windows, system elements, four key
applications and accessibility. It builds on the agreed principles in
[Architecture](architecture.md#custom-window-look-custom-title-bar)
(decorations drawn by the system, trust badge, system dialogs in their own
layer, [smoothness](architecture.md#user-interface-responsiveness-plan),
[task overview](architecture.md#task-overview-agreed)).

This page is a **design**, not a description of code. The compositor `compd`,
the UI library `libs/ui` and the terminal window are built by milestone 6c
(the *Graphics and windows* page). They take the appearance and behavior from
here; the values come as data from `design/tokens.toml`.

**Clickable prototypes** are in `design/` (open `design/index.html` in a
browser). Every page has a switch for theme, high contrast, font size and
reduced motion.

| Prototype | What it shows |
|---|---|
| `design/design-system.html` | colors with a contrast table, typography, grid, corner radii, shadows, icons, motion, components |
| `design/okna.html` | window anatomy, trust badge (removing permissions), window modes, states |
| `design/plocha.html` | bar in 6 positions, window switching, notifications, system dialogs, file picker, secure screen, task manager, agent in the bar |
| `design/terminal.html` | output blocks, tables with a schema, type-based completion, dry run and undo, errors, task panel, notation B / C |
| `design/soubory.html` | file versions, "Try aside", applying selected changes, conflict, agent working aside |
| `design/nastaveni.html` | security profiles, individual options, appearance, accessibility |
| `design/prihlaseni.html` | lock screen, account selection, security phrase, mode with Ctrl+Alt+Del |
| `design/varianty-stylu.html` | the three original style variants (decision archive) |

## Style decisions (agreed)

The user chose from three variants side by side (`design/varianty-stylu.html`):
A Calm (minimalist), B Technical (dense, lines instead of shadows),
C Warm (more playful, strong accent).

| # | Question | Decision | Consequence |
|---|---|---|---|
| 1 | overall character | **C, Warm** | purple accent, warm background, corner radius 18 px for windows and 10 px for controls, pill-shaped buttons, colored soft shadows |
| 2 | bar position | **configurable**, default **inset strip at the bottom** | 6 positions: inset at the bottom, floating dock, full-width at the bottom, left, right, top |
| 3 | icon style | **line, rounded** | 1.75 px stroke, 24 px grid, color from the text |
| 4 | typeface | **Inter + JetBrains Mono** | both under SIL OFL 1.1; Tuffy and Noto Sans Mono stay as a fallback |
| 5 | default theme | **by time of day** | light during the day, dark after sunset within the configured time zone |
| 6 | AI agent marking | **reserved color + icon + word** | coral color only for the agent, always with an icon and the word "agent" (never by color alone) |

## Principles

1. **Trust is visible.** An application's identity and its permissions are
   always in the same place (the badge at the left of the title bar) and in a
   color that no application gets. What the system draws, an application
   cannot imitate.
2. **Smoothness over effects.** The compositor animates (opacity, offset,
   scale of layers), not applications. An animation does not wait for the
   application and never blocks input.
3. **Accessible from the ground up.** The accessibility tree is part of the
   window protocol. Everything works from the keyboard, and the font can be
   enlarged to 200 % without clipping.
4. **An agent is always recognizable.** What an AI agent does is marked with
   the reserved color, an icon and the word.
5. **Safe experimenting.** Risky changes go as a dry run or "aside" in a
   clone; the UI offers them as the ordinary path, not as a hidden option.
6. **Our own face.** Inspiration yes, a copy of someone else's system no:
   rounded shapes, warm background, the trust badge and the bar with tasks
   are ours.

## Design system

### Tokens as data

All values (colors, typefaces, spacing, radii, shadows, motion, layers) are
in a single file, `design/tokens.toml`. The UI library loads them as data, so
changing a color or a spacing needs no change in the code of the elements.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Token flow">
<defs><marker id="vz1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="80" width="170" height="70" rx="6"/><text class="t b" x="95" y="106" text-anchor="middle">design/tokens.toml</text><text class="m" x="95" y="124" text-anchor="middle">single source of truth</text><text class="m" x="95" y="140" text-anchor="middle">light, dark, contrast</text>
<path class="ln" d="M180 100 L268 60" marker-end="url(#vz1)"/>
<path class="ln" d="M180 130 L268 170" marker-end="url(#vz1)"/>
<rect class="box" x="270" y="20" width="200" height="74" rx="6"/><text class="t b" x="370" y="44" text-anchor="middle">build-tokens.mjs</text><text class="m" x="370" y="62" text-anchor="middle">WCAG contrast check</text><text class="m" x="370" y="78" text-anchor="middle">failure = exit code 1</text>
<rect class="box" x="270" y="136" width="200" height="74" rx="6"/><text class="t b" x="370" y="160" text-anchor="middle">libs/ui (6c)</text><text class="m" x="370" y="178" text-anchor="middle">loads TOML at startup</text><text class="m" x="370" y="194" text-anchor="middle">compositor switches theme</text>
<path class="ln" d="M470 57 L558 57" marker-end="url(#vz1)"/>
<rect class="ok" x="560" y="20" width="190" height="74" rx="6"/><text class="t b" x="655" y="44" text-anchor="middle">assets/tokens.css + .js</text><text class="m" x="655" y="62" text-anchor="middle">prototypes in design/</text><text class="m" x="655" y="78" text-anchor="middle">contrast table</text>
<path class="ln" d="M470 173 L558 173" marker-end="url(#vz1)"/>
<rect class="ok" x="560" y="136" width="190" height="74" rx="6"/><text class="t b" x="655" y="160" text-anchor="middle">applications, decorations</text><text class="m" x="655" y="178" text-anchor="middle">same values everywhere</text><text class="m" x="655" y="194" text-anchor="middle">trust-* for system only</text>
</svg>
<figcaption>Tokens are data. The prototypes and the UI library take their values from the same file.</figcaption>
</figure>

Format example (a subset of TOML: sections, `key = "text"` or a number):

```toml
[radius]
control = 10
window = 18

[color.light]
accent = "#6b3fd6"
on_accent = "#ffffff"

[contrast]
on_accent = "on_accent accent 4.5"   # foreground, background, minimum
```

```bash
node design/tools/build-tokens.mjs          # writes design/assets/tokens.css and tokens.js
node design/tools/build-tokens.mjs --check  # check only (contrast + whether the output is up to date)
```

Units: `px` are logical pixels before HiDPI scaling, time is in ms.
The compositor scales fractionally (125 %, 150 %), so dimensions are
multiples of 4.

### Colors

Colors are **semantic** (roles), not shades. Every theme (light, dark, high
contrast) has the same keys.

| Role | Light | Dark | Use |
|---|---|---|---|
| `bg` | `#fbf1ea` | `#17111c` | desktop background (with a warm gradient `wall-a`, `wall-b`) |
| `surface` | `#fffdfb` | `#221a28` | window surface |
| `surface-2` | `#fff4ee` | `#2b2232` | raised surface: title bar, sidebar |
| `surface-3` | `#f6e9e2` | `#362b3e` | hover, fill of controls |
| `border` / `border-strong` | `#eedad0` / `#8f7a87` | `#3d3146` / `#8a7894` | divider line / control boundary (3 : 1) |
| `ink` / `ink-muted` | `#2a1f2d` / `#665569` | `#f6edf3` / `#bba9c4` | text / secondary text |
| `accent` / `accent-soft` / `accent-ink` | `#6b3fd6` / `#efe7ff` / `#5027bd` | `#b79bff` / `#382b5c` / `#d8c9ff` | main action / selection / accent text |
| `focus` | `#6b3fd6` | `#c9b4ff` | focus ring 2 px with a 2 px offset |
| `trust-bg` / `trust-ink` / `trust-border` | `#ffe4d6` / `#7a2806` / `#e9a586` | `#4d2414` / `#ffd2bf` / `#9a5130` | **system only**: trust badge, system dialogs |
| `ai` / `ai-soft` / `ai-ink` | `#c3264d` / `#ffe4ea` / `#9c1b3d` | `#ff8fa8` / `#4a1d2a` / `#ffc2cf` | **AI agent only** |
| `ok`, `warn`, `danger` | `#1b7543`, `#8a5000`, `#bf283f` | `#6fd39a`, `#f2b54f`, `#ff7d92` | states (with a `-soft` background) |
| `term-*` | 9 colors | 9 colors | terminal: background, text, muted, red to cyan |

**Reserved colors:**

- `trust-*` belongs to the compositor only (badge, system dialog strip,
  secure screen). The UI library does not offer it to applications. An
  application may draw a similar shade into its own buffer, but not in place
  of the badge and not into the dialog layer (see
  [System dialogs](#system-dialogs)).
- `ai-*` is only for the AI agent, always together with the agent icon and
  the word "agent". A color-blind user thus recognizes the agent by the icon
  and the text.

**Contrast (WCAG 2.2):** text at least 4.5 : 1 (1.4.3), control boundaries,
icons and focus at least 3 : 1 (1.4.11). In high contrast, text 7 : 1 (AAA)
and boundaries 4.5 : 1. The required pairs are in the `[contrast]` section
of the tokens; the generator verifies them for all three themes (currently
102 of 102 pairs pass) and fails with an error when one does not.

**Default theme by time of day:** light from sunrise to sunset in the
configured time zone (the system knows the time and the zone, see
[Real time](time.md)), dark otherwise. The transition is a 240 ms
cross-fade and is postponed while the user is typing (for at most 2 minutes).

### Typography

| Typeface | Use | Why |
|---|---|---|
| **Inter** | the whole UI | designed for screens, large x-height, tabular numerals, full Czech support |
| **JetBrains Mono** | terminal, code, paths | distinct `0 O`, `1 l I`, wide characters, legible from 12 px |
| Tuffy, Noto Sans Mono | fallback | already bundled in `libs/font/fonts/` (6c) |

Type scale (size / line height in px, weight):

| Token | Size | Line height | Weight | Use |
|---|---|---|---|---|
| `caption` | 12 | 16 | 500 | secondary captions, labels |
| `small` | 13 | 18 | 400 | metadata, hints |
| `body` | 14 | 20 | 400 | basic UI text |
| `body-lg` | 16 | 24 | 400 | dialog text, introductory paragraphs |
| `title` | 18 | 24 | 600 | dialog or window heading |
| `heading` | 22 | 28 | 650 | application page heading |
| `display` | 32 | 40 | 700 | welcome, large headings |
| `mono` | 13 | 20 | 400 | terminal (one cell = one character width) |
| `clock` | 64 | 72 | 300 | clock on the lock screen |

- Tabular numerals (`tnum`) in tables, times and clocks.
- **No ligatures** in the terminal: one character = one grid cell.
- **Font size** (Accessibility): 100, 125, 150, 200 %. The whole interface
  scales and the layout is recomputed, nothing is clipped.

⚠️ **Typeface risks (stated up front):**

- `libs/font` reads kerning only from the `kern` table (format 0). Inter and
  JetBrains Mono have kerning in `GPOS`; without `GPOS` support the text will
  have no kerning (with Inter it is visible in pairs such as "Te", "Vo").
  Solution: add reading of `GPOS` pair kerning (type 2), or convert the
  kerning into a `kern` table at build time.
- Inter is mainly a variable font; **static instances** (400, 500, 600, 700)
  in TTF format with `glyf` outlines, which the rasterizer can handle, are to
  be bundled. Size: ~300 kB per instance, ~1.5 MB in total with
  JetBrains Mono.
- Without hinting, text below 12 px is softer. That is why the smallest size
  is 12 px and the base is 14 px.

### Grid and dimensions

Everything lies on a **4 px grid**. Spacing: 2, 4, 8, 12, 16, 20, 24, 32, 40,
48, 64 px (`s1` to `s11`). Smaller gaps inside a control than between
controls.

| Token | Value | Use |
|---|---|---|
| `hit-min` | 40 px | smallest click and touch target (touch is the default input); the "larger targets" option gives 48 px |
| `control-h` | 36 px | button, field (dense variant 28 px) |
| `title-h` | 40 px | window title bar |
| `title-btn` | 28 px | visible window button; the 40 px target extends into the title bar edge |
| `badge-h` | 26 px | trust badge |
| `bar-h` | 52 px | bar; 8 px offset from the edge |
| `resize-edge` | 6 px | invisible edge for resizing (corner 18 px) |

### Radii and shadows

| Token | Value | Use |
|---|---|---|
| `xs` | 6 px | key, small label |
| `badge` | 8 px | trust badge |
| `control` | 10 px | field, list item |
| `card` | 14 px | card, menu, notification |
| `window` | 18 px | window, bar, dialog |
| `pill` | 999 | buttons and labels |

The shadow tells what is higher: inactive window < active window < menu <
dialog. In the light theme the window shadow has a purple tint
(`rgba(107,63,214,.16)`), in the dark one it is black. **The compositor does
not draw shadows by blurring every frame**: for every kind of shadow and
radius it has a precomputed texture made of 9 pieces (corners, edges,
center), so moving a window does not make the shadow more expensive. In high
contrast, shadows are replaced by a 2 px border.

### Icons

Line and rounded: 24 px grid, 1.75 px stroke, rounded caps and joins, 2 px
margin. Sizes 16, 20, 24 and 32 px. The color comes from the text
(`currentColor`), so the icons switch to the dark and the contrast theme on
their own. The set has 64 icons (`design/assets/mo.js`, object `ICONS`); the
agent icon is two sparks.

An icon never carries meaning alone: in the bar it has a label or tooltip, in
the accessibility tree a name.

### Motion

A frame at 60 Hz is 16.7 ms. The compositor schedules by the frame deadline
([Smoothness](architecture.md#user-interface-responsiveness-plan));
animations change only the properties of layers that it composes itself
(opacity, offset, scale, clip). The application draws nothing during an
animation, so the animation does not stall even when the application is
computing.

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Frame budget">
<rect class="box" x="10" y="40" width="70" height="44" rx="4"/><text class="t b" x="45" y="66" text-anchor="middle">input</text>
<rect class="acc" x="82" y="40" width="300" height="44" rx="4"/><text class="t b" x="232" y="60" text-anchor="middle">compositing damaged regions</text><text class="m" x="232" y="76" text-anchor="middle">~5 ms in software, less with 3D</text>
<rect class="ok" x="384" y="40" width="120" height="44" rx="4"/><text class="t b" x="444" y="66" text-anchor="middle">flush ~2 ms</text>
<rect class="box" x="506" y="40" width="244" height="44" rx="4"/><text class="t" x="628" y="66" text-anchor="middle">slack until the deadline</text>
<path class="ln" d="M10 104 L750 104"/><text class="m" x="10" y="122">0 ms</text><text class="m" x="750" y="122" text-anchor="end">16.7 ms (vsync)</text>
<rect class="warn" x="82" y="132" width="420" height="26" rx="4"/><text class="m" x="292" y="150" text-anchor="middle">the application draws its buffer in parallel, off the animation path</text>
</svg>
<figcaption>Approximate budget of one frame. Animations are only changes of layer properties.</figcaption>
</figure>

| Token | Duration | Curve | Use |
|---|---|---|---|
| `fast` | 90 ms | standard | hover, press, switch |
| `base` | 160 ms | standard | menu, state change |
| `slow` | 240 ms | enter | opening a window, dialog, theme change |
| `max` | 320 ms | - | upper bound |
| `reduced` | 90 ms | cross-fade | replacement of all offsets under "Reduce motion" |

Curves: standard `cubic-bezier(0.2, 0, 0, 1)`, enter
`(0.05, 0.7, 0.1, 1)`, exit `(0.3, 0, 1, 1)`.

**Rules:**

- Leaving is shorter than arriving (close = `fast`, open = `slow`).
- Every animation can be interrupted: new input ends it in its current state
  and continues from there.
- Moving and resizing a window follow the pointer directly, without delay
  and without springiness.
- Nothing flashes faster than 3 times per second (WCAG 2.3.1).
- When the compositor misses the deadline, it skips an intermediate frame of
  the animation, but never delays the response to input.
- **Reduce motion** (Accessibility or system setting): offsets and scales are
  replaced by a 90 ms cross-fade.

## Windows

### Anatomy

<figure class="diagram">
<svg viewBox="0 0 760 260" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Window anatomy">
<rect class="box" x="60" y="20" width="640" height="200" rx="18"/>
<rect class="acc" x="60" y="20" width="640" height="40" rx="18"/>
<rect class="warn" x="72" y="27" width="150" height="26" rx="8"/><text class="m" x="147" y="45" text-anchor="middle">badge: Notes · folder</text>
<text class="t b" x="380" y="45" text-anchor="middle">shopping.md - Notes</text>
<circle class="box" cx="616" cy="40" r="14"/><circle class="box" cx="646" cy="40" r="14"/><circle class="err" cx="676" cy="40" r="14"/>
<text class="t" x="380" y="140" text-anchor="middle">content: drawn by the app, the accessibility tree describes everything</text>
<text class="m" x="147" y="80" text-anchor="middle">1 badge (system)</text>
<text class="m" x="380" y="80" text-anchor="middle">2 title (text from the app)</text>
<text class="m" x="646" y="80" text-anchor="middle">3 buttons (system)</text>
<text class="m" x="700" y="236" text-anchor="end">5 edge 6 px, corner 18 px (compositor)</text>
<text class="m" x="60" y="236">4 content (app)</text>
<path class="ln" d="M40 20 L40 60"/><text class="m" x="34" y="44" text-anchor="end">40</text>
</svg>
<figcaption>A standard window. The badge, title, buttons and edges are held by the compositor, so they work even with a hung application.</figcaption>
</figure>

1. **Trust badge** on the left: the name from the manifest (passed on by
   init, not by the application) and icons of what the application may do
   right now. Click or Enter opens the overview.
2. **Title** in the center (the left is reserved for the badge), at most 256
   characters, truncated with three dots. Double click = maximize.
3. **Window buttons** minimize, maximize, close; the application only picks
   the side (left / right).
4. **Content** is drawn by the application into its buffers.
5. **Edges** for resizing are held by the compositor.

### Modes

| Mode | The app draws | The system keeps | Controls |
|---|---|---|---|
| **standard** (default) | content only | frame, title, badge, buttons | usual |
| **custom title bar** | the title bar area too (tabs, search, tools) | badge on the left, window buttons, moving via declared `DRAG` / `NO_DRAG` regions | 44 px title bar; the compositor drags the window itself, even when the app does not respond |
| **frameless** | everything | badge at the top edge on hover and for 3 s on a permission change | moving only via the `DRAG` region |
| **full screen** | everything | on hovering the top edge a strip with the badge and an "Exit full screen" button slides down; **Esc held for 1 s** always exits | the secure attention shortcut and system dialogs always work |

### Trust badge

- Shape: a 26 px pill in the `trust-*` colors, a shield icon, the application
  name, and after a separator the icons of the currently valid permissions
  (folder, network, camera, microphone, screen capture). A **red dot** =
  the device is running right now.
- System applications have the word "System" instead of a name and a shield
  with a check mark.
- Click, Enter or Space opens the **permission overview** (drawn by the
  compositor in the menu layer): the application, signature, a list of
  permissions with the time of last use and a **Remove** button. Removal
  applies immediately; the application asks for the right again with a
  system dialog.
- When a permission changes, the badge pulses three times (1 s) and the
  screen reader announces the change.
- Accessibility tree: role `trust-badge`, name "Notes, verified
  application. May: Documents folder".

### Window states

| State | Look |
|---|---|
| **active** | title bar `surface-2`, bold name, `window` shadow, badge in full colors; has keyboard focus |
| **inactive** | title bar blends with the content, name in the secondary color, `window-inactive` shadow; the content is **not dimmed** (it can be read) |
| **not responding** | after 3 s without `PONG` the compositor grays out the content and shows the options "Wait" / "Quit"; moving, minimizing and closing keep working |
| **agent acting** | 3 px frame in the `ai` color and a label "agent is acting in this window" at the top; a "Stop" button in the bar |

## System elements

### Compositor layers

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Compositor layers">
<rect class="err" x="120" y="10" width="520" height="30" rx="6"/><text class="t b" x="380" y="30" text-anchor="middle">2000 secure screen (Ctrl+Alt+Del)</text>
<rect class="warn" x="120" y="44" width="520" height="30" rx="6"/><text class="t b" x="380" y="64" text-anchor="middle">1000 system dialogs</text>
<rect class="warn" x="120" y="78" width="520" height="30" rx="6"/><text class="t" x="380" y="98" text-anchor="middle">900 dimming (scrim) over everything, even the bar</text>
<rect class="acc" x="120" y="118" width="520" height="30" rx="6"/><text class="t" x="380" y="138" text-anchor="middle">400 notifications</text>
<rect class="acc" x="120" y="152" width="520" height="30" rx="6"/><text class="t" x="380" y="172" text-anchor="middle">300 menus, permission overview, launcher</text>
<rect class="acc" x="120" y="186" width="520" height="30" rx="6"/><text class="t" x="380" y="206" text-anchor="middle">200 bar</text>
<rect class="box" x="120" y="220" width="520" height="30" rx="6"/><text class="t" x="380" y="240" text-anchor="middle">100 application windows (+ badge and buttons over the content)</text>
<rect class="box" x="120" y="254" width="520" height="30" rx="6"/><text class="t" x="380" y="274" text-anchor="middle">0 desktop</text>
<text class="m" x="110" y="64" text-anchor="end">compositor only</text>
<text class="m" x="110" y="30" text-anchor="end">compositor only</text>
<text class="m" x="650" y="172">system</text>
<text class="m" x="650" y="240">applications</text>
</svg>
<figcaption>Compositing order. Only the compositor may draw into layers 900 and above.</figcaption>
</figure>

### Bar

The default position is an **inset strip at the bottom** (8 px from the
edges, 18 px radius, 52 px height). The position can be changed in
Settings > Appearance or with a right click on the bar: floating dock,
full-width at the bottom, left, right, top (40 px).

Content from left to right:

| Part | What it does |
|---|---|
| **Applications** | launcher: search of applications, files and actions (action API), application grid, account, lock, shut down |
| **Windows** | pinned and running applications; a dot under the icon = running, a longer accent one = active; click = activate / minimize |
| **Agent** | when an AI agent is acting, a coral pill "Agent is controlling Files" with a **Stop** button |
| **Tasks** | number of running tasks and the agent label; a click opens the task manager |
| **Status** | network, volume, notifications with the count of new ones |
| **Clock** | time and date (tabular numerals) |

In the vertical position the labels are hidden and the icons with a tooltip
remain; the clock is written vertically.

### Window switching

**Alt+Tab** opens a switcher in the middle of the screen: window previews in
most-recently-used order, the previous window is preselected. Another Tab
(Shift+Tab backwards) or the arrows move the selection, releasing Alt
switches, Esc cancels. The previews are layers that the compositor already
has (nothing is redrawn).

### Notifications

They slide in at the top right (24 px offset + cross-fade, 240 ms) and after
6 s disappear into the notification center (the bell in the bar). An agent
notification has a left strip in the `ai` color and the agent icon. For the
screen reader they are in a `status` region (announced but not
interrupting). Muting until a given time is in the notification center.

### System dialogs

A permission, password, file picker or new device dialog is drawn by the
compositor in layer 1000. A forgery inside an application window always
looks different, because:

1. **Dimming over everything, even the bar.** An application cannot dim the
   bar or other windows.
2. **A "System" strip** at the top in the `trust-*` colors with the user's
   **security phrase** (set in Settings > Security). An application does not
   know the phrase.
3. **Physical input only**: synthetic input (`SYNTHETIC`) is ignored by the
   dialog.
4. **300 ms protection**: the buttons are inactive for the first 300 ms (the
   second click of a double click or a half-typed key confirms nothing).
5. **Ctrl+Alt+Del** always shows the real system.

Controls: Tab and the arrows change the choice, Enter confirms, Esc selects
the last choice (denial). Focus stays inside the dialog until it closes.

Kinds in the prototype (`design/plocha.html`):

| Dialog | Choices |
|---|---|
| **permission** (camera) | Just now / Always for this app / Deny |
| **password** (relaxing security) | password field; Confirm / Cancel |
| **file picker** (powerbox) | system file list; the application gets a handle **only to the selected files**, read-only |
| **new keyboard** (BadUSB) | a 3-digit code is typed **on the new keyboard**; until then its key presses are discarded; Allow / Block the device |

### Secure screen

Ctrl+Alt+Delete is caught by `inputd` before the compositor. The screen in
layer 2000 shows the "System" strip with the phrase and the choices Lock,
Task manager, Switch user, Sign out, Back. Only physical input closes it.

### Task manager

A window with the same data as the `tasks` command ([Task overview](architecture.md#task-overview-agreed)):

| Column | Content |
|---|---|
| Task | human-readable name |
| Started by | you (terminal), application, service, **agent** (label), unregistered process |
| State, progress | running / in the background / done 2 min ago; progress bar (agent in the `ai` color) |
| Memory | consumption from the job budget |
| Actions | pause, cancel; detail: attach, log |

Filters All / Mine / Agent / System. Agent rows have a left strip `ai` and a
light `ai-soft` background. A process without registration is shown as
"unknown process in job N". Rows can be navigated with the arrows.

## Applications

### Terminal

A window in the **custom title bar** mode: session tabs (sessions belong to
the `termd` service, a detached session has a label), the notation switch
**B / C**, the system badge and buttons. Content:

- **Output blocks**: every command is a block with a left strip by state
  (`ok` green, running accent, error red, dry run yellow, agent coral), a
  command line, duration and time. The block actions (collapse, copy, run
  again, compare two runs) show on hover or focus.
- **Tables with a schema**: columns with a type (`size` size in units,
  `mtime` local time), sorting by clicking the header, tabular numerals.
- **Type-based completion** (Tab): commands in the command position, input
  fields from the schema of the previous command after `where`, `sort`,
  `select`, paths elsewhere. The list is a `listbox` at the cursor; arrows,
  Enter / Tab, Esc. A gray suggestion from history after the cursor is
  accepted with the right arrow.
- **Dry run and undo**: a command over the lock limit (default 20 files)
  shows a block with the changes and the buttons **Apply** / **Discard**;
  after applying, an **Undo** button (`undo`) appears in the hint line.
- **Errors**: a frame with a code (`NOT_FOUND`, `SH_UNKNOWN`), a description,
  the place in the line, a suggested fix ("Did you mean `~/Projekty`?") and
  an `explain` link.
- **Task panel** on the right: async and background tasks with progress;
  agent tasks with a label and a coral strip.
- **Notation B / C**: the switch in the title bar or Ctrl+Shift+F rewrites
  the displayed commands into the other notation (done by the `shlang`
  formatter).

The terminal colors are `term-*` from the tokens and follow the theme. The
6b thread made an animated terminal design as a demo in the conversation; the
`design/terminal.html` prototype unifies it with the design system and
supports the same looped playback of demos (with pause) as well as manual
typing.

### Files

The window: a toolbar (back, forward, path, search, **Try aside**), a sidebar
(Places, Storage = FS v3 repositories, Aside, Trash), a list and a details
panel.

- **Versions**: a timeline of versions of the selected file (automatic
  versions, commits with a message, changes taken over from an agent).
  Selecting a version offers **Restore** (the current version goes to the
  trash, nothing is lost), **Copy alongside**, **Compare**.
- **Try aside** creates a clone of the folder (`aside_begin`). A yellow strip
  says "You are working aside... Changes will not take effect until you
  apply them". Changed rows have a label (deleted, renamed, new).

<figure class="diagram">
<svg viewBox="0 0 760 220" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Try aside in the file manager">
<defs><marker id="vz2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="80" width="120" height="56" rx="6"/><text class="t b" x="70" y="104" text-anchor="middle">folder</text><text class="m" x="70" y="122" text-anchor="middle">original</text>
<path class="ln" d="M130 108 L178 108" marker-end="url(#vz2)"/><text class="m" x="134" y="98">aside</text>
<rect class="warn" x="180" y="80" width="140" height="56" rx="6"/><text class="t b" x="250" y="104" text-anchor="middle">clone</text><text class="m" x="250" y="122" text-anchor="middle">you or the agent edit</text>
<path class="ln" d="M320 108 L368 108" marker-end="url(#vz2)"/>
<rect class="acc" x="370" y="80" width="150" height="56" rx="6"/><text class="t b" x="445" y="104" text-anchor="middle">diff review</text><text class="m" x="445" y="122" text-anchor="middle">select changes</text>
<path class="ln" d="M520 96 L578 52" marker-end="url(#vz2)"/>
<path class="ln" d="M520 108 L578 108" marker-end="url(#vz2)"/>
<path class="ln" d="M520 122 L578 168" marker-end="url(#vz2)"/>
<rect class="ok" x="580" y="24" width="170" height="50" rx="6"/><text class="t b" x="665" y="46" text-anchor="middle">apply selected</text><text class="m" x="665" y="63" text-anchor="middle">atomically, as a commit</text>
<rect class="err" x="580" y="84" width="170" height="50" rx="6"/><text class="t b" x="665" y="106" text-anchor="middle">conflict</text><text class="m" x="665" y="123" text-anchor="middle">merge / save alongside</text>
<rect class="box" x="580" y="144" width="170" height="50" rx="6"/><text class="t b" x="665" y="166" text-anchor="middle">discard clone</text><text class="m" x="665" y="183" text-anchor="middle">in a single step</text>
</svg>
<figcaption>Try aside: changes are made in a clone and only the selected ones go into the original.</figcaption>
</figure>

- **Review and apply** opens a list of changes with checkboxes. A conflict
  (the file has meanwhile changed in the original too) is not applied on its
  own: it offers **Merge** or **Save alongside** (`aside_apply` only reports
  conflicts). After applying, a green strip "N changes applied" with an Undo
  button.
- **Agent aside**: the folder where the agent works has the agent icon in
  the sidebar and a coral strip "The assistant is tidying this folder aside.
  You see the original; the agent has access only to the clone." with the
  buttons **Review the proposal** and **Stop the agent**.

### Settings and security profiles

Categories on the left (role `tablist`, up and down arrows), content on the
right. A **profile** sets the options at once; every option can be changed
separately. A changed option has a label "modified", the value according to
the profile and a **Revert** button; the profile shows "modified: N" and
offers "Revert all to the profile". **Relaxing** (the Relaxed profile or a
more permissive value) is confirmed by a system dialog with a password;
tightening is not.

| Option | Standard (default) | Strict | Relaxed |
|---|---|---|---|
| verification of a new input device (BadUSB) | keyboards only | all input devices | off |
| new network connection of an application | ask for applications without a manifest | ask for every new destination | per manifest, without asking |
| running programs | unsigned ones ask | signed only | allow all |
| synthetic input | ask every time | forbid | allow with an indicator |
| screen capture | remember the permission | ask every time | remember the permission |
| lock of dangerous operations in the terminal | over 20 files | over 5 files | over 100 files |
| cloud AI models | off | off and locked | allow with a prompt |
| agent works only aside | always diff review | review + password on apply | apply without review up to 5 changes |
| automatic lock | after 5 minutes | after 1 minute | after 15 minutes |

Synthetic input is **visible** in all profiles (the bar shows who controls
the keyboard); the profile changes only whether the system asks.

Other pages of the prototype: **Appearance** (theme by time of day / light /
dark, bar position with previews, typeface) and **Accessibility** (font size,
high contrast, screen reader, reduce motion, always highlight focus, larger
click targets). In the prototype these options really switch the appearance.

### Sign-in

- **Lock screen**: a large clock (64 px, weight 300), the date, a summary of
  notifications (without content) and a label that an agent is waiting for
  review. A click or a key opens sign-in; in the Strict profile
  **Ctrl+Alt+Del** is required.
- **Sign-in**: account selection (role `radiogroup`), a password field as a
  pill with an arrow button, a Caps Lock warning, the **security phrase**
  (shown only after the account is selected, a forged screen does not know
  it). A wrong password: the field shakes (240 ms, just a color change under
  Reduce motion) and a text appears; after 3 attempts a 30 s delay. Other
  methods: security key.
- At the bottom left accessibility (high contrast, screen reader
  **Ctrl+Alt+S**) and the keyboard layout, at the right network and shut
  down.

## Accessibility

The accessibility tree is part of the window protocol (`guiproto::a11y`).
The UI library derives it from the description of the elements; applications
write nothing by hand. Rules:

- **Every element has a role and a name.** An icon-only button has a name
  ("Minimize"), the badge has a name with the permissions, a state (selected,
  checked, expanded, unavailable) is a property of the node.
- **Keyboard only**: Tab / Shift+Tab between groups, arrows within a group
  (list, tabs, switch, segments, table), Enter / Space activates, Esc closes
  a menu or dialog. Focus is always visible (2 px `focus`). Every prototype
  page has a "Skip to content" link.
- **Large font** up to 200 % without clipping; the layout is recomputed.
- **High contrast**: black background, white text, yellow accent, 2 px
  borders, no shadows and no transparency.
- **Motion**: Reduce motion replaces offsets with a cross-fade.
- Live regions: terminal output `log`, notifications `status`, errors
  `alert`.

Roles that the prototypes use and that `guiproto::a11y::Role` does not have
yet (6c has `window`, `titlebar`, `button`, `label`, `text-input`,
`checkbox`, `list`, `list-item`, `group`, `image`, `progress`, `terminal`).
Proposed additions:

| Role | Where | Accessibility action |
|---|---|---|
| `switch` | switches in Settings | `toggle` |
| `radio-group`, `radio` | segments, profiles, account selection | `select` |
| `tab-list`, `tab`, `tab-panel` | terminal tabs, Settings categories | `select` |
| `table`, `row`, `cell`, `column-header` | terminal tables, task manager, file list | `sort` (header) |
| `combobox`, `listbox`, `option` | command line with completion | `expand`, `select` |
| `menu`, `menu-item` | bar menu | `activate` |
| `dialog`, `alert-dialog` | application dialogs / system dialogs (compositor only) | `dismiss` |
| `log`, `status`, `alert` | terminal output, notifications, errors | - |
| `trust-badge` | trust badge (compositor only) | `activate` (permission overview) |

## Keyboard shortcuts

| Shortcut | What it does |
|---|---|
| Ctrl+Alt+Del | secure screen (caught by `inputd`) |
| Alt+Tab, Alt+Shift+Tab | window switching |
| Super | application launcher with search |
| Super+arrows | snap a window to a half / maximize / restore |
| Esc (hold 1 s) | exit full screen |
| Ctrl+Alt+S | screen reader (also on sign-in) |
| Tab / Shift+Tab, arrows, Enter, Esc | operating controls and dialogs |
| in the terminal: Tab, Up / Down, Ctrl+C, Ctrl+Shift+P, Ctrl+Shift+F | complete, history, interrupt, dry run, switch notation B / C |

## For implementation (6c)

What to take from the design and where:

| What | Where | Note |
|---|---|---|
| tokens | `libs/ui` (TOML loading), `compd` (decorations) | the compositor switches the theme and sends an event to windows; applications have no colors of their own for `trust-*` |
| decorations per Anatomy and Modes | `compd` | title bar 40 px (custom 44 px), buttons 28 px with a 40 px target, edge 6 px |
| badge and permission overview | `compd` + permission broker | permission data from init, not from the application |
| shadows as 9-piece textures | `compd` | precompute for `window`, `window-inactive`, `popover`, `dialog` |
| system dialogs per the prototype | `compd`, `guiproto::control` | "System" strip, phrase, 300 ms, physical input only (already in the 6c design) |
| bar, launcher, switcher, notifications, task manager | a new system application "desktop shell" with a capability for layers 200-400 | task data from `taskd` |
| accessibility roles | `guiproto::a11y::Role` | add per the table above |
| typeface | `libs/font` | static instances of Inter and JetBrains Mono, kerning from `GPOS` (see the risks) |

⚠️ **Risks (stated up front):**

- **Kerning and the typeface**: without reading `GPOS`, Inter will have no
  kerning (see [Typography](#typography)). It can be solved by preprocessing
  at build time.
- **Blur under the bar** (`backdrop-filter` in the prototype) is expensive in
  software compositing (blurring a region every frame in which something
  under the bar changes). In the 2D backend the bar should have an opaque
  `surface` background at 90 % opacity without blur; blur only with the 3D
  backend.
- **Theme by time of day** needs a location (sunrise and sunset). Without a
  location a fixed time of 7:00-20:00 per the time zone is used.
- **Imitating a system dialog in a full-screen application** cannot be ruled
  out by appearance alone; the protection is the phrase, physical input and
  Ctrl+Alt+Del (the same risk as in the 6c design).
- **The prototypes load typefaces from Google Fonts** (only for the preview
  in a browser); without a network the browser's system font is used and the
  look differs slightly. The system will have the typefaces bundled.

## Design versions

| Version | Date | What |
|---|---|---|
| v1 | 2026-11-07 | first design: style C Warm, design system, windows, system elements, 4 applications, accessibility, tokens |
