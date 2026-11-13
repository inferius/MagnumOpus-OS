# File manager

The **Files** app (`user/gui-files`) browses the user's file system and
works with it: folders, datasets, the trash, version history. It is built
on the [UI library](gui.md#native-ui-library-libsui) and the
[fsproto](file-system.md#the-fsproto-protocol) protocol. Copying and
moving are done by the helper process **`files-copy`**, so a task
survives closing the window, shows up in the [task overview](architecture.md)
and continues after an interruption. The look follows the prototype
`design/soubory.html`.

Code:

| Part | Where |
|---|---|
| model and interface (host tests) | `libs/filesview` |
| copy engine, task journal, protocol with `files-copy` | `libs/fileops` |
| type associations and "open a path" messages | `libs/openwith` |
| app (backend over `FsClient`) | `user/gui-files` |
| copy helper process | `user/files-copy` |
| start per session, "open a path" service | `user/init/src/files.rs`, `user/init/src/open.rs` |
| QEMU tests | `user/init/src/tests/gui_files.rs` (suite `gui`) |

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="File manager and its processes">
<defs><marker id="fm1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="fm1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="210" height="110" rx="6"/><text class="t b" x="115" y="45" text-anchor="middle">gui-files (window)</text><text class="m" x="115" y="66" text-anchor="middle">filesview: listing, preview</text><text class="m" x="115" y="84" text-anchor="middle">trash, versions, clipboard</text><text class="m" x="115" y="102" text-anchor="middle">files.* actions (palette, shell)</text>
<rect class="acc" x="280" y="20" width="200" height="110" rx="6"/><text class="t b" x="380" y="45" text-anchor="middle">files-copy</text><text class="m" x="380" y="66" text-anchor="middle">fileops: copy in steps</text><text class="m" x="380" y="84" text-anchor="middle">conflicts, recovery</text><text class="m" x="380" y="102" text-anchor="middle">state in the home</text>
<rect class="box" x="550" y="20" width="200" height="50" rx="6"/><text class="t b" x="650" y="42" text-anchor="middle">taskd</text><text class="m" x="650" y="60" text-anchor="middle">task overview</text>
<rect class="box" x="550" y="90" width="200" height="60" rx="6"/><text class="t b" x="650" y="112" text-anchor="middle">fsd</text><text class="m" x="650" y="130" text-anchor="middle">user's FS session</text>
<rect class="ok" x="10" y="190" width="210" height="80" rx="6"/><text class="t b" x="115" y="214" text-anchor="middle">init</text><text class="m" x="115" y="234" text-anchor="middle">starts per session</text><text class="m" x="115" y="252" text-anchor="middle">"open a path" service</text>
<rect class="box" x="280" y="190" width="200" height="80" rx="6"/><text class="t b" x="380" y="214" text-anchor="middle">compd</text><text class="m" x="380" y="234" text-anchor="middle">window, app actions</text><text class="m" x="380" y="252" text-anchor="middle">accessibility</text>
<path class="ln-acc" d="M220 75 L278 75" marker-end="url(#fm1a)"/><text class="m" x="249" y="67" text-anchor="middle">task</text>
<path class="ln" d="M480 45 L548 45" marker-end="url(#fm1)"/>
<path class="ln" d="M480 110 L548 115" marker-end="url(#fm1)"/>
<path class="ln" d="M220 30 L250 30 L250 10 L700 10 L700 88" marker-end="url(#fm1)"/><text class="m" x="470" y="8" text-anchor="middle">listing, trash, versions</text>
<path class="ln" d="M115 188 L115 132" marker-end="url(#fm1)"/><text class="m" x="160" y="164" text-anchor="middle">open a path</text>
<path class="ln" d="M220 230 L278 230" marker-end="url(#fm1)"/><text class="m" x="249" y="222" text-anchor="middle">action</text>
<path class="ln" d="M380 188 L380 132" marker-end="url(#fm1)"/>
<text class="m" x="10" y="292">The window and files-copy each have their own FS session with the user's identity; neither gets a capability to a driver.</text>
</svg>
<figcaption>The window and the copy helper; init starts them for the user's session.</figcaption>
</figure>

## What it does (v1)

| Feature | Status |
|---|---|
| sidebar: Home, datasets (`DATASETS`), Volume, Trash | **[verified in QEMU]** (test `gui: compd: spravce souboru...`) |
| breadcrumbs, Back, Forward, Up (after going up the folder you came from is selected) | **[implemented]** (host `navigate_into_and_up_reselects`) |
| table Name, Modified, Size, Kind; sort by clicking the header, folders always first, natural name order (`2` before `10`) | **[verified in QEMU]** |
| search in the current folder (case-insensitive) | **[implemented]** (host `natural_sort_dirs_first_and_unknown_last`) |
| preview: text (first 64 KiB, CRLF tolerated), BMP (size and depth), metadata | **[verified in QEMU]** (text), BMP host `preview_text_bmp_binary` |
| new folder, rename (name and collision check), to trash with "Undo" | **[verified in QEMU]** (trash), the rest host |
| trash of all datasets: restore, delete permanently (with confirmation) | **[implemented]** (host `trash_undo_and_trash_view`) |
| file version history: Restore, Copy beside | **[implemented]** (host `preview_shows_three_states_and_versions`) |
| copy / cut / paste through the window clipboard | **[verified in QEMU]** |
| copy and move as a `files-copy` task with progress, pause and cancel | **[verified in QEMU]** |
| conflict: overwrite / skip / keep both, "for all" | **[verified in QEMU]** (keep both), the rest host `fileops` |
| task recovery after an interruption (also after a restart) | **[verified in QEMU]** (test `gui: files-copy: uloha s konfliktem prezije restart procesu`) |
| app actions `files.*` (command palette, assistant, shell) | **[verified in QEMU]** (`files.show`) |
| app keys F2, Delete, Backspace, Ctrl+C/X/V | **[designed]** - waits for app keys in `libs/ui`; today through the palette (Ctrl+K) and buttons |
| clipboard between windows and apps (a list of paths) | **[designed]** - waits for a clipboard with formats in `libs/ui` |
| mouse dragging (inside the window, between windows) | **[designed]** - waits for drop targets in `libs/ui` |
| icon view (grid), thumbnails | **[designed]** |
| "Try aside", taking over changes, Compare versions, editing rights | **[designed]** (prototype `design/soubory.html`) |
| "Open with..." dialog (`design/dialogy.html`) | **[designed]** - once system dialogs exist |

## Three states of values

For every value the FS says whether it is **known**, **hidden** (it
exists but the session must not see it, typically a folder summary
without the `MANAGE` right) or **unavailable** (the FS does not have it,
e.g. the summary at a dataset boundary); masks `known`/`hidden`, bits
`fsproto::avail`. The file manager never shows a zero instead of an
unknown value:

- the table shows "—" instead of the value,
- the preview gives the reason: "Velikost: — (skryto: chybí právo
  Spravovat)", "— (není k dispozici)",
- sorting always puts unknown values last (ascending and descending).

The size and item count of a folder are instant from the subtree summary
(`tree_bytes`, `tree_files`, `tree_dirs`), nothing is walked.
**[verified in QEMU]**, host test `preview_shows_three_states_and_versions`.

## Copying and moving

The engine is in `libs/fileops` (`Job`) and runs in `files-copy`. Each
step does bounded work (one instant copy, one rename or a 256 KiB block),
so between steps the process handles messages (pause, cancel, a conflict
decision).

- The **instant FS copy** (`clone_path`, block sharing) works across the
  whole volume, including between datasets; copying by reading and
  writing is only the fallback for `NOT_SUPPORTED` (an encrypted vault).
  **[verified in QEMU]**
- An item is assembled under a temporary name next to the target
  (`.<name>.kopie-<task>`, hidden in the listing) and renamed to the target
  when done: **the target is never half-written**.
- **Overwrite** never deletes: the old target goes to the trash (it can
  be restored).
- **Move** within one dataset is a rename (instant); between datasets a
  copy and the source to the trash.
- A folder cannot be copied into itself (an item error, the others go
  on).
- A copy into the same folder gets a free name without asking
  (`zprava (2).txt`).

**Recovery after an interruption.** The task state (`Job::encode`) is
saved to `<home>/apps/org.magnumopus.files/ulohy/<id>.uloha` after every
item and at the latest after 8 MiB of data, written through a temporary
file and `RENAME_REPLACE`. Data and state go through the same FS with
atomic commits in order, so after a power cut the state never runs ahead
of the data. When the session starts, `files-copy` loads unfinished
tasks: an instant copy is repeated, a copy by reading continues from the
saved position, a conflict asks again. The host test
`resume_after_interruption_*` interrupts a task after **every** step and
checks the same result as without the interruption. The short window
between finishing an item and saving the state can, after recovery, ask
about a conflict with its own copy (rare, nothing is lost).

**Task overview.** `files-copy` is a `taskd` producer: the task
"Kopírování 3 položek do Projekty" with progress, and Cancel, Pause and
Resume from the Task manager and the `tasks` command.

## Opening files and type associations

An app declares in its manifest what it opens:

```toml
[open]
action = "editor.open"        # action with a required path parameter of type path
extensions = ["txt", "md"]
folders = false               # opens folders (the file manager)
reveal = false                # shows a file no app opens (the file manager)
```

The **"open a path"** service in init (`user/init/src/open.rs`) has its
own channel for each session, so it checks the path with the user's
identity (`STAT` through their FS session) and starts the app in their
session if needed. Flow **[verified in QEMU]** (file manager test, part
"otevri cestu"):

1. a request `open <path>` with a channel for the reply;
2. a folder -> the app with `folders`, a file -> the first app with its
   extension, otherwise the app with `reveal` (the file manager selects
   the file in its folder);
3. init calls the app action through the compositor; if the app is not
   running it starts it in the session and repeats the call;
4. the reply `ok <app>`, `reveal <app>` or `err <reason>`.

Enter or a double click on a file in the file manager sends the path to
the service; the window does not wait for the reply (the service may call
its own action). Hooking up the shell (a command and "bare" paths)
**[designed]**: `open` in the shell already reads a file as bytes, the
coordinator decides the command name. Limitation: the compositor picks
the app by id, not by session (several users logged in at once do not
exist yet).

## Performance

The listing is sorted and filtered once on a change (O(n log n)); row
texts (date, size) are made only for visible rows and the table reads data
lazily. The accessibility tree carries only visible rows.

| Measurement (folder with 10,000 files) | Result |
|---|---|
| host, debug (`36c93f0`): load + sort, rebuilding the interface | 39 ms + 28 ms, 0.16 ms (test `ten_thousand_items_are_fast`) |
| QEMU TCG, release (`b0ccf9d*`): `LIST_STAT_PAGE` in fsd | 0.56-0.8 s |
| QEMU TCG, release: the `files.show` action in the app in total | 0.63-0.87 s |
| QEMU TCG, debug: `LIST_STAT_PAGE` in fsd | 3.2 s |

The most expensive part is the listing with metadata in fsd (a `STAT` by
path for each item); speeding it up is work for fsd. Redrawing after an
app action has a delay of ~2.6 s in the compositor or `libs/ui`
regardless of the folder size (reported to the GUI thread); a mouse click
redraws at once.

## Security

- The window and `files-copy` get an FS session with the user's identity
  (`Profile::user_mounts`, the whole tree, rights decide), not a
  capability to the disk driver.
- The "System" badge is drawn by the compositor (manifest `system = true`).
- File contents (preview, BMP) and the saved task state are untrusted
  input: bounded sizes, no panics, a broken state is set aside as
  `.vadna`.
