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

## What it does (v1.1)

| Feature | Status |
|---|---|
| sidebar: Home, datasets (`DATASETS`), Volume, Trash | **[verified in QEMU]** (test `gui: compd: spravce souboru...`) |
| breadcrumbs, Back, Forward, Up (after going up the folder you came from is selected) | **[implemented]** (host `navigate_into_and_up_reselects`) |
| table Name, Modified, Size, Kind; sort by clicking the header, folders always first, natural name order (`2` before `10`) | **[verified in QEMU]** |
| search in the current folder (case-insensitive) | **[implemented]** (host `natural_sort_dirs_first_and_unknown_last`) |
| preview: text (first 64 KiB, CRLF tolerated), BMP image (24 and 32 bits up to 2048 x 2048 and 24 MiB as a picture, otherwise size and depth), metadata | **[verified in QEMU]** (text and picture), decoding host `preview_text_bmp_binary`, `bmp_preview_shows_image` |
| new folder, rename (name and collision check), to trash with "Undo" | **[verified in QEMU]** (trash), the rest host |
| trash of all datasets: restore, delete permanently (with confirmation) | **[implemented]** (host `trash_undo_and_trash_view`) |
| file version history: Restore, Copy beside | **[implemented]** (host `preview_shows_three_states_and_versions`) |
| copy / cut / paste through the system clipboard, also between windows: formats `application/x-mo-paths` (with the operation), `text/uri-list`, text; a list over 60 KiB only within the window | **[verified in QEMU]** (TCG and WHPX; the clipboard gesture is the serial number of the key press, not a time); formats host `clipboard_formats_roundtrip` |
| copy and move as a `files-copy` task with progress, pause and cancel | **[verified in QEMU]** |
| conflict: overwrite / skip / keep both, "for all" | **[verified in QEMU]** (keep both), the rest host `fileops` |
| task recovery after an interruption (also after a restart) | **[verified in QEMU]** (test `gui: files-copy: uloha s konfliktem prezije restart procesu`) |
| app actions `files.*` (command palette, assistant, shell) | **[verified in QEMU]** (`files.show`) |
| keys: F2 rename, Delete to trash (in the Trash delete permanently with confirmation), Backspace and Alt+Up up, Alt+Left/Right back/forward, F5, Ctrl+C/X/V (by the layout's character), Ctrl+Shift+N new folder, Escape cancels name entry | **[verified in QEMU]** (Ctrl+C, Ctrl+V from QMP), the rest host `keys_map_to_actions` |
| row context menu (right button, Shift+F10, the Menu key) and in the Trash | **[implemented]** (host `context_menu_and_open_reply`) |
| dragging selected items onto a folder = move (a rename within a dataset) | **[implemented]** (host `drop_on_folder_moves`); between windows and apps **[designed]** |
| the "open a path" service reply in the status line (second window channel) | **[implemented]** (host `context_menu_and_open_reply`) |
| a folder without the list right (someone else's home in `/home`): a lock icon, kind "Folder without access" (Czech UI text "Složka bez přístupu"); opening it shows "You do not have access to folder X." instead of the contents (another error: "Folder X cannot be opened: reason."). A refused listing is never shown as "The folder is empty" | **[implemented]** (host `foreign_folder_has_lock`, `denied_listing_is_not_empty_folder`) |
| removable media in Places, Eject, copy and move home <-> medium ([Removable media](media.md#media-in-files)) | **[implemented]** (host `media_places_and_eject`), copy and move **[verified in QEMU]** |
| "Download from address…" (File menu, palette): address check, download into `~/Stažené` via the session fetch, progress in Tasks ([Downloads](downloads.md)) | **[implemented]** (host `download_from_address`), downloading **[verified in QEMU]** |
| icon view (grid), thumbnails | **[designed]** |
| the Properties panel (context menu, Alt+Enter, action `files.properties`): the General tab and the Permissions tab (owner, shares with a level, inherited rows with their origin, inheritance into a folder's contents, turning inheritance off) | **[verified in QEMU]** (test `gui: compd: spravce souboru...`: an inherited grant from fsd, adding a share, saving); levels, read-only, confirmation host `props_tests.rs` |
| "Try aside", taking over changes, Compare versions | **[designed]** (prototype `design/soubory.html`) |
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
its own action).

### The service interface for clients

A client (the file manager, the shell) talks to the service like this
**[verified in QEMU]** (part "otevri cestu" of the file manager test):

| What | How |
|---|---|
| channel | an end of the session's service channel: init gives it to the process in the start message (`crate::open::endpoint(session)`; a duplicate, each process of the session has its own handle of the same channel) |
| request | `openwith::msg::request(path)` = the text `open <absolute path>` (UTF-8, spaces in the path allowed) and **exactly one handle**: the end of a new channel the reply comes to. `request_app(path)` = `open-app <path>`: only an app with an association, a file without one is not shown (a bare path in the shell) |
| reply | `openwith::msg::Reply::decode`: `ok <app id>` (opened), `reveal <app id>` (no app opens the type, the file manager showed the file), `none <extension>` (no app opens the path: an `open-app` request or no file manager with `reveal`; nothing was opened; the extension in lower case, empty for a folder and a file without one), `err <reason for a human>` (the path does not exist: `<path>: NOT_FOUND`, no right, the app cannot be started, it did not answer within 20 s) |
| identity | init checks the path with the FS session of the user the channel belongs to (the client must complete a relative path itself; relative = `err`) |
| invalid message | without a handle it is dropped; with a handle the reply is `err neplatná žádost` |

The shell **[verified in QEMU]** (part "otevri cestu" of the file manager
test, `shell_start`): `open` stays reading a file. The command
`start <path>` sends `open` (like a double click: a file with no app is
shown by the file manager, a folder opens in it). A bare path to a file
sends `open-app`; on `none` it shows a content summary like `open` and the
note "Pro typ .xyz není aplikace." (no app for the type). The terminal
prints where the path was opened. Only the terminal window of a logged-in
user gets the channel: init adds it to termd (tag `term::ctl::tag::OPEN`)
and termd sends a duplicate to every shell session (`SessMsg::Open`). The
fallback console has no channel: `start` is `NOT_SUPPORTED` there and a
bare path shows the summary. Limitation: the compositor picks
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

| QEMU WHPX, debug (`16fad2d*`): `LIST_STAT_PAGE` in fsd | 0.23 s |
| QEMU WHPX, debug: the `files.show` action / rows visible (with the redraw) | 0.30-0.33 s / 0.53-0.56 s |
| QEMU WHPX, debug: back to home (3 items), rows visible | 0.23 s |

The most expensive part is the listing with metadata in fsd (a `STAT` by
path for each item); speeding it up is work for fsd. In QEMU without
acceleration (TCG) the compositor composes a frame on the CPU in
0.6-1.4 s, so new content after an action appears only after ~2.6 s (two
frames) regardless of the folder size; with WHPX it is 0.2-0.3 s (the
measurements above). Under TCG the app handles a key only ~1.5 s after
the press; the clipboard does not mind, because the gesture is the serial
number of the physical press (`clipproto`), not a time window.

## Properties and permissions

The **Properties** panel opens from the row context menu, with Alt+Enter
or the action `files.properties`; with nothing selected it shows the
current folder. The panel replaces the listing (the sidebar stays),
Escape or "Close properties" (Czech UI "Zavřít vlastnosti") closes it.
The **General** tab shows the kind, location, size, folder contents,
dates and your rights (three value states like the preview).

The **Permissions** tab (`libs/filesview/src/props.rs`, data from fsd
`ACL_VIEW`, saving via `SET_ACL`):

- **Owner** and a sentence that the owner always has all rights; only
  the system changes the owner (`SET_OWNER` only for the control
  session), so it is read-only.
- **Shared:** the object's own rows. Each row has a recipient (an
  account, "Users of the computer", "Everyone"), a level and for a folder
  a switch "also for the folder's contents" (grant `INHERIT`). "Add
  share" offers accounts without the owner and without those who already
  have a row.
- **Inherited:** grants from folders above with their origin ("inherited
  from folder /home/jana/Dokumenty"), fixed here (accessibility reports
  them as disabled); they are changed in the origin folder. The switch
  "Inherit permissions from the parent folder" off = `ACL_PROTECTED`;
  before turning it off the panel asks whether to copy the inherited rows
  as own ones ("Copy inherited", "Do not copy", "Keep inheritance").
- Changes are saved only with "Save permissions" (one `SET_ACL`),
  "Discard changes" returns the loaded state.

| Level | Rights |
|---|---|
| View (Zobrazit) | list, traverse, read |
| Run (Spouštět) | View + execute |
| Edit (Upravovat) | Run + write, create, delete |
| Manage (Spravovat) | everything including changing permissions, snapshots and datasets |
| Custom (Vlastní ...) | another combination (e.g. from the `acl` command); stays unchanged until someone switches it |

The levels are ascending: each contains the previous one (host test
`levels_are_ascending_and_custom_is_kept`).

**Who may change them:** only with the Manage right (the owner always has
it). Without it the tab is read-only with an explanation ("Read only:
permissions can be changed by the owner or whoever has the Manage right.
You may: ...") and without change buttons. Someone who is not the owner
and would lose their own Manage right by saving (a row for themselves,
"Users of the computer" or "Everyone") must confirm it ("Save anyway").
The panel does not know other groups, so they do not count.

**Names:** the window gets a `names` channel to the account service from
init (operation `NAMES`, account names only, see
[User accounts](accounts.md)); an unknown number shows as "Account 1234"
or "Group 1000001", the window's user has "(you)". The number of the
window's user is the owner of the home folder.

## Security

- The window and `files-copy` get an FS session with the user's identity
  (`Profile::user_mounts`, the whole tree, rights decide), not a
  capability to the disk driver.
- The "System" badge is drawn by the compositor (manifest `system = true`).
- File contents (preview, BMP) and the saved task state are untrusted
  input: bounded sizes, no panics, a broken state is set aside as
  `.vadna`.
