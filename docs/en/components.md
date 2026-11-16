# Component library

Specification of the basic GUI controls: sizes, tokens, states and
keyboard behaviour for desktop, tablet and TV. The `libs/ui` library is
tuned against it. At the end is a list of gaps in the current
implementation (`libs/ui` and the `user/gui-komponenty` demo).

**Status:** design **[designed]**. Values come from `design/tokens.toml`.
Numbers marked *proposed* are not in the tokens yet; they are listed
under [New tokens](#new-tokens-designed).

> **Clickable prototype:** [design/komponenty.html](../../design/komponenty.html).
> Each component has:
> - a live demo;
> - a row of every state, forced statically: default, hover, pressed,
>   focus, disabled, error, selected;
> - a short spec;
> - a box with the gap against `libs/ui`.
>
> The switch at the top changes density (desktop / tablet / TV). The
> prototype bar changes theme, high contrast, text size and motion.

It builds on [Look and feel](appearance.md) (design system, tokens) and
[Graphics and windows](gui.md) (how `libs/ui` works inside).

## Common state rules

| State | Look | Tokens |
|---|---|---|
| Default | surface, 1 px border | `surface`, `border_strong` (boundary contrast at least 3 : 1), `ink` |
| Hover | darker surface | `surface_3`; filled variants mix 88 % colour + `ink` |
| Pressed | darker still, scaled to 98 % | 76 % + `ink`, `motion.fast` |
| Focus | ring outside the control | `focus`, `size.focus_ring` 2, `size.focus_offset` 2; fields use offset 0 |
| Disabled | 50 % opacity, no reaction | `opacity.disabled` |
| Error | 2 px border, icon and a sentence below | `danger`, `danger_soft` |
| Selected / on | soft accent, full accent for choices | `accent_soft`, `accent`, `accent_ink` |

Principles:

- **Activation on release.** A button fires when the mouse button or the
  key is released. Moving off a pressed control cancels the action.
- **Focus ring from the keyboard only.** The ring is drawn after keyboard
  or remote control use, not after a mouse click.
- **Never colour alone for errors.** There is always an `error` icon and
  a sentence that says how to fix it.
  - The sentence is linked to the control, so a screen reader reads it
    as the description.
  - It is announced when the form is submitted.
  - The 2 px border is drawn inside, so the layout does not move.
- **Selection and focus are separate.**
  - The selected row has `accent_soft`.
  - The active row (the cursor) has the ring.
  - They can be on different rows.
- **Disabled controls.** Disabled buttons and fields are not in the Tab
  order. A disabled menu item can still be reached with the arrows, and
  its tooltip says why it is unavailable.

## Densities

The application does not pick the density. It follows from the window
context: `posture`, `min_hit`, `pointers` and `viewing_distance` (see
[Tablet and TV](appearance.md#tablet-and-tv-design)).

| Size | Desktop | Tablet | TV | Source |
|---|---|---|---|---|
| Control | 36 | 44 | 56 | `control_h`, `hit_min_touch`; TV proposed |
| Small control | 28 | 36 | 44 | `control_h_compact`; tablet, TV proposed |
| List, table, tree row | 36 | 44 | 56 | `max(control_h, min_hit)` as today |
| Menu item | 32 | 44 | 56 | proposed |
| Body text | 14 / 20 | 14 / 20 | 21 / 30 | `type.body`; TV &times; 1.5 proposed |
| Icon in a control | 20 | 20 | 28 | `icon_m`; TV proposed |
| Checkbox, radio | 18 | 22 | 28 | proposed |
| Switch (w &times; h, knob) | 44 &times; 26, 18 | 52 &times; 32, 24 | 64 &times; 36, 26 | `switch_*`; tablet, TV proposed |
| Focus (width / offset) | 2 / 2 | 2 / 2 | 4 / 3, scale 1.04 and shadow | proposed for TV |
| Hover | yes | no | no | from `pointers` |
| Tooltip | hover 500 ms, focus at once | long press | 800 ms on focus | a constant today |

On TV focus is the only highlight. There is no pointer, so focus must be
visible from 3 m; that is why it is 4 px and scaled.

## Keyboard in general

```
Tab / Shift+Tab   -> next / previous STOP (group)
                     checkbox, switch, button = 1 stop
                     radio, tabs, segments, list, table,
                     tree, menu, tag group = 1 stop
Arrows            -> move INSIDE the stop (remembers the last item)
Enter             -> default action (button, open, confirm dialog)
Space             -> toggle (button, checkbox, row selection)
Esc               -> closes the innermost layer and returns focus:
                     tooltip -> menu -> popover -> dialog
Letters           -> type-ahead (list, menu, select, tree; 1 s)
Shift+F10, Menu   -> context menu for the focused control
F6                -> next area of the window (e.g. notifications)
```

On TV the same applies with the remote control:

- arrows;
- OK = Enter;
- Back = Esc;
- the Menu button = context menu.

## Components

The full spec and the states are in the prototype. This is the summary
for `libs/ui`: sizes, key behaviour and implementation status (**yes**,
**partial**, **no**).

| Component | `libs/ui` | Sizes and tokens | Keyboard |
|---|---|---|---|
| Button: primary, secondary, danger, icon | yes | height `--k-h`, min width 2 &times; height, `s5`, pill; primary `accent`, danger `danger`; *busy* state (spinner, fixed width) | Enter, Space on release |
| Text field, multiline | yes | `--k-h`, `radius.control`, `s4`; label above, hint / error below | arrows, Ctrl+arrows, Shift selection, clipboard, undo; multiline Ctrl+Enter submits |
| Number | yes | like a field, right aligned, unit, stepper buttons outside the Tab order | &uarr; &darr; step, PgUp / PgDn 10 steps, Home / End |
| Password | yes | field + eye toggle (`aria-pressed`), Caps Lock warning | like a field, never suggestions |
| Search | yes | magnifier on the left, clear button only with text, filtering with a 150 ms delay; a trailing slot (`.trailing`: a `.kbd` key label only in an empty field, an icon, short `ink_muted` text) for every single-line field | Esc clears, second Esc leaves, &darr; into the results |
| Checkbox | partial | box 18 / 22 / 28, `radius.xs`, 2 px border; indeterminate state | Space |
| Switch | yes | `switch_*`; immediate effect | Space |
| Radio | partial | circle 18 / 22 / 28; the group is one stop | arrows select directly |
| Slider | yes | 4 px track (2 today), knob `switch_knob` | arrows, PgUp / PgDn 10 %, Home / End; wheel only with focus |
| Select | yes | like a field; list `radius.card`, at most 8 items | Enter / Space / Alt+&darr; / F4, arrows, letters, Esc, Tab picks the active item |
| Tabs | yes | `--k-h`, max width 180, selected = 3 px `accent` underline | &larr; &rarr; Home End with automatic activation, Ctrl+PgUp / PgDn |
| Segmented control | yes | track `surface_3`, selected segment `surface` + `accent_ink` | &larr; &rarr; Home End |
| Menu, context menu | yes | `radius.card`, item `--k-item`, shortcut on the right, separator, group heading, submenu, danger item last | &uarr; &darr; wrapping, &rarr; / &larr; submenu, letters, Esc, Tab closes |
| Tooltip | yes | `ink` on `surface`, max 280, `radius.xs` | shown on focus, Esc hides |
| Popover | no | `radius.card`, 12 px arrow, width 300, non-modal | focus moves in, Esc returns to the trigger |
| Application dialog | no | `dialog_w`, `radius.window`, `shadow.dialog`, `scrim` | focus trap, Enter default, Esc cancel, focus returns |
| Toast | no | max 440, min height 48, inverted colours, 5 s / 8 s with an action, at most 3 | does not take focus; F6, Esc |
| List | yes | row `--k-row`, `radius.control` | arrows, Ctrl+Space, Shift+arrows, Ctrl+A, letters, Enter |
| Table | yes | row `--k-row`, header `type.caption` on `surface_2`, numbers right aligned | like the list, Enter on a header sorts |
| Tree | no | row `--k-row`, 20 px per level, triangle rotated 90&deg; | &rarr; expand / child, &larr; collapse / parent, `*` siblings |
| Progress, spinner, skeleton | partial | 8 px bar; spinner 24 / 16 / 40; skeleton 12 px | not focusable; announced every 10 % |
| Badge, status tag, filter tag | partial | badge 18; tag 24 `type.caption`; filter tag `--k-h-sm` | tag group: arrows, Space, Delete |
| Breadcrumbs | no | `--k-h-sm`, `/` separator in `ink_faint`, collapsed into "..." | each link is a Tab stop |
| Disclosure (accordion) | no | header `--k-h` + 12, `radius.card`, chevron rotated 180&deg; | Enter / Space, &uarr; &darr; inside a group |
| Empty state | yes | 72 px circle with an `icon_xl` icon, `type.title` heading, one action | focus on the action |
| Panel (bar, Settings) | yes | width `size.panel_w` (360), row at least `size.panel_row` (40), icon-text gap `size.panel_gap` (10), header and footer edge `size.panel_edge` (14), sides `s5`; header with an icon, a bold title (`body_strong`) and a summary (`caption`, `ink_muted`), a row with an icon, title and description (a button with `surface_3` hover, a radio option with a check mark on the right, a switch on the right, information), a section (`caption` in capitals), a state row (`surface_3` / `danger_soft`, `radius.control`), a footer with a `border` line. Built from ordinary elements (`libs/ui/src/panel.rs`: `panel_header`, `.panel_row(icon, desc)`, `panel_section`, `panel_state`, `panel_footer`); the row description is the value for a screen reader | radio options are a group (arrows select), Tab between the group, the switch and the footer; focus as an inner ring `radius.control` |

When to use which:

- **Switch vs checkbox.**
  - A switch has an immediate effect (Wi-Fi).
  - A checkbox applies only on confirmation, or picks several items.
- **Radio vs select.** Use radio for 2 to 5 options and a select for
  more.
- **Tabs vs segments.**
  - Tabs switch content (a panel).
  - Segments switch the mode of the same content.
- **Toast vs dialog vs system notification.**
  - A toast is a short in-window message about what just happened, with
    at most one action.
  - An error that needs a decision belongs in a dialog.
  - System notifications (outside the window, with history) are drawn by
    the system.
- **Application dialog vs system dialog.**
  - An application dialog is modal only for its own window.
  - A system dialog (permission, password, permanent delete) is drawn by
    the compositor with the "System" band and a `trust_border` frame.
  - An application cannot imitate it, because it never gets the
    `trust_*` colours.

## Gaps in libs/ui

Found in the code of `libs/ui/src/` and the `user/gui-komponenty` demo,
as of 2026-11-14. Ordered by impact.

**Missing widgets:**

1. **Application dialog.** It needs a modal layer in the window, a focus
   trap, default and cancel buttons, and returning focus. The
   `dialog_w` token already exists.
2. **Toast.** It needs a notification area in the window, a queue, a
   timer that pauses, and screen reader announcement.
3. **Tree.** The file manager and settings need it. I propose building
   it on top of the list (row + level + expanded), not as a new layer.
4. ~~**Number field** and **search field.**~~ Done (`number_input`,
   `search_input`, [Text fields and the clipboard](gui.md#text-fields-and-the-clipboard)).
   **[implemented]**
5. **Generic popover.** The `popup` layer only handles menus, selects
   and suggestions today.
6. Also missing:
   - breadcrumbs;
   - disclosure;
   - empty state;
   - circular spinner;
   - skeleton;
   - generic badge;
   - filter tag.

**Missing variants and states:**

1. The button has only one style. Missing:
   - primary;
   - danger;
   - ghost;
   - toggle (`pressed`);
   - *busy* state.
2. The checkbox has no indeterminate state, no hover, and no disabled
   look (`st.disabled` is ignored).
3. The radio does not fade its surface when disabled. ~~The application
   builds the radio group itself, and arrows between radios do not
   work.~~ Sibling radios are a group with one Tab stop and arrows.
   **[implemented]**
4. The error state exists only on text fields. It is missing on:
   - select;
   - checkbox;
   - slider;
   - tab;
   - segment.
5. ~~Fields have no hover. The password field has no eye toggle and no
   Caps Lock warning.~~ Done (hover darkens the border, eye toggle, Caps
   Lock warning). **[implemented]**
6. ~~The list has arrows only.~~ Done: `list(..).multi_select(true)`
   shares the selection logic with the table (`libs/ui/src/selection.rs`).
   **[implemented]** Originally: the list has arrows only. Missing:
   - Ctrl+Space;
   - Shift+arrows;
   - Ctrl+A;
   - type-ahead.

   The table already does all of this, so the list should share its
   selection logic.
7. The menu has no ~~submenus~~, group headings, danger item ~~or
   type-ahead~~. Submenus and mnemonics are done. **[implemented]**
8. ~~Select: Tab in the open list closes it without a change.~~ Tab picks
   the active item and moves on; a closed select has type-ahead and
   Home, End. **[implemented]**
9. Tabs: the selected tab is only a `surface_2` surface. In the light
   theme it has less than 3 : 1 contrast against `surface`. I propose a
   3 px `accent` underline. Disabled tabs are not skipped by the arrows.
10. ~~The slider changes value on the mouse wheel even without focus.~~
    The wheel needs focus. **[implemented]** The track is 2 px; I
    propose 4.

**Density:**

- `control_h_compact` (28) is not wired up in `look.rs`.
- Today density changes only the height via `min_hit`; its default of 32
  is a literal in `context.rs`. Missing:
  - scaling text, icons, boxes and focus for TV;
  - turning hover off on touch.

**Numbers outside the tokens.** They are small, but they break the rule
"no appearance numbers outside `tokens.toml`":

- `TIP_DELAY` 500 ms and `TIP_MAX_W` 280 (`popup.rs`);
- `TAB_MAX` 180 (`controls.rs`);
- the 30 % width of the indeterminate bar;
- the 3 px switch knob offset;
- the default `min_hit` of 32 (`context.rs`);
- the fixed table height of 8 rows + header.

**The `user/gui-komponenty` demo:**

- it shows the error state only for the password;
- it shows the disabled state only for the button;
- it never shows the indeterminate state or densities.

Once the widgets exist, it should match `design/komponenty.html`, with
the same order and the same states. That way the appearance snapshots
(`libs/ui/tests/shots/`) can be compared with the design.

## New tokens (designed)

These are proposed additions to `design/tokens.toml`. They are not in
the tokens yet; the prototype has them as `--k-*` variables.

| Token | Value | Purpose |
|---|---|---|
| `size.control_h_tv` | 56 | control and row on TV |
| `size.control_h_compact_touch` / `_tv` | 36 / 44 | small control on tablet and TV |
| `size.menu_item_h` | 32 | menu item (tablet and TV = control height) |
| `size.check` | 18 (tablet 22, TV 28) | checkbox and radio |
| `size.slider_track` | 4 | slider track |
| `size.tab_max` | 180 | max tab width (`TAB_MAX` today) |
| `size.tip_max_w` | 280 | max tooltip width (`TIP_MAX_W` today) |
| `size.toast_w` | 440 | max in-window toast width |
| `size.focus_ring_tv` / `focus_offset_tv` | 4 / 3 | focus on TV |
| `type.scale_tv` | 1.5 | text multiplier on TV (on top of the user's text size) |
| `motion.tip_delay` | 500 (TV 800) | tooltip delay (`TIP_DELAY` today) |
| `motion.toast_ms` / `toast_action_ms` | 5000 / 8000 | how long a toast stays |

The tablet and TV variants can be separate keys, or new sections
`[density.tablet]` and `[density.tv]`. Sections are cleaner, but
`libs/theme` has to be able to read them; the implementation decides.

## Risks

- **Animating the disclosure height** needs a layout pass in every
  frame. For `libs/ui` I propose a cross-fade with a height jump instead;
  it is cheap and fits in the frame budget.
- **Automatic tab activation** is right only when the panel renders
  within one frame. A slow panel needs manual activation (arrows only
  move focus, Enter switches); otherwise the arrows stutter.
- **A toast does not take focus.** A keyboard user reaches its Undo
  action only through F6 or Ctrl+Z, so every toast action must also be
  available another way.

## Versions

| Version | Date | What |
|---|---|---|
| v1 | 2026-11-14 | first version: 25 components in all states, desktop / tablet / TV densities, keyboard, gaps in `libs/ui`, proposed new tokens |
