# Removable media: flash drives with FAT, exFAT and NTFS

A flash drive or card with a foreign file system appears in
`/media/<label>` once inserted, and programs work with it the same way as
with files in the home folder. The foreign file system is never parsed by
the `fsd` service or the kernel: every medium gets its own `volfs` process
that sees only the channel to the medium's partition.

Owner page for: the `volfs` media process, the `volproto` protocol, the
`MEDIA_*` operations in `fsd`, the rules for writing to media
(`diskguard::media`), media tests. Protection of foreign disks in general
is in [Disks](disks.md#risk-and-foreign-disk-protection), the `fsproto`
protocol in [File system](file-system.md#the-fsproto-protocol).

## Overview

<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Removable media data path">
<defs><marker id="md1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">apps, Files</text><text class="m" x="95" y="64" text-anchor="middle">fsd session with the</text><text class="m" x="95" y="80" text-anchor="middle">/media path (user)</text>
<path class="ln" d="M180 55 L248 55" marker-end="url(#md1)"/><text class="m" x="190" y="46">fsproto</text>
<rect class="acc" x="250" y="20" width="200" height="70" rx="6"/><text class="t b" x="350" y="45" text-anchor="middle">fsd</text><text class="m" x="350" y="64" text-anchor="middle">permissions, handles, /media;</text><text class="m" x="350" y="80" text-anchor="middle">never parses a foreign FS</text>
<path class="ln" d="M450 55 L538 55" marker-end="url(#md1)"/><text class="m" x="462" y="46">volproto</text>
<rect class="warn" x="540" y="20" width="210" height="70" rx="6"/><text class="t b" x="645" y="45" text-anchor="middle">volfs (1 per medium)</text><text class="m" x="645" y="64" text-anchor="middle">FAT, exFAT, NTFS;</text><text class="m" x="645" y="80" text-anchor="middle">partition channel only</text>
<path class="ln" d="M645 90 L645 158" marker-end="url(#md1)"/><text class="m" x="652" y="130">blockproto</text>
<rect class="ok" x="540" y="160" width="210" height="80" rx="6"/><text class="t b" x="645" y="185" text-anchor="middle">usb-storage</text><text class="m" x="645" y="204" text-anchor="middle">partition view (diskguard):</text><text class="m" x="645" y="220" text-anchor="middle">the driver guards range, writes</text>
<path class="ln" d="M540 200 L452 200" marker-end="url(#md1)"/><text class="m" x="460" y="192">block-media</text>
<rect class="box" x="250" y="160" width="200" height="80" rx="6"/><text class="t b" x="350" y="185" text-anchor="middle">init</text><text class="m" x="350" y="204" text-anchor="middle">starts volfs per partition,</text><text class="m" x="350" y="220" text-anchor="middle">hands it to fsd (MEDIA_ADD)</text>
<path class="ln" d="M350 160 L350 92" marker-end="url(#md1)"/>
<text class="m" x="10" y="275">A volfs crash (damaged image) = the medium disappears from /media, fsd and the system keep running.</text>
</svg>

1. The disk driver (today `usb-storage`) finds partitions with FAT, exFAT
   or NTFS on a removable medium while reporting disks (`diskguard::media`)
   and sends the device manager a `block-media <fs> <index> <model>`
   channel for each **[verified in QEMU]**.
2. init starts a `volfs <fs> <ro|rw>` process for each partition with only
   two handles (partition channel, channel to fsd) and hands it to fsd with
   the `MEDIA_ADD` operation **[verified in QEMU]**.
3. fsd names the medium after the volume label (no label = "USB disk",
   equal names get " (2)") and forwards `/media/<name>/...` paths to the
   media process **[verified in QEMU]**.

Test: `cargo xtask test --only "vymenna media"` (suite `vymenna media`,
`user/init/src/tests/removable.rs`).

## Isolation and a media process crash

- `volfs` gets only the partition channel (`blockproto`) and the channel
  to fsd. It has no network, FS, windows or other disks; the sector range
  and write permission are enforced by the driver (`Kind::Media` view),
  not by `volfs` **[verified in QEMU]**.
- Data from the medium is untrusted. A parser error is an operation error,
  a process crash is the loss of the medium: fsd answers waiting sessions
  with an error and removes the medium **[implemented]** (fuzzing of
  damaged images in the host tests `cargo test -p volfs`).
- fsd disconnects a process that does not answer within 30 s **[verified
  in QEMU]** (the bug with two disks of the same model showed up exactly
  this way).

## When a medium may be written

Writing is possible only on a **removable USB medium** (the RMB bit from
SCSI INQUIRY) and only on a **FAT or exFAT** partition (both the partition
type and the boot sector match) **[verified in QEMU]**. Everything else is
read only:

| Medium | Writes |
|---|---|
| removable USB, FAT12/16/32 or exFAT | yes **[verified in QEMU]** |
| removable USB, NTFS | no (NTFS is read only) **[verified in QEMU]** |
| hard disk over USB (no RMB), internal disk | no |
| ESP partition, MBR extended partition, overlapping partitions | the medium is not offered |
| disk with a MagnumOpus partition | the medium is not offered (own disk) |

The host tests `cargo test -p diskguard --test media` cover internal FAT,
the ESP and MBR staying read only.

**[designed]** Strict profile (organization management): the user enables
writing for each medium separately (variant B); the default is today's
rule.

## Owner and permissions

FAT and exFAT have no permissions, NTFS ACLs are ignored. All files on the
medium belong to the **logged-in user** of the session (`Stat.owner` = the
session identity) **[verified in QEMU]**. This is a temporary solution
matching the "Work folders" (the medium is a place where the user keeps
their own data); exact mapping of owners and permissions is in the
deviations in [Architecture](architecture.md#current-deviations-from-the-design).

The read-only attribute of a file is reported in `Stat`; extended
attributes (e.g. the `sys.origin` of downloaded files,
[Downloads](downloads.md#the-sysorigin-file-origin)) are not kept by FAT,
exFAT or NTFS: a copy from home to the medium loses the origin and
files-copy logs that **[implemented]**.

## File systems

| FS | Read | Write | Implementation |
|---|---|---|---|
| FAT12/16/32, long names | yes | yes | crate `fatfs` 0.3.6 (MIT) **[verified in QEMU]** |
| exFAT | yes | yes | our own (`user/volfs/src/exfat/`) **[verified in QEMU]** |
| NTFS | yes | no | crate `ntfs` 0.4.0 (MIT/Apache-2.0) **[verified in QEMU]** |

Licences and patents: the `THIRD-PARTY.md` file in the repository root
(section Removable media file systems). exFAT: the specification is
public, the OIN patent licence covers only the "Linux System", expiry is
estimated for 2029-2030; supporting it now was the user's decision on
2026-11-16.

### exFAT

The Rust exFAT crates were not mature (no file truncation, reads past
`ValidDataLength` did not return zeros), so we have our own implementation
in the same media process:

- **Boot region:** the checksum (sectors 0-10 against sector 11) and all
  fields are checked against the partition size. A damaged main region =
  the backup one (sectors 12-23) and a read-only volume; both damaged =
  the volume is not mounted **[implemented]**.
- **Names:** entry sets (File, Stream Extension, 1-17 File Name) with a
  set checksum and a name hash; case-insensitive comparison by the
  up-case table from the medium (diacritics included). A damaged set is
  left out of listings and its slots stay used. New names have no
  characters forbidden in Windows and no trailing dot or space
  **[implemented]**.
- **Allocation:** the bitmap is kept in memory (changes go to its sector
  right away), new files are contiguous (`NoFatChain`), non-contiguous
  growth converts them to a FAT chain. `ValidDataLength`: growing a file
  only sets its length, bytes past the valid data read as zeros; a write
  past them first fills in zeros **[implemented]**.
- **Safe removal:** the first change sets the "dirty volume" flag,
  unmounting clears it and writes the percentage in use. A medium pulled
  out without unmounting is offered for a check by Windows **[verified
  in QEMU]**.
- **Read only:** TexFAT (two FATs), the media failure flag, a missing or
  damaged up-case table; files with unknown benign secondary entries are
  not changed (`NOT_SUPPORTED`) so the entries are not lost
  **[implemented]**.

The host tests `cargo test -p volfs exfat` format the image with the
independent `exfat-fs` crate (MIT, tests only) and read it back with it
after writing: long Czech names, nested folders, fragmented files, holes,
300 files in a folder (folder growth), renames, a full medium, the dirty
flag, a damaged boot region and entry set, and 300 randomly damaged
images without a crash. `exfat-fs` cannot read an empty file with first
cluster 0 (allowed by the specification and created that way by Windows),
so the tests verify only files with data.

## Writes and caching

`volfs` keeps sectors in memory and writes them to the medium at the end
of **every** changing operation; after fsd's reply the data is on the
medium. Pulling a flash drive out after a finished copy is safe, during a
copy it is not (the FS may stay inconsistent, as in Windows) **[verified
in QEMU]**. An error in the middle of an operation discards the pending
changes.

## Safe removal and pulling out

- **Eject** (Files, `FsClient::media_remove`): fsd sends `UNMOUNT`, the
  process writes the data and marks the volume clean, the medium
  disappears from `/media` **[verified in QEMU]**. A session with `/media`
  mounted for writing or an administrator may call it.
- **Pulling out without ejecting:** the `usb-storage` driver ends when the
  USB device channel closes (`blockproto::server::Disks::gone`), `volfs`
  ends with code 4 and fsd removes the medium; open handles return
  `NOT_FOUND` **[verified in QEMU]**.
- **Inserting while running:** the device manager hands a new
  `block-media` to init, which starts `volfs` as at boot **[verified in
  QEMU]**.

## The volproto protocol

`libs/volproto`: between fsd and the media process, **path based and
stateless** (fsd keeps open files as handles `1 << 62` and up, the process
gets paths). Message = `id u64`, `op u32`, fields; reply = `id u64`,
status `i32`, fields. fsd may have several requests of different sessions
in flight and pairs them by `id`; the next request of the same session
waits for the reply (ordering). `READ`/`WRITE` data at most 32 KiB.

| Op | Number | Fields -> reply |
|---|---|---|
| `INFO` | 1 | -> version, FS, label, size, free, read only |
| `STAT` | 2 | path -> `Stat` |
| `LIST` | 3 | path -> count, (name, `Stat`)... |
| `READ` | 4 | path, offset, length -> data |
| `WRITE` | 5 | path, offset, data -> written |
| `CREATE` | 6 | path, kind, exclusive -> () |
| `REMOVE` | 7 | path -> () |
| `RENAME` | 8 | from, to, replace -> () (within the medium) |
| `SET_LEN` | 9 | path, length -> () |
| `SYNC` | 10 | -> () |
| `UNMOUNT` | 11 | -> (), the process ends |
| `OPEN` | 12 | path, flags -> `Stat` |

A rename between a medium and home returns `NOT_SUPPORTED`; Files does it
as a copy **[verified in QEMU]**.

## fsd operations

In `fsproto` (details in [File system](file-system.md#the-fsproto-protocol)):

| Op | Number | Who | What |
|---|---|---|---|
| `MEDIA_ADD` | 134 | init (control session, system) | name, read only, the media process channel |
| `MEDIA_REMOVE` | 135 | administrator, session with `/media` for writing | safe removal |
| `MEDIA_LIST` | 136 | any session | name, label, FS, read only, size, free |

A session sees `/media` only when it has it among its mounted paths
(`Profile::user_mounts` gives it to user sessions) **[verified in QEMU]**.

## Tests

- `cargo xtask test --only "vymenna media"`: xtask prepares flash drives
  (`xtask/src/mediadisks.rs`): FAT32 with MBR and a Czech file name, exFAT
  with MBR (prepared by our implementation), NTFS from the `testfs1` image
  and a superfloppy FAT for inserting while running (QMP `device_add`,
  `device_del`). The test reads, writes (temporary name and rename),
  checks NTFS is read only, ejects, inserts and pulls out a medium; then
  xtask checks on the host that the FAT write is read by `fatfs` and the
  exFAT write by the independent `exfat-fs` **[verified in QEMU]**
  (x86_64).
- `cargo test -p volfs`, `cargo test -p diskguard --test media`,
  `cargo test -p volproto`.

## Limitations and next steps

- **Pulling out during a write** may leave FAT/exFAT inconsistent (as
  elsewhere); a check and repair (`chkdsk`) does not exist yet.
- Large folders are read whole on every operation (no folder cache):
  thousands of files in one folder slow copying down **[implemented]**.
- FAT time has no time zone and is taken as UTC (Windows writes local
  time); exFAT stores and reads the zone.
- The list of media in Files is refreshed by Refresh (F5), a click on a
  place and after Eject; live updates on insertion (an event from fsd)
  **[designed]**.

## Media in Files

- **Places:** every medium is in the side panel under its label (a
  read-only medium marked "(jen čtení)"), path `/media/<name>`
  **[implemented]** (host `media_places_and_eject`).
- **Eject:** a button in a medium's folder (and the `files.eject`
  action): safe removal, back home and the message "the medium X is
  ejected, you can pull it out" **[implemented]** (host
  `media_places_and_eject`; the ejection itself **[verified in QEMU]**).
- **Notification:** after attaching, init sends a system notification
  "medium attached (FAT/exFAT/NTFS…)" ([Notifications](notifications.md))
  **[implemented]**.
- **Copying and moving** home <-> medium go through `files-copy` (a task
  with progress in Tasks): a medium has no instant copy, it copies by
  reading and writing; name conflicts (Keep both = "name (2)"), moving
  from the medium home (the source on the medium disappears) and an
  interruption (process crash, a new process finishes the task, no
  temporary files left) **[verified in QEMU]**
  (`user/init/src/tests/removable_copy.rs` in the `vymenna media` suite).
  Dragging onto a folder uses the same move. There is no trash on a
  medium (fsd returns `NOT_SUPPORTED` for `TRASH` on a medium): the source
  of a move from the medium and an overwritten target on the medium are
  deleted permanently by files-copy **[verified in QEMU]**; Move to trash
  on a medium in Files ends with an error **[implemented]**, permanent
  delete with confirmation **[designed]**. An instant copy (`CLONE`)
  with a medium and a rename into a medium from our FS return
  `NOT_SUPPORTED` (copy by reading and writing), `DATASET_OF` of a path on
  a medium is `/media/<name>`.

## A host folder as a flash drive

`cargo xtask run --share <folder>` attaches a host folder as a removable
USB flash drive (QEMU vvfat: FAT16 with MBR, at most 504 MB of data)
**read only**: the disk has `readonly=on`, the driver reports write
protection and the medium is read only in `/media` **[implemented]**.
`--share-rw <folder>` allows writing; xtask prints a warning at start that
vvfat writing is experimental in QEMU and may damage files in the folder
(a backed-up folder only) **[implemented]**.
