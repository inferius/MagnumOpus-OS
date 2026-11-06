# Extended file system API for Windows applications

On a drive mounted with `osfs mount` (see [osfs.md](osfs.md)) you can work
normally through Win32 - open, read, write, rename, delete. This API adds
what Win32 cannot do and what our FS knows:

| What | What for |
|---|---|
| **instant directory sizes** (`tree_bytes`, `tree_files`, `tree_dirs`) | a Size column for folders without walking the subtree |
| **object identity** (`oid` + `generation`) | track a file across renames and moves |
| **names hidden on Windows** (`CON.txt`, `a.`, names with U+F0xx, links) | show them, read them, rename them to a valid name |
| **symbolic link targets** | show a link and where it points |
| **snapshots** | list, create, browse and read old versions |
| **volume state** | generation, free space, read-only, log recovery |

Source: protocol and client `libs/osfs-proto`, server `tools/osfs/src/api.rs`
(called from the driver `tools/osfs/src/mount/ops.rs`, function `control`).

## How it works

```text
 application (MagnumOpus Explorer, osfs info, ...)
     |  Client::open("Q:\")  ->  CreateFileW(drive root)
     |  DeviceIoControl(IOCTL_READ / IOCTL_WRITE, request -> response)
     v
 Windows kernel -> WinFsp -> osfs.exe: Control callback
     |  api::handle: decode, check the code, execute under the Fs lock
     v
 libs/fs (Fs)
```

- **Channel:** `DeviceIoControl` on a handle of the drive root. The future
  native kernel driver will serve the same codes, so the application does
  not see which driver runs and nothing needs to change.
- **Recognizing the volume:** `GetVolumeInformationW` returns the file
  system name `MagnumOpusFS` (`osfs_proto::FS_NAME`, `Client::is_osfs`).
- **Two codes:** `IOCTL_READ` (read-only operations, a handle opened for
  reading is enough) and `IOCTL_WRITE` (rename, snapshot, commit - Windows
  lets it through only on a handle also opened for writing). The server
  also rejects a write operation sent with the read code (`ACCESS_DENIED`).
- **Messages:** small binary messages with a `MOFS` header, version and
  operation code; little-endian numbers, string = u16 length + UTF-8
  (`libs/osfs-proto/src/msg.rs`). Decoding never panics - the input comes
  from another process (and, in the native driver, into the kernel).
- **4 KiB limit:** WinFsp passes `DeviceIoControl` only up to 4 KiB in each
  direction (`FSP_FSCTL_DEVICECONTROL_SIZEMAX`). That is why listings and
  the snapshot list are paged and reads go in 4080 B pieces (`MAX_READ`).
  The server reports its limit in `Hello` (`max_message`); the native
  driver may allow more.

## Operations

| Operation | Code | Input | Output |
|---|---|---|---|
| `Hello` | read | - | version, capabilities (`feature::*`), server name, `max_message` |
| `VolumeInfo` | read | - | label, blocks, free, generation, checksum algorithm, read-only, recovery |
| `Stat` | read | `at`, path | `EntryInfo` (stored name, including case) |
| `List` | read | `at`, path, cursor | a page of `EntryInfo` + next page cursor |
| `Read` | read | `at`, path, offset, length | data (at most `MAX_READ`) |
| `Snapshots` | read | `after` (id) | a page of snapshots + `next` |
| `Rename` | write | from, to | generation |
| `SnapshotCreate` | write | name | snapshot id (saves = commit) |
| `Commit` | write | - | generation of the saved state |

- **`at`** = snapshot name, empty = live state.
- **Path** is either `PathArg::Fs("/a/CON.txt")` - names as they are in our
  FS (hidden ones too), or `PathArg::Windows("\a\b")` - as Windows sees it
  (translated the same way as when opening a file;
  `PathArg::from_windows_full("Q:\a\b")`).
- **`EntryInfo`:** `name` (our FS), `windows_name` (empty = hidden),
  `hidden` (why), `kind`, `oid`, `generation`, `size`, times `mtime`,
  `ctime`, `btime` (ns since 1970), `tree_*`, `link_target`, `truncated`.
  `windows_name` describes only the name itself: an entry inside a hidden
  directory is unreachable from Windows even if its own name is valid.
- **Deleting a snapshot** is deliberately not offered: snapshots protect
  against ransomware and any user program could otherwise delete them. It
  will come with permissions (v3 design).

### Errors

The response carries `status`: 0 = OK, otherwise a code from
`osfs_proto::error` (same numbers as `abi::Error` in MagnumOpus OS, checked
by a test). The client turns them into `io::Error` with a readable name,
e.g. `osfs: NOT_FOUND (-8)`.

## Example (Rust)

```rust
use osfs_proto::{Hidden, PathArg, client::Client};
use std::path::Path;

let root = Path::new("Q:\\");
if Client::is_osfs(root) {
    let mut c = Client::open(root)?;
    let v = c.volume_info()?;
    println!("{}: free {} B", v.label, v.free_blocks * v.block_size as u64);

    // Folder listing including hidden names and instant sizes.
    for e in c.list_all("", &PathArg::from_windows_full("Q:\\Photos"))? {
        let size = if e.kind == osfs_proto::Kind::Dir { e.tree_bytes } else { e.size };
        let note = if e.hidden == Hidden::Visible { "" } else { " (hidden on Windows)" };
        println!("{} {size} B{note}", e.name);
    }

    // An old version of a file from a snapshot.
    let old = c.read("yesterday", &PathArg::Fs("/Photos/notes.txt".into()), 0, 1 << 20)?;

    // Rename a hidden name to a valid one (then Windows sees it too).
    c.rename(&PathArg::Fs("/CON.txt".into()), &PathArg::Windows("\\CON_.txt".into()))?;
}
# Ok::<(), std::io::Error>(())
```

The same from the command line (a demo and a manual test of the API; the
tool's messages are in Czech):

```text
> osfs info Q:
server: osfs 0.1.0 (WinFsp) (protokol 1, schopnosti 0x7)
svazek 'Zkouska': 16384 bloku po 4096 B, volnych 13498, generace 17

> osfs ls Q: /
file           11  CON.txt   [ve Windows skryte: rezervovane jmeno]
dir      10846218  slozka2
file           11  x:y?.txt

> osfs snapshot Q: before-edit
snapshot 'before-edit' vytvoren (id 1)

> osfs cat Q: /version.txt --at before-edit
original
```

## Using it in another project

`libs/osfs-proto` is a standalone crate without dependencies (edition 2021,
Rust 1.85+, `no_std` + `alloc`; the `client` feature adds the Windows
client). A project without access to this repository (e.g. with CI on
GitHub) can copy it; the protocol is versioned (`VERSION`, `Hello`), so the
copy stays compatible until the version changes.

## Performance (rough, WinFsp, debug build)

| Operation | Time |
|---|---|
| listing 3000 entries (`list_all`) | 0.12 s |
| reading 10 MiB (`read`, 4 KiB at a time) | 0.25 s |

For normal file reading and writing, plain Win32 through the drive is
faster; `Read` through the API is mainly for hidden names and snapshots.

## Limitations and risks

- The listing cursor is the folded name of the last entry (it survives
  directory changes between pages). For names longer than 1 KiB the
  position in the listing snapshot is used, which is valid only with the
  same handle (`Client` keeps it).
- An entry that does not fit into 4 KiB even on its own (a ~4 KiB name + a
  link target) arrives truncated with `truncated = true`.
- A rename through the API happens behind the Windows cache's back:
  Explorer may show the old name for up to 1 s (`FileInfoTimeout`).
- The `fs` API cannot read a link target inside a snapshot yet
  (`link_target` empty).
- `Client` is not `Sync`: each thread opens its own (opening is cheap).
