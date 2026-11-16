# GUI deviations from the design

A comparison of the real GUI with the prototypes in `design/` and with
`design/tokens.toml`. It is the list of fixes for the GUI thread (milestone
6c). The design is described in [Look and feel](appearance.md), the
controls in the [Component library](components.md).

**Status:** list as of 2026-11-16 on `main` `d6078624`.

How it was captured:

- the system ran via `cargo xtask run --release --headless`, x86_64,
  development profile, QEMU 1280 &times; 800;
- the screenshots were taken with QMP `screendump`;
- it was night, so the *Automatic* appearance was dark.

The screenshots are in
[design/snimky/odchylky/](../../design/snimky/odchylky/01-plocha-okna.png).

Not checked:

- the light theme of system elements;
- the permission dialog and the file picker (could not be triggered);
- sign-in with account selection;
- the Settings app, which does not exist in the system yet (it is in
  neither the launcher nor the palette).

## How to read the list

| Field | Meaning |
|---|---|
| Priority | **P1** looks broken or like a different system; **P2** visibly different from the design but works; **P3** detail |
| Size | **S** up to half a day, **M** 1 to 2 days, **L** 3 to 5 days (estimate including tests) |
| Library | `ui` (libs/ui), `theme`, `comp` (libs/comp, compositor), `app` (`user/gui-*`) |
| ID | **K** = shared component (fixed once, applies everywhere), **O** = one screen |

## Shared components (fix first)

These deviations are not a bug of one screen. Each comes from one place in
`ui` or `comp` and shows wherever the element is used. One fix repairs the
whole system, so they go first.

| ID | What differs | Prototype | Library | P | Size |
|---|---|---|---|---|---|
| K1 | **Applications do not switch to the dark theme.** The content of every window stays light while the bar, title bars and launcher are dark. Affected: Files, Task manager, Components, Demo, Player, Terminal. The compositor sends `ServerMsg::Theme` after `HELLO` (`scene.rs`) and on change (`style.rs`), and `ui` turns it into `Event::Theme` (`client/mod.rs`), but the window is not redrawn with the new `Look`. Screenshot [01](../../design/snimky/odchylky/01-plocha-okna.png).<br>**Fixed:** there was only one cause - the `ui` client dropped `THEME` from the greeting (`b16ea897`); redrawing after `Event::Theme` worked (`dark.png` from the appearance test). Verified by the test "an application started in dark mode is dark from the first frame" (Demo and terminal), screenshot [01 after](../../design/snimky/odchylky/01-plocha-okna-po.png). | [komponenty.html](../../design/komponenty.html) (theme switch) | ui | P1 | S to M |
| K2 | **Bar panels and menus are not built from `ui`.** `flyout.rs` draws the Appearance, Network, Sound, Notifications and Developer panels and the app shortcut menu as a rectangle with lines of text. Missing:<br>- icons;<br>- a header with an icon and a summary;<br>- separators and sections;<br>- hover and focus;<br>- the open state of the bar icon (`surface_3` + `accent` underline);<br>- the `shadow.popover` shadow and the material (blur);<br>- controls (switch, slider, tiles, footer button).<br>The sizes differ too: width follows the text (prototype 360), rows are 24 px (prototype 40). The launcher and palette already use `ui` (`sysui.rs`); the panels do not. Proposal: a shared *panel* component in `ui` (header, row with icon and description, section, footer, switch, slider), with the panels as `sysui` views. Screenshots [02](../../design/snimky/odchylky/02-panel-vzhled.png), [03](../../design/snimky/odchylky/03-panel-zvuk.png), [04](../../design/snimky/odchylky/04-panel-vyvojar.png), [06](../../design/snimky/odchylky/06-nabidka-liste.png).<br>**Partly fixed:** the panel component in `ui` (`panel.rs`) and the Appearance, Network, Sound, Notifications and Developer panels as views (`barpanel.rs`) with token sizes, shadow, material with a budget fallback and the open icon state. Remaining: the app shortcut menu (O7) and sound control through the sound service (O2). Screenshots [02 after](../../design/snimky/odchylky/02-panel-vzhled-po.png), [notifications after](../../design/snimky/odchylky/02b-panel-oznameni-po.png). | [bubliny-panely.html](../../design/bubliny-panely.html) | comp, ui | P1 | L |
| K3 | **Icon button without an icon.** In the Components demo (Fields tab) the third button is an empty pill; its focus ring is visible. Screenshot [15](../../design/snimky/odchylky/15-komponenty-pole.png).<br>**Fixed:** the demo was missing the icon (`.icon("settings")`); `ui` draws an icon button correctly. | [komponenty.html#tlacitka](../../design/komponenty.html#tlacitka) | ui | P1 | S |
| K4 | **Only one button style.**<br>- The "Primary" button looks secondary: outline, no `accent` fill.<br>- The primary, danger and ghost variants are missing.<br>- Files therefore has 13 identical pills with nothing to guide the eye.<br>**Fixed in `ui`:** `.primary()`, `.danger()`, `.ghost()` (`node::Variant`, hover of the filled variants darkened towards `ink`, disabled at `opacity.disabled`); the Components demo shows them. Use in Files (one main action) comes with E. Screenshot [K4 after](../../design/snimky/odchylky/k4-tlacitka-po.png). | [komponenty.html#tlacitka](../../design/komponenty.html#tlacitka) | ui | P2 | M |
| K5 | **Floating layers without a shadow.**<br>- The menu and the select list have only a 1 px border and no `shadow.popover`, so they blend with light content.<br>- The check mark of the chosen item is on the left; the prototype has it on the right.<br>Screenshots [10](../../design/snimky/odchylky/10-vyber.png), [11](../../design/snimky/odchylky/11-nabidka-tabulka.png).<br>**Fixed:** menus and the select list have the `shadow.popover` shadow (masks cached in `ui`, the popover layer has a margin for it), the check mark of the chosen item is on the right, the disabled reason on the right instead of the shortcut. Screenshot [11 after](../../design/snimky/odchylky/11-nabidka-po.png). | [komponenty.html#vyber](../../design/komponenty.html#vyber), [#nabidky](../../design/komponenty.html#nabidky) | ui | P2 | S |
| K6 | **Menu items without icons or a group heading.** The context menus of Files and of the table show only text and shortcuts.<br>**Fixed:** `MenuEntry::icon` and `menu_header` (a group heading cannot be chosen, a screen reader reports it as a label); icons in the Files, Trash and field menus, the bar shortcut (heading "Okna") and power in the launcher. Screenshot [06 after](../../design/snimky/odchylky/06-nabidka-liste-po.png). | [kontextove-nabidky.html](../../design/kontextove-nabidky.html), [komponenty.html#nabidky](../../design/komponenty.html#nabidky) | ui | P2 | S |
| K7 | **Selected tab** is a `surface_2` surface without the 3 px `accent` underline, nearly invisible in the light theme.<br>**Fixed:** the selected tab has `accent_ink` text and a 3 px `accent` underline (`size.bar_indicator_h`), a `border` line under the row, hover `surface_3`. Screenshot [K7 after](../../design/snimky/odchylky/k7-zalozky-po.png). | [komponenty.html#zalozky](../../design/komponenty.html#zalozky) | ui | P2 | S |
| K8 | **No empty state for an empty result.** The palette with the query "nast" shows an empty frame instead of "Nothing found" with a suggestion. Screenshot [12](../../design/snimky/odchylky/12-paleta-prazdna.png).<br>**Fixed:** `ui::empty_state` (circle with an icon, heading, description, action); the palette without a result shows "Nic nenalezeno". Screenshot [12 after](../../design/snimky/odchylky/12-paleta-prazdna-po.png). | [komponenty.html#prazdno](../../design/komponenty.html#prazdno) | ui, comp | P2 | S |
| K9 | **List drawn as a table.** The launcher has:<br>- a nested table frame;<br>- a line under every row;<br>- a narrow right column ("běží, + no...").<br>The prototype has sections and rows without lines.<br>**Fixed:** a `.plain()` table (no frame or lines, rounded selection) in the launcher and palette. Screenshot [05 after](../../design/snimky/odchylky/05-spoustec-po.png). | [spoustec.html](../../design/spoustec.html) (variant d) | ui, comp | P2 | M |
| K10 | **Segmented control** stretched to the full width; the prototype is only as wide as its content.<br>**Fixed:** in a column it is as wide as its content (full width with `.grow()`). Screenshot [K10, K11 after](../../design/snimky/odchylky/k10-k11-volby-po.png). | [komponenty.html#zalozky](../../design/komponenty.html#zalozky) | ui | P3 | S |
| K11 | **Slider** has a 2 px track and a grey knob border (`border_strong`); the prototype has a 4 px track and an `accent` border.<br>**Fixed:** a 4 px `surface_3` track with a `border` edge, a knob with a 2 px `accent` border, larger on hover, `accent_soft` background when pressed. | [komponenty.html#posuvnik](../../design/komponenty.html#posuvnik) | ui | P3 | S |
| K12 | **Bar tooltip** is a single line; the prototype has a bold title and a description (state, shortcut).<br>**Fixed:** a `body_strong` title, a `small` `ink_muted` description (wrapped, at most 3 lines), a Super key label at the launcher (`TipText`). Screenshot [K12 after](../../design/snimky/odchylky/k12-bublina-po.png). | [bubliny-panely.html](../../design/bubliny-panely.html) (Tooltip) | comp | P3 | S |

## Bar and desktop

| ID | What differs | Prototype | Library | P | Size |
|---|---|---|---|---|---|
| O1 | **Appearance panel** (the user's report) is a title and four lines of text, the chosen one with a check mark. It looks like a rectangle with a list of texts, not like a menu. K2 fixes it. I added it to the prototype, which did not have it before:<br>- a radio group of three choices (icon, name, "follows sunrise and sunset" under Automatic), check mark on the right, arrow keys;<br>- a state row "Light now, following sunrise and sunset. Dark from 16:21";<br>- a High contrast switch;<br>- an *Appearance settings* button.<br>Screenshot [02](../../design/snimky/odchylky/02-panel-vzhled.png).<br>**Fixed:** a radio group with icons, a description and a check mark on the right (arrows select), the state row ("Tmavý od 16:21 (západ slunce)"), the High contrast switch, a disabled *Appearance settings* button with a reason. Screenshot [02 after](../../design/snimky/odchylky/02-panel-vzhled-po.png). | [bubliny-panely.html](../../design/bubliny-panely.html) (moon icon) | comp, ui | P1 | within K2 |
| O2 | **Sound panel** is only "Volume 100 %". Missing:<br>- the slider and mute;<br>- output and input;<br>- per-app volume;<br>- *Sound settings*.<br>Screenshot [03](../../design/snimky/odchylky/03-panel-zvuk.png).<br>**Fixed:** slider and mute, output and input (radio groups), the recording indicator, application volume by the manifest identity (Wine from several processes = one entry), a disabled *Sound settings*; new `audioproto` operations `OVERVIEW`, `APPS`, `APP_VOLUME`, `SELECT_INPUT` (test in the `zvuk` suite). Screenshot [03 after](../../design/snimky/odchylky/03-panel-zvuk-po.png). | [bubliny-panely.html](../../design/bubliny-panely.html) (Sound) | comp | P1 | M (after K2) |
| O3 | **Network panel** is only "Wired: connected". Missing:<br>- the Wi-Fi / Airplane / VPN tiles;<br>- the speed and IP row with the *active* tag;<br>- *Network settings*.<br>**Partly:** the Ethernet row with state, IP address from `netd` and an "active" label. `netd` does not report link speed. The Wi-Fi / Airplane / VPN tiles **wait for services** (no Wi-Fi driver, no VPN); *Network settings* disabled with a reason. | [bubliny-panely.html](../../design/bubliny-panely.html) (Network) | comp | P2 | M (after K2) |
| O4 | **Developer mode panel**:<br>- the action is a text row ("Turn on developer mode") with no switch;<br>- the panel shows no warning when the mode is on.<br>I added it to the prototype: switch, `danger_soft` warning row, bar pill. Screenshot [04](../../design/snimky/odchylky/04-panel-vyvojar.png).<br>**Fixed:** the panel with a switch and a `danger_soft` warning (K2), the bar pill with an icon (`code`, when on `shield-alert` on `danger`). | [bubliny-panely.html](../../design/bubliny-panely.html) (Developer) | comp | P2 | S (after K2) |
| O5 | **Clock** opens nothing on click; the prototype has a month calendar and world time.<br>**Fixed:** a click on the clock opens a panel with the month calendar (today `accent`, paging with arrows), world time (London, New York, Tokyo from `libs/tz`, only zones other than the local one) and a disabled *Time settings*; events are planned. Screenshot [O5 after](../../design/snimky/odchylky/o5-hodiny-po.png). | [bubliny-panely.html](../../design/bubliny-panely.html) (Clock) | comp | P2 | M |
| O6 | **Notifications** is only "No notifications"; the Do not disturb switch and the footer are missing.<br>**Waits for the notification service** (`notifyproto`, the Scheduler thread): the panel is over an empty list. | [bubliny-panely.html](../../design/bubliny-panely.html) (Notifications) | comp | P2 | S (after K2) |
| O7 | **App shortcut menu in the bar** is a text card ("Files / Home - Files / + New window") with no icons and no hover. Screenshot [06](../../design/snimky/odchylky/06-nabidka-liste.png).<br>**Fixed:** a `ui` menu with hover, keys, a check mark on the active window, a shortcut and a disabled item with a reason (icons and the group heading come with K6). Screenshot [06 after](../../design/snimky/odchylky/06-nabidka-liste-po.png). | [kontextove-nabidky.html](../../design/kontextove-nabidky.html) (bar icon) | comp | P2 | within K2 |
| O8 | **Tasks icon** opens the Task manager instead of the panel with progress. `flyout.rs` does this on purpose when the Task manager runs. The design and the code differ, so it needs a decision. | [bubliny-panely.html](../../design/bubliny-panely.html) (Tasks) | comp | P3 | S |
| O9 | **Running app markers:** after boot most of them are orange (`warn` = wants attention). They should be grey (`ink_faint`), with only the active one `accent`. Asking for attention right after start is noise.<br>**Fixed:** windows opened in the background within `STARTUP_QUIET` (15 s) after the session opens (autostarted apps) do not ask for attention; the mark is grey, the active one `accent`. A window that opens by itself later still asks. | [plocha.html](../../design/plocha.html) (bar) | comp, app | P2 | S |
| O10 | **Bar icon of the open panel** has no open state (`surface_3` + `accent` underline), so it is unclear which icon the panel belongs to.<br>**Fixed:** the icon of the open panel has `surface_3` and an `accent` underline. | [bubliny-panely.html](../../design/bubliny-panely.html) | comp | P2 | within K2 |
| O11 | **Right click on the desktop** opens nothing; the prototype has a desktop menu.<br>**Waits for services:** the menu items (New folder, Sort icons, Change wallpaper, Display settings, Customize desktop) need a desktop folder with icons and the Settings app; a menu with only disabled items would not help. | [kontextove-nabidky.html](../../design/kontextove-nabidky.html) (desktop) | comp | P3 | M |
| O12 | **Cursor over the launcher** shows the resize arrows of the window below it. The launcher does not hide window edges from the cursor shape. Screenshot [05](../../design/snimky/odchylky/05-spoustec.png).<br>**Fixed:** above the launcher, palette and bar panel the cursor is an arrow (`Comp::over_system`; host test). | - | comp | P2 | S |

## Launcher

| ID | What differs | Prototype | Library | P | Size |
|---|---|---|---|---|---|
| O13 | **Launcher footer** shows "Signed-in user" instead of the avatar and name; the Settings icon is missing.<br>**Fixed:** a circle with the initial and the display name, Settings (disabled with a reason). Screenshot [05 after](../../design/snimky/odchylky/05-spoustec-po.png). | [spoustec.html](../../design/spoustec.html) (d) | comp | P2 | S |
| O14 | **Launcher sections:**<br>- the system has Running / More... (1) / System, the prototype has Running / Apps / Recent;<br>- "More..." does not react to a click;<br>- the search field has no magnifier and no Super hint.<br>**Fixed:** sections Running / Applications / Recent without "more...", a field with a magnifier and the Super hint in the field's trailing slot (`Trailing::Key`, only in an empty field). | [spoustec.html](../../design/spoustec.html) (d) | comp | P2 | M |

## Windows and decorations

| ID | What differs | Prototype | Library | P | Size |
|---|---|---|---|---|---|
| O15 | **Trust badge** on Files and the Task manager says "System". The prototype shows the app name and its permission icon (Files + home).<br>**Fixed:** the badge shows the application name, a system app is marked by a shield with a check mark (init supplies the name and the mark from the manifest). | [okna.html](../../design/okna.html) (Trust badge), [soubory.html](../../design/soubory.html) | comp | P2 | S |
| O16 | **Window buttons with a custom title** (Terminal) are grey as in an inactive window, even when the window has focus. Screenshot [08](../../design/snimky/odchylky/08-terminal.png).<br>**Not reproducible on the host:** the CPU and the reference GPU path draw custom-title window buttons in the same `ink_muted` as a standard window (checked with a probe in the comp tests). It needs a screenshot from QEMU or hardware with the backend that produced screenshot 08. | [okna.html](../../design/okna.html) (Window modes) | comp | P3 | S |

## Files

| ID | What differs | Prototype | Library | P | Size |
|---|---|---|---|---|---|
| O17 | **File context menu has every action disabled.** Right click does select the row, but the menu is built from the state before the selection. It looks broken. Screenshot [07](../../design/snimky/odchylky/07-kontextova-soubory.png).<br>**Fixed** in `ui`: an open context menu takes the items from the rebuilt view (`refresh_context`), so it matches the selection made by the right click (host test `context_menu_reflects_selection_made_by_right_click`). | [kontextove-nabidky.html](../../design/kontextove-nabidky.html) (file) | app | P1 | S |
| O18 | **Toolbar** is two rows of text pills: Back, Forward, Up, Home, Reload, New folder, Rename, To trash, Copy, Cut, Paste.<br>The prototype has one row:<br>- Back / Forward icons;<br>- breadcrumbs;<br>- search with a magnifier;<br>- one action.<br>Item actions live in the menu and in the row below the list. | [soubory.html](../../design/soubory.html) | app, ui (breadcrumbs) | P2 | M |
| O19 | **Preview pane overflows** the right edge of the window ("Smíte: číst, měnit, přidávat, mazat, sprav..." is cut off); text does not wrap. | [soubory.html](../../design/soubory.html) (Details) | app, ui | P2 | S |
| O20 | **Files details:**<br>- large "Places" and "Preview" headings; the prototype has none;<br>- places without icons;<br>- file time is "dnes 0:20 UTC" instead of local time. | [soubory.html](../../design/soubory.html) | app | P3 | S |

## Terminal and demo apps

| ID | What differs | Prototype | Library | P | Size |
|---|---|---|---|---|---|
| O21 | **Terminal is light in the dark theme.** The cause is the same as K1, but the terminal draws its own `term_*` colours; check it together with K1.<br>**Fixed** with K1: the terminal takes the mode from the greeting, screenshot [01 after](../../design/snimky/odchylky/01-plocha-okna-po.png). | [terminal.html](../../design/terminal.html) | ui, app | P1 | within K1 |
| O22 | **Task manager** is a list of texts "Application X - running"; the prototype has a table with columns, progress and actions.<br>**Fixed:** a table with columns Task (origin icon), Started by, State, Progress (bar, `ai` for the agent) and Memory; the agent row has an `ai_soft` background, a left stripe and an "agent" chip; filter segments All / Mine / Agent / System; icon buttons in every row: pause (resume for a paused task) and cancel (`Cells::actions`, disabled when `taskd` does not allow them; Right arrow moves onto them, Enter presses); at the bottom a detail strip with the name, who started it, the state and the log. Screenshot [O22 after](../../design/snimky/odchylky/o22-ulohy-po.png). | [plocha.html](../../design/plocha.html) (Task manager) | app | P2 | M |
| O23 | **Read-only field** in the Components demo shows its text scrolled (the start is cut off).<br>**Fixed:** a single-line field without focus starts at the beginning of its text; it scrolls to the caret only with focus (test `unfocused_long_field_shows_start`). | [komponenty.html#pole](../../design/komponenty.html#pole) | ui | P3 | S |

## System dialogs, lock and sign-in

| ID | What differs | Prototype | Library | P | Size |
|---|---|---|---|---|---|
| O24 | **Lock screen in the dark theme: clock, date and name are invisible.**<br>- They are drawn with `on_accent` (`greeter.rs`), which in the dark theme is dark purple (`#1d0f40`) on a dark wallpaper.<br>- The prototype uses `ink` for the time and `ink_muted` for the date.<br>- The name is "Vyvojar", without diacritics.<br>Screenshot [14](../../design/snimky/odchylky/14-zamcena-obrazovka.png).<br>**Fixed:** clock and name `ink`, date and no-account texts `ink_muted` (as in the prototype); `on_accent` only on accent (avatar, button). The development account is named "Vývojář" (it was only the account name without diacritics).<br>**Guarded by the test** `zamek_citelny_ve_vsech_rezimech` (`libs/comp`): the contrast of clock, date and name computed from pixels against the background is at least 4.5:1 in the light, dark and contrast modes. The test also caught the light theme: the `scrim` turned the background into a mid-dark area (date 2.2:1); the lock screen now uses it only in the dark and contrast modes (the prototype has none). Screenshots [14 after](../../design/snimky/odchylky/14-zamcena-obrazovka-po.png), [light after](../../design/snimky/odchylky/14b-zamcena-obrazovka-svetla-po.png). | [prihlaseni.html](../../design/prihlaseni.html) | comp | P1 | S |
| O25 | **Submitting an empty password** gives no feedback (no message below the field).<br>**Fixed:** "Zadejte heslo." below the field (PIN, recovery key by method), nothing goes to the service, typing clears the message (test `prazdne_heslo_rekne_proc`). | [prihlaseni.html](../../design/prihlaseni.html) (wrong password) | comp | P3 | S |
| O26 | **Secure screen** matches the design. Small differences: no default (primary) button and no severity icon tile. Screenshot [13](../../design/snimky/odchylky/13-bezpecna-obrazovka.png).<br>**Fixed:** the default (safe) choice is an `accent` primary button; a severity icon tile next to the heading (info `accent_soft`, warning `warn_soft`, danger `danger_soft`; for all system dialogs). Screenshot [13 after](../../design/snimky/odchylky/13-bezpecna-obrazovka-po.png). | [dialogy.html](../../design/dialogy.html) | comp | P3 | S |

## Added to the prototypes

- **Appearance panel** in [bubliny-panely.html](../../design/bubliny-panely.html)
  (moon icon):
  - Light / Dark / Automatic as a radio group with icons and a check mark;
  - a state row that follows the choice: "Light now, following sunrise and
    sunset. Dark from 16:21 (sunset in Prague)", or for a manual choice
    "Dark now, set manually";
  - a High contrast switch;
  - an *Appearance settings* button;
  - the tooltip "Appearance: automatic (light now)".
- **Developer mode** in the same prototype:
  - a bar pill, `danger` when on;
  - a panel with a switch and a description;
  - a warning row when the mode is on;
  - a "Development build only" footer.

## Recommended order of fixes

1. **K1** (dark theme in apps) and **O24** (lock screen): small and the
   most visible.
2. **K2** (bar panels through `ui`): fixes O1 to O7 and O10 at once, so it
   is the largest task.
3. **K3, K4, K5, K6** (buttons, shadow, menu icons) and **O17** (Files
   menu).
4. The other P2 items per screen; P3 along the way.

## Versions

| Version | Date | What |
|---|---|---|
| v1 | 2026-11-16 | first comparison on `d6078624`: 12 shared-component deviations, 26 per screen; Appearance panel and Developer mode added to the prototype |
