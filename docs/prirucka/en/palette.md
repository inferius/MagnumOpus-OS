# Command palette

The Command palette is the **gateway to the whole system**. With one
shortcut you reach windows, apps, system actions and the commands of the
app you are working with. You do not have to remember where things are:
just type what you want.

## How to open it

Press **`Ctrl+K`** (or **`Super+K`**, the key with the logo). A search box
and a list of actions open in the middle of the upper part of the screen.

The palette reacts only to a real keyboard. A program cannot open it or
pick anything in it for you.

<!-- snimek: paleta-otevrena -->

<figure class="diagram">
<svg viewBox="0 0 760 270" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Command palette">
<rect class="box" x="130" y="10" width="500" height="250" rx="16"/>
<rect class="acc" x="150" y="26" width="460" height="38" rx="10"/><text class="t" x="168" y="50">🔍 term</text>
<text class="m b" x="160" y="88">RECENT</text>
<rect class="acc" x="150" y="96" width="460" height="30" rx="8"/><text class="t" x="168" y="116">Terminal</text><text class="m" x="595" y="116" text-anchor="end">App</text>
<text class="m b" x="160" y="146">WINDOW</text>
<text class="t" x="168" y="170">~/projekt - Terminal</text><text class="m" x="595" y="170" text-anchor="end">switch</text>
<text class="m b" x="160" y="196">SYSTEM</text>
<text class="t" x="168" y="220">Diagnostic snapshot</text><text class="m" x="595" y="220" text-anchor="end">Ctrl+Alt+Shift+S</text>
<text class="m" x="380" y="250" text-anchor="middle">↑ ↓ select · Enter run · Esc close</text>
</svg>
<figcaption>The palette: type a few letters and pick an action. Recently used ones are at the top.</figcaption>
</figure>

## How to use it

1. Start typing. Capital letters and accents do not matter:
   "terminal" also finds "Terminál".
2. Use the up and down arrows to pick an action.
3. **Enter** runs it, **Esc** closes the palette.

An action that cannot be undone (such as shutting down) asks you to
confirm once more.

## What you will find in the palette

| Group | Examples |
|---|---|
| Recent | the last 8 actions you used |
| Window | switch to any open window, Full screen |
| App | start an app (Terminal, Task manager, Demo) |
| System | Diagnostic snapshot, Restart, Shut down |
| Active app | commands from its menu, if the app offers them |

So the palette also replaces window switching: type part of a window's name
and press Enter.

Locking the screen from the palette is *coming soon*.
