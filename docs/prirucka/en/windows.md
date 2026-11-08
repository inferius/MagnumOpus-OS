# Windows

Every window has a **title** at the top. On the left is the **trust badge**
(who the window belongs to and what the app is allowed to do). On the right
are the buttons **minimize**, **maximize** and **close**. The system draws
the title and the buttons, so they work even when the app freezes.

<!-- snimek: okno-titulek -->

<figure class="diagram">
<svg viewBox="0 0 760 120" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Window title">
<rect class="box" x="10" y="10" width="740" height="100" rx="14"/>
<rect class="acc" x="10" y="10" width="740" height="40" rx="14"/>
<rect class="warn" x="22" y="17" width="130" height="26" rx="8"/><text class="m" x="87" y="35" text-anchor="middle">trust badge</text>
<text class="t b" x="380" y="36" text-anchor="middle">window name (grab here with the mouse)</text>
<text class="t" x="650" y="36">—</text><text class="t" x="684" y="36">□</text><text class="t" x="716" y="36">✕</text>
<text class="m" x="380" y="85" text-anchor="middle">app content</text>
</svg>
<figcaption>The window title: the badge on the left, the name in the middle, the buttons on the right.</figcaption>
</figure>

## Moving and resizing

- **Move:** grab the window by its title and drag.
- **Resize:** drag any edge or corner of the window.
- **Maximize:** the □ button in the title. Click again to restore the window.
- **Minimize:** the — button. The window stays in the bar; click it to bring it back.
- **Close:** the ✕ button.

## Snapping to half of the screen

Hold the mouse over the **maximize** button for a moment (about half a
second). A menu appears:

| Option | What it does |
|---|---|
| Maximize | the window fills the whole desktop above the bar |
| Full screen | the window takes the whole screen, the bar is hidden |
| Snap left | the window fills the left half |
| Snap right | the window fills the right half |

Snapping by dragging a window to an edge and by keys is *coming soon*.

## Full screen

- **Turn on:** `Super+Shift+F`, the **Full screen** action in the palette,
  or the menu over the maximize button.
- **Exit:** press `Super+Shift+F` again, use **Exit full screen** in the
  palette, or move the mouse to the top edge: a badge with an exit button
  slides out.

In full screen the bar is hidden and notifications are not shown. The `F11`
key belongs to apps (for example a video player); the system does not take it.

## Switching between windows

- Click the app icon in the **bar**.
- Or open the **palette** (`Ctrl+K`) and type part of the window's name.

Switching with the `Alt+Tab` keys is *coming soon*.

## When an app does not respond

When an app does not answer for 3 seconds, the system dims its window and
offers to **wait** or **close**. The other windows and the bar keep working,
and you can still move and close the window.
