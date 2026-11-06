# The osfs tool: the file system on the host and on Windows

`osfs` (source `tools/osfs`) works with our file system (`libs/fs`, format
described in [file-system.md](file-system.md)) on the host computer:

- creates and formats a **volume image file**,
- checks it (fsck, optionally scrub),
- lists a directory, prints a file, uploads a file,
- on Windows **mounts the image as a drive** (e.g. `Q:`) through WinFsp,
  so it can be used from Explorer and from any program.

The tool uses only the library's public API (`fs::Fs`); it does not change
the library.

> ⚠️ **For trying things out only, not for real data.** The FS is young.
> Keep data you do not want to lose backed up outside of it.
>
> ⚠️ **No real disks yet.** Formatting a disk is irreversible (it erases
> everything on it). That is why `osfs` currently refuses anything that
> looks like a disk or partition (`E:`, `\\.\PhysicalDrive1`,
> `\\?\Volume{...}`, `/dev/sdb`) and only works with image files. Disks
> will come once the safeguards are done (refuse the system disk and disks
> with mounted Windows partitions, confirmation by typing the disk number)
> - see the [v3 design](fs-v3-design.md#windows-driver).

## Installation and building

1. **WinFsp** (only for `osfs mount`): download from <https://winfsp.dev>
   and install it (needs administrator rights). `osfs` finds
   `winfsp-x64.dll` through the registry (`HKLM\SOFTWARE\WOW6432Node\WinFsp`,
   value `InstallDir`), otherwise in `C:\Program Files (x86)\WinFsp`.
2. Build:

```text
cargo build -p osfs --release
```

The result is `target\release\osfs.exe`. The WinFsp binding is our own and
small (`tools/osfs/src/mount/ffi.rs`, structures transcribed by hand from
the `inc\winfsp\*.h` headers), without a binding generator and without
clang; the DLL is loaded at run time, so the tool builds even without
WinFsp (only `osfs mount` fails).

Tests (name translation including the property test):

```text
cargo test -p osfs
```

## Commands

| Command | What it does |
|---|---|
| `osfs format <image> [--size N] [--label X] [--blake3]` | creates a **new** image file (`N` e.g. `64M`, `1G`; default 256 MiB) and formats it; never overwrites an existing file |
| `osfs check <image> [--scrub]` | consistency check **without writing** (read-only mount); `--scrub` also verifies data checksums; also lists entries Windows will not show |
| `osfs ls <image> [path]` | directory listing (default `/`), hidden names are marked `[ve Windows skryte]` |
| `osfs cat <image> <path>` | file contents to standard output |
| `osfs put <image> <file> <path>` | copies a local file into the image (overwrites an existing one) and commits |
| `osfs mount <image> <X:>` | mounts the image as drive `X:`; runs until unmounted |
| `osfs unmount <X:>` | unmounts the drive from another window (same as Ctrl+C in the `mount` window) |

Paths inside the image are written with `/` from the root
(`/documents/a.txt`). Case is not distinguished (just like on Windows);
names are stored as you typed them. The tool's own messages are in Czech.

### Example: from an empty image to a drive in Explorer

```text
> osfs format D:\test.img --size 1G --label Test
D:\test.img: naformatovano, 1024 MiB, kontrolni soucty Xxh3

> osfs put D:\test.img C:\Windows\win.ini /win.ini
/win.ini: zapsano 92 B

> osfs ls D:\test.img
file           92  win.ini

> osfs mount D:\test.img Q:
D:\test.img pripojeno jako Q: ('Test'). Odpojeni: Ctrl+C.
POZOR: FS je mlady - jen na zkouseni, ne na ostra data.
```

Explorer now shows drive `Q:` (file system `MagnumOpusFS`): creating,
opening, editing, copying, renaming and deleting files and folders works as
usual. To unmount: Ctrl+C in the `osfs mount` window, or from another
window

```text
> osfs unmount Q:
Q:: odpojeni vyzadano (osfs mount ulozi zmeny a skonci)
```

and then a check:

```text
> osfs check D:\test.img --scrub
svazek 'Test': 262144 bloku po 4096 B, volnych 262127, generace 2, soucty Xxh3
generace 2: uzly stromu 4 bloku, data 0 bloku, metadata 4 bloku
v poradku
```

If the mount ends without unmounting (crash, power loss), the image stays
in the state after the last commit plus changes confirmed by `fsync` - see
[saving](#saving-commit-and-fsync).

## How `osfs mount` works

```text
 program on Windows (Explorer, editor, ...)
        |  CreateFile / ReadFile / MoveFileEx / ...
        v
 Windows kernel -> WinFsp driver (winfsp-x64.sys)
        |  requests through the WinFsp DLL, from several threads
        v
 osfs.exe: tools/osfs/src/mount/ops.rs
        |  1. one lock around the whole Fs
        |  2. name translation   \A\b<U+F03A>c  ->  /A/b:c
        |  3. fs::Fs call        stat / read / write / create / rename / ...
        v
 libs/fs (Fs<FileDevice>)  ->  image file
```

- **Concurrency:** WinFsp calls from several threads, `Fs` is single
  threaded - to start with, there is **one lock** around it (operations run
  one at a time).
- **Path based:** an open file only remembers its path in our FS. The
  handle layer (deleted but still open files, sharing modes) comes later.
  A rename rewrites the paths of open handles under the renamed name.
- **Case:** the volume reports "case-insensitive, case-preserving". On
  open, WinFsp gets the real name back (normalized, looked up in the parent
  listing); without it WinFsp would use the upper-cased path as the file
  name and would treat a rename `a.txt` -> `A.TXT` as "no change".
- **Times:** our ns since 1970 -> FILETIME (100 ns since 1601). Creation
  time = `btime`, last write and last access = `mtime`, change = `ctime`.
- **File ID** (`IndexNumber`) = the object's `oid`.

### Saving: commit and fsync

| Event | What happens |
|---|---|
| every 5 s, when something changed | commit (atomic save of all changes) |
| `FlushFileBuffers` on a file | `fsync` - changes go to the intent log and survive power loss |
| flush of the whole drive | commit |
| unmount (Ctrl+C, `osfs unmount`, closing the window) | commit, then exit |

On power loss or an `osfs` crash at most the last few seconds of changes
not confirmed by `fsync` are lost; the image stays consistent
(copy-on-write).

## Name translation

Our FS allows almost everything (only `/`, control characters, `.`, `..`
and text direction characters are forbidden), Windows much less. The
translation follows the table in the [v3 design](fs-v3-design.md#windows-driver)
exactly and is built so that **every displayed name opens exactly the
object it came from**:

| Name in our FS | On Windows | Example |
|---|---|---|
| valid on Windows too | unchanged | `Příliš žluťoučký.txt` |
| contains a forbidden character `: \ * ? " < > \|` | character -> U+F000 + character code (like WSL and Cygwin) | `a:b` -> `a<U+F03A>b` |
| already contains a character from U+F000-F0FF | **hidden** (would collide with a translated name) | `x<U+F041>` |
| reserved name, also with an extension or a space before it | **hidden** | `CON`, `nul.txt`, `COM1.log`, `LPT9`, `NUL .txt` |
| ends with a dot or a space | **hidden** (Win32 strips them on open) | `report.`, `a ` |
| longer than 255 UTF-16 units | **hidden** | |
| symbolic link | **hidden** (the first version does not show links) | |

Reserved names are `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`,
`LPT1`-`LPT9`; in addition to the design table we also hide `COM¹ COM² COM³`
and `LPT¹ LPT² LPT³` (Windows 11 treats them as reserved) and `CONIN$`,
`CONOUT$` - hiding more is safe. `COM0` and `COM10` are not reserved.

Hidden entries still exist, Windows just does not see them. **`osfs check`
lists them** (and `osfs ls` marks them):

```text
ve Windows se nezobrazi (2):
  /CON.txt  - rezervovane jmeno Windows
  /link  - symbolicky odkaz (zatim se nezobrazuje)
```

**Writing from Windows:** a character from U+F000-F0FF that corresponds to
a forbidden character is stored as that original character
(`a<U+F03A>b` -> `a:b`). A name is rejected (`STATUS_OBJECT_NAME_INVALID`)
when:

- it contains another character from U+F000-F0FF (e.g. U+F041),
- the result would be hidden on Windows (reserved name, trailing dot),
- our FS rejects it (text direction characters - otherwise
  `invoice[U+202E]fdp.exe` could be displayed as `invoiceexe.pdf`).

The resulting rule, checked by a test: `from_windows(w) == n` exactly when
`to_windows(n) == w`. A hidden name therefore cannot be entered from
Windows at all (not even through U+F0xx or a trailing dot), so you cannot
accidentally open or delete a different object than the one Explorer
showed.

**Acceptance (property) test** `tools/osfs/tests/names.rs`: for random
names with traps (forbidden characters, U+F0xx, reserved names, trailing
dots and spaces, decomposed accents, case, long names) it creates objects
in an in-memory FS and checks that every displayed name translates back to
a path that opens exactly the object it came from, that no two names are
displayed the same (not even ignoring case), and that a hidden name cannot
be entered from Windows.

## Limitations of the first version

- Image files only, no disks (see the warning at the top).
- Attributes (read-only, hidden, ...) and manually set times are **not
  stored** - the `fs` API has no function for that yet; `SetBasicInfo` is
  accepted and returns the real state.
- No permissions (ACLs): everybody has full access.
- Symbolic links are not shown; creating a file with the same name fails
  ("name already exists").
- No named data streams (`file:stream`), extended attributes or reparse
  points.
- No deleted-but-still-open files (POSIX delete) yet - that needs the
  handle layer.
- Renaming a folder that contains an open file is refused by Windows
  itself (same as NTFS).
- Case rules differ in details: our FS compares after Unicode lower-casing
  and NFC, Windows uses its own upper-case table. Pairs like `ı`/`i`
  (Turkish dotless i) are different names for our FS but the same for
  Windows; the Windows cache may then mix them up. Each name still leads
  only to its own object in our FS.
- Free space in the drive properties becomes accurate only after a commit
  (delayed allocation).
- A panic in `osfs` = immediate exit (`panic = "abort"` profile); data stay
  in the state after the last commit.

## Debugging

`OSFS_DEBUG=1` prints the operations that change names (create, delete,
rename) to standard error:

```text
> set OSFS_DEBUG=1
> osfs mount D:\test.img Q:
osfs: create /Folder/a.txt (volby 0x3400060)
osfs: rename /Folder/a.txt -> /Folder/A.TXT (nahradit: 0)
osfs: delete /Folder/A.TXT
```

Common problems:

| Message | Cause |
|---|---|
| `nejde nacist ...winfsp-x64.dll` | WinFsp is not installed |
| `pripojeni na Q: selhalo: 0x...` | the drive letter is already taken |
| `Q: uz ho pripojuje jiny proces osfs mount` | another window is already mounting `Q:` |
| `svazek je jen pro cteni` | uncertain recovery or unreadable log - run `osfs check` |
| Windows offers to "format" a disk after inserting it | **do not click** - Windows does not know our format (once disks are supported) |

## What the `fs` API is missing (for later versions)

- **Entry name by path** (`stat` does not return the stored name): the
  driver looks it up in the parent listing for the normalized name (with a
  cache cleared on every name change). Slow for large directories.
- **Setting times and attributes** (`set_times`, read-only flag).
- **"Is there anything to save"** - commits are currently tracked by our
  own flag.
- **Atomic replace on rename** (`rename` overwriting the target) - now
  `remove` + `rename` under one lock (commit and `fsync` capture both steps
  or neither).
- **Operations by handle/oid** instead of paths (handle layer).
