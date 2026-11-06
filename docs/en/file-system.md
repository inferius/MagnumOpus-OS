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
> design](fs-v3-design.md). The first part of phase 1 is done: 16 KiB nodes,
> v3 pointer, superblock and object record, names up to 1024 characters,
> symbolic links and the file size limit. A v2 volume is not mounted
> (`NOT_SUPPORTED`); data is not converted yet, disks from the `xtask` tool
> are simply recreated.

## What the format contains and what the driver uses

Format v3 has room for things the driver cannot do yet. To make it clear
what works:

| Item | In the format | Driver today |
|---|---|---|
| volume UUID, feature flags | yes | **uses** (no feature bit is defined yet) |
| device number in `BlockPtr`, `device_index/count` in the superblock | yes | **one device only**: the device must be 0 and the count 1, otherwise `CORRUPTED`; several disks (mirror, RAID) after phase 5 |
| `BlockPtr` flags (compression, encryption, sharing, cloud) | reserved | **unused**; a node with a flag = `NOT_SUPPORTED` |
| extent flags (preallocation) | reserved | **unused**; an extent with a flag = `NOT_SUPPORTED` |
| object flag `INODE_REMOTE` | reserved | **unused** (phase 6) |
| permissions (`acl`), owner, access time, extended attribute count in the object record | yes | **writes 0**, does not read (phase 1d and later) |
| object generation, creation time | yes | **uses** (`Stat`, log replay) |
| symbolic links, names up to 1024 characters, size limit | yes | **uses** |
| datasets, clones linked to the origin snapshot | yes | **uses** ([Datasets](#datasets-and-clones)) |
| diff of versions, restore as a copy | yes | **uses** ([Diff of versions](#diff-of-versions-and-restore-as-a-copy)) |
| handles, protocol v2 | no (coming) | plan: [v3 design](fs-v3-design.md#implementation-order) |

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
| Writable clones, native git, indexes, search | planned | [Versioning and git](#versioning-and-git) |

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
| 80 | 40 | root | `BlockPtr` of the B+tree root |
| 120 | 40 | space | `BlockPtr` of the AG table index |
| 160 | 8 | zil_block | first intent log block of this epoch |
| 168 | 8 | zil_seed | seed of the intent log checksums |
| 176 | 8 | next_oid | next free object number |
| 184 | 8 | snapshot_generation | generation of the last snapshot |
| 192 | 8 | next_snapshot | number of the next snapshot (= dead list of the live version) |
| 200 | 8 | dead_seq | sequence of the next dead list entry |
| 208 | 4 | unicode_version | Unicode table version for names |
| 212 | 2 | device_index | this device in the volume (0 for now) |
| 214 | 2 | device_count | number of devices of the volume (1 for now) |
| 216 | 32 | label | volume name (UTF-8, zero padded) |
| 4080 | 16 | csum | XXH3-128 of bytes 0..4080 |

**Feature flags** work as in ext4: every bit = one format feature
(compression, encryption, ...). A driver that does not know a bit from
`ro_compat` or `incompat` refuses the volume with `NOT_SUPPORTED` - so it
never damages it by not understanding new structures. No bits are defined
yet.

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

### Pointer (`BlockPtr`, 40 B)

> **Change from v2:** from 32 B to 40 B: device number (16 bits), length in blocks and flags. **Why:** a node has 4 blocks and the format is ready for several disks and for compression, encryption and shared blocks (visible right on the pointer). **The driver so far** supports only one device (number 0) and no flag - see [What the format contains](#what-the-format-contains-and-what-the-driver-uses). ([all changes](fs-versions.md#transition-v2---v3))

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | address: block number (bits 0-47) \| volume device (bits 48-63) |
| 8 | 8 | birth (generation of creation) |
| 16 | 16 | checksum of the content (all blocks it points to) |
| 32 | 4 | length in blocks (tree node = 4) |
| 36 | 2 | flags: 1 = compressed, 2 = encrypted, 4 = shared blocks, 8 = cloud only (reserved; a node with a flag is refused with `NOT_SUPPORTED`) |
| 38 | 2 | reserved (zeros) |

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
| `DATASET` | 8 | dataset number (object 0, only in the root dataset) | `DatasetRecord` 100 B: root, next oid, next snapshot, generation of the last snapshot, deadlist sequence, clone origin (dataset, snapshot, generation), flags |

### Object record (inode, 128 B)

> **Change from v2:** from 64 B to 128 B: creation and access time, generation, permissions, owner, extended attributes, flags. **Why:** oid + generation is a stable identity for handles; permissions and attributes will not need another format change. ([all changes](fs-versions.md#transition-v2---v3))

| Offset | Field | Meaning |
|---|---|---|
| 0 | kind u8 | 1 = file, 2 = directory, 3 = symbolic link |
| 1 | flags u8 | object flags, reserved: 1 = data only in the cloud (0 for now; sync settings are `sync.*` attributes, not bits) |
| 4 | acl u32 | permission descriptor number (0 = inherit; comes in phase 1d) |
| 8 | size | file: size in bytes (at most 2^63-1); link: target length |
| 16 | mtime | time of the last content change |
| 24 | ctime | time of the metadata change |
| 32 | btime | creation time |
| 40 | atime | time of the last access (0 = not tracked) |
| 48 | parent | oid of the parent directory (every object has exactly one parent) |
| 56 | generation | generation (txg) in which the object was created |
| 64 | tree_bytes | directory: total size of files in the whole subtree (saturating) |
| 72 | tree_files | directory: number of files and links in the subtree |
| 80 | tree_dirs | directory: number of directories in the subtree (excluding itself) |
| 88 | owner | owner (0 = system; comes with user management) |
| 96 | xattrs u32 | number of extended attributes |
| 100 | - | 28 B reserved (zeros) |

Times are in nanoseconds from `Fs::set_time` (the fsd service: since boot,
the `xtask` tool: since 1970). The
**generation** together with `oid` identifies an object uniquely: the oid of
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
      internal (64 B):  key 24 B, child BlockPtr 40 B
    leaf values are stored from the end of the node downwards
```

Limits: a value is at most 8144 B (`MAX_VALUE`, guarantees that an
overfull leaf can always be split), an internal node has at most 255
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
| `birth == txg` and the intent log does not reference it | free **immediately** | no saved state points to it |
| `birth > snapshot_generation` | free after a **delay** (`REUSE_DELAY` = 4 commits) | previous saved states may point to it (generation fallback) |
| `birth ≤ snapshot_generation` | into the **deadlist** | a snapshot holds it; freed when the snapshot is deleted |

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
checksums).

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
| `datasets()` | a list (number, clone origin) |
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

## Snapshots

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

**Done:** snapshots give instant versions of the whole volume and reading
any file from any version. The checksums in pointers form a Merkle tree -
the same principle git is built on.

**Planned** (see [architecture](architecture.md#content-addressed-fs-and-native-versioning-proposal)):

- writable snapshot clones (branches) and A/B system updates,
- the difference of two versions without walking the whole tree (comparing
  subtree checksums),
- with BLAKE3 content addressing: deduplication and mapping to git objects
  (blob/tree) - a git repository without duplicating data in `.git`,
- programmatic access to file history and extended attributes,
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
| 2 | `STAT` | path | `Stat` (oid, kind, size, mtime, ctime, subtree summaries) |
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

Example message `READ "/a.txt", offset 0, 100 B`:

```text
04 00 00 00   06 00 2f 61 2e 74 78 74   00 00 00 00 00 00 00 00   64 00 00 00
op = 4        path (length 6, "/a.txt")  offset = 0                length = 100
```

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
| `tests/versions.rs` | the first demo: snapshot, clone, changes, diff (including a move), restoring a file and a folder as a copy, the same with a crash in the middle of a write |
| `tests/datasets.rs` | datasets: own trees, rules, snapshots, two clones of one snapshot after its deletion, clone snapshots, fsync across several datasets, power loss during deletion, space accounting |
| `tests/crash.rs` | power loss during commit and fsync with a random subset of unwritten blocks |
| `tests/mount.rs` (extended) | an unreadable newest slot = read-only and the newest generation survives; an unreadable log batch does not lose a confirmed fsync; read-only `check` without writes |
| `tests/ondisk.rs` | damaged values with a correct checksum: extent outside the volume, freeing free blocks, small file length, parent cycle and operation rollback |
| `tests/errors.rs` | device errors: a failed data write loses no data and can be retried, an overwrite leaves no hole, nothing is confirmed after a failed flush |
| `tests/mount.rs` | superblock selection: unreadable slot, transient error without fallback, unknown `incompat` and newer version = refusal, unknown `ro_compat` = read-only without writes, nonsensical values = reported fallback, zeroing newer slots, device without a durability guarantee |
| `tests/objects.rs` | format v3: object record, names up to 1024 characters, text direction controls rejected, symbolic links (also through the intent log), file size limit and summary saturation, object generation |
| `tests/perf.rs` | performance and overhead (run manually) |
| init `tests/fs.rs` | 11 tests in the system through fsd and the real virtio-blk driver, including a file written on the host and a power-loss simulation |

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
