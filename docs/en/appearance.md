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

> **Visual language handbook:** [design/prirucka.html](../../design/prirucka.html)
> (Czech and English). Rules for anyone drawing a window, writing an
> application, copy or an icon: values, logo, color, type, icons, layout,
> components, motion, voice and copy, trust and the agent, accessibility,
> with "do / don't" examples. This page is the technical design and plan;
> the handbook is a concise guide for authors. The link works in the
> repository (open the file in a browser) and on the documentation website
> (`cargo xtask docs` copies the `design/` folder to `target/docs/design/`).

**Clickable prototypes** are in `design/` (open `design/index.html` in a
browser). Every page has a switch for theme, high contrast, font size and
reduced motion.

| Prototype | What it shows |
|---|---|
| `design/prirucka.html` | **visual language handbook** (cs / en) |
| `design/design-system.html` | colors with a contrast table, typography, grid, corner radii, shadows, icons, motion, components |
| `design/okna.html` | window anatomy, trust badge (removing permissions), window modes, states |
| `design/plocha.html` | bar in 6 positions, window switching, notifications, system dialogs, file picker, secure screen, task manager, agent in the bar |
| `design/terminal.html` | output blocks, tables with a schema, type-based completion, dry run and undo, errors, task panel, notation B / C |
| `design/soubory.html` | file versions, "Try aside", applying selected changes, conflict, agent working aside |
| `design/nastaveni.html` | security profiles, individual options, appearance, accessibility |
| `design/prihlaseni.html` | lock screen, account selection, security phrase, mode with Ctrl+Alt+Del |
| `design/varianty-stylu.html` | the three original style variants (decision archive) |
| `design/start.html` | boot screen: logo with a ring, log on Esc, step delay, boot failure, transition into the avatar |
| `design/start-varianty.html` | three boot screen variants (decision archive) |
| `design/pad.html` | critical crash screen: calm description, countdown, `D` technical report, notification after the next boot |
| `design/pad-varianty.html` | three crash screen variants (decision archive) |
| `design/instalace.html` | installation: welcome on the live USB, an 8-step installer (computer check, disk, change preview, account, security), progress, first run |

## Style decisions (agreed)

The user chose from three variants side by side (`design/varianty-stylu.html`):
A Calm (minimalist), B Technical (dense, lines instead of shadows),
C Warm (more playful, strong accent).

| # | Question | Decision | Consequence |
|---|---|---|---|
| 1 | overall character | **C, Warm** | purple accent, warm background, corner radius 18 px for windows and 10 px for controls, pill-shaped buttons, colored soft shadows |
| 2 | bar position | **configurable**, default **inset strip at the bottom** | 6 positions: inset at the bottom, floating dock, full-width at the bottom, left, right, top |
| 3 | icon style | **pastel gradient (F)**, changed on 2026-11-08 from line icons | soft shapes, 2 px stroke, fill with a gradient of two pastels and drawing in the saturated shade by category; chosen from the directions in `design/ikony-smery.html` and `design/ikony-smery-2.html` |
| 4 | typeface | **Inter + JetBrains Mono** | both under SIL OFL 1.1; Tuffy and Noto Sans Mono stay as a fallback |
| 5 | default theme | **by time of day** | light during the day, dark after sunset within the configured time zone |
| 6 | AI agent marking | **reserved color + icon + word** | coral color only for the agent, always with an icon and the word "agent" (never by color alone) |
| 7 | logo | **M symbol + MagnumOpus OS wordmark** (vectors from the user) | `design/assets/logo/`; purple `#6b3fd6` on light, lavender `#b79bff` on dark, white on color |
| 8 | boot screen | **A (logo with a ring) + detailed log from B**, ending with a **transition of the mark into the avatar** | chosen from three variants in `design/start-varianty.html` |
| 9 | critical crash screen | **A Calm** for users, **the D key switches to the technical report** (Workshop); in a development build Workshop directly | chosen from three variants in `design/pad-varianty.html` |
| 10 | installation | **try from USB, then install**; installer with a disk change preview; first run | recommended option (alternatives: disk image + first-run wizard only, installing from Windows) |

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
(`rgba(107,63,214,.16)`), in the dark one it is black. On the GPU the
compositor draws the shadow with a shader and caches it per window size, so
moving a window does not make the shadow more expensive. The 2D fallback
path uses a precomputed texture made of 9 pieces (corners, edges, center).
In high contrast, shadows are replaced by a 2 px border.

### Icons

**Pastel gradient (direction F):** soft shapes on a 24 px grid, 2 px stroke,
rounded caps and joins. Part of the shape is filled with a soft gradient of
two pastels, the drawing is in the saturated shade of the same color (not
black). The color is set by the icon's **category** and lives in the tokens
(`icon_<category>_bg` for the fill, `icon_<category>` for the drawing; the
drawing has at least 3 : 1 contrast against the surface and the fill):

| Category | Color | Examples |
|---|---|---|
| `files` | butter | folder, file, disk, trash, backup |
| `system` | lavender | settings, terminal, shield, lock, user |
| `network` | sky | network, wifi, notifications, search, tasks |
| `devices` | mint | sound, battery, camera, keyboard, USB |
| `time` | teal | history, clock, try aside, compare, conflict |
| `agent` | coral (reserved) | agent |
| `ui` | text color, no fill | arrows, close, plus, check |

In buttons, chips, the badge and other controls icons are single-colored by
the text and without fill (CSS variables `--ic-fill`, `--ic-ink`). In high
contrast the fill disappears and the drawing uses a bright category color.
The set has 226 icons (162 from the source set + 64 added; `design/assets/mo.js`, generated by
`node design/tools/build-icons.mjs <svg/24>` from the source set); the
`ic-*` IDs do not change. Redrawn: USB, compare changes, change conflict,
touchpad. Peach is not used for icons (it belongs to the trust badge).

An icon never carries meaning alone: in the bar it has a label or tooltip, in
the accessibility tree a name.

### Logo

The logo consists of the **symbol** (the letter M in an arch) and the
**wordmark** "MagnumOpus OS". Both are closed Bezier curves without fonts or
bitmaps (`design/assets/logo/`, description in `CTI-ME.txt`).

| File | Use |
|---|---|
| `logo-svetle-pozadi.svg` | purple symbol `#6b3fd6` and dark wordmark `#221a28`: light theme |
| `logo-tmave-pozadi.svg` | lavender symbol `#b79bff` and light wordmark `#fffdfb`: dark theme |
| `logo-jednobarevne.svg` | single color (print, high contrast) |
| `symbol-fialovy.svg`, `symbol-levandulovy.svg`, `symbol-bily.svg` | the mark alone: system icon, bar, boot, "System" badge |

The logo colors are exactly the `accent` tokens (light and dark theme) and
the dark theme's `surface`, so the logo always matches the accent. In the
system the symbol is drawn with the fill from the `accent` token (one path,
the theme gives the color); in high contrast white or the yellow accent.
Minimum symbol size 16 px, with clear space of at least a quarter of its
width around it.

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

### Context menus and material (agreed 2026-11-08)

Prototype: [design/kontextove-nabidky.html](../../design/kontextove-nabidky.html).
All **[designed]**.

- **Material (b)**: menus, the palette, panels from the bar and
  notifications have a translucent background blurring what is beneath,
  a subtle tint and ~5 % noise (against banding). Worst-case text contrast
  on a colourful wallpaper 8.8:1 (light theme). No live blur behind whole
  windows.
- **The active window's title bar** has a subtle tint from the wallpaper
  (sampled once when the wallpaper changes, not a live blur).
- **Solid background** always: the "Reduce transparency" setting, high
  contrast, and without a GPU (fallback CPU compositing) automatically.
- **Speed**: a menu opens within one frame. Items are data in advance;
  extensions of other apps declare their items statically in the
  manifest (file type, conditions) and no foreign code runs on opening.
- **Dynamic items** have a known count and place in the manifest. On
  opening a skeleton (a muted bar with a subtle animation) holds their
  place, a delivered item replaces it in the same place; after the
  deadline it stays disabled with the reason "unavailable". **The content
  of an open menu never shifts and the menu never changes size.**
- Shortcuts on the right, disabled items with a reason, typing filters
  the menu.

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

### Boot screen

The boot screen is drawn by init into the framebuffer before the compositor
exists (`design/start.html`). Therefore only cheap effects: solid colors,
gradients, opacity and translate/scale of one layer, no blur. Only the
changed area around the mark is drawn, tens of kB per frame.

| Time | What happens |
|---|---|
| 0 ms | firmware; init takes over the framebuffer (UEFI GOP) without a flicker, black background |
| +300 ms | the warm desktop background and the mark (symbol in a ring) fade in over 900 ms, the mark starts to breathe gently, the wordmark below it |
| ongoing | the **progress ring** fills according to init's real steps: kernel, devices, files, network, desktop (not a time estimate) |
| a step longer than 2 s | below the mark "waiting for: network (DHCP)"; **Esc** shows the detailed boot log at any time and back |
| end + 200 ms | **transition into the avatar**: the mark shrinks so that the ring exactly outlines the avatar circle (88 px), moves into it (600 ms, standard curve) and dissolves (300 ms) while the avatar appears |
| +120 ms | name, password field and security phrase slide in one after another; the compositor takes over the last boot frame, so sign-in continues the same picture |

- **Boot failure**: instead of a stuck logo, a screen with what failed
  (service, code) and the options **Try again**, **Start the previous
  version** (FS snapshot) and **Recovery mode**.
- **Theme**: from the last sign-in (stored with the boot loader), otherwise
  by time of day.
- **Reduce motion**: no breathing, the ring fills in steps, a 90 ms
  crossfade instead of the move into the avatar.
- The variants that were chosen from (A breathing, B steps, C transition
  into the clock) are in `design/start-varianty.html`.

### System crash

The screen for a **critical crash**: a kernel panic or an init crash
(`design/pad.html`). Crashes of services, drivers and the compositor are
handled by [recovery](#how-the-gui-recovers) and never show this screen.

It is drawn by **the kernel itself** into the firmware framebuffer (UEFI
GOP): no compositor, no drivers, no memory allocation. The typeface and the
symbol are pre-rasterized in the kernel image (`40 kB), colors are fixed
(dark theme), switching views is instant and the only motion is the
countdown.

| View | What it shows |
|---|---|
| **for users** (default) | dark purple background, the symbol in a ring that fills with the countdown to restart (15 s); "The system had to stop", reassurance that files are safe and work in progress comes back; error code and crash id (`PAGE_FAULT_KERNEL · #MO-7F3A-21C9`); a QR link to the error explanation in the documentation |
| **Workshop** (key **D**) | on the left a short description and the keys, on the right the technical report: kind of panic, version, CPU, address, thread and job, backtrace with symbols and lines, registers, last 5 log messages, where the report is stored; in a **development build** it is shown directly |

Keys: **Enter** restart now, **D** switch view, **P** pause or resume the
countdown, **arrows** scroll the technical report. The state is also
written to the serial line.

- **Report**: the kernel stores it in reserved memory that survives a warm
  restart; after the next boot init writes it to `/system/pady/` and after
  sign-in shows a "The system crashed last time" notification (Show
  report, Save to USB). **Nothing is sent automatically**; the QR code only
  links to the documentation.
- **Repeated crash**: 3 crashes during boot in a row = the next boot offers
  the previous system version (FS snapshot) or recovery mode.
- **Accessibility**: text at least 7 : 1, nothing flashes, the countdown
  can be paused.

### Installation

The system ships as a **live USB** (`design/instalace.html`): it boots from
the USB and can be tried without changing anything on the computer's disk
(changes go to memory only). The welcome screen offers **Try** and
**Install**; language, keyboard and accessibility (contrast, larger text,
screen reader) are at the top from the very first screen. In try mode the
desktop has an "Install MagnumOpus" icon and the bar a "changes are not
saved" label.

The **installer** is a system application ("System" badge) with the steps
on the left:

| Step | What it does |
|---|---|
| 1 Language and accessibility | language, keyboard layout with a field to try it, accessibility (also applies to the installed system) |
| 2 Computer check | graphics, disk, memory, network, input, Secure Boot; the result is **verified** because the system is running on the machine right now |
| 3 Disk | **next to Windows** (recommended; a size slider with limits: Windows at least the used space + reserve, MagnumOpus at least 40 GB), **whole disk**, **custom layout**; disk map "now" |
| 4 Change preview | disk map **now** and **after installation**, a list of changes (unchanged, shrink, new, delete); nothing has been written yet. Shrinking Windows requires confirming "I have a backup", erasing the whole disk requires typing the disk name |
| 5 Account | name, password with a strength meter, **security phrase** with a preview of the "System" strip it will appear in |
| 6 Security | profile Standard / Strict / Relaxed, **disk encryption** (on by default) and a **recovery key** (save to USB, print, QR) |
| 7 Summary | overview of the choices, Install button |
| 8 Installation | progress ring with the symbol (as at boot), percent, time estimate, list of phases; the system can still be tried meanwhile |

Then an **Installed** screen with Secure Boot instructions for the given
computer and a restart. The **first run** offers the theme and the bar
position and introduces in three cards what is different about the system:
the trust badge, try aside, agent marking.

⚠️ **Risks (stated up front):**

- **Shrinking the Windows partition (NTFS)** is the riskiest step: a power
  loss while moving data can damage Windows. The installer checks the
  partition first, shrinks only free space and asks for a backup
  confirmation. Implementing NTFS shrinking is a separate, large piece of
  work; until then only "whole disk" or free space is possible.
- **Secure Boot**: common computers have it on and will not start an
  unsigned boot loader. Long term a signed boot loader (via shim like
  Linux), until then instructions to disable it.
- **Drivers on real hardware**: the live USB needs USB storage, the disk
  (NVMe/AHCI), input and a picture at least through the firmware
  framebuffer.
- **First keyboard**: BadUSB protection does not apply during installation
  (nothing could be typed without a keyboard); verification of new devices
  is enabled after installation according to the chosen profile.
- **Updates** of the system will be a new version in the FS (snapshot), so
  going back to the previous version is the same choice as after a crash.

### Mobile environment (design, stage A)

> **Change from v1.6:** a new design of the mobile environment following
> the adaptive UI brief, plus system dialogs, the launcher and bar panels.
> This is a prototype design only; none of it runs yet.

The [Mobile look](../../design/mobil.html) prototype simulates a narrow
and a wide phone and a tablet, portrait and landscape, with larger text,
in light and dark themes. It walks the required flow: lock screen, Home,
launching Files, opening a file, editing with the keyboard, rotation,
overview and return, permission dialog. Next to the phone it shows the
**window context** an application would get (size, insets, area covered
by the keyboard, layout class, generation).

| Topic | Design |
|---|---|
| Navigation | gestures: swipe up for Home, swipe and hold for Overview, swipe from a side for Back; button navigation as an accessibility option |
| Back | 1. closes a temporary layer, 2. steps back in the app, 3. previous app or Home; unsaved work is protected by file versioning |
| Gesture conflicts | 16 px system band at the edges, app exclusion at most 200 px per side, recognition after 12 px and then the touch is cancelled in the app |
| Layout | `compact` &lt; 600, `medium` 600 to 839, `expanded` &ge; 840 logical units, hysteresis 24; the keyboard does not change the class. Tokens `breakpoint.*`, in applications `cx.class` in `view` ([GUI: adaptive layout](gui.md#adaptive-layout-context-in-view)) |
| Trusted elements | only the system draws the status bar; a double press of power shows "Who is on screen"; dialogs with the System band and phrase |
| Keyboard | a system component in its own process; apps get text and composition; sensitive fields get a secure keyboard without suggestions and clipboard |
| Agent | its own colour and icon, also on the lock screen and in the status bar; its text changes are underlined and can be undone |
| Monitor | mirror, extend, companion controls (roles `presentation` and `controls`); connecting alone publishes nothing |

⚠️ **Risks and limits (stated up front):** the text engine cannot yet do
character composition (CJK), bidirectional text or Czech hyphenation;
the keyboard in the design does not promise those languages. Phone
functions (calls, mobile data) are only simulated. A generic wireless
receiver may keep showing the last frame after a connection loss.

More designs waiting for decisions: [bubbles and bar panels](../../design/bubliny-panely.html),
[launcher and palette name](../../design/spoustec.html),
[system dialogs](../../design/dialogy.html) (desktop and phone).

### Tablet and TV (design)

> **Change from v1.7:** two designs building on the mobile environment.
> Launcher: variant D chosen (compact grouped list).

**Decided 2026-11-13:** tablet and laptop modes switch automatically by
the sensor, with a manual switch in quick settings (automatic, always,
never). In portrait the tiles are stacked. TV mode is not developed
further; content above the profile rating is hidden from children. The
foundations take the points from the table below: 1, 2 and 4 are
implemented by milestone 6c, points 3, 5 and 6 are **[designed]** in
[device interfaces](device-interfaces.md#media-actions-and-playback-session-designed),
[architecture](architecture.md#household-profiles-and-content-policy-designed)
and [device interfaces](device-interfaces.md#video-plane-designed).

[Tablet mode](../../design/tablet.html) shows a tablet in landscape and
portrait with finger, pen and an attached keyboard. At most two tiles
(1/2, 1/3 + 2/3) and one floating window; in portrait the tiles are
stacked. Attaching or detaching the keyboard closes and rearranges
nothing: only window title bars, the bar, target sizes and the window
context change. Half the screen in landscape (578 units) is `compact`,
as on common tablets.

[TV mode](../../design/televize.html) is a 3-metre interface: remote
control or gamepad, household profiles and parental controls. The main
result is a table of what the model already covers on a TV. The
foundations need:

| # | What | Why |
|---|---|---|
| 1 | `posture`, `viewing_distance`, `safe` in the window context | density and safe area independent of the width class |
| 2 | spatial navigation and focus groups with memory in the component library | linear Tab order is not enough on a remote |
| 3 | semantic media actions and a system playback session | one set for remote, phone, voice and lock screen |
| 4 | input source class (local physical, remote, synthetic) | PIN and trusted dialogs only from a local source |
| 5 | household profile and content policy | the system enforces parental controls, not the app |
| 6 | video plane in the compositor | 4K and HDR without exhausting the CPU |

Risks: commercial streaming services require certified DRM that a new
system will not get; pen, touch and HDMI-CEC need drivers that do not
exist yet.

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

## Responsiveness, smoothness and recovery (requirement)

The foundation of the whole GUI is that it is **snappy and smooth** and
that it **recovers on its own** from a crash of any part, and at the same
time **beautiful**. Both count equally: effects (blur, shadows, animations)
are designed to run on the GPU within the frame budget, and when they would
exceed it, the system simplifies them (smaller blur, shorter animation)
instead of delaying the response. Responsiveness and recovery have
measurable goals and tests fail when the system misses them; the look is
guarded by screenshot tests against the prototypes.

### Goals

Measured on a reference machine and in QEMU with virgl; at 120 Hz the same
goals apply in milliseconds, animations are time-based, not frame-based.

| Metric | Goal |
|---|---|
| mouse movement -> cursor on screen | within 1 frame (16.7 ms at 60 Hz); cursor on a hardware plane |
| key press -> character in a text field | within 2 frames |
| click -> visible reaction of the control | in the next frame |
| dropped frames during an animation | 0 in normal operation; at most 1 in 60 in the QEMU test |
| opening the launcher, window switcher, menu | within 50 ms |
| first frame of a newly started application | within 150 ms (skeleton from the saved UI state) |
| window resize | the edge follows the pointer every frame, even when the application lags |
| idle compositor | 0 % CPU and GPU (when nothing changes, nothing is drawn) |
| picture after a compositor crash | within 500 ms, windows in their original places, focus kept |
| bar after a desktop shell crash | within 300 ms; windows are not affected at all |
| GPU driver crash or hang | at most 1 dropped frame (switch to 2D), back on the GPU after the driver restarts |

### How responsiveness is ensured

- **The compositor never waits for an application.** It composes the
  window's last committed buffer. A late application shows an older frame
  in its window; other windows and the cursor are not held up. During a
  resize it waits for a new buffer at most 1 frame, then uses the old one
  stretched with the edge filled.
- **Compose as late as possible before vsync**: the compositor measures
  how long composition takes and starts just in time to meet the deadline
  with a 2 ms margin. The pointer position and the latest input are filled
  in at the last moment (late latching). The cursor is on a hardware plane,
  so it moves even when composition takes longer.
- **Full screen without composition**: a game or video in full screen goes
  straight to the output (direct scanout); the compositor only adds the
  badge when needed.
- **Priorities**: the compositor and the focused window have QoS
  `interactive`, priority is inherited across IPC into services, reads for
  the UI overtake background work
  ([Responsiveness](architecture.md#user-interface-responsiveness-plan)).
- **Fast application start**: default tokens are compiled into the code (no
  file read at start), the typeface lives in a **shared glyph cache** (a
  read-only memory object filled by the font service), the application
  shows its first frame from the saved UI state and only then loads data.
  The launcher, switcher and menus are part of the already running desktop
  shell.
- **Work off the UI thread**: the UI library does I/O asynchronously; the UI
  thread only reacts and draws.

### How the GUI recovers

| What crashes | What happens | What the user sees |
|---|---|---|
| **an application** | only its window disappears; after a restart it comes back from the state in its dataset | nothing else changes |
| **an application hangs** | `PING` without `PONG` for 3 s: the window turns grey and offers Wait / Quit | other windows keep running |
| **desktop shell** (bar, launcher, on-screen notifications) | the supervisor starts it again; the bar state is in the configuration, tasks in `taskd`, notifications in the notification service | the bar disappears briefly (up to 300 ms), windows stay |
| **notification service** | the supervisor restarts it; the queue is stored in its dataset | nothing is lost |
| **GPU driver** | the compositor detects the crash or hang (fence not signalled within 250 ms), switches the same layer tree to the 2D fallback path, the supervisor restarts the driver, the compositor returns to the GPU | blur disappears briefly, the picture keeps running |
| **compositor** | see below | the picture is gone for at most 500 ms, windows return to their places |
| **compositor 3 times in a minute** | init switches to recovery mode: a text terminal on the framebuffer (`term::screen`) with an option to restart the GUI | the system stays usable |

**Restarting the compositor without losing windows:**

1. The compositor continuously saves the **scene state** into a memory
   object held by init (it survives the compositor's crash): windows by
   application identity and window key, position, size, order, mode,
   focus, theme. Writing is cheap (changes only) and atomic (two copies,
   the valid one is switched).
2. After a crash the supervisor starts it again; the new compositor loads
   the scene state and shows the desktop and the bar right away.
3. The UI library in the application (`ui::client`) notices the closed
   channel, reconnects and **sends the whole window state in one
   `COMMIT`**: the layer tree, the accessibility tree and the handles of
   its buffers. The buffers belong to the application, so nothing is
   redrawn: the window is back in the next frame.
4. Meanwhile `inputd` holds input for the focus owner (at most 1 s, then
   drops it), so half-typed text does not go to another window. The new
   compositor first restores focus, then receives the held events.
5. The trust badge needs nothing from the application: init gives the
   identity and permissions to the compositor again.

Applications that draw themselves (without `libs/ui`) get the same in the
low layer `ui::client::Window`: a "reconnected" callback in which they send
their layer tree.

**Watchdog:** the compositor sends the supervisor a heartbeat every frame
it draws, and once a second when idle. Without a heartbeat for 2 s
(deadlock, infinite loop) the supervisor kills it and starts it again
following the steps above.

## Implementation plan

**Assumption:** the compositor gets a GPU backend (virgl now, Vulkan via
Venus later). The design therefore counts on blur under the bar and menus,
soft shadows and gloss. Software compositing over virtio-gpu 2D stays as a
**fallback path** (machine without a supported GPU, recovery mode): the
same layer tree, just without blur (the bar then has a solid `surface`
background at 90 % opacity) and with shadows from precomputed textures.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Order of implementation threads">
<defs><marker id="vz3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="m" x="10" y="16">phase 1</text><text class="m" x="270" y="16">phase 2</text><text class="m" x="530" y="16">phase 3 (in parallel)</text>
<rect class="acc" x="10" y="30" width="200" height="56" rx="6"/><text class="t b" x="110" y="53" text-anchor="middle">G1 GPU rendering</text><text class="m" x="110" y="71" text-anchor="middle">shapes, shadows, blur, motion</text>
<rect class="acc" x="10" y="110" width="200" height="56" rx="6"/><text class="t b" x="110" y="133" text-anchor="middle">G2 type and icons</text><text class="m" x="110" y="151" text-anchor="middle">Inter, GPOS, glyph atlas</text>
<rect class="box" x="10" y="190" width="200" height="56" rx="6"/><text class="t b" x="110" y="213" text-anchor="middle">S1 taskd</text><text class="m" x="110" y="231" text-anchor="middle">task service (shell thread)</text>
<rect class="box" x="10" y="270" width="200" height="56" rx="6"/><text class="t b" x="110" y="293" text-anchor="middle">S2 fsproto: clones, diff</text><text class="m" x="110" y="311" text-anchor="middle">FS thread</text>
<path class="ln" d="M210 58 L268 70" marker-end="url(#vz3)"/><path class="ln" d="M210 138 L268 80" marker-end="url(#vz3)"/>
<path class="ln" d="M210 58 L268 150" marker-end="url(#vz3)"/><path class="ln" d="M210 138 L268 160" marker-end="url(#vz3)"/>
<rect class="acc" x="270" y="46" width="200" height="56" rx="6"/><text class="t b" x="370" y="69" text-anchor="middle">G3 system layer</text><text class="m" x="370" y="87" text-anchor="middle">decorations, badge, dialogs</text>
<rect class="acc" x="270" y="126" width="200" height="56" rx="6"/><text class="t b" x="370" y="149" text-anchor="middle">G4 UI controls</text><text class="m" x="370" y="167" text-anchor="middle">libs/ui, a11y roles</text>
<rect class="warn" x="270" y="206" width="200" height="56" rx="6"/><text class="t b" x="370" y="229" text-anchor="middle">G5 recovery and speed</text><text class="m" x="370" y="247" text-anchor="middle">compd restart, metrics</text>
<path class="ln" d="M210 58 L268 230" marker-end="url(#vz3)"/>
<path class="ln" d="M470 154 L528 50" marker-end="url(#vz3)"/><path class="ln" d="M470 154 L528 110" marker-end="url(#vz3)"/><path class="ln" d="M470 154 L528 170" marker-end="url(#vz3)"/><path class="ln" d="M470 154 L528 230" marker-end="url(#vz3)"/><path class="ln" d="M470 154 L528 290" marker-end="url(#vz3)"/>
<path class="ln" d="M470 74 L528 290" marker-end="url(#vz3)"/>
<path class="ln" d="M210 218 L528 54" marker-end="url(#vz3)"/><path class="ln" d="M210 298 L528 176" marker-end="url(#vz3)"/>
<rect class="ok" x="530" y="26" width="220" height="44" rx="6"/><text class="t b" x="640" y="53" text-anchor="middle">A1 desktop shell</text>
<rect class="ok" x="530" y="86" width="220" height="44" rx="6"/><text class="t b" x="640" y="113" text-anchor="middle">A2 terminal</text>
<rect class="ok" x="530" y="146" width="220" height="44" rx="6"/><text class="t b" x="640" y="173" text-anchor="middle">A3 files</text>
<rect class="ok" x="530" y="206" width="220" height="44" rx="6"/><text class="t b" x="640" y="233" text-anchor="middle">A4 settings</text>
<rect class="ok" x="530" y="266" width="220" height="44" rx="6"/><text class="t b" x="640" y="293" text-anchor="middle">A5 sign-in</text>
</svg>
<figcaption>Threads and dependencies. G1 and G2 run in parallel, then G3, G4 and G5, then the applications.</figcaption>
</figure>

### Phase 1: foundation (in parallel)

> **Change from v1.5:** phase 1 is done in the system on the 2D path
> and in GPU composition with identical pixels (no blur). Shadows are
> precomputed masks; the glyph atlas is a CPU cache for now. Details and
> deviations: [GUI: Look per the design](gui.md#look-per-the-design-phase-1).

| Thread | Content | Where | Done when |
|---|---|---|---|
| **G1 GPU rendering** | rounded rectangles and borders as a shader (distance field, anti-aliased edges in one pass); shadows as a blurred shape (two-pass Gaussian or dual Kawase) cached per size; **background blur** for the bar, menus and dimming (only under the region, at half size); gradients; group opacity through an intermediate buffer; **animation timeline** in `compd` (curves and durations from tokens, interruptible, drop a frame instead of delaying input); token loading and theme switching | `compd`, `libs/comp`, virgl backend | a frame with 10 windows, shadows and a blurred bar within 4 ms of GPU time in QEMU; window-open animation without a dropped frame; the 2D fallback draws the same without blur |
| **G2 type and icons** | static instances of Inter (400, 500, 600, 700) and JetBrains Mono (400, 600) in `libs/font/fonts/`; pair kerning from `GPOS`; **GPU glyph atlas** with subpixel positioning (1/4 px); cache for scaling 100-200 % and HiDPI; icons: convert the paths from `design/assets/mo.js` at build time into a format the rasterizer understands (same curves as glyphs) and draw them into the atlas like glyphs | `libs/font`, `libs/gfx`, build script | "Te", "Vo" kerned; Czech without errors; the corrupted-font test still passes without panic; 64 icons in 4 sizes |

### Phase 2: system and controls (in parallel, after phase 1)

| Thread | Content | Where | Done when |
|---|---|---|---|
| **G3 system layer** | decorations per [Anatomy](#anatomy) and [Modes](#modes) (title bar 40 / 44 px, buttons 28 px with a 40 px target, edge 6 px); window states including "agent acting"; the **trust badge** and the permission overview with revoke (data from init and the permission broker); system dialogs in the prototype look ("System" strip, phrase, 300 ms); secure screen; Alt+Tab window switcher (thumbnails are layers the compositor already has); full-screen strip and Esc held for 1 s | `compd`, `guiproto::control`, init | all modes and states from `design/okna.html` in QEMU; revoking a permission shows within one frame; the dialog rejects synthetic input |
| **G4 UI controls** | new controls: switch, segmented control, tabs, sortable table, combobox with completion, menu, popover, tooltip, notification, application dialog, scrollbar; styled by tokens (radii, pills, 2 px focus); keyboard operation per [Accessibility](#accessibility); **new roles** in `guiproto::a11y::Role`; large text and high contrast | `libs/ui`, `libs/guiproto` | a demo application with all controls matches `design/design-system.html` in both themes and in high contrast; accessibility-tree tests for every control |
| **G5 recovery and responsiveness** | [compositor restart without losing windows](#how-the-gui-recovers) (scene state in a memory object held by init, reconnect in `ui::client`, input held in `inputd`); switch to 2D when the GPU driver crashes; watchdog; recovery mode; compose before vsync with late latching, hardware cursor, direct scanout; shared glyph cache; **metrics**: the compositor records input, composition and output times of every frame and `cargo xtask` turns them into a report | `compd`, `libs/ui`, `inputd`, init, supervisor | all [goals](#goals) met in a QEMU test; the test kills the compositor, the desktop shell and the GPU driver and measures the recovery |

### Phase 3: applications (in parallel, after G4)

| Thread | Content | Depends on | Done when |
|---|---|---|---|
| **A1 desktop shell** | a new system application with a capability for layers 200-400: bar in 6 positions, launcher with search (action API), notification center (notification service), task manager, "agent acting" pill with Stop | G3, G4, **S1 `taskd`** (shell thread) | `design/plocha.html` without the system dialogs (those are G3) |
| **A2 terminal** | `gui-term` draws blocks, tables, errors and dry runs as UI controls on top of `termd`; completion as a combobox; task panel; B / C switch | G4, `termd` | the demos from `design/terminal.html` against the real `termd` |
| **A3 files** | file explorer: list, details, version timeline and restore, try aside, applying selected changes, conflicts | G4, **S2 clones and diff in `fsproto`** (FS thread) | the whole "aside" flow from `design/soubory.html` on the real FS |
| **A4 settings** | the Settings application and a **configuration service** (TOML with a schema, versioned with the FS); security profiles and individual options wired to `inputd` (BadUSB), `fwd` (network), the launch gate (only planned so far), the terminal lock; appearance and accessibility | G4, G3 (password dialog) | a profile change takes effect in services without restart; relaxing requires a password |
| **A5 sign-in** | lock screen and sign-in in the system layer, user session manager, password storage (slow hash, e.g. Argon2id), lock on idle, Ctrl+Alt+Del mode | G3, G4, init (session = a set of capabilities) | sign-in, lock, unlock and user switch in QEMU |

**Why this order:** after phase 1 and G3 the system already "looks like
itself" (windows, decorations, badge, shadows, typeface), even while the
applications are still simple. The applications then run in parallel
because they do not need each other; they will only be held back by
services outside the GUI (`taskd`, clones in `fsproto`, configuration).

**Tests (all threads):**

- **screenshots**: QEMU `screendump` via QMP compared to a reference image
  with a tolerance (light, dark, high-contrast theme; text at 100 and 200 %);
- **smoothness and responsiveness**: the compositor records the times of
  each frame; the test fails when more than 1 frame in 60 is dropped during
  a window animation or when input latency exceeds the [goals](#goals);
- **recovery**: the test kills the compositor, the desktop shell, the
  notification service and the GPU driver and checks that windows came back
  to their places in time and that no key press went to another window;
- **accessibility**: the accessibility tree of every screen has a role and a
  name on all controls; Tab traversal reaches every control;
- **contrast**: `node design/tools/build-tokens.mjs --check` in `cargo xtask test`.

**Implementation decisions (agreed 2026-11-07):**

| Question | Options | Decision |
|---|---|---|
| how tokens get into code | A) `build.rs` generates Rust constants from `tokens.toml`; B) the library reads TOML at run time; C) A for defaults + B for user themes | **C**: fast start without a parser, themes can be added without a rebuild |
| who draws the bar and launcher | A) the compositor; B) a separate application with a capability for system layers | **B**: a crashing bar does not take the compositor down, the supervisor restarts it |
| notification service | A) part of the desktop shell; B) a separate service | **B**: notifications must survive a bar restart and can be sent without the GUI (terminal, log) |

⚠️ **Risks (stated up front):**

- **Kerning and the typeface**: without reading `GPOS`, Inter will have no
  kerning (see [Typography](#typography)). It can be solved by preprocessing
  at build time.
- **Blur and the GPU in QEMU**: virgl needs a host with OpenGL
  (`egl-headless` for tests); where it is missing, tests run only on the 2D
  fallback path and blur is not verified. Blur costs GPU time proportional
  to the area: therefore only under the bar, menus and dialog, at half size,
  and only when something under it changed.
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
| v1.1 | 2026-11-07 | implementation plan in 3 phases; assumes the GPU backend (blur, shader shadows), 2D as a fallback path |
| v1.2 | 2026-11-07 | responsiveness, smoothness and recovery as a requirement with measurable goals; compositor restart without losing windows; thread G5; implementation decisions (C, B, B) |
| v1.3 | 2026-11-07 | logo (vectors from the user) in the prototypes and the design system; boot screen with the transition into the avatar |
| v1.4 | 2026-11-07 | critical crash screen (A Calm + technical report on D), notification after the next boot; visual language handbook |
| v1.5 | 2026-11-08 | installation: live USB, installer with a disk change preview, first run |
| v1.6 | 2026-11-08 | **phase 1 implemented** in the system: tokens as data (`libs/theme`), Inter and JetBrains Mono with `GPOS` kerning, windows, badge, bar, desktop, dialog and terminal following the prototypes; deviations in [GUI: Look per the design](gui.md#look-per-the-design-phase-1) |
| v1.7 | 2026-11-12 | designs: mobile environment (stage A), system dialogs for desktop and phone, launcher and palette names, bubbles and bar panels |
| v1.8 | 2026-11-13 | designs: tablet mode (window tiling, pen, 2-in-1), TV mode with a foundations review; compact file picker; launcher variant D |
| v1.9 | 2026-11-13 | decisions: tablet switches automatically, stacked tiles in portrait, TV mode not developed further, content above the rating hidden from children; points 3, 5 and 6 into the foundations as a design; permission infographic and severity colours for icons |
