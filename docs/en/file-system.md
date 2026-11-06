# File system

Our own file system (working name "OSFS", format v2). The code lives in the
`libs/fs` library (`no_std`, the same on the host and in the system), the
service on top of a disk is `user/fsd`, and `libs/fsproto` defines the
protocol between a client and the service.

Design goals: **never lose consistency** (not even on power loss or a lying
disk), **detect every data corruption**, **be fast** (contiguous writes,
cheap `fsync`) and **not waste space** (metadata overhead ~0.4 %).

## What it offers

| Feature | Status | Where in this document |
|---|---|---|
| Copy-on-write: nothing live is overwritten in place | done | [Commit](#commit-step-by-step) |
| Atomic commit, consistency after power loss at any moment | done | [Power loss](#what-happens-on-power-loss) |
| Merkle tree: a 128-bit checksum in every pointer | done | [Checksums](#checksums-and-the-merkle-tree) |
| Checksum choice XXH3-128 / BLAKE3 | done | [Checksums](#checksums-and-the-merkle-tree) |
| Fallback by 1-3 generations when the newest state is damaged | done | [Generation fallback](#generation-fallback) |
| Fast `fsync` via an intent log (one write + flush) | done | [fsync and the intent log](#fsync-and-the-intent-log) |
| Extents, delayed allocation, contiguous writes | done | [Writing data](#writing-data-delayed-allocation-and-extents) |
| Allocation groups with space logs (no bitmap) | done | [Free space](#free-space-management) |
| TRIM/DISCARD for SSDs | done | [TRIM](#trim) |
| Background defragmentation | done | [Fragmentation](#fragmentation-and-defragmentation) |
| Instant directory size (subtree summaries) | done | [Summaries](#instant-directory-sizes) |
| Case-insensitive names (Unicode NFC) | done | [Names](#file-names) |
| Read-only snapshots and their deletion (deadlists) | done | [Snapshots](#snapshots) |
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
let opts = FormatOptions { label: "data".into(), algo: Algo::Xxh3, ag_blocks: None };
let dev = Fs::format(dev, &opts)?;          // a new empty file system
let mut fs = Fs::mount(dev)?;               // mount (+ intent log replay)
fs.create("/welcome.txt", Kind::File)?;
fs.write("/welcome.txt", 0, b"Hello!\n")?;
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
| **pointer (`BlockPtr`)** | (block number, birth, 128-bit checksum of the content). |
| **object (oid)** | A file or a directory; number `oid`. The root has `oid` 1, system metadata 0. |
| **extent** | A contiguous range of file data: physical start, length (at most 120 blocks), checksum of every block. |
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

The superblock carries **its own** XXH3-128 checksum in the last 16 bytes
of the block (it must be verifiable before the chosen algorithm is known).
The valid one is the one with the highest generation whose state can be
loaded.

| Offset | Size | Field | Meaning |
|---|---|---|---|
| 0 | 8 | magic | `OSFS\0\0v2` |
| 8 | 4 | version | 2 |
| 12 | 1 | algo | 1 = XXH3-128, 2 = BLAKE3 |
| 16 | 8 | generation | commit number |
| 24 | 8 | block_count | volume size in blocks |
| 32 | 8 | ag_blocks | allocation group size |
| 40 | 32 | root | `BlockPtr` of the B+tree root |
| 72 | 32 | space | `BlockPtr` of the AG table index |
| 104 | 8 | zil_block | first block of this epoch's intent log |
| 112 | 8 | zil_seed | seed of the intent log checksums |
| 120 | 8 | next_oid | next free object number |
| 128 | 8 | snapshot_generation | generation of the last snapshot |
| 136 | 8 | next_snapshot | number of the next snapshot (= the live version's dead list) |
| 144 | 8 | dead_seq | sequence of the next deadlist item |
| 152 | 4 | unicode_version | Unicode tables version for names |
| 160 | 32 | label | volume label (UTF-8, zero-padded) |
| 4080 | 16 | csum | XXH3-128 of bytes 0..4080 |

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

### Pointer (`BlockPtr`, 32 B)

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | block number |
| 8 | 8 | birth (generation of creation) |
| 16 | 16 | checksum of the block content |

## Metadata B+tree

All metadata lives in **one** copy-on-write B+tree. The key is
`(oid u64, kind u8, offset u64)`, ordered in that order - everything about
one object is stored together (one leaf read yields the inode and small
content).

| Kind | Value | Key offset | Item value |
|---|---|---|---|
| `INODE` | 1 | 0 | object record, 64 B (see below) |
| `DIRENT` | 2 | FNV-1a hash of the folded name | list of entries with the same hash: (oid u64, kind u8, name length u16, name) |
| `INLINE` | 3 | 0 | the whole content of a file up to 1984 B (`INLINE_MAX`) |
| `EXTENT` | 4 | first logical block | phys u64, count u32, reserved u32, birth u64, checksums 16 B × count |
| `SNAPSHOT` | 5 | snapshot number (object 0) | root `BlockPtr`, generation u64, name |
| `DEADLIST` | 6 | list << 32 \| sequence (object 0) | up to 80 dead ranges (start u64, length u64, birth u64) |

### Object record (inode, 64 B)

| Offset | Field | Meaning |
|---|---|---|
| 0 | kind u8 | 1 = file, 2 = directory |
| 8 | size | file size in bytes |
| 16 | mtime | time of the last content change (ns) |
| 24 | ctime | time of the metadata change (ns) |
| 32 | parent | oid of the parent directory (each object has exactly one parent) |
| 40 | tree_bytes | directory: total size of files in the whole subtree |
| 48 | tree_files | directory: number of files in the subtree |
| 56 | tree_dirs | directory: number of directories in the subtree (excluding itself) |

### Tree node (4 KiB)

```text
0   magic "BT", level u8 (0 = leaf), reserved, item count u16
8   generation of creation u64
32  items:
      leaf (32 B):      key 24 B, value offset u16, length u16, reserved
      internal (56 B):  key 24 B, child BlockPtr 32 B
    leaf values are stored from the end of the block downwards
```

Limits: a value is at most 2000 B (`MAX_VALUE`, guarantees an overfull leaf
can always be split), an internal node has at most 72 children.

- **Leaf split:** when appending at the end (sequential writes) the new item
  alone goes right and the left leaf stays full. Otherwise it splits into
  the most balanced halves. Without this, gradual filling would leave leaves
  half empty (measured: overhead 0.85 % instead of 0.43 %).
- **Removal:** empty nodes are removed, a root with one child is replaced by
  the child, adjacent underfull leaves (left ≤ 1/4 block, both ≤ 1/2) are
  merged.

## File names

- Stored as the user typed them (in Unicode NFC) - case is preserved.
- Compared **case-insensitively** system-wide: key = NFC(lowercase(NFC(name))).
  `Foto.JPG`, `foto.jpg` and `fóto` written decomposed (`o` + acute) are
  the same name. The conversion does not depend on language (no Turkish
  rules for `i`).
- A valid name: 1-255 bytes of UTF-8, not `.` or `..`, no `/` and no
  control characters. Otherwise `INVALID_ARGS`.
- Renaming only to a different case (`/photos` → `/PHOTOS`) is allowed.
- The Unicode tables version is in the superblock (`unicode_version`).

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

1. `write` puts data into a **buffer in memory** (at most 16 MiB, then it
   is written earlier). A partially overwritten block is read first.
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
[generation fallback](#generation-fallback).

## Generation fallback

Blocks freed by commit `g` are reused only in transaction
`g + REUSE_DELAY` (4). The last 4 saved states therefore stay untouched on
disk. The ring has 8 superblock slots.

At mount the newest valid superblock is tried; when its state cannot be
loaded (damaged tree root or space metadata), older ones are tried - at
most 3 generations back (older ones may already be overwritten).
`info().fallback` tells how many generations the system went back; the fsd
service reports it as a warning. After a fallback the intent log is not
replayed (it belongs to the newer state) and a commit with a new log epoch
follows immediately.

## fsync and the intent log

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

Data blocks referenced by a log batch are not reused until the commit, even
when they were created in the same transaction (otherwise a later `fsync`
would overwrite them). When the log grows over 2048 blocks, `fsync` does a
commit instead.

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

Part of the space (1/128 of the volume, 24 blocks up to 256 MiB) is
reserved for commits (metadata, logs). An ordinary write that would consume
it returns `NO_SPACE` **immediately** - not at commit time. If blocks are
waiting for a delayed free, the system first tries commits (up to 5) to
release them.

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

## Consistency check and scrub

`check(scrub)` first saves changes with a commit, then walks the live
tree, all snapshots, the free-space metadata and the intent log:

- verifies the checksums of all nodes (and with `scrub = true` of all data
  blocks - reads the whole content),
- every used block must be allocated according to the allocator,
- no allocated block may be "lost" (nothing points to it),
- the live tree must not use one block twice.

It returns a `CheckReport` with block counts (tree, data, metadata) and a
list of problems (empty = OK). The tests call it after every scenario.

## Performance and overhead

Measured by `tests/perf.rs` (in-memory device = the pure cost of the FS
without a disk, release, one thread):

| Measurement | Result |
|---|---|
| sequential write of 512 MiB in 1 MiB pieces + commit | ~650-720 MiB/s, **1 contiguous range** |
| sequential read | ~2.8-3.5 GiB/s |
| metadata overhead for data (tree, extents, checksums) | **0.43 %** of capacity |
| 20,000 files of 4 KiB (create + write + commit) | ~83,000 files/s |
| `stat` of a directory with 20,000 files (subtree size) | 13.5 µs |

Run: `cargo test -p fs --features std --release -- --ignored --nocapture perf`.

## The fsd service

`user/fsd` keeps one mounted volume and serves a client over a channel.

- **Start:** at boot, init reads blocks 1-8 of all disks and starts the
  service on the first one with the signature `OSFS\0\0v2`. Startup message:
  a channel to the disk driver and a channel for the client (both
  `kind USER`). The first message on the bootstrap channel is the
  configuration: space-separated words, `debug` enables test operations.
- **Device:** an FS block = 8 sectors; contiguous ranges go in one driver call.
- **Commit** every 5 s (`COMMIT_INTERVAL`); a client's `fsync` is durable
  immediately through the intent log.
- **When idle** after a commit it defragments in steps of 256 blocks.
- **TRIM** enabled.
- **End:** when the client closes the channel, the service commits and exits.

The service's messages are described in [messages.md](messages.md#fsd-service-and-file-system).

## The fsproto protocol

Messages over a channel (version 1, synchronous, path-based). Request: `op`
u32 and parameters in order; reply: `status` i32 (0 or error code),
reserved u32, result. Numbers little-endian, string = length u16 + UTF-8,
data = rest of the message. One read/write moves at most 32 KiB
(`MAX_IO`); `FsClient` splits larger requests.

| op | Name | Parameters | Result |
|---|---|---|---|
| 1 | `INFO` | - | volume label, block size, blocks, free, delayed, generation, algorithm, fallback, replayed |
| 2 | `STAT` | path | `Stat` (oid, kind, size, mtime, ctime, subtree summaries) |
| 3 | `LIST` | path | count u32 + (kind u8, oid u64, name) - sorted by name |
| 4 | `READ` | path, offset u64, length u32 | data (shorter at the end of the file) |
| 5 | `WRITE` | path, offset u64, data | - |
| 6 | `CREATE` | path, kind u8 (1 file, 2 directory) | - |
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
| 20 | `CHECK` | scrub u8 | count u32 + problem descriptions |
| 100 | `SIMULATE_POWER_LOSS` | - | replayed operations u64; only with `debug`, otherwise `ACCESS_DENIED` |

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
| `INVALID_ARGS` | relative path, invalid name, moving a directory into its own subtree, offset + length overflow, formatting a device smaller than 64 blocks |
| `NO_SPACE` | a write or create would consume the commit reserve |
| `CORRUPTED` | a block checksum does not match or a structure makes no sense; for data reads = a damaged block (other files remain readable) |
| `IO_ERROR` | a device error; after a failed commit all further changes (mount again) |
| `NOT_SUPPORTED` | unknown format version or algorithm; unknown protocol operation |
| `ACCESS_DENIED` | `SIMULATE_POWER_LOSS` without `debug` mode |

## Tests

| Test | What it verifies |
|---|---|
| `tests/basic.rs` | files, directories, names, summaries, errors, tree splits, space reuse, full disk, snapshots, corruption detection |
| `tests/features.rs` | contiguous writes, fsync + replay, generation fallback, snapshot deletion, defragmentation, TRIM, many AGs + BLAKE3, log condensing |
| `tests/model.rs` | property test: random sequences (create, write, truncate, remove, move, commit, fsync, "crash", snapshots) against an in-memory model; fsck at the end |
| `tests/crash.rs` | power loss during commit and fsync with a random subset of unwritten blocks |
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
