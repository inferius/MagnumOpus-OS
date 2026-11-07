# File system format versions

An overview of all on-disk format versions, what changed between them and
**why**. The current format is described in detail in
[file-system.md](file-system.md); every chapter that changed compared to
the previous version starts with a **"Change from v2"** box. Planned changes
are in the [FS v3 design](fs-v3-design.md).

| Version | Signature | Status | Description |
|---|---|---|---|
| v1 | - | internal prototype, never released (not in the git history) | - |
| v2 | `OSFS\0\0v2` | replaced (up to commit `ff2bc6b`) | [v2 archive](fs-v2.md) |
| **v3** | `OSFS\0\0v3` | **current**, introduced in phases | [file-system.md](file-system.md) |

**Compatibility between versions:** a driver mounts only its own version;
another version (older or newer) = `NOT_SUPPORTED`. There is no v2 -> v3
conversion yet (disks from `xtask` are recreated). From v3 on, smaller
changes are added with **feature flags** (compat / ro_compat / incompat)
without a new version - see [Mount](file-system.md#mount-and-generation-fallback).

## Transition v1 -> v2

v1 was the first prototype during development; only v2 made it into the
repository. The documented change:

| Area | v1 | v2 | Why |
|---|---|---|---|
| Checksum | CRC32C (32 bits) | **XXH3-128** (default) or **BLAKE3** | 32 bits = 1 undetected error in 4 billion; a 10 TB disk has 2.5 billion blocks. 128 bits = 1 : 2^128. BLAKE3 also resists deliberate collisions (deduplication, signatures). |

What v2 brought (the starting point for the next transition): a Merkle
tree with a checksum in every pointer, copy-on-write and an atomic commit,
an intent log for a fast `fsync`, extents with delayed allocation,
allocation groups with space logs (no bitmap), TRIM, defragmentation,
subtree summaries, case-insensitive names, snapshots with deadlists, fsck
and scrub.

## Transition v2 -> v3

The design and reasons for all of v3: [fs-v3-design.md](fs-v3-design.md).
Done changes:

### On-disk format

| Area | v2 | v3 | Why |
|---|---|---|---|
| B+tree node | 4 KiB, level only 0/1 | **16 KiB** (4 blocks), level stored and checked | names up to 4 KiB and an extent with 496 checksums fit into an entry, the tree is one level shallower; the level check prevents a loop in a damaged tree |
| Pointer (`BlockPtr`) | 32 B: block, birth, checksum | **40 B**: + device (16 b), length, flags | a node has several blocks; volumes over several disks; compression/encryption/sharing is visible on the pointer |
| Superblock | no UUID or features | **UUID, compat/ro_compat/incompat feature flags**, volume devices | safely extending the format without a new version; identifying the volume |
| Object record | 64 B | **128 B**: creation and access time, generation, permissions, owner, extended attributes | stable identity (oid + generation) for handles; room for permissions and attributes without another format change |
| Names | 255 B | **1024 characters / 4 KiB**, text direction controls forbidden | long names from other systems; protection against a spoofed extension (`invoice[U+202E]fdp.exe`) |
| Object kinds | file, directory | + **symbolic link** | git repositories and Linux sources need them |
| File size | no limit (overflow) | **2^63-1 B**, checked arithmetic, saturating summaries | sizes and differences always fit `i64`; huge sparse files do not overflow summaries |
| Extent | at most 120 blocks, reserved u32 | **496 blocks**, flags u32 (reserved: preallocation) | larger node; fewer entries per large file |
| Intent log | records 1-6 | + 7 symlink, generation on create, 8 dataset marker, 9 set times | a symlink survives `fsync`; the generation stays the original after replay; one batch for several datasets; set times survive a crash |
| Permissions | none | **owner, inherited allow-only grants, visibility, shared descriptors** (`ACL`, `RO_COMPAT_ACL`), no execution on a dataset; in fsd a session identity from init and mounted paths | sharing between users, applications with only what they were given ([permissions](file-system.md#permissions-and-session-identity)) |
| Times | u64 ns since 1970 | **seconds i64 + nanoseconds u32** (flag `INCOMPAT_TIME_SECS`) | dates before 1970 and no 2262 limit - see below |
| Orphans | - | an `ORPHAN` entry (kind 9) for an open object without a name | an open file survives deletion and an atomic replace; cleanup at mount after a crash |
| Datasets | one tree per volume | **datasets** (own trees, a `DATASET` record in the root dataset, a directory entry of kind 4), snapshots with a clone count and a "hidden" flag | history per folder (project, home), writable clones without copying |

### Behaviour (no format change)

| Area | v2 | v3 | Why |
|---|---|---|---|
| Mount | unknown feature/version silently skipped | **select the generation by header, then decide**: other version/incompat = refuse, ro_compat = read-only | an older driver must not write into an old state and overwrite newer data |
| Unreadable slot / log | slot = skipped, log = end of chain | unreadable slot for `newest + 1` = **read-only**, another slot = a warning only; unreadable log = read-only with the log kept, or knowingly discarded (`discard_unreadable_log`) | an uncertain recovery must not overwrite a confirmed commit or `fsync`; a bad sector must not make the volume read-only forever |
| Generation fallback | on any error | only on `CORRUPTED`, newer slots zeroed | a transient disk error must not lose newer data |
| Device without flush | writable | **read-only** (`Durability`) | `fsync` would lie |
| `fsync` | pending changes cleared before writing | cleared only after flush, a log write error = error state | nothing may be confirmed as durable that is not |
| Overwriting data | free first, then write | write new blocks first, then relink | a write error must not leave a hole or lose data |
| `write` | error even when data was accepted | `Ok` = accepted into the buffer, error = nothing | the client must know what was accepted |
| Space | a dirty node = 1 block | a node = 4 blocks, only runs for whole nodes, reserve for freeing operations | a commit must not loop or fail on a fragmented disk; a snapshot must be deletable on a full disk |
| Values from disk | taken on trust | checks of extents, frees, lengths, parent cycles; all-or-nothing operations | a software bug with a correct checksum must not damage the allocator |
| fsck | map block -> who, always commits | bitmaps (fixed memory), works read-only too | 1 TiB volume = 96 MiB; checking during uncertain recovery without writes |
| Writing data | assembling byte by byte | whole blocks | 2x faster writes (0.7 -> 1.4 GiB/s) |

### Phase 2: history (`RO_COMPAT_HISTORY`)

A change within v3, recognised by the flag `RO_COMPAT_HISTORY` (ro_compat
bit 1). In detail: [FS history](fs-history.md#on-disk).

| Area | Phase 1 | Phase 2 | Why |
|---|---|---|---|
| tree items | 1-12 | + 13 `JOURNAL`, 14 `JSTATE`, 15 `SNAPMETA`, 16 `SETTINGS`, 17 `TRASH`, 18 `XATTR`, 19 `EXPIRY`, 20 `LHASH`, 21 `BRANCH`, 22 `HEAD`, 23 `DSLINK` | change journal, snapshot and commit metadata, dataset settings, trash, attributes, expiry index, logical hash cache, branches, dataset back-reference (fast path lookup) |
| fixed objects | 0 META, 1 root | + **2 trash** (`TRASH_OID`, parent 0) | deleting to the trash is a move, no copy |
| attribute count in the object record | always 0 | **number of `XATTR` items** (fsck checks it) | attributes exist |
| intent log | records 1-13 | + 14 `Actor`, 15 `Trash`, 16 `Restore`, 17 `Purge`, 18 `SetXattr` | the same journal numbers and users after replay; trash and attributes survive a crash |
| errors | down to -27 | + `JOURNAL_GAP` (-28) | older records were discarded by retention - never a silent skip |
| protocol v2 | op 30-53 | + **54-86** (`fsproto::hist`) | history for the file manager, indexer, sync, agents; dataset creation and deletion |

**An older volume** (without the bit) is upgraded by the driver on the first
writable mount: it sets the bit and from then on maintains the journal
(starting at number 1); snapshots from before phase 2 have no metadata
(manual, time 1970). An older driver only reads a volume with the bit -
otherwise it would not maintain the journal or the indexes when writing.

**Behaviour (no format change):** `remove` stays permanent, only `trash`
moves to the trash; every modifying operation reserves room for journal
nodes (`JOURNAL_ROOM`), otherwise a commit on a full disk might have nowhere
to store the tree; changes the commit adds itself (the journal) do not
count as an operation in progress.

### Phase 3: data (`INCOMPAT_PTR_COPY`, `INCOMPAT_DATA`)

> **Change from v3 (phase 2):** a 48 B pointer with the address of a second
> copy, an extent with flags and compression, a deadlist with a copy, items
> 24-29. **Why:** metadata 2x with self-healing, compression, instant
> copies, deduplication and preallocation need room in the format. In
> detail: [FS data](fs-data.md#on-disk).

A change within v3, recognised by the flags `INCOMPAT_PTR_COPY` (incompat
bit 2, **required**) and `INCOMPAT_DATA` (incompat bit 3); formatting sets
both.

| Area | Phase 2 | Phase 3 | Why |
|---|---|---|---|
| pointer (`BlockPtr`) | 40 B | **48 B**: + address of the second copy (offset 40) | every node 2x, repair from the copy without falling back a generation |
| superblock | root 80-120, space 120-160, label 216-248 | root **80-128**, space **128-176**, other fields from 176, label **232-264** | pointers are longer |
| internal node | 64 B entry, up to 255 children | **72 B** entry, up to **227** children | 48 B child pointer |
| extent | flags reserved (0) | bit 0 unwritten, 1 shared, 2 copy (+ address), 8-15 compression (+ physical length and bytes, one checksum) | preallocation, instant copies and deduplication, data 2x, compression in 128 KiB chunks |
| deadlist | 24 B (start, length, birth) | **32 B**: + start of the copy | the copy is freed with the first |
| tree items | 1-23 | + 24 `DATAOPT`, 25 `SHARE`, 26 `DEDUP`, 27 `SCRUB`, 28 `SCRUBERR`, 29 `DEDUPPOS` | data settings, sharing table, deduplication index, scrub state and errors, deduplication position |
| intent log | extent: block, checksums | extent in **full encoding** (compression, copy); writes into preallocated blocks | `fsync` of compressed data and in-place writes |
| protocol v2 | op 54-86 | + **110-117** (`fsproto::data`; 87-92 nested datasets) | copies, preallocation, data settings, scrub |

**An older volume** (without `INCOMPAT_PTR_COPY`, 40 B pointer) is
**rejected** by the driver (`NOT_SUPPORTED`) - a different pointer layout in
all nodes would mean double code on the hottest path; the FS is in
development, volumes are created again.

**Behaviour (no format change):** writing zero blocks creates holes (they
used to be stored); restoring from a snapshot, file versions and merge share
blocks (instant, no extra space); a shared extent is never freed
immediately; a read with a bad first copy repairs the copy (during mounting
only once the mode is decided); defragmentation skips shared, compressed and
preallocated extents; compression is **off** by default (on NVMe it would
slow a long write several times).

### Remaining in v3

Phases 4-6 (protection, other, cloud) - order in the [design](fs-v3-design.md#implementation-order).

## Object times: u64 ns -> i64 s + u32 ns

A change within v3, recognised by the flag `INCOMPAT_TIME_SECS` (incompat
bit 1).

| | Before | Now |
|---|---|---|
| storage | u64 nanoseconds since 1970 (offsets 16-40) | seconds i64 (16-40) + nanoseconds u32 (112-128, formerly reserved) |
| range | 1970 to 2262 | ±292 billion years (also 1601 from NTFS, dates before 1970) |
| intent log | a time is 8 B | a time is 12 B, new record 9 (set times) |
| protocol `Stat` | mtime, ctime u64 ns | mtime, ctime, **btime**, each seconds i64 + nanoseconds u32 |
| API | `set_time(u64)` | `set_time(Timestamp)`, new `set_times` and `set_times_h` |

**Why:** copying from old archives and from NTFS lost times before 1970, and
the 2262 limit is unnecessary. Alternatives: i128 nanoseconds (one number,
but 16 B per time - it would not fit into the 128 B record, MSVC has no
`__int128`, conversions need 128-bit division) and 100 ns since 1601 like
NTFS (range until 30828, coarser resolution). The chosen form matches
`timespec` and `SystemTime` and fits into today's record.

**Older volumes** (without the flag) are still read and written by the
driver in the old form; an older driver refuses new volumes. There is no
conversion of an old volume to the new form yet.

## Protocol fsproto v1 -> v2

| Area | v1 | v2 | Why |
|---|---|---|---|
| addressing | paths | **handles** (dataset, oid, generation) | a file survives a rename and deletion; an operation does not re-evaluate the path |
| clients | one | **sessions** via `CONNECT`, own handles | an editor, an agent and sync work at the same time and isolated |
| write | a result without a count | **number of accepted bytes** | the client knows what the service accepted, also for a write in several messages |
| save | overwrite | **conditional save** with a revision (`CONFLICT`) | no silent overwrite of someone else's change |
| locks and sharing | - | range locks, share modes, delete on close | the Windows layer and databases |
| listing | pages by cursor | also on a handle (`LIST_H`) | the same guarantees when the directory changes |

v1 operations remain (compatibility); a new client asks with `HELLO`.
