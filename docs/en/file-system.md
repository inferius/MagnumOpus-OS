# File system

Our own file system (working name "OSFS", format v3). The code lives in the
`libs/fs` library (`no_std`, the same on the host and in the system), the
service on top of a disk is `user/fsd`, and `libs/fsproto` defines the
protocol between a client and the service.

Design goals: **never lose consistency** (not even on power loss or a lying
disk), **detect every data corruption**, **be fast** (contiguous writes,
cheap `fsync`) and **not waste space** (metadata overhead ~0.4 %).

> This description is for format v3. What changed from v2 and why is
> summarised in [Format versions](fs-versions.md) and in the "Change from
> v2" boxes of individual chapters; the old description is in the
> [v2 archive](fs-v2.md).
>
> Format v3 is being introduced in phases according to the [FS v3
> design](fs-v3-design.md). Done are phase 1 (format, datasets and clones,
> handles, permissions, protocol v2) and **phase 2 - history**: change
> journal, trash, undo of operations, file versions, automatic snapshots,
> commits with a logical ID, branches and merge, "try changes aside",
> extended attributes and expiry - described in the separate chapter
> [FS history](fs-history.md) - and **phase 3 - data**: compression,
> instant copies, deduplication, automatic holes, metadata 2x with
> self-healing, data 2x, background scrub and preallocation - chapter
> [FS data](fs-data.md). A v2 volume is not mounted
> (`NOT_SUPPORTED`); data is not converted yet, disks from the `xtask` tool
> are simply recreated.

## What the format contains and what the driver uses

Format v3 has room for things the driver cannot do yet. To make it clear
what works:

| Item | In the format | Driver today |
|---|---|---|
| volume UUID, feature flags | yes | **uses**; defined bits `INCOMPAT_TIME_SECS` ([times](#object-times)), `INCOMPAT_PTR_COPY`, `INCOMPAT_DATA` ([data](fs-data.md#on-disk)), `RO_COMPAT_ACL`, `RO_COMPAT_HISTORY` ([history](fs-history.md#on-disk)) |
| device number in `BlockPtr`, `device_index/count` in the superblock | yes | **one device only**: the device must be 0 and the count 1, otherwise `CORRUPTED`; several disks (mirror, RAID) after phase 5 |
| second copy in `BlockPtr` (metadata 2x) | yes | **uses** ([self-healing](fs-data.md#metadata-2x-and-self-healing)) |
| `BlockPtr` flags (compression, encryption, sharing, cloud) | reserved | **unused** for nodes (a node with a flag = `NOT_SUPPORTED`); data compression and sharing are in the extent |
| extent flags (unwritten, shared, copy, compression) | yes | **uses** ([extent](fs-data.md#extent)) |
| object flag `INODE_REMOTE` | reserved | **unused** (phase 6) |
| permissions (`acl`), owner | yes | **uses** ([Permissions](#permissions-and-session-identity)) |
| extended attributes (`XATTR`), count in the object record | yes | **uses** ([attributes](fs-history.md#extended-attributes-and-expiry)) |
| change journal, trash, snapshot metadata, commits, branches, expiry index | yes | **uses** ([FS history](fs-history.md)) |
| access time (`atime`) | yes | **only by setting it** (`set_times`); reading does not change it |
| object generation, creation time | yes | **uses** (`Stat`, log replay) |
| symbolic links, names up to 1024 characters, size limit | yes | **uses** |
| datasets, clones linked to the origin snapshot | yes | **uses** ([Datasets](#datasets-and-clones)) |
| diff of versions, restore as a copy | yes | **uses** ([Diff of versions](#diff-of-versions-and-restore-as-a-copy)) |
| handles, orphans, share modes, range locks | yes | **uses** ([Handles](#open-files-handles)) |
| protocol v2 in fsd (sessions, handles, conditional save) | yes | **uses** ([Protocol v2](#protocol-v2-sessions-handles-conditional-save)) |

## What it offers

| Feature | Status | Where in this document |
|---|---|---|
| Copy-on-write: nothing live is overwritten in place | done | [Commit](#commit-step-by-step) |
| Atomic commit, consistency after power loss at any moment | done | [Power loss](#what-happens-on-power-loss) |
| Merkle tree: a 128-bit checksum in every pointer | done | [Checksums](#checksums-and-the-merkle-tree) |
| Checksum choice XXH3-128 / BLAKE3 | done | [Checksums](#checksums-and-the-merkle-tree) |
| Fallback by 1-3 generations when the newest state is damaged | done | [Generation fallback](#mount-and-generation-fallback) |
| Fast `fsync` via an intent log (one write + flush) | done | [fsync and the intent log](#fsync-and-the-intent-log) |
| Extents, delayed allocation, contiguous writes | done | [Writing data](#writing-data-delayed-allocation-and-extents) |
| Allocation groups with space logs (no bitmap) | done | [Free space](#free-space-management) |
| TRIM/DISCARD for SSDs | done | [TRIM](#trim) |
| Background defragmentation | done | [Fragmentation](#fragmentation-and-defragmentation) |
| Instant directory size (subtree summaries) | done | [Summaries](#instant-directory-sizes) |
| Case-insensitive names (Unicode NFC) | done | [Names](#file-names) |
| Names up to 1024 characters, text direction controls forbidden | done | [Names](#file-names) |
| Symbolic links | done (library; listing only in the protocol) | [Symbolic links](#symbolic-links) |
| Files up to 2^63-1 B, checked arithmetic | done | [Limits](#format-limits) |
| Volume UUID and feature flags (compat/ro/incompat) | done | [Superblock](#superblock) |
| Read-only snapshots and their deletion (deadlists) | done | [Snapshots](#snapshots) |
| Datasets (folders with their own history), writable clones | done | [Datasets](#datasets-and-clones) |
| Consistency check (fsck) and data verification (scrub) | done | [Check](#consistency-check-and-scrub) |
| Change journal with sequence numbers, change watching (`JOURNAL_WATCH`), a view with a position | done | [History: journal](fs-history.md#change-journal) |
| Trash, undo of operations, file versions, automatic snapshots | done | [History](fs-history.md#trash) |
| Commits with a logical ID, branches, three-way merge, "try changes aside" | done | [History: commits](fs-history.md#commits-and-logical-id) |
| Extended attributes, file expiry | done | [History: attributes](fs-history.md#extended-attributes-and-expiry) |
| Per-chunk compression (LZ4, Zstd; off by default) | done | [Data: compression](fs-data.md#compression) |
| Instant copies of files, ranges and folders (shared blocks) | done | [Data: instant copies](fs-data.md#instant-copies-reflink) |
| Deduplication in the background and at write | done | [Data: deduplication](fs-data.md#deduplication) |
| Automatic holes, preallocation with in-place writes | done | [Data: preallocation](fs-data.md#preallocation) |
| Metadata 2x with self-healing, optional data 2x, background scrub | done | [Data: self-healing](fs-data.md#metadata-2x-and-self-healing) |
| Git layer, indexes, search | planned | [Versioning and git](#versioning-and-git) |

## Quick example

In a program in the system (through the fsd service):

```rust
use fsproto::{FsClient, kind};

fn example(fs: &FsClient) -> libos::Result<()> {
    fs.create("/Documents", kind::DIR)?;
    fs.create("/documents/Notes.txt", kind::FILE)?; // names are case-insensitive
    fs.write("/DOCUMENTS/notes.txt", 0, "Hello world\n".as_bytes())?;
    fs.fsync()?; // survives a power loss from now on

    let st = fs.stat("/documents")?;
    // The size of the whole subtree is available at once, without a walk:
    libos::println!("{} B in {} files", st.tree_bytes, st.tree_files);

    fs.snapshot("before-edit")?; // a read-only version
    fs.truncate("/documents/notes.txt", 0)?;
    let old = fs.snapshot_read("before-edit", "/documents/notes.txt", 0, 100)?;
    assert_eq!(old, "Hello world\n".as_bytes());
    Ok(())
}
```

Directly on a device (host tools, tests):

```rust
use fs::{Algo, FileDevice, FormatOptions, Fs, Kind};

let dev = FileDevice::open("data.img".as_ref())?;
let opts = FormatOptions { label: "data".into(), algo: Algo::Xxh3, ag_blocks: None, uuid: None };
let dev = Fs::format(dev, &opts)?;          // a new empty file system
let mut fs = Fs::mount(dev)?;               // mount (+ intent log replay)
fs.create("/welcome.txt", Kind::File)?;
fs.write("/welcome.txt", 0, b"Hello!\n")?;
fs.symlink("/intro", "/welcome.txt")?;     // a symbolic link (the target is text)
assert_eq!(fs.readlink("/intro")?, "/welcome.txt");
fs.commit()?;                               // atomically saves everything
let report = fs.check(true)?;               // fsck + scrub
assert!(report.problems.is_empty());
```

## Terms

| Term | Meaning |
|---|---|
| **block** | 4 KiB (`BLOCK_SIZE`), the unit of allocation and checksumming. On a disk with 512 B sectors a block = 8 sectors. |
| **transaction (txg)** | All changes since the last commit. Numbered `txg = generation of the last commit + 1`. |
| **generation** | The number of a saved state (commit). Increases by 1. |
| **birth** | The generation in which a block was created. Decides when the block may be freed. |
| **node** | A node of the metadata B+tree: 16 KiB = 4 contiguous blocks (`NODE_BLOCKS`). |
| **pointer (`BlockPtr`)** | (device, block number, length, birth, 128-bit checksum of the content, flags). |
| **object (oid)** | A file, a directory or a symbolic link; number `oid`. The root has `oid` 1, system metadata 0. |
| **extent** | A contiguous range of file data: physical start, length (at most 496 blocks ~ 2 MiB), checksum of every block. |
| **allocation group (AG)** | A range of the disk (1 GiB by default) with its own free-space record. |

## On-disk layout

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Disk layout">
<rect class="box" x="10" y="40" width="60" height="60" rx="6"/><text class="t" x="40" y="66" text-anchor="middle">block 0</text><text class="m" x="40" y="84" text-anchor="middle">unused</text>
<rect class="acc" x="76" y="40" width="190" height="60" rx="6"/><text class="t b" x="171" y="64" text-anchor="middle">blocks 1-8: superblock ring</text><text class="m" x="171" y="84" text-anchor="middle">commit g → slot 1 + g mod 8</text>
<rect class="box" x="272" y="40" width="478" height="60" rx="6"/><text class="t b" x="511" y="64" text-anchor="middle">blocks 9 … end: allocated space</text><text class="m" x="511" y="84" text-anchor="middle">tree nodes, data, space logs, AG table, intent log</text>
<rect class="ok" x="272" y="120" width="150" height="34" rx="6"/><text class="t" x="347" y="142" text-anchor="middle">metadata: from the start</text>
<rect class="warn" x="430" y="120" width="320" height="34" rx="6"/><text class="t" x="590" y="142" text-anchor="middle">data: best-fit, from the middle, aligned to 1 MiB</text>
<text class="m" x="10" y="25">All numbers little-endian. Only superblocks have a fixed place; everything else is found through the pointer tree.</text>
</svg>
<figcaption>The disk: only blocks 0-8 are fixed, the allocator hands out the rest.</figcaption>
</figure>

### Superblock

> **Change from v2:** added the volume UUID, feature flags (compat/ro_compat/incompat) and volume devices; pointers are 40 B. **Why:** the format can be extended without a new version and an older driver safely recognises what it cannot handle. ([all changes](fs-versions.md#transition-v2---v3))

The superblock has its **own** XXH3-128 checksum in the last 16 bytes of the
block (it must be verifiable before the chosen algorithm is known). The
valid one is the one with the highest generation whose state can be loaded.

| Offset | Size | Field | Meaning |
|---|---|---|---|
| 0 | 8 | magic | `OSFS\0\0v3` |
| 8 | 4 | version | 3 |
| 12 | 1 | algo | 1 = XXH3-128, 2 = BLAKE3 |
| 16 | 8 | generation | commit number |
| 24 | 8 | block_count | volume size in blocks |
| 32 | 8 | ag_blocks | allocation group size (64 blocks up to 2^20 groups) |
| 40 | 16 | uuid | unique volume ID |
| 56 | 8 | features_compat | features an older driver may use normally |
| 64 | 8 | features_ro_compat | features an older driver may only read |
| 72 | 8 | features_incompat | features without which the volume must not be mounted |
| 80 | 48 | root | `BlockPtr` of the B+tree root |
| 128 | 48 | space | `BlockPtr` of the AG table index |
| 176 | 8 | zil_block | first intent log block of this epoch |
| 184 | 8 | zil_seed | seed of the intent log checksums |
| 192 | 8 | next_oid | next free object number |
| 200 | 8 | snapshot_generation | generation of the last snapshot |
| 208 | 8 | next_snapshot | number of the next snapshot (= dead list of the live version) |
| 216 | 8 | dead_seq | sequence of the next dead list entry |
| 224 | 4 | unicode_version | Unicode table version for names |
| 228 | 2 | device_index | this device in the volume (0 for now) |
| 230 | 2 | device_count | number of devices of the volume (1 for now) |
| 232 | 32 | label | volume name (UTF-8, zero padded) |
| 4080 | 16 | csum | XXH3-128 of bytes 0..4080 |

**Feature flags** work as in ext4: every bit = one format feature
(compression, encryption, ...). A driver that does not know a bit from
`ro_compat` or `incompat` refuses the volume with `NOT_SUPPORTED` - so it
never damages it by not understanding new structures.

| Bit | Group | Name | Meaning |
|---|---|---|---|
| 0 | incompat | `INCOMPAT_REMOTE` | reserved: data only in the cloud (phase 6); the driver still refuses it |
| 1 | incompat | `INCOMPAT_TIME_SECS` | times are seconds i64 + nanoseconds u32 ([times](#object-times)); new volumes always have it |
| 2 | incompat | `INCOMPAT_PTR_COPY` | 48 B pointer with the address of a second copy ([data](fs-data.md#48-b-pointer-and-the-superblock)); **required** - a volume without it (40 B pointer) is rejected with `NOT_SUPPORTED` |
| 3 | incompat | `INCOMPAT_DATA` | phase 3 extents: compression, shared blocks, unwritten, with a copy ([data](fs-data.md#extent)) |
| 0 | ro_compat | `RO_COMPAT_ACL` | the volume has permission descriptors with reference counts ([permissions](#permissions-and-session-identity)); an older driver only reads |

The **UUID** comes from `FormatOptions::uuid` at format time; host tools
(`xtask`) pass a random one. Without it, it is derived from the label, size
and a stack address (there is no randomness source in `no_std`) - then
uniqueness is not guaranteed. A superblock with `ag_blocks < 64`, more than
2^20 groups or zero devices is `CORRUPTED`.

## Checksums and the Merkle tree

Every block except the superblock and intent log blocks is verified by the
**checksum stored in the parent's pointer**, not in the block itself. The
result is a Merkle tree with the superblock at the top:

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Merkle tree">
<defs><marker id="mk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="290" y="10" width="180" height="46" rx="6"/><text class="t b" x="380" y="30" text-anchor="middle">superblock</text><text class="m mono" x="380" y="47" text-anchor="middle">root = (block, birth, csum)</text>
<path class="ln" d="M380 56 L380 84" marker-end="url(#mk1)"/>
<rect class="box" x="280" y="86" width="200" height="46" rx="6"/><text class="t b" x="380" y="106" text-anchor="middle">internal node</text><text class="m mono" x="380" y="123" text-anchor="middle">[(key, BlockPtr)] × up to 72</text>
<path class="ln" d="M330 132 L190 166" marker-end="url(#mk1)"/><path class="ln" d="M430 132 L570 166" marker-end="url(#mk1)"/>
<rect class="box" x="90" y="168" width="200" height="46" rx="6"/><text class="t b" x="190" y="188" text-anchor="middle">leaf</text><text class="m" x="190" y="205" text-anchor="middle">inodes, directory entries</text>
<rect class="box" x="470" y="168" width="200" height="46" rx="6"/><text class="t b" x="570" y="188" text-anchor="middle">leaf</text><text class="m" x="570" y="205" text-anchor="middle">EXTENT: phys, birth, csum[n]</text>
<path class="ln" d="M520 214 L450 248" marker-end="url(#mk1)"/><path class="ln" d="M570 214 L570 248" marker-end="url(#mk1)"/><path class="ln" d="M620 214 L690 248" marker-end="url(#mk1)"/>
<rect class="ok" x="400" y="250" width="100" height="36" rx="6"/><text class="t" x="450" y="273" text-anchor="middle">data 4 KiB</text>
<rect class="ok" x="520" y="250" width="100" height="36" rx="6"/><text class="t" x="570" y="273" text-anchor="middle">data 4 KiB</text>
<rect class="ok" x="640" y="250" width="100" height="36" rx="6"/><text class="t" x="690" y="273" text-anchor="middle">data 4 KiB</text>
<text class="m" x="10" y="250">Every arrow = a pointer with the target's checksum.</text><text class="m" x="10" y="268">Changing any byte changes the checksums</text><text class="m" x="10" y="286">all the way to the root.</text>
</svg>
<figcaption>The Merkle tree: through checksummed pointers the superblock verifies the whole volume.</figcaption>
</figure>

Why in the parent: when a disk acknowledges a write but does not perform it
(a "lost write"), the old block, valid on its own, stays in place. A
checksum inside the block would pass; the checksum in the parent does not.
A write to a wrong address is detected the same way.

### Algorithms

Chosen at format time (`FormatOptions::algo`), valid for the whole volume.

| | XXH3-128 (`Algo::Xxh3`, default) | BLAKE3-128 (`Algo::Blake3`) |
|---|---|---|
| Speed on x86 (one core) | ~30-60 GB/s (AVX2) | ~2-7 GB/s |
| Undetected random error | 1 : 2^128 | 1 : 2^128 |
| Resists a deliberate collision | no | yes |
| Use | ordinary volumes | deduplication, signed system snapshots |

Why not CRC32C (format v1): 32 bits means 1 undetected error in 4 billion;
a 10 TB disk has 2.5 billion blocks. For comparison: btrfs offers
crc32c/xxhash64/sha256/blake2b, ZFS fletcher4 and sha256/blake3, APFS only
checksums metadata.

The hashes currently come from the `xxhash-rust` and `blake3` libraries
(pure Rust, `no_std`). Note: `blake3` 1.8 reports a future-incompatibility
warning with newer Rust - to be solved by an update or our own
implementation.

### Pointer (`BlockPtr`, 48 B)

> **Change from v3 (phase 2):** from 40 B to 48 B: the address of a second
> copy (metadata 2x). **Why:** a damaged node is repaired from the copy
> without data loss (self-healing); the copy lies elsewhere on the disk.
> The driver rejects the old layout. ([FS data](fs-data.md#metadata-2x-and-self-healing))

> **Change from v2:** from 32 B to 40 B: device number (16 bits), length in blocks and flags. **Why:** a node has 4 blocks and the format is ready for several disks and for compression, encryption and shared blocks (visible right on the pointer). **The driver so far** supports only one device (number 0) and no flag - see [What the format contains](#what-the-format-contains-and-what-the-driver-uses). ([all changes](fs-versions.md#transition-v2---v3))

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | address: block number (bits 0-47) \| volume device (bits 48-63) |
| 8 | 8 | birth (generation of creation) |
| 16 | 16 | checksum of the content (all blocks it points to) |
| 32 | 4 | length in blocks (tree node = 4) |
| 36 | 2 | flags: 1 = compressed, 2 = encrypted, 4 = shared blocks, 8 = cloud only (reserved; a node with a flag is refused with `NOT_SUPPORTED`) |
| 38 | 2 | reserved (zeros) |
| 40 | 8 | address of the second copy (same layout as the address; 0 = no copy); the checksum holds for both |

48 bits of block number = up to 1 EiB per device; the device number is
prepared in the format for volumes spanning several disks, **the driver so
far accepts only device 0**. A pointer to a node must have
length exactly 4, device 0 and the whole range inside the volume, otherwise the node
is `CORRUPTED` (checked before it is read and its checksum verified).

## Metadata B+tree

All metadata lives in **one** copy-on-write B+tree. The key is
`(oid u64, kind u8, offset u64)`, ordered in that order - everything about
one object is stored together (one leaf read yields the inode and small
content).

| Kind | Value | Key offset | Item value |
|---|---|---|---|
| `INODE` | 1 | 0 | object record, 128 B (see below) |
| `DIRENT` | 2 | FNV-1a hash of the folded name | list of entries with the same hash: (oid u64, kind u8, name length u16, name) |
| `INLINE` | 3 | 0 | the whole content of a file up to 1984 B (`INLINE_MAX`) |
| `EXTENT` | 4 | first logical block | phys u64, count u32 (1-496), flags u32 (1 = preallocated, reserved; must be 0 for now), birth u64, checksums 16 B × count |
| `SNAPSHOT` | 5 | snapshot number (object 0) | root `BlockPtr`, generation u64, clone count u32, flags u32 (1 = hidden), name |
| `DEADLIST` | 6 | list << 32 \| sequence (object 0) | up to 80 dead ranges (start u64, length u64, birth u64) |
| `SYMLINK` | 7 | 0 | target of a symbolic link (UTF-8, 1-4096 B) |
| `ACL` | 10 | descriptor number (object 0) | permission descriptor with a reference count ([permissions](#on-disk)) |
| `ACL_INDEX` | 11 | descriptor hash (object 0) | descriptor numbers u64 with the same hash |
| `PROPS` | 12 | 0 (object 0) | dataset properties: flags u32 (1 = no execution) |
| `ORPHAN` | 9 | orphan oid (object 0) | empty - an open object without a name; deleted by the last `close` or at mount |
| `DATASET` | 8 | dataset number (object 0, only in the root dataset) | `DatasetRecord` 100 B: root, next oid, next snapshot, generation of the last snapshot, deadlist sequence, clone origin (dataset, snapshot, generation), flags |

### Object record (inode, 128 B)

> **Change from v2:** from 64 B to 128 B: creation and access time, generation, permissions, owner, extended attributes, flags. **Why:** oid + generation is a stable identity for handles; permissions and attributes will not need another format change. ([all changes](fs-versions.md#transition-v2---v3))

| Offset | Field | Meaning |
|---|---|---|
| 0 | kind u8 | 1 = file, 2 = directory, 3 = symbolic link, 5 = `AF_UNIX` socket node (no data) |
| 1 | flags u8 | object flags, reserved: 1 = data only in the cloud (0 for now; sync settings are `sync.*` attributes, not bits) |
| 4 | acl u32 | permission descriptor number (0 = inherit; comes in phase 1d) |
| 8 | size | file: size in bytes (at most 2^63-1); link: target length |
| 16 | mtime | time of the last content change: seconds i64 since 1970 UTC |
| 24 | ctime | time of the metadata change: seconds i64 |
| 32 | btime | creation time: seconds i64 |
| 40 | atime | time of the last access: seconds i64 (0 and 0 ns = not tracked) |
| 48 | parent | oid of the parent directory (every object has exactly one parent) |
| 56 | generation | generation (txg) in which the object was created |
| 64 | tree_bytes | directory: total size of files in the whole subtree (saturating) |
| 72 | tree_files | directory: number of files and links in the subtree |
| 80 | tree_dirs | directory: number of directories in the subtree (excluding itself) |
| 88 | owner | owner (0 = system; comes with user management) |
| 96 | xattrs u32 | number of extended attributes |
| 100 | - | reserved u32 (zero) |
| 104 | version | content version (change counter; the revision for a conditional save) |
| 112 | mtime_ns u32 | nanoseconds of `mtime` (0..10⁹) |
| 116 | ctime_ns u32 | nanoseconds of `ctime` |
| 120 | btime_ns u32 | nanoseconds of `btime` |
| 124 | atime_ns u32 | nanoseconds of `atime` |

#### Object times

> **Time format change (flag `INCOMPAT_TIME_SECS`):** each time used to be
> u64 nanoseconds since 1970 at offsets 16-40; now those hold signed i64
> seconds and the nanoseconds are in the former reserve 112-128. **Why:**
> u64 nanoseconds could not store dates before 1970 (old archives; NTFS
> counts from 1601) and ended in 2262. The new form has a range of ±292
> billion years, the same as POSIX `timespec` and Rust `SystemTime`, and it
> fits into the 128 B record without changing its size.
> ([versions](fs-versions.md#object-times-u64-ns---i64-s--u32-ns))

- **New volumes** have the `INCOMPAT_TIME_SECS` bit. An older driver that
  does not know it refuses them (`NOT_SUPPORTED`), so it never reads seconds
  as nanoseconds.
- **Volumes from an older version** (without the bit) are mounted normally
  by this driver, which keeps reading and writing their times in the old
  u64 ns form (a time before 1970 is clamped to 1970 there). There is no
  conversion to the new form yet (it would mean rewriting every object
  record).
- **Nanoseconds ≥ 10⁹** = a damaged record (`CORRUPTED`); the same in the
  intent log.
- **The time source** is `Fs::set_time(Timestamp)`. The fsd service uses
  real time ([Real time](time.md)); host tools use the host time.
- **Setting times:** `Fs::set_times(path, SetTimes)` and
  `set_times_h(handle, ...)` (a handle with write access) change `mtime`,
  `btime`, `atime` (`None` = keep). `ctime` cannot be set: it is set to the
  current time because it records that the object changed. The change is
  written to the intent log (record 9), so it survives `fsync` and a crash.

```rust
use fs::{SetTimes, Timestamp};

// A copy from an old archive keeping its time (1 Jan 1601 and before 1970).
fs.set_times("/archive/letter.txt", SetTimes {
    mtime: Some(Timestamp { secs: -11_644_473_600, nanos: 0 }),
    btime: Some(Timestamp { secs: -86_400, nanos: 500 }),
    atime: None,
})?;
```

The **generation** together with `oid` identifies an object uniquely: the oid of
a removed object is not reused, but protocol v2 handles are still verified
against the generation. A record with an unknown kind or a size above
2^63-1 is `CORRUPTED`.

### Tree node (16 KiB)

> **Change from v2:** from 4 KiB to 16 KiB and the level is stored and checked. **Why:** a name of up to 4 KiB and an extent with 496 checksums fit into an entry, the tree is shallower and a damaged pointer cannot create a loop. ([all changes](fs-versions.md#transition-v2---v3))

A node takes 4 contiguous blocks (`NODE_BLOCKS`) and is allocated and
written as a whole.

```text
0   magic "BT", level u8 (0 = leaf, at most 16), reserved, entry count u16
8   generation of creation u64
32  entries:
      leaf (32 B):      key 24 B, value offset u16, length u16, reserved
      internal (72 B):  key 24 B, child BlockPtr 48 B
    leaf values are stored from the end of the node downwards
```

Limits: a value is at most 8144 B (`MAX_VALUE`, guarantees that an
overfull leaf can always be split), an internal node has at most 227
children. The level is stored in every node and checked on every descent
(a child must be exactly one level lower) - so a damaged pointer cannot turn
the tree into a loop or slip in a leaf in place of an internal node.

Why 16 KiB: a name of up to 4 KiB and an extent with 496 checksums fit into
one entry, the tree is one level shallower (fewer reads per lookup) and the
header overhead drops. The cost: changing one entry rewrites 16 KiB instead
of 4 KiB, which copy-on-write turns into contiguous writes at the end of
the commit.

- **Leaf split:** when appending at the end (sequential write) the new
  entry goes right alone and the left leaf stays full. Otherwise it splits
  into the most balanced halves. Without this, gradual filling would leave
  leaves half empty (measured: overhead 0.85 % instead of 0.43 %).
- **Removal:** empty nodes are removed, a root with one child is replaced by
  that child, adjacent underfull leaves (left <= 1/4 of a node, both <= 1/2)
  are merged.

## File names

> **Change from v2:** from 255 B to 1024 characters (4 KiB UTF-8), text direction controls forbidden too. **Why:** long names from other systems and protection against a spoofed extension. ([all changes](fs-versions.md#transition-v2---v3))

- Stored as the user entered them (in Unicode NFC) - letter case is kept.
- Compared **case-insensitively** across the whole system: the key =
  NFC(lowercase(NFC(name))). `Foto.JPG`, `foto.jpg` and `fóto` written
  decomposed (`o` + acute) are the same name. The conversion does not depend
  on the language (no Turkish rules for `i`).
- A valid name: **1-1024 Unicode characters and at most 4096 bytes of
  UTF-8** (`NAME_MAX_CHARS`, `NAME_MAX`), measured on the NFC form that is
  actually stored. Forbidden are `.` and `..`, the `/` character, control
  characters (including NUL) and **text direction controls** U+202A-202E,
  U+2066-2069 and the direction marks U+200E, U+200F, U+061C
  (`is_bidi_control`). Otherwise `INVALID_ARGS`.
- Renaming to a different case only (`/fotky` -> `/FOTKY`) is allowed.
- The Unicode table version is in the superblock (`unicode_version`).

Why text direction controls are forbidden: U+202E reverses the display
direction of the rest of the name, so a file `invoice[U+202E]fdp.exe` shows
in a file manager as `invoiceexe.pdf` - the classic trick for slipping in an
executable. Other characters that some systems cannot handle (`:` `\` `*`
`?` on Windows, more than 255 bytes on Linux) are allowed by the FS; the
Windows (WinFsp) and Linux (FUSE) drivers translate them for display.

```rust
use fs::{validate_name, is_bidi_control, NAME_MAX_CHARS};

assert!(validate_name("Holiday 2026 - Italy.jpg").is_ok());
assert!(validate_name(&"x".repeat(NAME_MAX_CHARS)).is_ok());
assert!(validate_name("invoice\u{202E}fdp.exe").is_err());
assert!(is_bidi_control('\u{2067}'));
```

Names with the same hash share one `DIRENT` entry. If long names did not fit
into it (over 8144 B - in practice only with a deliberately crafted FNV hash
collision), `create` returns `NO_RESOURCES`.

## Symbolic links

> **Change from v2:** a new object kind. **Why:** git repositories and Linux sources need them. ([all changes](fs-versions.md#transition-v2---v3))

A symbolic link is an object of kind 3 whose content is **the target as
text** (1-4096 bytes of UTF-8, no NUL; `SYMLINK_MAX`). The target does not
have to exist and may be relative or absolute. It is stored in a `SYMLINK`
entry; the object size = the target length.

```rust
fs.symlink("/project/README", "docs/README.md")?;
assert_eq!(fs.readlink("/project/README")?, "docs/README.md");
assert_eq!(fs.stat("/project/README")?.kind, Kind::Symlink);
fs.rename("/project/README", "/project/READ")?; // move/remove like a file
fs.remove("/project/READ")?;
```

- The library does **not follow** links while walking a path: a link in the
  middle of a path gives `NOT_A_DIRECTORY`, `read`/`write` of a link
  `WRONG_TYPE`. Following (with a depth limit against loops) will be done by
  the layer above (VFS/fsd).
- In directory summaries a link counts as a file of size 0.
- `symlink` is written to the intent log (record 7), so it survives `fsync`
  even without a commit.
- Protocol fsproto v1 only shows a link (`kind::SYMLINK` in `STAT` and
  `LIST`); creating one through the service comes with protocol v2.

## Format limits

> **Change from v2:** a new chapter: a file is at most 2^63-1 B, checked arithmetic, saturating summaries, an extent of 496 blocks. **Why:** sizes could overflow before. ([all changes](fs-versions.md#transition-v2---v3))

| What | Limit | Constant |
|---|---|---|
| file size | 2^63-1 B (8 EiB) | `MAX_FILE_SIZE` |
| name | 1024 characters / 4096 B UTF-8 | `NAME_MAX_CHARS`, `NAME_MAX` |
| symbolic link target | 4096 B | `SYMLINK_MAX` |
| file stored directly in the tree | 1984 B | `INLINE_MAX` |
| extent | 496 blocks (~2 MiB) | `MAX_EXTENT` |
| tree entry value | 8144 B | `node::MAX_VALUE` |
| tree depth | 17 levels | `node::MAX_LEVEL` |
| volume device | 2^48 blocks (1 EiB) | 48 address bits in `BlockPtr` |

A write or `truncate` beyond `MAX_FILE_SIZE` returns `OUT_OF_RANGE`. Thanks
to the limit, a size and the difference of two sizes always fit into
`i64`. Directory summaries **saturate** at 2^64-1: sparse files may together
have a logical size larger than can be summed - a saturated summary is only
informative (the data is fine).

## Instant directory sizes

Every directory carries a **summary of its whole subtree**: total size of
files, number of files and number of directories. Every size change,
creation, removal and move updates it at the parent and all ancestors (cost
= depth of the path). `stat` of a directory therefore returns the size of a
folder with thousands of files in microseconds (measured 13.5 µs for 20,000
files), without a walk.

```rust
let st = fs.stat("/projects")?;
println!("{} B, {} files, {} subdirectories", st.tree_bytes, st.tree_files, st.tree_dirs);
```

## Writing data: delayed allocation and extents

> **Change from v2:** the result of `write` means "accepted", not "durable"; an overwrite writes new blocks first and then relinks the extents; blocks are assembled in whole pieces. **Why:** a write error must not lose data or leave a hole, and the client must know what was accepted; writing is 2x faster. ([all changes](fs-versions.md#transition-v2---v3))

1. `write` puts data into a **buffer in memory** (at most 16 MiB, then it
   is written earlier). A partially overwritten block is read first.

   **The result of `write` = accepted, not durable.** `Ok` = the data is
   fully in the buffer (reads see it, the next commit or `fsync` stores it);
   an error = nothing from this call was accepted. Durability is confirmed
   only by `fsync`/`commit`. When writing the buffer to disk fails, the data
   stays in the buffer and the error is reported at `fsync`/`commit`; only
   when the buffer reaches 32 MiB (`DIRTY_HARD_LIMIT`) and cannot be
   emptied is the next `write` refused.
2. Only when written to disk (commit, `fsync`, full buffer) do consecutive
   blocks get **one contiguous range** - aiming right after the file's
   previous extent (the file grows contiguously), otherwise the smallest
   large-enough free range (best-fit). Large ranges (≥ 1 MiB) are aligned
   to 1 MiB.
3. The data is written with one request per range; the checksum of every
   block goes into the `EXTENT` item.

A file written in pieces (e.g. 64 KiB at a time) thus ends up in one
contiguous range. A file that disappears again before the commit is never
written to disk at all.

**Overwriting part of an extent** splits it; the overwritten physical
blocks are freed according to the rules below:

<figure class="diagram">
<svg viewBox="0 0 760 150" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Extent split">
<text class="m" x="10" y="20">before: extent of logical blocks 0-9 → physical 500-509</text>
<rect class="box" x="10" y="30" width="740" height="34" rx="4"/><text class="t mono" x="380" y="52" text-anchor="middle">EXTENT@0: phys 500, 10 blocks</text>
<text class="m" x="10" y="92">after writing blocks 3-5: three extents, blocks 503-505 are freed</text>
<rect class="box" x="10" y="102" width="220" height="34" rx="4"/><text class="t mono" x="120" y="124" text-anchor="middle">EXTENT@0: phys 500, 3</text>
<rect class="acc" x="236" y="102" width="220" height="34" rx="4"/><text class="t mono" x="346" y="124" text-anchor="middle">EXTENT@3: phys 812, 3 (new)</text>
<rect class="box" x="462" y="102" width="288" height="34" rx="4"/><text class="t mono" x="606" y="124" text-anchor="middle">EXTENT@6: phys 506, 4</text>
</svg>
<figcaption>Block-level copy-on-write: new data goes elsewhere, the rest of the extent stays.</figcaption>
</figure>

**Small files** (up to 1984 B) live directly in the tree (`INLINE`), with
no data block. When they grow over the limit they are converted to
extents and back. Holes (unwritten parts) read as zeros and take no space.

### Block release rules

A block with generation of creation `birth`, when overwritten or removed:

| Condition | What happens | Why |
|---|---|---|
| `birth == txg`, the intent log does not reference it and the extent is not shared | free **immediately** | no saved state points to it |
| `birth > snapshot_generation` | free after a **delay** (`REUSE_DELAY` = 4 commits) | previous saved states may point to it (generation fallback) |
| `birth ≤ snapshot_generation` | into the **deadlist** | a snapshot holds it; freed when the snapshot is deleted |

The second copy is always freed too (for nodes and data with a copy). A
data range in the **sharing table** (instant copies, deduplication) is only
decremented when finally freed and is freed with its last reference - see
[Sharing table](fs-data.md#sharing-table).

## Commit step by step

`commit` saves all changes atomically:

1. Pending file data is written (delayed allocation, see above).
2. Dead blocks from memory are written into the live version's deadlist.
3. Pending tree nodes are written **from the leaves to the root** (a parent
   needs its children's checksums) into one contiguous range - the
   commit's metadata reaches the disk as one sequential write.
4. Blocks of the previous intent log epoch are freed (delayed) and the
   first block of the new epoch is allocated.
5. Space log and AG table changes are written.
6. **flush** - everything above is durable.
7. The new superblock goes to slot `1 + generation mod 8`.
8. **flush** - the commit is complete.
9. Blocks whose delay expired become free and go to the TRIM queue (if
   enabled).

If a commit fails midway (I/O error), the state in memory no longer matches
the disk: the instance is marked as damaged and further changes return
`IO_ERROR`. The volume has to be mounted again - the last completed commit
is valid on disk.

## What happens on power loss

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Power loss during commit">
<defs><marker id="mk2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<path class="ln-acc" d="M20 60 L740 60" marker-end="url(#mk2)"/>
<rect class="box" x="20" y="35" width="200" height="50" rx="6"/><text class="t" x="120" y="58" text-anchor="middle">new data, nodes, logs</text><text class="m" x="120" y="75" text-anchor="middle">into free blocks</text>
<rect class="warn" x="235" y="35" width="80" height="50" rx="6"/><text class="t" x="275" y="65" text-anchor="middle">flush</text>
<rect class="acc" x="330" y="35" width="170" height="50" rx="6"/><text class="t" x="415" y="58" text-anchor="middle">superblock g+1</text><text class="m" x="415" y="75" text-anchor="middle">into another slot</text>
<rect class="warn" x="515" y="35" width="80" height="50" rx="6"/><text class="t" x="555" y="65" text-anchor="middle">flush</text>
<rect class="ok" x="610" y="35" width="130" height="50" rx="6"/><text class="t" x="675" y="65" text-anchor="middle">done</text>
<rect class="ok" x="20" y="110" width="480" height="40" rx="6"/><text class="t" x="260" y="135" text-anchor="middle">power loss here → state g after mount (new state lost, nothing broken)</text>
<rect class="ok" x="510" y="110" width="230" height="40" rx="6"/><text class="t" x="625" y="135" text-anchor="middle">power loss here → state g+1</text>
<text class="m" x="20" y="180">A torn superblock write is detected by its checksum → the previous slot is used.</text>
<text class="m" x="20" y="198">Changes confirmed by fsync are restored from the intent log in both cases.</text>
</svg>
<figcaption>No moment of power loss leads to a damaged volume.</figcaption>
</figure>

| Moment of power loss | Result after mount |
|---|---|
| any time before the superblock write completes | state of the last commit - nobody overwrote its blocks |
| during the superblock write | the torn superblock fails its checksum, the previous one is valid |
| after the second flush | the new state |
| during `fsync` | state before or after the `fsync` (depending on whether the whole log batch made it) |
| at any moment | changes confirmed by a completed `fsync` are replayed from the intent log |

These guarantees are verified by a test that simulates power loss after the
N-th write and lets a random subset of unwritten blocks survive in random
order (`tests/crash.rs`, tens of thousands of combinations).

Assumption: after an acknowledged `flush` the device really has the data.
Disks that lie about flush are covered by the
[generation fallback](#mount-and-generation-fallback).

## Mount and generation fallback

> **Change from v2:** first selecting the generation by header, then the compatibility decision; uncertain recovery (unreadable slot) read-only; fallback only on damage; devices without flush read-only. **Why:** before, an older driver or a transient error could lead to writing on top of an older generation and losing newer data. ([all changes](fs-versions.md#transition-v2---v3))

Blocks freed by commit `g` are reused only in transaction
`g + REUSE_DELAY` (4). The last 4 saved states therefore stay untouched on
disk. The ring has 8 superblock slots.

Mounting (`Store::open`) has three separate steps:

1. **Selecting the generation.** Among all slots with a valid *header* (the
   format family signature `OSFS\0\0` and its own checksum) the highest
   generation is taken. An unreadable slot (I/O error) or a slot with a bad
   checksum is just an invalid slot; the others are still tried.
2. **Can this driver open it?**

| Situation in the newest generation | Result |
|---|---|
| a different format version (also a newer `v4`) | `NOT_SUPPORTED` |
| an unknown `incompat` bit | `NOT_SUPPORTED` |
| an unknown `ro_compat` bit | **read-only** mount, the intent log is replayed only in memory, nothing is written to disk |
| everything known | normal mount |

   An older generation is never mounted silently: otherwise an older driver
   would get write access to an old state and the next commit would throw
   the newer data away.
3. **Recovery after damage.** When the state of the selected generation
   cannot be loaded because of **damage** (`CORRUPTED`: a bad checksum of the
   root or space metadata, nonsensical values in the superblock), older ones
   are tried - at most 3 generations back (older ones may already be
   overwritten). `info().fallback` tells how many generations the system
   went back; the fsd service reports it as a warning. Any other error (I/O,
   out of memory) is returned immediately - a transient disk error must not
   lead to falling back to an older state.

**Uncertain recovery = read-only.** Commits are sequential and generation
`g` lives in slot `1 + g mod 8`, so a generation newer than the newest one
read (`newest`) could only be **in the slot for `newest + 1`** (and
`newest + 2` only after it). So the slots after the newest are walked: a
slot with an older generation or an empty one = no newer state exists; a
slot with a bad checksum may have been a completed commit whose superblock
was damaged later, so the walk continues. When a slot on this path is
unreadable (an I/O error), the newest state cannot be determined safely: the volume is **read-only** with
`info().newest_uncertain` (fsd prints a warning) - writing on top of an
older generation would overwrite the newer one with the next commit. Another
unreadable slot (a bad sector holding an old generation) does not put the
newest state in doubt: the volume is writable, only
`info().unreadable_superblocks` > 0 and fsd prints a warning (otherwise a
bad sector would make the volume read-only forever).

**Mount options** (`Fs::mount_with`, `MountOptions`):

| Option | Meaning |
|---|---|
| `read_only` | force read-only (checking or rescuing data without writes) |
| `discard_unreadable_log` | when the intent log cannot be read completely, **knowingly discard** the unreadable rest and mount writable (`info().log_discarded`); changes after the last readable batch (even confirmed by `fsync`) are lost - only when the read error does not go away |

After a fallback, the slots with a newer (damaged) generation are **zeroed
before the first commit** - otherwise they would win at the next mount and
point to blocks that were given to someone else in the meantime. After a
fallback the intent log is not replayed (it belongs to the newer state) and
a commit with a new log epoch follows immediately.

### Devices without a durability guarantee

`BlockDevice::durability()` says whether the device really stores data:

| Value | Meaning | Mount |
|---|---|---|
| `Flush` | `flush` stores everything from the cache on the medium | writable |
| `WriteThrough` | the device has no write cache | writable |
| `None` | no or unknown guarantee | **read-only** (`info().read_only`, changes return `READ_ONLY`) |

In fsd it comes from the disk driver through the block protocol operation
`FEATURES` (bits `FLUSH`, `WRITE_THROUGH`, `DISCARD`, `READ_ONLY`).
virtio-blk without the `VIRTIO_BLK_F_FLUSH` feature reports no guarantee and
`FLUSH` returns `NOT_SUPPORTED` (it used to return success without doing
anything - `fsync` would then lie). An older driver without `FEATURES` = no
guarantee.

⚠️ **Limitation:** a disk that lies about flush (confirms it but keeps the
data in its cache) cannot be detected by the FS. Only the generation
fallback and checksums help then - the last confirmed changes may be lost.

## fsync and the intent log

> **Change from v3 (phase 1):** new records 14 `Actor` (who performed the
> following operations), 15 `Trash`, 16 `Restore`, 17 `Purge`, 18
> `SetXattr`; merged `CONTENT` journal records get their number at the end
> of the batch. **Why:** the change journal must come out with the same
> numbers and users after log replay, and the trash and attributes must
> survive a crash. ([FS history](fs-history.md#on-disk))

> **Change from v2:** pending changes are cleared only after flush, log blocks are allocated in advance, a log read error is not the end of the chain, a new record 7 (symlink) and the generation on create. **Why:** a confirmed `fsync` must not be lost and nothing may be confirmed falsely. ([all changes](fs-versions.md#transition-v2---v3))

A commit rewrites many blocks and does two flushes - expensive for every
editor save or database transaction. The intent log (like the ZIL in ZFS
or the log tree in btrfs) makes changes durable with one write and one
flush:

1. Since the last commit, a list of directory operations (create, remove,
   rename, size change) and a set of changed file blocks are kept in memory.
2. `fsync` writes the changed data into new blocks (delayed allocation) and
   encodes the operations and the data mapping into a **batch**.
3. The batch is written into a chain of log blocks, then **one flush**.

<figure class="diagram">
<svg viewBox="0 0 760 140" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Intent log chain">
<defs><marker id="mk3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="40" width="120" height="50" rx="6"/><text class="t" x="70" y="62" text-anchor="middle">superblock</text><text class="m" x="70" y="80" text-anchor="middle">zil_block, seed</text>
<path class="ln" d="M130 65 L170 65" marker-end="url(#mk3)"/>
<rect class="box" x="172" y="40" width="160" height="50" rx="6"/><text class="t mono" x="252" y="62" text-anchor="middle">sequence 0</text><text class="m" x="252" y="80" text-anchor="middle">next = 731 (in advance)</text>
<path class="ln" d="M332 65 L372 65" marker-end="url(#mk3)"/>
<rect class="box" x="374" y="40" width="160" height="50" rx="6"/><text class="t mono" x="454" y="62" text-anchor="middle">sequence 1</text><text class="m" x="454" y="80" text-anchor="middle">next = 902</text>
<path class="ln" d="M534 65 L574 65" marker-end="url(#mk3)"/>
<rect class="err" x="576" y="40" width="170" height="50" rx="6"/><text class="t" x="661" y="62" text-anchor="middle">block 902</text><text class="m" x="661" y="80" text-anchor="middle">bad checksum = end</text>
<text class="m" x="10" y="120">Each block has an XXH3-128 checksum with a seed (epoch + sequence): old content never passes.</text>
</svg>
<figcaption>The log chain: the next block is allocated in advance, its number is in the previous one.</figcaption>
</figure>

Log block format:

```text
0   magic "ZL", 2 flag (1 = last block of the batch)
8   sequence u64, 16 next block u64, 24 data length u32
32  XXH3-128 checksum (checksum field zeroed) with seed zil_seed + sequence
64  batch data
```

Batch records (first byte = kind): 1 create (parent, oid, kind, time,
name), 2 remove (parent, time, name), 3 rename (source, target, time, name,
new name), 4 size change (oid, size, time), 5 small file content (oid,
size, mtime, data), 6 data mapping (oid, size, mtime, list of extents with
checksums), 7 symbolic link, 8 dataset marker, 9 set times (oid, mask u8:
bit 0 mtime, 1 btime, 2 atime, change time, the present times), 10/11
create/symlink with an owner (owner u64, then record 1/7), 12 set
permission descriptor, 13 change owner. A time in a
record takes 12 B (seconds i64 + nanoseconds u32); on a volume without
`INCOMPAT_TIME_SECS` 8 B (u64 ns).

**At mount:** the log is read from `zil_block` while magic, sequence and
checksum match. Complete batches are replayed on top of the last commit's
state - but only those whose data is fully on disk (data block checksums
match; otherwise the power loss came before the flush and the batch is
discarded). A commit follows immediately and a new epoch starts.
`info().replayed` = number of replayed operations.

**A log read error is not the end of the chain.** An already confirmed
`fsync` may follow an unreadable block. On an I/O error (reading a log
block or a batch's data) only what can be read is replayed, the volume is
**read-only** (`info().log_unreadable`) and the log stays untouched on
disk for another attempt. The end of the chain is only a block with a bad
magic, sequence or checksum. When an unreadable block persists (a bad sector), the log can be
discarded knowingly with `discard_unreadable_log` (see
[Mount](#mount-and-generation-fallback)).

Data blocks referenced by a log batch are not reused until the commit, even
when they were created in the same transaction (otherwise a later `fsync`
would overwrite them). When the log grows over 2048 blocks, `fsync` does a
commit instead.

**Atomicity of `fsync`:** the list of pending changes is cleared only
after a successful flush.

- An error **before** the first log block is written (writing data, no space
  for log blocks - they are allocated in advance) breaks nothing: the data
  stays in the buffer and the next `fsync` tries again.
- An error **while** writing the log or flushing = an uncertain state (part
  of the batch may be on disk). The volume enters the error state: all
  further changes and `fsync` return `IO_ERROR`, so nothing later confirms
  as durable something that is not. Recovery = mount again (only complete
  batches are replayed).

**Writing data into new blocks** (`write_run`) first allocates and writes
the new blocks and only then relinks the extents. When allocation or the
write fails, the old data stays mapped (no hole after an overwrite), the new
blocks are returned immediately and the unwritten data goes back to the
buffer.

## Free space management

### Allocation groups and space logs

The disk is divided into allocation groups (262,144 blocks = 1 GiB by
default; larger on huge disks so that there are at most ~10,800 groups).
For each group:

- **in memory:** free ranges indexed both by position and by size (like
  XFS) - best-fit search in O(log n), memory grows only with fragmentation,
- **on disk:** a **space log** - an appended record of allocations and
  frees (like ZFS space maps). A commit writes only new records; the cost
  is proportional to the changes, not to the disk size.

<figure class="diagram">
<svg viewBox="0 0 760 190" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Space log">
<defs><marker id="mk4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="120" height="44" rx="6"/><text class="t" x="70" y="47" text-anchor="middle">superblock.space</text>
<path class="ln" d="M130 42 L168 42" marker-end="url(#mk4)"/>
<rect class="box" x="170" y="20" width="150" height="44" rx="6"/><text class="t" x="245" y="40" text-anchor="middle">index</text><text class="m" x="245" y="56" text-anchor="middle">≤ 127 table blocks</text>
<path class="ln" d="M320 42 L358 42" marker-end="url(#mk4)"/>
<rect class="box" x="360" y="20" width="170" height="44" rx="6"/><text class="t" x="445" y="40" text-anchor="middle">AG table</text><text class="m" x="445" y="56" text-anchor="middle">≤ 85 groups per block</text>
<path class="ln" d="M445 64 L445 98" marker-end="url(#mk4)"/>
<rect class="ok" x="360" y="100" width="170" height="44" rx="6"/><text class="t" x="445" y="120" text-anchor="middle">AG log head</text><text class="m" x="445" y="136" text-anchor="middle">newest records</text>
<path class="ln" d="M530 122 L568 122" marker-end="url(#mk4)"/>
<rect class="box" x="570" y="100" width="170" height="44" rx="6"/><text class="t" x="655" y="120" text-anchor="middle">older log block</text><text class="m" x="655" y="136" text-anchor="middle">→ … → oldest</text>
<text class="m" x="10" y="170">A 24 B record: start u64, length u32, alloc/free u8, txg u64; 168 records per block.</text>
</svg>
<figcaption>Free space: index → group table → each group's space log chain.</figcaption>
</figure>

- **Commit:** a non-full last log block is replaced with a new one
  (copy-on-write), a full one stays and a new one is chained after it.
  Allocating blocks for the logs themselves changes the free space, which
  has to be written again - the preparation therefore repeats allocations
  until the state settles, and only then writes everything.
- **Mount:** the logs are replayed from the oldest record. Frees from the
  last `REUSE_DELAY` transactions go to the delayed ones.
- **Condense:** when a log has more than twice the blocks of its condensed
  form (+8), it is rewritten as a list of used ranges and delayed frees.

### Placement (against fragmentation)

| What | Strategy |
|---|---|
| tree nodes, logs | first-fit from the start of the disk - metadata together, a commit as one contiguous write |
| data | exactly after the file's previous extent; otherwise the smallest large-enough range (best-fit) from the middle of the disk; otherwise the largest available and the rest separately |
| large data (≥ 1 MiB) | start aligned to 1 MiB when the range allows |

### Reserve and running out of space

> **Change from v2:** a dirty node counts as 4 blocks, only space for whole nodes counts, space-freeing operations may use half of the reserve. **Why:** a commit looped or failed on a fragmented disk and a snapshot could not be deleted on a full disk. ([all changes](fs-versions.md#transition-v2---v3))

Part of the space (1/128 of the volume, 24 blocks up to 256 MiB) is
reserved for commits (metadata, logs). An ordinary write that would consume
it returns `NO_SPACE` **immediately** - not at commit time. If blocks are
waiting for a delayed free, the system first tries commits (up to 5) to
release them.

The check counts only space **where a whole tree node fits** (runs of at
least 4 contiguous blocks, `Space::node_room`), and every dirty node as 4
blocks. If data written during a commit took the last such runs, the commit
would have nowhere to store the tree and the volume would go read-only. The
cost: on a heavily fragmented disk `NO_SPACE` comes a few blocks before the
disk is really full. Node allocation (`alloc_min`) skips runs shorter than 4
blocks.

**Space can be freed even on a full volume.** Operations that free space
(`remove`, `delete_snapshot`, truncating a file) may use half of the commit
reserve (`has_room_freeing`); the other half stays for the commit.
Otherwise not even a snapshot holding most of the space could be deleted
on a full volume. Freed blocks become usable after the grace period
(`REUSE_DELAY` commits).

## TRIM

An SSD needs to know which blocks are free (otherwise performance drops and
wear grows). After a commit, when a block's delay expires, it goes into the
TRIM queue; adjacent ranges are merged and sent to the device
(`BlockDevice::discard`, in the system the `DISCARD` operation of the block
protocol → virtio-blk `VIRTIO_BLK_T_DISCARD`). A TRIM error is not fatal (it
is only a hint).

- `set_discard(true)` - continuous TRIM (fsd enables it).
- `trim_all()` - TRIM of all free blocks at once (like `fstrim`).

## Fragmentation and defragmentation

`fragments(path)` returns the number of contiguous physical ranges of a
file (1 = not fragmented). A file is worth defragmenting when it has more
ranges and the average range is shorter than 4 MiB.

Defragmentation reads the file into the buffer as if it were written again
and delayed allocation finds it a contiguous range. Thanks to
copy-on-write it is safe - the old version is valid until the commit.
**Blocks shared with snapshots are skipped** (rewriting would duplicate
them). Large files are processed in windows (8 MiB); the next window
continues physically right after the previous one.

- `defrag_file(path)` - one file, returns the number of rewritten blocks.
- `defrag_step(budget)` - a background step: walks further files until it
  rewrites `budget` blocks; `done` = the pass is complete. fsd calls it
  when idle after a commit, but only when changes arrived since the last
  pass.

## Datasets and clones

> **Change from v2:** new. v2 had one tree for the whole volume. **Why:**
> the unit of history (snapshots, clones, later commits and vaults) is a
> folder - a project, a home, the system - not the whole disk.
> ([all changes](fs-versions.md#transition-v2---v3))

A **dataset** is a folder with its own tree: its own object numbers, its own
snapshots and deadlists. For the user it is an ordinary folder; in the
directory it is an entry of kind `KIND_DATASET` (`Kind::Dataset`) and a path
simply crosses into it. The volume root is the **root dataset** (number 1,
state in the superblock); the others have a `DatasetRecord` in the root
dataset's tree.

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Datasets and clones">
<defs><marker id="dk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="200" height="60" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">root dataset (1)</text><text class="m" x="110" y="64" text-anchor="middle">dataset records</text>
<rect class="box" x="280" y="20" width="200" height="60" rx="6"/><text class="t b" x="380" y="45" text-anchor="middle">/project (2)</text><text class="m" x="380" y="64" text-anchor="middle">own tree</text>
<rect class="ok" x="280" y="120" width="200" height="50" rx="6"/><text class="t b" x="380" y="141" text-anchor="middle">snapshot S</text><text class="m" x="380" y="158" text-anchor="middle">clones = 1</text>
<rect class="warn" x="550" y="120" width="200" height="60" rx="6"/><text class="t b" x="650" y="145" text-anchor="middle">/trial (3, clone)</text><text class="m" x="650" y="164" text-anchor="middle">origin = (2, S, gen S)</text>
<path class="ln" d="M210 50 L278 50" marker-end="url(#dk1)"/>
<path class="ln" d="M380 80 L380 118" marker-end="url(#dk1)"/>
<path class="ln" d="M548 150 L482 145" marker-end="url(#dk1)"/>
<text class="m" x="10" y="215">A clone shares blocks reachable from S's root (born up to S's generation); overwriting puts them on the clone's</text>
<text class="m" x="10" y="233">deadlist, only S may free them. Deleting S while it has a clone only hides it; it is really deleted with the last clone.</text>
</svg>
<figcaption>A dataset, its snapshot and a clone linked to its origin snapshot.</figcaption>
</figure>

```rust
let id = fs.create_dataset("/project")?;          // a new folder with its own history
fs.write("/project/cfg.toml", 0, b"a = 1")?;      // ordinary operations unchanged
fs.snapshot_of("/project", "before-refactor")?;   // a snapshot of this dataset only
fs.clone_snapshot("/project", "before-refactor", "/trial")?; // a writable clone
fs.write("/trial/cfg.toml", 0, b"a = 2")?;        // the original does not change
fs.delete_dataset("/trial")?;                     // frees only the clone's blocks
```

| Operation | Behaviour |
|---|---|
| `create_dataset(path)` | a new empty dataset; commits at once |
| `snapshot_of(dataset, name)`, `snapshots_of`, `delete_snapshot_of`, `snapshot_read_of/stat_of/list_of` | as for the root, for the dataset rooted at `dataset` (`"/"` = root; the old functions without `_of` = root) |
| `clone_snapshot(dataset, snapshot, path)` | a writable clone (instant, shares blocks); commits at once |
| `delete_dataset(path)` | deletes the dataset with its snapshots and frees the blocks only it held; `NOT_EMPTY` with a nested dataset, `BAD_STATE` when a clone of its snapshot exists |
| `datasets()` | a list (number, path in the live version, clone origin) |
| `stat` | the `dataset` field = number of the dataset the object lives in |

**Rules:**

- **Moving between datasets** (`rename`) = `NOT_SUPPORTED` - they are
  different trees, it would be a data copy. Renaming or moving a dataset
  within its parent works.
- `remove` of a dataset = `WRONG_TYPE` (it is deleted with
  `delete_dataset`); `create(..., Kind::Dataset)` = `INVALID_ARGS`.
- A **snapshot view** does not cross into another dataset
  (`NOT_SUPPORTED`): a root snapshot does not know what the dataset freed
  in the meantime.
- Directory size summaries stop at a dataset boundary (a dataset does not
  count into its parent).
- Dataset operations (create, clone, delete) commit at once, so they are
  not in the intent log. Ordinary changes in several datasets are written
  by `fsync` in one batch; the records of each dataset start with a dataset
  marker (log record 8).

**Who keeps blocks alive (clone).** A clone comes from snapshot S: root =
S's root, ownership boundary (`snapshot_generation`) = S's generation, the
`origin` record = (dataset, S, S's generation). The clone's tree derives
from S's root, so a block born up to S's generation is in the clone exactly
a block reachable from S - the clone only shares it. Overwriting it in the
clone puts it on the clone's deadlist, but only S may free it. A block born
after the clone was created is owned by the clone. Snapshot S keeps a clone
count; deleting it while it has clones only **hides** it (`SNAP_HIDDEN`, it
keeps holding blocks) and the real deletion happens with the last clone.
Deleting the clone's own snapshots uses S's generation as the lower bound.

**Deleting a dataset** walks its stored tree and frees nodes and data born
after the boundary (for a clone S's generation); subtrees born earlier are
skipped (shared with S). All in one commit - power loss during deletion
leaves the state before or after (test `power_loss_during_dataset_deletion`).

**Implementation:** operation code always works with the active dataset
(`Store::root`, `next_oid`, the data buffer, pending log changes); the other
loaded datasets are kept in memory separately and `activate` swaps them.
`enter` walks a path and leaves the target's dataset active. A commit
writes the tree and record of every changed dataset and finally the root
dataset - all in one transaction.

⚠️ **Limitations of the first version:** the strict check for a data block used twice in fsck applies
only to the root dataset (clones share data legitimately); background
defragmentation walks only the active dataset; the fsd service and protocol
v1 only show datasets (kind 4 in listings), creating them will come with
protocol v2 or the `osfs` tool.

### Diff of versions and restore as a copy

```rust
// What changed in the project since snapshot v1 (against the live version):
for c in fs.diff("/project", "v1", None)? {
    println!("{:?} {} {:?}", c.kind, c.path, c.old_path); // Modified /cfg.toml None
}
// What a clone changed against the snapshot it came from ("try on the side"):
let changes = fs.diff_clone("/project-trial")?;
// One file from a snapshot next to the current one:
fs.restore_copy("/project", "v1", "/cfg.toml", "/project/cfg (v1).toml")?;
```

| Function | What it does |
|---|---|
| `diff(dataset, from, to)` | the difference between snapshot `from` and snapshot `to` (or the live version, `to = None`); sorted by path |
| `diff_clone(clone)` | the difference of a clone against its origin snapshot |
| `restore_copy(dataset, snapshot, path, dest)` | restores a file, link or whole folder from a snapshot as a copy `dest` (must not exist); nested datasets are skipped |
| `snapshot_readlink_of` | a link target in a snapshot |

Kinds of change (`ChangeKind`): `Added`, `Removed`, `Modified` (file content
or link target; not reported for a directory - its changes are changes of
its entries), `Moved` (a different path, the same object - `old_path`),
`MovedModified`. Because the oid is stable within a dataset, **a move is
detected exactly**, not heuristically by content as in git.

**How it works:** for each version a summary is loaded: object -> (kind,
size, content fingerprint) and object -> (parent, name). The content
fingerprint is an XXH3 checksum over the data entries (extents with block
checksums, a small file, a link target), so no data is read. A diff against
the live version first writes the data buffer (without a commit) so that it
also sees unsaved changes.

⚠️ **Limitations of the first version:** the diff walks both trees fully
(cost by dataset size, not by change) - planned: skipping subtrees with the
same pointer. Restore as a copy really copies the data; an instant copy via
shared blocks comes with instant file copies (phase 3).

## Open files (handles)

> **Change from v2:** new. v2 worked only with paths. **Why:** an open file
> must survive a rename, deletion and an atomic replace, and the Windows
> layer needs share modes and range locks.
> ([all changes](fs-versions.md#transition-v2---v3))

A handle holds an **object** (dataset, oid, generation), not a path.

```rust
use fs::{access, share};

let a = fs.open("/report", access::READ, share::ALL)?;   // A reads the report
fs.create("/report.tmp", Kind::File)?;
fs.write("/report.tmp", 0, b"new version")?;
fs.rename_replace("/report.tmp", "/report")?;            // B atomically replaces it
let old = fs.read_h(a, 0, 100)?;                          // A still reads the original
let b = fs.open("/report", access::READ, share::ALL)?;   // a new open = the new object
fs.close(a)?;                                             // the original object goes away
```

| Function | What it does |
|---|---|
| `open(path, access, share)` | opens a file or directory (a link is not followed: `WRONG_TYPE`); access `access::READ/WRITE/DELETE`, sharing `share::READ/WRITE/DELETE/ALL` |
| `read_h`, `write_h`, `set_len_h`, `stat_h`, `list_h` | operations through a handle (checks access, generation and mandatory locks) |
| `close(h)` | closes; with the last handle an orphan disappears and delete on close happens |
| `rename_replace(from, to)` | atomic replace: an existing target (file for file, empty directory for directory) goes away in the same operation |
| `set_delete_on_close(h, yes)` | the name disappears when the last handle closes (needs `access::DELETE`) |
| `lock_range(h, from, length, exclusive, mandatory)`, `unlock_range` | a range lock (length 0 = to the end of the file) |

**Lifetime:**

- **Orphan:** removing the last name of an open object does not delete the
  data - the record gets parent 0 and an `ORPHAN` entry (`META_OID`, kind 9).
  Whoever has it open keeps reading and writing; it disappears with the
  last `close`. It does not count into directory size summaries.
- **After a crash or restart** handles are gone (they live only in memory)
  and mounting deletes orphans. Writes to an orphan in the intent log are
  skipped on replay (the object no longer has a record).
- **Generation:** a handle checks the object's generation; an object that
  has gone away in the meantime = `BAD_HANDLE`.

**Share modes** (like Windows `FILE_SHARE_*`): a new open must have the
requested access shared by all open handles and must itself share their
access, otherwise `LOCKED`. **Deleting and renaming** an open object
requires all handles to share deletion. Path operations (`write`,
`truncate`) behave like an open with that access.

**Range locks** (like `LockFileEx`): shared or exclusive; an **advisory**
lock only guards against other locks, a **mandatory** one also blocks
writes (exclusive: reads too) by other handles and paths. They belong to a
handle and go away when it is closed.

**POSIX locks** (`fcntl` `F_SETLK`/`F_SETLKW`/`F_GETLK` and `flock`, message
`POSIX_LOCK`) **[verified in QEMU]** (suite `cstd`, test `ctest lock`: two
processes over one file): advisory range locks owned by the **session**
(process; `flock`: the handle). fsd keeps them in memory
(`libs/fs/src/posix_lock.rs`, host tests `cargo test -p fs --features std
posix_lock`); they are not written to disk.

- Own locks never conflict: a new lock in a range replaces the old one
  (change of kind, splitting), adjacent locks of the same kind merge;
  unlocking a part splits the range. `F_GETLK` returns the first foreign
  conflicting lock.
- Release: closing the session's last handle on the file (`flock`: closing
  the handle) and the end of the session (end of the process); an fsd
  restart drops the locks (a client whose handle had a lock gets
  `SERVICE_RESTARTED`).
- **Waiting** (`F_SETLKW`) is done by the client by retrying (from a
  millisecond up to fifty milliseconds between attempts): a blocking request
  would hold the session's only channel and stop the process's other file
  operations. fsd remembers an attempt announced as waiting (`SET_WAIT`:
  who waits for whom, valid for one second without renewal) and reports a
  closed wait cycle as a deadlock (`EDEADLK`).
- `flock` and `fcntl` do not affect each other (as on Linux).

⚠️ **Limitations:** handles live in the memory of one library instance (a
service, a driver) and are gone after a restart; there are no durable
handles yet. Range locks (`LOCK`) do not wait - a conflict returns
`LOCKED` at once; POSIX locks (above) do wait.

⚠️ **Limitation:** the protocol has no open relative to a directory
handle; operations go by path. The `openat`, `unlinkat`, `mkdirat`,
`fstatat` functions in [cstd](cstd.md#functions-for-c-step-7a) therefore
join the directory path remembered when it was opened with the name: they
do not protect against the directory being renamed between the open and
the call (Linux binds them to the directory inode). Fixed only with an
open relative to a directory handle in fsd **[designed]**.

## Snapshots

> **Change from v3 (phase 1):** a snapshot has metadata (the `SNAPMETA`
> item): kind (manual, automatic, commit, view, aside), creation time and
> journal position; `SnapshotInfo` returns them. Temporary views (`VIEW`)
> are not in the listing. **Why:** automatic snapshots are pruned by time
> and only their own, and undo of operations looks for "the last snapshot
> before the operation" by the journal. ([FS history](fs-history.md))

A snapshot is the tree root saved at creation time - read-only, created
instantly (one commit), takes no space until live data changes.

```rust
fs.snapshot("before-update")?;                       // commit + record
for s in fs.snapshots()? { println!("{} {}", s.id, s.name); }
let st = fs.snapshot_stat("before-update", "/etc/config.toml")?;
let data = fs.snapshot_read("before-update", "/etc/config.toml", 0, 4096)?;
fs.delete_snapshot("before-update")?;                // frees what only it held
```

Snapshot names follow the same rules as file names (including case
insensitivity).

### Deadlists: deletion without walking trees

Blocks born no later than the generation of the last snapshot are not freed
by the live version when overwritten; they are written into a **deadlist**
instead (list `L` = blocks "killed" between the previous snapshot and
snapshot `L`; the live version uses list `next_snapshot`). The algorithm is
the same as in ZFS:

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Deadlists">
<path class="ln" d="M20 60 L740 60"/>
<circle class="acc" cx="140" cy="60" r="10"/><text class="t b" x="140" y="35" text-anchor="middle">P</text><text class="m" x="140" y="90" text-anchor="middle">previous</text>
<circle class="err" cx="380" cy="60" r="10"/><text class="t b" x="380" y="35" text-anchor="middle">S (being deleted)</text>
<circle class="acc" cx="620" cy="60" r="10"/><text class="t b" x="620" y="35" text-anchor="middle">N or the live version</text>
<text class="m" x="260" y="90" text-anchor="middle">deadlist S</text><text class="m" x="500" y="90" text-anchor="middle">deadlist N</text>
<text class="t" x="20" y="125">1. Blocks in deadlist N born after P were held only by S → free them.</text>
<text class="t" x="20" y="147">2. The rest of N stays; the whole deadlist S moves into N.</text>
</svg>
<figcaption>Deleting snapshot S: walking one list is enough, no reference counting.</figcaption>
</figure>

## Versioning and git

> **Change from v3 (phase 1):** versioned commits with a logical ID,
> branches as clones, three-way merge, file versions and extended
> attributes are done. **Why:** phase 2 of the design. ([FS history](fs-history.md))

**Done:** snapshots and datasets give instant versions, writable clones
and the diff of versions; phase 2 added **commits** (author, message,
parents, a BLAKE3 logical ID independent of block placement), **branches**,
**merge**, file versions, undo of operations and the change journal - see
[FS history](fs-history.md). The checksums in pointers form a Merkle tree -
the same principle git is built on.

**Planned** (see [architecture](architecture.md#content-addressed-fs-and-native-versioning-proposal)):

- A/B system updates on top of clones,
- the difference of two versions without walking the whole tree (comparing
  subtree checksums),
- a git layer (phase 5): git object IDs as extended attributes, repository
  import and export, push/pull over the network,
- indexes for search (name, type, time, size).

## Values from disk and all-or-nothing operations

> **Change from v2:** a new chapter. **Why:** a software bug with a correct checksum could damage the allocator (double free) or make a walk loop. ([all changes](fs-versions.md#transition-v2---v3))

The checksum in a pointer protects against media damage, but not against a
software bug that writes a nonsensical value with a correct checksum. So
values from disk are checked when used as well:

| Value | Check | On error |
|---|---|---|
| extent | the whole range inside the volume, the offset does not overflow | `CORRUPTED` (no read or free outside) |
| a range being freed (extent, deadlist, node) | inside the volume, fully allocated and not waiting for a delayed free (`Space::free`, an index of delayed ranges) | not freed, `CORRUPTED` and the error state (`fsync` confirms nothing either) - a double free would damage the allocator |
| content of a small file | length exactly equal to the size in the record | `CORRUPTED` |
| the parent chain (summaries, move) | at most 65,536 steps (`MAX_DEPTH`) | `CORRUPTED` instead of a loop |
| a space log record | range without overflow | mount fails (`CORRUPTED`) |

**All-or-nothing operations** (`Fs::guarded`): public modifying operations
(`create`, `symlink`, `remove`, `rename`, writing into the buffer,
`truncate`, snapshot deletion, defragmentation) count changes of the
in-memory state.

- A failure **before** the first change (bad name, does not exist, not
  enough space, a directory entry would not fit - it is prepared in
  advance) endangers nothing.
- A failure **after** a change (e.g. a node read error in the middle) would
  leave the operation half done. The volume enters the error state: no
  commit or `fsync` confirms it, all further changes return `IO_ERROR`.
  Mount again = the state of the last commit + the intent log.

A full in-memory rollback (without the error state) would need a copy of
the dirty tree before every operation; it comes with application
transactions (phase 5).

## Consistency check and scrub

> **Change from v3 (phase 2):** also checks the copies of nodes and
> extents, the sharing table (a block used more than once must be shared,
> a record without used blocks = an error) and the deduplication index;
> scrub reads both copies and repairs. In addition a **background scrub**
> in steps. **Why:** every format change must be verifiable.
> ([FS data](fs-data.md#scrub))

> **Change from v3 (phase 1):** also checks the history structures: a
> contiguous journal sequence and its state, snapshot metadata, the trash
> (record <-> entry in the trash), the attribute count, the expiry index
> against the attributes, the logical hash cache. **Why:** every format
> change must be verifiable. ([FS history](fs-history.md#on-disk))

> **Change from v2:** checks the stored state also on a read-only mount, fixed memory (bitmaps), a limited number of reports, checks of object kinds and link targets. **Why:** during uncertain recovery the state must be checked without writing, and a large or badly damaged volume must not overwhelm the check. ([all changes](fs-versions.md#transition-v2---v3))

`check(scrub)` checks the **stored state** (the last commit on disk):

- a writable volume: a commit first, so the stored state = the live one,
- **read-only** (uncertain recovery, a device without flush): nothing is
  written; the stored generation and the space metadata loaded again from
  disk are checked. Changes replayed from the log only in memory are not
  checked.

The result says what it checked: `generation` (the generation number) and
`live_not_checked` (there are unchecked changes in memory). It walks the
stored tree, all snapshots, the free-space metadata and the intent log:

- verifies the checksums of all nodes (and with `scrub = true` of all data
  blocks - reads the whole content),
- every used block must be allocated according to the allocator,
- no allocated block may be "lost" (nothing points to it),
- the live tree must not use one block twice,
- the kind in a directory entry must match the object record, and a
  symbolic link must have a target whose length equals its size (and
  nothing else may have a target).

**Check memory:**

| Part | Size | Depends on |
|---|---|---|
| 3 bitmaps (used, data, seen nodes) | 3 bits per block: a 1 TiB volume = 96 MiB | volume size |
| object map (kind, size) and the list of directory entries | ~40 B per object + ~16 B per entry: a million files ~ 56 MiB | **number of files** in the live version |
| problem descriptions | at most 1,000 (the total is in `problems_total`) | - |

A damaged volume with millions of errors cannot overwhelm the check; a
large number of files does increase memory though (planned: checking
objects in parts by oid range). A node or extent outside the volume is a
reported problem, not a crash. **A damaged object record or directory
entry** is a reported problem - before, the decoding error was skipped and
the check reported a clean volume although `stat` returned `CORRUPTED`. Likewise **extents are processed in batches** (256):
truncating or deleting a huge file (1 TiB = ~540,000 extents with
checksums, ~4 GiB) never loads all extents into memory at once.

It returns a `CheckReport` with block counts (tree, data, metadata) and a
list of problems (empty = OK). The tests call it after every scenario.

## Performance and overhead

Measured by `tests/perf.rs` (in-memory device = the pure cost of the FS
without a disk, release, one thread):

| Measurement | Result |
|---|---|
| sequential write of 512 MiB in 1 MiB pieces + commit | ~1.4 GiB/s, **1 contiguous range** |
| sequential read | ~2.7-3.0 GiB/s |
| metadata overhead for data (tree, extents, checksums) | **0.42 %** of capacity |
| 20,000 files of 4 KiB (create + write + commit) | ~100,000 files/s |
| `stat` of a directory with 20,000 files (subtree size) | 10-13 µs |

Run: `cargo test -p fs --features std --release -- --ignored --nocapture perf`.

**Where the write limit is** (breakdown by step on 512 MiB, release):

| Step | Speed |
|---|---|
| memory copy (memcpy) | ~10 GiB/s |
| XXH3-128 checksum per 4 KiB | ~19 GiB/s |
| delayed allocation buffer (4 KiB `Vec` in a `BTreeMap`) | ~4.3 GiB/s |
| assembling blocks into a contiguous buffer | ~5.7 GiB/s (byte by byte before: 1.7 GiB/s) |

So the limit is in the implementation, not in the format: the 4 KiB block
buffer costs the most. Planned: a buffer of large runs (one allocation per
contiguous write) and writing straight from the client's buffer. **In the
running system** the limit is elsewhere today: protocols fsproto v1 and
blockproto copy data in channel messages and are synchronous (one request
at a time). Protocol v2 with shared memory and request queues solves that.

## The fsd service

> **Change from v2:** an error sending a reply does not end the service, `CHECK` only in `debug` mode, the reason for read-only mode is reported, signature `v3`. **Why:** a crash of the service would leave the volume without a final commit. ([all changes](fs-versions.md#transition-v2---v3))

`user/fsd` keeps one mounted volume and serves a client over a channel.

- **Start:** at boot, init reads blocks 1-8 of all disks and starts the
  service on the first one with the signature `OSFS\0\0v3`. Startup message:
  a channel to the disk driver and a channel for the client (both
  `kind USER`). The first message on the bootstrap channel is the
  configuration: space-separated words, `debug` enables test operations.
- **Device:** an FS block = 8 sectors; contiguous ranges go in one driver call.
- **Commit** every 5 s (`COMMIT_INTERVAL`); a client's `fsync` is durable
  immediately through the intent log.
- **When idle** after a commit it defragments in steps of 256 blocks.
- **TRIM** enabled.
- **End:** when the client closes the channel, the service commits and exits.
- **An error sending a reply** (e.g. the reply does not fit into a message)
  does not end the service: it sends `BUFFER_TOO_SMALL` instead. It ends
  only when the channel is closed - otherwise the volume would be left
  without a final commit.
- **`CHECK` only in `debug` mode** (a long operation over the whole volume;
  v1 has no client authentication, with protocol v2 it will follow
  permissions).

The service's messages are described in [messages.md](messages.md#fsd-service-and-file-system).

## The fsproto protocol

> **Change from v2:** new operations `LIST_PAGE` and `SNAP_LIST_PAGE` (paged listing), `CHECK` returns the generation and the total count, kind 3 = symbolic link. **Why:** a large directory did not fit into one message and the service ended because of it. ([all changes](fs-versions.md#transition-v2---v3))

Messages over a channel (version 1, synchronous, path-based). Request: `op`
u32 and parameters in order; reply: `status` i32 (0 or error code),
reserved u32, result. Numbers little-endian, string = length u16 + UTF-8,
data = rest of the message. One read/write moves at most 32 KiB
(`MAX_IO`); `FsClient` splits larger requests.

| op | Name | Parameters | Result |
|---|---|---|---|
| 1 | `INFO` | - | volume label, block size, blocks, free, delayed, generation, algorithm, fallback, replayed |
| 2 | `STAT` | path | `Stat` (oid, kind, size, mtime, ctime, btime, owner, rights, subtree summaries, masks `known` and `hidden` u16); a time = seconds i64 + nanoseconds u32 |
| 3 | `LIST` | path | count u32 + (kind u8, oid u64, name) - sorted by name; kind 1 file, 2 directory, 3 symbolic link; `BUFFER_TOO_SMALL` when the listing does not fit into a message |
| 4 | `READ` | path, offset u64, length u32 | data (shorter at the end of the file) |
| 5 | `WRITE` | path, offset u64, data | - |
| 6 | `CREATE` | path, kind u8 (1 file, 2 directory; other = `INVALID_ARGS`) | - |
| 7 | `REMOVE` | path | - |
| 8 | `RENAME` | from, to | - |
| 9 | `TRUNCATE` | path, size u64 | - |
| 10 | `COMMIT` | - | - |
| 11 | `FSYNC` | - | - |
| 12 | `SNAPSHOT` | name | snapshot number u64 |
| 13 | `SNAPSHOTS` | - | count u32 + (id, generation, name) |
| 14 | `DELETE_SNAPSHOT` | name | - |
| 15 | `SNAP_STAT` | snapshot, path | `Stat` |
| 16 | `SNAP_LIST` | snapshot, path | as `LIST` |
| 17 | `SNAP_READ` | snapshot, path, offset, length | data |
| 18 | `DEFRAG` | path | rewritten blocks u64 |
| 19 | `FRAGMENTS` | path | number of ranges u64 |
| 20 | `CHECK` | scrub u8 | generation u64, total problems u64, unchecked changes in memory u8, count u32 + descriptions (as many as fit); only with `debug` |
| 21 | `LIST_PAGE` | path, cursor (name of the last entry of the previous page, empty = from the start), at most u32 | more u8 (1 = something is left) + a page like `LIST` (as much as fits into 60 KiB) |
| 22 | `SNAP_LIST_PAGE` | snapshot, path, cursor, at most u32 | like `LIST_PAGE` |
| 87 | `LIST_STAT_PAGE` | path, cursor, at most u32 | more u8 + count u32 + (name, kind u8, oid u64, size u64, mtime, rights u8, subtree summaries, masks `known` and `hidden` u16) - as much as fits into 60 KiB; rights `LIST` and `TRAVERSE` on the directory |
| 100 | `SIMULATE_POWER_LOSS` | - | replayed operations u64; only with `debug`, otherwise `ACCESS_DENIED` |

`FsClient::list` and `snapshot_list` read page by page - this solves the
**single message limit** (64 KiB), not memory:

- **The cursor is a name** - the last entry (case-insensitive comparison,
  the same order as the listing). A directory change during the listing: an
  entry that exists the whole time comes exactly once; a new one after the
  cursor appears, before it does not; a removed one disappears. Never a
  duplicate or a skipped existing entry (paging by index would have both).
- ⚠️ **Memory:** the service builds the whole listing once and keeps it for
  further pages (any other operation drops it); the client keeps the whole
  result. Both grow with the directory (~100 B per entry: 1 million
  entries ~ 100 MiB on each side). Listing without keeping the whole list
  comes with protocol v2 (a cursor in the tree, the client processes pages
  one by one).

**`LIST_STAT_PAGE`** (`FsClient::list_stat` -> `Vec<StatEntry>`) is a
listing with metadata for `ls -l`: the client does not have to send `STAT`
for every entry (thousands of messages for a large directory). Cursor and
paging like `LIST_PAGE`. The entries are the same as with `LIST` (only
those the caller has some right to); kind and oid like in `LIST` (for a
dataset entry kind 4 and the dataset number), size and mtime like `STAT`
of the entry. Because the metadata is the `STAT` of the entries, the
operation requires `TRAVERSE` on the directory besides `LIST` (without it
`STAT` of the entries would fail; otherwise `ACCESS_DENIED`). Works for v1
and v2 sessions (the same handler).

#### Value availability: known, hidden, unavailable

Both `Stat` and `StatEntry` end with two u16 masks with the same bits
(`fsproto::avail`) **[implemented]**. Host tests: `cargo test -p fsproto`
(codec), `cargo test -p fs --features std avail`.

| bit | constant | field |
|---|---|---|
| 0 | `SIZE` | size (for a directory `tree_bytes`) |
| 1 | `MTIME` | `mtime` |
| 2 | `CTIME` | `ctime` |
| 3 | `BTIME` | `btime` |
| 4 | `OWNER` | `owner` |
| 5 | `TREE` | `tree_files`, `tree_dirs` |
| 6 | `RIGHTS` | `rights` |

- **`known`** = the value is valid. An empty folder with the `MANAGE` right
  is a known 0 B / 0 files / 0 folders.
- **`hidden`** = the value exists, but the caller must not see it; the
  field is 0. Subtree summaries of a directory without the `MANAGE` right
  are `SIZE | TREE` in `hidden` (before, just zeros that could not be told
  apart from an empty folder).
- **Neither bit** = unavailable: the FS does not keep it, or it does not
  belong to the object kind. `TREE` for a file and a link; for the boundary
  of a nested dataset in a clone the size, times, owner and summaries
  (`known` is only `RIGHTS`).
- The codec rejects a message with `known & hidden != 0` or with an unknown
  bit (`INVALID_ARGS`).
- `StatEntry` carries only the bits of its own fields (`SIZE`, `MTIME`,
  `RIGHTS`, `TREE`); fsd clears the other bits.

On the wire this is +4 B at the end of `Stat` (`Stat::SIZE`) and of every
`LIST_STAT_PAGE` entry (`StatEntry::FIXED`). Because of it the protocol has
version 3 (`HELLO`); the older message shape is not read.

> **Change from protocol version 2:** the `known` and `hidden` masks.
> **Why:** zero meant "you may not see it", "empty" and "the FS does not
> keep it" and a client (tables in the shell) could not tell them apart;
> now it shows a hidden and an unavailable value differently from zero.

Example message `READ "/a.txt", offset 0, 100 B`:

```text
04 00 00 00   06 00 2f 61 2e 74 78 74   00 00 00 00 00 00 00 00   64 00 00 00
op = 4        path (length 6, "/a.txt")  offset = 0                length = 100
```

### Protocol v2: sessions, handles, conditional save

> **Change from v1:** v1 works only with paths and has one client.
> **Why:** a file must survive a rename and deletion, several programs
> (an editor, an agent, sync) need their own sessions and a write must not
> silently overwrite someone else's change. v1 operations remain valid.
> ([format and protocol versions](fs-versions.md#protocol-fsproto-v1---v2))

```rust
use fsproto::v2::{access, share, open_flags};

let editor = fs.connect()?;                                  // its own session
let f = editor.open("/config.toml", access::READ, share::ALL, 0)?;
let text = editor.read_h(f.handle, 0, 4096)?;
editor.close(f.handle)?;
// ... the user edits ...
match editor.save_if("/config.toml", Some(f.revision), b"a = 3") {
    Ok(rev) => {}                         // saved, a new revision
    Err(Error::Conflict) => {}            // someone else changed it: nothing changed
    Err(e) => return Err(e),
}
```

| op | Name | Parameters | Result |
|---|---|---|---|
| 30 | `HELLO` | - | version u32 (3), max_io u32 |
| 31 | `CONNECT` | - | a new session channel (a handle in the message) |
| 32 | `OPEN` | path, access u8, share u8, flags u8 (`CREATE` 1, `EXCL` 2, `TRUNCATE` 4, `DIR` 8) | handle u64, `Stat`, `Revision` |
| 33 | `CLOSE` | handle | - |
| 34 | `READ_H` | handle, offset u64, length u32 | data |
| 35 | `WRITE_H` | handle, offset u64, data | **accepted u32** |
| 36 | `SET_LEN` | handle, size u64 | - |
| 37 | `STAT_H` | handle | `Stat`, `Revision` |
| 38 | `LIST_H` | handle, cursor, at most u32 | more u8 + entries |
| 39 | `LOCK` | handle, from u64, length u64 (0 = to the end), flags u8 (`EXCLUSIVE` 1, `MANDATORY` 2) | - |
| 40 | `UNLOCK` | handle, from, length | - |
| 41 | `DELETE_ON_CLOSE` | handle, enable u8 | - |
| 42 | `RENAME_REPLACE` | from, to | - |
| 43 | `REPLACE_BEGIN` | path, has revision u8, `Revision` | handle of a temporary file |
| 44 | `REPLACE_COMMIT` | handle, force u8 | `Revision` of the new content, or `CONFLICT` |
| 45 | `REPLACE_ABORT` | handle | - |

`Revision` = dataset u64, oid u64, generation u64, content version u64.

- **Sessions:** `CONNECT` returns a new channel; every session has its own
  handles (a handle of another session = `BAD_HANDLE`). Closing the channel
  closes all handles of the session: orphans disappear, locks are released,
  temporary files of a conditional save are deleted. The number of
  sessions is limited only by memory: the fsd loop waits via a port
  (`libos::Waiter`), not via `OBJECT_WAIT_MANY`. Closing the main channel (from the startup message)
  ends the service.
- **Write = accepted:** the `WRITE_H` reply says how many bytes the service
  accepted (they are in the buffer, durable only after `FSYNC`). A write
  split into several messages is several separate writes;
  `FsClient::write_h` stops after the first error and returns the number of
  bytes accepted before it.
- **Conditional save:** `REPLACE_BEGIN` creates a temporary file
  `.~name.N.tmp` in the same directory (with delete on close); `WRITE_H`
  writes the new data into it; `REPLACE_COMMIT` atomically replaces the
  target only at the expected revision (`has revision = 0` = the target must
  not exist), otherwise `CONFLICT` and nothing changes - the temporary file
  stays (save alongside, confirm with `force`, or `REPLACE_ABORT`).
  `FsClient::save_if` does it all at once. Whoever had the original file
  open keeps reading the original content.

History operations (the journal and watching it, view, trash, undo of
operations, versions, dataset settings, commits, branches, merge, "aside",
attributes, expiry, dataset creation and deletion, version difference, the
dataset of a path and reading a dataset snapshot) have numbers **54-86** and **88-92** - the table is in
[FS history](fs-history.md#protocol-operations-54-92). The listing with metadata
`LIST_STAT_PAGE` has number 87 (module `fsproto::op`, the table above).

⚠️ **Limitations:** handles are valid until the service restarts; after a
crash and intent log replay a file may have a different revision (the
content version is not logged) - the client then gets `CONFLICT` (the safe
direction). Data is still copied in messages (32 KiB per message); shared
memory and request queues come later.

## Permissions and session identity

> **Change from v2:** v2 had no permissions (whoever had the channel could
> do anything). **Why:** user files, sharing between users and applications
> that see only what the user gave them. The format has been ready since v3
> (owner and descriptor number in the object record); now it is used.
> ([all changes](fs-versions.md#transition-v2---v3))

<figure class="diagram">
<svg viewBox="0 0 760 270" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Session identity and permissions">
<defs><marker id="pm1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="180" height="70" rx="6"/><text class="t b" x="100" y="45" text-anchor="middle">init / session manager</text><text class="m" x="100" y="64" text-anchor="middle">fsd control channel</text><text class="m" x="100" y="80" text-anchor="middle">(system)</text>
<path class="ln" d="M190 55 L268 55" marker-end="url(#pm1)"/><text class="m" x="229" y="47" text-anchor="middle">CONNECT_AS</text>
<rect class="box" x="270" y="15" width="220" height="80" rx="6"/><text class="t b" x="380" y="38" text-anchor="middle">application session</text><text class="m" x="380" y="56" text-anchor="middle">identity: Alice (1000)</text><text class="m" x="380" y="72" text-anchor="middle">/app -> /home/alice/apps/editor</text><text class="m" x="380" y="88" text-anchor="middle">/picked/photo.jpg (read only)</text>
<path class="ln" d="M490 55 L568 55" marker-end="url(#pm1)"/><text class="m" x="529" y="47" text-anchor="middle">FS path</text>
<rect class="ok" x="570" y="20" width="180" height="70" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">libs/fs</text><text class="m" x="660" y="64" text-anchor="middle">owner, grants,</text><text class="m" x="660" y="80" text-anchor="middle">inheritance, visibility</text>
<path class="ln" d="M100 90 L100 150 L268 150" marker-end="url(#pm1)"/><text class="m" x="185" y="142" text-anchor="middle">GRANT / REVOKE</text>
<rect class="box" x="270" y="125" width="220" height="50" rx="6"/><text class="t" x="380" y="147" text-anchor="middle">Open dialog (powerbox)</text><text class="m" x="380" y="165" text-anchor="middle">adds a mounted path to the session</text>
<text class="m" x="10" y="215">Outside mounted paths = NOT_FOUND, ".." = INVALID_ARGS. Inside, FS permissions decide by the session identity;</text>
<text class="m" x="10" y="233">an object without any right is invisible (NOT_FOUND); with a right but not the needed one = ACCESS_DENIED.</text>
<text class="m" x="10" y="251">A client cannot invent an identity: it only gets a channel from whoever created the session.</text>
</svg>
<figcaption>init mints the session identity and namespace; the FS evaluates permissions.</figcaption>
</figure>

### Model

- **Who asks** (`Principal`): a user and their groups. User 0 is the
  system: checks do not apply (host tools, the Windows driver, internal
  operations). The FS gets the identity from the caller
  (`Fs::set_principal`); the fsd service sets it per session.
- The **owner** of an object has all rights. A new object belongs to whoever
  created it. Only the system changes the owner (`set_owner`).
- **Rights:** `LIST`, `TRAVERSE`, `READ`, `WRITE`, `CREATE`, `DELETE`,
  `EXECUTE`, `MANAGE` (permissions, snapshots, datasets). Levels:
  **View** `VIEW` = list + traverse + read, **Edit** `EDIT` = view + write +
  create + delete, **Run** `RUN` = view + execute.
- **Grants:** a permission descriptor (`Acl`) = a list "who -> rights",
  **allow only**. A grant applies to the object itself (`THIS`), is
  inherited by descendants (`INHERIT`), or both. The grantee is a user, a
  group or `EVERYONE` (everyone except the system).
- **Inheritance:** an object has its parent's inherited grants plus its own.
  The `PROTECTED` flag stops inheritance - a private folder inside a shared
  one without any deny entries.
- **Visibility:** an entry the caller has no right to is not listed and
  `stat` returns `NOT_FOUND` for it (not `ACCESS_DENIED` - not even the name
  leaks). Every directory along the path needs `TRAVERSE`. Subtree summaries
  (`tree_*`) are shown only to the owner and `MANAGE`.

| Operation | Required right |
|---|---|
| `stat`, `revision` | any (visibility) |
| `list` | `LIST` |
| `read`, `readlink`, `fragments` | `READ` |
| `write`, `truncate`, `set_times` | `WRITE` |
| `create`, `symlink`, `create_dataset` | `CREATE` on the parent |
| `remove` | `DELETE` on the object |
| `rename`, `rename_replace` | `DELETE` on the source (and on a replaced target) + `CREATE` on the target parent |
| `open` | by access: `READ`, `WRITE`, `DELETE`, `EXECUTE` |
| `set_acl`, `set_noexec`, `snapshot_of`, `delete_snapshot_of`, `diff`, `clone_snapshot` (source) | `MANAGE` |
| `delete_dataset` | `MANAGE` + `DELETE` |
| `snapshots_of`, reading in a snapshot | `READ` on the dataset root, inside the permissions **from the snapshot** |
| `set_owner`, `check`, defragmentation | system only |

**Handles:** rights are checked at `open`; operations through the handle are
not checked again (a capability). Listing through a handle (`list_h`) hides
entries the same way as `list`.

**Snapshots** keep the permissions from when they were taken: a file the
author hid later is visible in an old snapshot to whoever could see it then
and has `READ` on the dataset root.

### On disk

- A **permission descriptor** is an `ACL` item (kind 10, object 0, offset =
  descriptor number) in the dataset: flags u32, grant count u16, reserved
  u16, **reference count u64**, grants (who u64, rights u8, flags u8).
  Objects point to it by number (`Inode::acl`, 0 = inherit only), so
  thousands of files with the same rights cost one record.
- The **index** `ACL_INDEX` (kind 11, offset = descriptor hash) finds the
  same descriptor when setting one: grants are sorted and merged before
  storing, so equal rights = the same descriptor. The last reference deletes
  the descriptor and its index entry.
- The **`RO_COMPAT_ACL`** flag (ro_compat bit 0) is set with the first
  descriptor: an older driver may only read the volume (when deleting it
  would not decrement reference counts).
- **Dataset properties** `PROPS` (kind 12, object 0): flags u32, bit 0 =
  **no execution** (`NOEXEC`). `open` with `EXECUTE` then fails for
  everyone, even the system - data datasets (downloads, documents) cannot
  hold runnable programs.
- **Intent log:** 10/11 = create/symlink with an owner (only when the owner
  is not the system - volumes without users stay readable by an older
  driver), 12 set descriptor, 13 change owner. They survive `fsync` and a
  crash.
- **fsck** checks that descriptors exist, reference counts match and the
  index is complete.

```rust
use fs::{Acl, Grant, Principal, grant, rights, acl_flags};

fs.set_owner("/home/alice", 1000)?;                    // system
fs.set_principal(Some(Principal::user(1000, &[])));     // from now on as Alice
fs.set_acl("/home/alice/shared", Some(Acl {
    flags: 0,
    grants: vec![Grant { who: 5000, rights: rights::EDIT, flags: grant::BOTH }], // team
}))?;
fs.set_acl("/home/alice/shared/private", Some(Acl {
    flags: acl_flags::PROTECTED, grants: vec![],        // private folder inside a shared one
}))?;
```

### Sessions in fsd: identity and mounted paths

- The **control session** = the main channel from the startup message (init
  has it, later the session manager). It is the system and sees the whole
  FS.
- **`CONNECT_AS`** (control only) creates a user session: identity (user,
  groups) and **mounted paths** `name in the session -> path in the FS`
  (read-only flag). It returns the session number and a channel that can be
  handed to an application. **A client cannot invent an identity** - the
  identity belongs to the channel.
- **Session namespace:** a path is translated by the longest mount at a
  folder boundary; outside the mounts `NOT_FOUND`, `..` and `.` =
  `INVALID_ARGS` (no escaping a mount). The FS does not follow links, so no
  escape through them either.
- **`GRANT` / `REVOKE`** (control only) add or remove a mounted path of a
  session - the Open/Save dialog (powerbox): the application gets only what
  the user picked; by the same mechanism a trusted application gets the
  working folders without a prompt
  ([Working folders](fs-v3-design.md#working-folders-and-protected-areas)
  **[designed]**).
- `CONNECT` from a user session creates another session with the same
  identity and paths (not a control one).
- Operations on the whole root dataset (v1 snapshots, check,
  defragmentation) are allowed only for a session with the mount `/ -> /`.

| op | Name | Parameters | Result |
|---|---|---|---|
| 46 | `CONNECT_AS` | user u64, groups (u32 + u64...), mounts (u32 + name, target, flags u8) | session number u64 + channel |
| 47 | `GRANT` | session u64, name, target, flags u8 | - |
| 48 | `REVOKE` | session u64, name | - |
| 49 | `PERMISSIONS` | path | owner u64, rights u8, has descriptor u8, [descriptor] |
| 50 | `SET_ACL` | path, has descriptor u8, [descriptor: flags u32, count u32, (who u64, rights u8, flags u8)...] | - |
| 51 | `SET_OWNER` | path, owner u64 (control only) | - |
| 52 | `SET_NOEXEC` | dataset root, enable u8 | - |
| 53 | `NOEXEC` | dataset root | flag u8 |

`Stat` additionally has the owner u64 and the caller's effective rights u8.
`OPEN` knows the `EXECUTE` access (8).

### Recovery after an fsd crash

> **Change from v3 (phase 3):** a new chapter. `fsd` runs under the init
> supervisor and is started again after a crash; clients with a lease carry
> on (the shared protocol [Service recovery](service-recovery.md)). New: the
> `REOPEN` operation (118), the `CONNECT` flag "control session", the disk
> operation `SYNC` (`blockproto`, 6), the mount option `keep_orphans`. No
> on-disk format change. **Why:** a service crash must not mean restarting
> the whole system or losing open files.

**Adapter** (`user/init/src/fsd.rs`): the supervisor keeps the end of the
channel to the disk driver (`Kept`), every instance gets a duplicate.
Configuration on the bootstrap: `start`, `debug` in a test run, after a
crash `restart <generation>`. The instance confirms the start with `ok`. A
new instance after a crash:

1. sends the disk a `SYNC` with a unique number and drops everything before
   its echo (late replies for the old instance - the block protocol has no
   request numbers),
2. mounts the volume with `keep_orphans`: orphans (deleted but open files)
   are kept for 30 s so clients can reopen them; then only those nobody
   reopened are cleaned up,
3. numbers handles from `generation << 40`, so a handle number of the old
   instance never points to someone else's file in the new one.

**Recipe** of a session (`libs/wire` encoding):

| Kind | Fields | Session |
|---|---|---|
| 0 system | - | another control session (`CONNECT` with the `CONTROL` flag) - held by init |
| 1 user | `u64` user, `u32` count + `u64` groups, `u32` count + mounts (name, target, flags) | `CONNECT_AS` from the adapter's control session |

**Narrowing** (for a child, lease `NARROW`): a list of mounts (`u32` count +
mounts). Each must lie inside one of the original ones (same target or
below it) and an original read-only mount must stay read-only; the user and
groups stay. A system recipe cannot be narrowed. Otherwise `ACCESS_DENIED`.

**Library** (`FsClient` over `svcclient::Conn`, `FsClient::with_lease`):

- every operation has a kind: reads, `stat`, listings, `info`, status and
  history reads are repeated automatically after a crash; the rest returns
  `SERVICE_RESTARTED` (`OPEN` without `CREATE`/`TRUNCATE` is repeated),
- **persistent handles**: the application gets a library handle number
  (unique within the process); the library keeps the object identity
  (dataset, oid, generation from `Revision`), access and sharing. After a
  crash it reopens the handle before the next operation (`REOPEN`): rights
  are checked again by the session identity, the path must lie in the
  session's mounts; an object with a different generation = `NOT_FOUND`. An
  orphan (no path) may be reopened only by its owner,
- reads through a handle (`READ_H`, `STAT_H`, `LIST_H`) are then repeated;
  writes and other handle operations return `SERVICE_RESTARTED` (the old
  instance might have performed them),
- a handle with a **lock**: locks died with the old instance - the first
  operation returns `SERVICE_RESTARTED` (the application takes the lock
  again), the next ones pass; "delete on close" is set again automatically,
- a **conditional save** handle (temporary file) is invalid after a crash:
  `SERVICE_RESTARTED`, then `BAD_HANDLE`; the application saves again.

| Op | Name | Fields | Result |
|---|---|---|---|
| 31 | `CONNECT` | flags u32 (1 = control, only from a control session) | new session (handle) |
| 118 | `REOPEN` | dataset u64, oid u64, generation u64, access u8, share u8 | handle u64, `Stat`, `Revision` (like `OPEN`) |
| 119 | `POSIX_LOCK` | handle u64, command u8 (`TEST` 0, `SET` 1, `SET_WAIT` 2, `UNLOCK` 3), class u8 (`FCNTL` 0, `FLOCK` 1), exclusive u8, from u64, length u64 (0 = to the end), process number u32 | result u8: `DONE` 0, `CONFLICT` 1 + exclusive u8, from u64, length u64, holder's process number u32 (0 = unknown); `DEADLOCK` 2 |
| 120 | `OPEN_STREAM` | handle u64, mode u8 (`READ` 0, `WRITE` 1, `APPEND` 2), position u64 | byte-stream channel (handle in the message) |
| 121 | `SOCKET_BIND` | path + handle of the listener channel | - (an existing name `ALREADY_EXISTS`) |
| 122 | `SOCKET_CONNECT` | path, process number u32 + handle of the connection end | - (`CONNECTION_REFUSED`, full queue `SHOULD_WAIT`) |
| 123 | `WHOAMI` | - | user u64 (0 = system), count u32, groups u64 |

**`OPEN_STREAM`** **[verified in QEMU]** (suite `cstd`, step `posix2`: the
output of a `posix_spawn` child into a file and 200 000 bytes of a file on
its stdin): a file as a byte stream for a child's stdin/stdout (the process
parameter block only carries stream channels,
[cstd](cstd.md#processes-posix_spawn-and-waitpid)). The stream may only do
what the client's handle may (read / write); fsd opens its own handle of
the same object (`reopen`), so the stream survives closing the client's
session. The fsd main loop serves it next to the sessions
(`user/fsd/src/stream.rs`): reading sends 60 KiB chunks until the channel
queue is full, then retries after 2 ms (a channel has no "writable"
signal) and closes the channel at the end of the file; writing takes the
channel's messages and writes them from the position (`APPEND` at the end).
At most 16 streams (`stream::MAX_STREAMS`); the loop waits through a port,
the number of sessions is limited only by memory. An fsd restart ends the
streams.

**`AF_UNIX` socket nodes** **[verified in QEMU]** (suite `cstd`, step
`unix` of the `ctest` test): `SOCKET_BIND` creates an object of kind 5
(name, owner, rights like a file; it has no data) and assigns a listener to
it - a channel into which fsd writes, on `SOCKET_CONNECT`, the client's
credentials (user and group from the session identity, verified by fsd; the
process number from the client) and the handle of the connection end.
`SOCKET_CONNECT` needs the write right on the node. The node -> listener
map lives only in fsd memory (`user/fsd/src/serve_sock.rs`): the end of the
listener or an fsd restart = `CONNECTION_REFUSED`, the node stays as on
Unix (`unlink` removes it, `bind` to an existing name = `ALREADY_EXISTS`).
The socket is neither in the kernel nor in the FS: a connection is a
channel pair between processes, fsd only arranges the first meeting
([cstd](cstd.md#af_unix-sockets)). **`WHOAMI`** returns the session
identity (C `getuid`, `getgroups`).

Sessions with a lease today: init (control), `fwd` (`/config`). The sessions
for `dnsd`, `ntpd` (certificate store) and `revocd` have no lease yet -
after an `fsd` crash they get `SERVICE_UNAVAILABLE` (wiring them belongs to
the network thread). Test: suite `obnova fsd`
(`user/init/src/tests/fs_recovery.rs`, client `user/fsrtest`).

### Profiles

A profile is a **system setting, not a format** (the boundary from the
design: what must hold for every write is in the FS; settings and decisions
are outside). The session manager uses it through `fsproto::perm::Profile`:

| | Standard (default) | Strict | Free |
|---|---|---|---|
| application session (`app_mounts`) | `/app` + picked items | `/app` + picked items | whole home + `/app` |
| remember picks (`remember_picks`) | yes | no (always the dialog) | yes |
| no execution on data (`noexec_on_data`) | yes | yes | no |
| user session (`user_mounts`) | `/ -> /`, permissions decide | same | same |

```rust
use fsproto::perm::{Mount, Profile, mount_flags};

let p = Profile::Standard;
let picked = [Mount::new("/picked/photo.jpg", "/home/alice/photo.jpg", mount_flags::READ_ONLY)];
let (id, app) = fs.connect_as(1000, &[], &p.app_mounts("/home/alice", "editor", &picked))?;
// ... later the user picks a folder:
fs.grant(id, &Mount::new("/documents", "/home/alice/Documents", 0))?;
```

⚠️ **Limitations:** subtree size summaries are not computed per user - who
lacks `MANAGE` sees 0. A dataset entry in a listing is judged by the
permissions of its root. Auditing (access log) and quotas come in phase 4.
The Windows driver (`osfs mount`) runs as the system: the mounted image is
entirely the Windows user's; permissions are kept but not enforced.

## Error codes

The meaning of the codes is in [errors.md](errors.md). When the file
system returns them:

| Code | When |
|---|---|
| `NOT_FOUND` | the path, parent or snapshot does not exist; at mount: the disk has no superblock |
| `ALREADY_EXISTS` | `create`/`rename`/`snapshot` to an existing name (case-insensitively) |
| `NOT_A_DIRECTORY` | a path component or the target of `list` is a file |
| `IS_A_DIRECTORY` | `read`/`write`/`truncate` of a directory |
| `NOT_EMPTY` | `remove` of a non-empty directory |
| `INVALID_ARGS` | relative path, invalid name or link target, moving a directory into its own subtree, formatting a device smaller than 64 blocks |
| `OUT_OF_RANGE` | a write or `truncate` beyond `MAX_FILE_SIZE` (2^63-1 B), offset + length overflow |
| `WRONG_TYPE` | `read`/`write`/`truncate` of a symbolic link, `readlink` of something that is not a link |
| `NO_RESOURCES` | a directory entry with colliding names would outgrow `MAX_VALUE` |
| `NO_SPACE` | a write or create would consume the commit reserve |
| `CORRUPTED` | a block checksum does not match or a structure makes no sense; for data reads = a damaged block (other files remain readable) |
| `IO_ERROR` | a device error; after a failed commit or a failed log write in `fsync` all further changes (mount again) |
| `NOT_SUPPORTED` | unknown format version or algorithm, an unknown `incompat` bit in the newest generation, a node or extent with an unknown flag; unknown protocol operation |
| `READ_ONLY` | a change on a volume mounted read-only (device without a durability guarantee, unknown `ro_compat` bit) |
| `ACCESS_DENIED` | `SIMULATE_POWER_LOSS` without `debug` mode |

## Tests

| Test | What it verifies |
|---|---|
| `tests/basic.rs` | files, directories, names, summaries, errors, tree splits, space reuse, full disk, snapshots, corruption detection |
| `tests/features.rs` | contiguous writes, fsync + replay, generation fallback, snapshot deletion, defragmentation, TRIM, many AGs + BLAKE3, log condensing |
| `tests/model.rs` | property test: random sequences (create, write, truncate, remove, move, commit, fsync, "crash", snapshots) against an in-memory model; fsck at the end |
| `tests/handles.rs` | handles: a reader keeps the original object after an atomic replace, a removed open file until close (also cleanup after a crash and log replay), share modes, delete on close, range locks, handles across datasets and renames |
| `tests/versions.rs` | the first demo: snapshot, clone, changes, diff (including a move), restoring a file and a folder as a copy, the same with a crash in the middle of a write |
| `tests/datasets.rs` | datasets: own trees, rules, snapshots, two clones of one snapshot after its deletion, clone snapshots, fsync across several datasets, power loss during deletion, space accounting |
| `tests/crash.rs` | power loss during commit and fsync with a random subset of unwritten blocks |
| `tests/mount.rs` (extended) | an unreadable newest slot = read-only and the newest generation survives; an unreadable log batch does not lose a confirmed fsync; read-only `check` without writes |
| `tests/ondisk.rs` | damaged values with a correct checksum: extent outside the volume, freeing free blocks, small file length, parent cycle and operation rollback |
| `tests/errors.rs` | device errors: a failed data write loses no data and can be retried, an overwrite leaves no hole, nothing is confirmed after a failed flush |
| `tests/mount.rs` | superblock selection: unreadable slot, transient error without fallback, unknown `incompat` and newer version = refusal, unknown `ro_compat` = read-only without writes, nonsensical values = reported fallback, zeroing newer slots, device without a durability guarantee |
| `tests/objects.rs` | format v3: object record, names up to 1024 characters, text direction controls rejected, symbolic links (also through the intent log), file size limit and summary saturation, object generation |
| `tests/journal.rs`, `tests/trash.rs`, `tests/history.rs`, `tests/commits.rs` | history (phase 2) - see [FS history](fs-history.md#tests) |
| `tests/perf.rs` | performance and overhead (run manually) |
| init `tests/fs.rs` | 11 tests in the system through fsd and the real virtio-blk driver, including a file written on the host and a power-loss simulation |
| init `tests/fs_hist.rs` | history in the system through fsd: journal and `JOURNAL_WATCH`, trash, commit, branch and merge, "aside", attributes, fsck |

Number of random cases: `PROPTEST_CASES=150000 cargo test -p fs --features std --release`
(verified without errors).

## Limitations and plans

- One service = one volume and one client; multiple clients and mounting
  several volumes come with the namespace service.
- Protocol v1 copies data in messages and is synchronous; planned: shared
  memory and request queues (asynchronous I/O by design).
- File permissions and owners do not exist yet (they come with user
  management).
- The node cache is cleared entirely when full (8192 nodes); planned: LRU.
- Changing the Unicode tables version does not recompute name hashes yet.
- Compression (lz4/zstd) and encryption: planned.
