# Getting started

MagnumOpus is a new operating system. This guide describes only what
really works today. Anything that is still planned is marked with the words
*coming soon*.

## Starting up

When you turn on the computer, the system goes straight to the **desktop**:
the bar is at the bottom and the app windows are above it. If the graphics
do not start, a text console appears and you can control the system with
commands.

There is no login screen yet (*coming soon*). The system signs you in
automatically. The system already supports accounts with a password, a PIN
and a security key. They just do not have a screen yet.

<!-- snimek: plocha-po-startu -->

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Desktop after startup">
<rect class="box" x="10" y="10" width="740" height="280" rx="10"/>
<rect class="acc" x="60" y="40" width="340" height="170" rx="12"/><text class="t b" x="230" y="64" text-anchor="middle">app window</text><text class="m" x="230" y="84" text-anchor="middle">title with a trust badge</text>
<rect class="box" x="300" y="80" width="360" height="160" rx="12"/><text class="t" x="480" y="160" text-anchor="middle">another window</text>
<rect class="ok" x="20" y="250" width="720" height="32" rx="10"/>
<text class="t b" x="44" y="271">⊞</text><text class="m" x="80" y="271">apps</text><text class="m" x="560" y="271">tasks · network · sound · notifications</text><text class="t b" x="700" y="271">14:05</text>
</svg>
<figcaption>The desktop: windows on top, the bar at the bottom. The launcher and apps are on the left, status and clock on the right.</figcaption>
</figure>

## First steps

1. **Open the Command palette** with the `Ctrl+K` key. Type what you want
   to do (for example "terminal") and press Enter. The palette is the
   fastest way to everything in the system, see
   [Command palette](palette.md).
2. **Start an app** by clicking its icon in the bar or in the launcher
   (the grid icon at the bottom left).
3. **Windows** move when you drag the title, and you resize them by
   dragging an edge, see [Windows](windows.md).

## What is in the system now

| App | What it is for |
|---|---|
| Terminal | commands, working with files, the `help` command |
| Task manager | what is running right now (with sample data for now) |
| Demo | a sample app with controls |

A file manager, Settings and other apps are *coming soon*.

## Shutting down and restarting

Open the palette (`Ctrl+K`) and type **Shut down** or **Restart**.
The system asks you to confirm. In the terminal, the commands `shutdown`
and `reboot` do the same.
