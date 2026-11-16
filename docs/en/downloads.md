# Downloading from the web into the Downloads folder

Files has a **Download from address…** command (File menu, Ctrl+K
palette). The file is saved into `~/Stažené` (Downloads), progress is in
Tasks (it can be cancelled from the Task manager), and a notification
arrives at the end. Every downloaded file remembers where and when it came
from (`sys.origin`), and no app or user can fake or remove this record.

Owner page for: the `save` command of the `fetch` program, the init
module `downloads.rs`, the `ORIGIN` mount flag and the `sys.origin`
attribute, streaming bodies in `libs/httpc` (`Sink`). The HTTP(S) client
in general: [TLS and HTTP](tls-http.md), notifications:
[Notifications](notifications.md).

## Overview

<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Downloads">
<defs><marker id="dl1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">Files</text><text class="m" x="95" y="64" text-anchor="middle">"Download from address…"</text><text class="m" x="95" y="80" text-anchor="middle">save-bg &lt;url&gt;</text>
<path class="ln" d="M180 55 L258 55" marker-end="url(#dl1)"/>
<rect class="warn" x="260" y="20" width="220" height="70" rx="6"/><text class="t b" x="370" y="45" text-anchor="middle">fetch (session job)</text><text class="m" x="370" y="64" text-anchor="middle">network only 80/443 and DNS,</text><text class="m" x="370" y="80" text-anchor="middle">FS only /Stazene (ORIGIN)</text>
<path class="ln" d="M480 40 L578 30" marker-end="url(#dl1)"/>
<path class="ln" d="M480 70 L578 100" marker-end="url(#dl1)"/>
<rect class="ok" x="580" y="10" width="170" height="50" rx="6"/><text class="t b" x="665" y="32" text-anchor="middle">~/Stažené</text><text class="m" x="665" y="50" text-anchor="middle">file + sys.origin</text>
<rect class="acc" x="580" y="80" width="170" height="50" rx="6"/><text class="t b" x="665" y="102" text-anchor="middle">taskd, notifyd</text><text class="m" x="665" y="120" text-anchor="middle">progress, notification</text>
<rect class="box" x="260" y="160" width="220" height="70" rx="6"/><text class="t b" x="370" y="185" text-anchor="middle">init (downloads.rs)</text><text class="m" x="370" y="204" text-anchor="middle">starts fetch per session,</text><text class="m" x="370" y="220" text-anchor="middle">mints sessions, channels</text>
<path class="ln" d="M370 160 L370 92" marker-end="url(#dl1)"/>
</svg>

- **Who starts fetch:** init, at the first Files window of the session,
  in the session job (downloads survive closing the window and end with
  logout) **[implemented]**. Files itself cannot make a session with
  `ORIGIN` or a network session; this is why it deviates from the design
  "Files starts fetch": the window only gets a command channel.
- **fetch capabilities:** a network session from the manifest (TCP 80 and
  443 only, DNS), `/certs` read only, a channel to `revocd`, **an FS
  session with the user's identity that sees only `/Stazene`**
  (`~/Stažené`, the `mount_flags::ORIGIN` flag), a taskd producer, a
  notification sender. It sees nothing else of the home folder
  **[verified in QEMU]** (test `stahovani: save: soubor, puvod, uloha,
  oznameni a chyby`).

## The save command

`save <url> [name]` (reply `ok:<name> <bytes>` or `err:<CODE> <text for
the user>`) and `save-bg` (no reply; Files):

1. the name comes from the address (the last path part, `%xx` decoded,
   without characters forbidden in Windows), a taken name =
   `name (2).ext`; an existing file is never overwritten **[verified in
   QEMU]**;
2. the body goes in parts (`httpc::Sink`, 256 KiB writes) into a hidden
   `.name.stahuje`, its size is not limited by the process memory
   **[verified in QEMU]** (1 MB over HTTP/2 and TLS);
3. a known length and too little space (less than the size + 16 MiB) =
   an error right away, not halfway **[implemented]**;
4. at the end a rename and writing `sys.origin` = `<url>\n<time ns since
   1970>` **[verified in QEMU]**;
5. an error = the temporary file is deleted, the task fails with a
   description, a High priority notification **[verified in QEMU]**.

Clear errors: 404 "the file does not exist on the server (404)", 401/403,
5xx, "the secure connection cannot be verified: …" (unknown CA, wrong
name, revoked certificate), "not enough space on the disk", "the server
is unreachable", "the server address does not exist (DNS)", a redirect
from https to http refused **[implemented]** (404 and unknown CA
**[verified in QEMU]**).

## The sys.origin file origin

- Only the system writes the `sys.` namespace (`libs/fs`, sessions
  without an identity). `sys.origin` may also be written by a session that
  sees the path through a mount with `mount_flags::ORIGIN` and has write
  permission to the file; fsd writes it as the system (`serve_hist.rs`,
  `origin_allowed`) **[verified in QEMU]**.
- A user or an app can neither fake nor remove the origin
  (`ACCESS_DENIED`); they may write their own `user.*` attributes
  **[verified in QEMU]**.
- A **copy** of the file does not carry the origin (a user session may
  not write `sys.*`); a move within the FS keeps it. A copy to a removable
  medium (FAT, exFAT, NTFS without extended attributes) logs
  `files-copy: … puvod souboru (sys.origin) se do … neprenese`
  **[implemented]**. Carrying the origin over in a copy by a service
  **[designed]**.

## Streaming in httpc

`httpc::Sink` gets the status and headers (`head`, with the length from
`Content-Length`) and then the data in parts (`data`), for HTTP/1.1
(length, chunked, until the connection closes) and HTTP/2. A sink error
(disk full, cancel) ends the request. `Client::request_to` does not pass
redirect bodies to the sink. The original API (`send`, `request`) wraps
`VecSink` with the `Limits::max_body` limit **[implemented]** (`cargo test
-p httpc sink`).

## Limitations

- Commands of one fetch run in order: a second download waits for the
  first (parallel downloads **[designed]**).
- Resuming an interrupted download (`Range`) **[designed]**.
- An icon and actions in the notification (Open, Show in folder)
  **[designed]**.
