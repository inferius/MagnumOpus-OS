# File system data (phase 3)

Phase 3 of format v3 ("data", see the [FS v3 design](fs-v3-design.md#implementation-order))
adds: per-chunk **compression**, **instant copies** (shared blocks),
**deduplication** in the background and at write time, **automatic holes**,
**metadata 2x with self-healing**, optional **data 2x**, **background
scrub** and **preallocation**. The general format is described in
[file-system.md](file-system.md); the format changes are summarised in
[Format versions](fs-versions.md#phase-3-data-incompat_ptr_copy-incompat_data).

> **Change from v3 (phase 2):** a new chapter. The pointer has 48 B (the
> address of a second copy), an extent carries flags (unwritten, shared,
> copy) and compression, a deadlist entry the address of the copy; new
> tree items 24-29, the feature bits `INCOMPAT_PTR_COPY` and
> `INCOMPAT_DATA`, protocol operations 110-117.
> **Why:** resilience (copies, self-healing, scrub) and saving space
> (copies without copying, deduplication, holes) must hold for every write
> and survive a crash, so they belong in the FS; when to scrub and
> deduplicate is decided by the service.

| What | Layer | Code |
|---|---|---|
| writing in chunks, compression, holes, data copies, in-place writes | invariant (FS) | `libs/fs/src/fs/blocks.rs`, `libs/fs/src/compress.rs` |
| sharing table (reference counts) | invariant (FS) | `libs/fs/src/fs/sharing.rs` |
| instant copies | mechanism (FS) | `libs/fs/src/fs/reflink.rs` |
| deduplication (index, background step, at write) | mechanism (FS) | `libs/fs/src/fs/dedup.rs` |
| preallocation, dataset data settings | mechanism (FS) | `libs/fs/src/fs/dataopt.rs` |
| scrub | mechanism (FS) | `libs/fs/src/fs/scrub.rs` |
| second copy of nodes, reading with self-healing | invariant (FS) | `libs/fs/src/store.rs` |
| compression, copies, deduplication mode | setting (dataset data) | `datarec::DataSettings` |
| when to defragment, deduplicate, scrub | decision (service) | `user/fsd/src/main.rs` |

```rust
use fs::{Compress, DataSettings, DedupMode, Fs, Kind, MemDevice};

let mut fs = Fs::mount(Fs::format(MemDevice::new(65536), &Default::default())?)?;
// Compression is off by default: turn it on for a dataset.
fs.set_data_settings("/", &DataSettings { compress: Some(Compress::Lz4), copies: 1, dedup: DedupMode::Background })?;
fs.create("/big.log", Kind::File)?;
fs.write("/big.log", 0, &data)?;
// Instant copy: shares blocks, done in time proportional to the extent count.
fs.clone_path("/big.log", "/copy.log")?;
// Preallocation: 1 GiB of contiguous space, reads as zeros.
fs.create("/db", Kind::File)?;
fs.fallocate("/db", 0, 1 << 30, false)?;
// Scrub in steps (the service calls it between requests).
fs.scrub_start(now)?;
while !fs.scrub_step(256, now)? {}
```

Through the fsd service the same is done by `FsClient` (`fsproto::data`),
see [Protocol](#protocol-operations-110-117).

## Defaults and cost

Defaults follow the rule "what slows things down is an option":

| Feature | Default | Cost | Saving |
|---|---|---|---|
| compression (LZ4 / Zstd) | **off**, dataset option | LZ4 on one thread ~400-700 MB/s write, ~2-4 GB/s read; on NVMe it slows a long write up to ~6x, a random 4 KiB read unpacks up to 128 KiB | ~5-15 % on a disk with media, ~30-50 % system, code, databases |
| automatic holes (zero blocks) | **on** | comparing a block with zero when writing (negligible) | files with zeros (images, pre-sized files) |
| instant copies | always available | none (only when copying) | the whole copy |
| background deduplication | **on** | reading new data when idle; with XXH3 a byte comparison | duplicate files, backups, copies made without reflink |
| deduplication at write | **off**, dataset option | an index lookup for every 128 KiB chunk, extents of 128 KiB | as in the background, but at once |
| metadata 2x | **always** | 2x space for nodes (~0.4 % -> ~0.8 % of capacity), 2x node writes | repairing a damaged node without data loss |
| data 2x | **off**, dataset option | 2x space and writes for data | repairing a damaged data block (one disk) |
| scrub | **weekly** + on request | reading all used space when idle | finds and repairs hidden damage before it is needed |

Profiles ([design](fs-v3-design.md#defaults-and-profiles)) change only
deduplication at write (Free: option); integrity (checksums, metadata 2x)
does not depend on the profile.

## On disk

### 48 B pointer and the superblock

The pointer (`BlockPtr`) gains, at offset 40, the **address of a second
copy** (device 16 bits + block 48 bits, 0 = no copy); the checksum holds
for both copies. The superblock therefore moved (root 80-128, space index
128-176, further fields from 176, volume label 232-264). An internal-node
entry is 8 B longer (72 B), so an internal node holds at most 227 children
(previously 255).

Feature bits: `INCOMPAT_PTR_COPY` (bit 2) and `INCOMPAT_DATA` (bit 3) are
set by formatting. A volume **without** `INCOMPAT_PTR_COPY` has the old
pointer layout (40 B) - the driver rejects it (`NOT_SUPPORTED`) and the
volume must be reformatted (the FS is in development, no conversion is
done).

### Extent

```text
0   physical start u64        8   logical blocks u32
12  flags u32                 16  birth generation u64
24  [start of the second copy u64]          only with EXTENT_COPY
..  [physical blocks u32, data bytes u32]   only when compressed
..  checksums: uncompressed = one per block, compressed = one over the
    whole data on disk, unwritten (preallocated) = none
```

Flags: bit 0 `UNWRITTEN` (preallocated, reads as zeros), bit 1 `SHARED`
(the blocks may be in the sharing table), bit 2 `COPY`, bits 8-15 the
compression algorithm (1 = LZ4, 2 = Zstd). An unknown bit = `NOT_SUPPORTED`.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Extent forms">
<text class="t b" x="20" y="24">logical blocks of a file</text>
<rect class="box" x="20" y="36" width="200" height="40" rx="6"/><text class="t" x="120" y="54" text-anchor="middle">uncompressed</text><text class="m" x="120" y="69" text-anchor="middle">checksum per block</text>
<rect class="acc" x="230" y="36" width="160" height="40" rx="6"/><text class="t" x="310" y="54" text-anchor="middle">compressed chunk</text><text class="m" x="310" y="69" text-anchor="middle">32 blocks -> N blocks</text>
<rect class="ok" x="400" y="36" width="160" height="40" rx="6"/><text class="t" x="480" y="54" text-anchor="middle">unwritten</text><text class="m" x="480" y="69" text-anchor="middle">reads zeros, no sums</text>
<rect class="warn" x="570" y="36" width="170" height="40" rx="6"/><text class="t" x="655" y="54" text-anchor="middle">shared (SHARED)</text><text class="m" x="655" y="69" text-anchor="middle">another extent's blocks</text>
<text class="t b" x="20" y="130">physical blocks</text>
<rect class="box" x="20" y="142" width="200" height="34" rx="6"/>
<rect class="acc" x="230" y="142" width="70" height="34" rx="6"/>
<rect class="ok" x="400" y="142" width="160" height="34" rx="6"/>
<rect class="box" x="600" y="142" width="140" height="34" rx="6"/><text class="m" x="670" y="163" text-anchor="middle">table: 2 references</text>
<path class="ln" d="M120 80 L120 138" marker-end="url(#fe1)"/>
<path class="ln" d="M310 80 L265 138" marker-end="url(#fe1)"/>
<path class="ln" d="M480 80 L480 138" marker-end="url(#fe1)"/>
<path class="ln" d="M655 80 L670 138" marker-end="url(#fe1)"/>
<path class="ln" d="M150 80 C 300 110 560 110 640 138" marker-end="url(#fe1)"/>
<defs><marker id="fe1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="m" x="20" y="210">Every extent may also have a second copy (EXTENT_COPY) of the same physical length elsewhere on the disk.</text>
</svg>
<figcaption>Extent forms: a shared extent points to blocks counted by the sharing table.</figcaption>
</figure>

**The birth generation is logical:** for shared blocks (copies,
deduplication) it is the generation in which this reference was created.
The ownership rules (snapshot, deadlist, [freeing a
block](file-system.md#block-release-rules)) thus hold for each
reference separately; the sharing table guards the physical freeing.

### Deadlist and tree items

A deadlist entry has 32 B: start, length, generation, **start of the copy**
(0 = no copy) - the copy is freed together with the first.

| Item | Where | Offset | Value |
|---|---|---|---|
| 24 `DATAOPT` | every dataset | 0 | compression u8, copies u8, deduplication u8 (`DataSettings`) |
| 25 `SHARE` | root dataset | physical start | length u32, references u32, copy u64, flags (1 = indexed), [hash 16 B] |
| 26 `DEDUP` | root dataset | first 8 B of the hash | list (hash 16 B, start u64) |
| 27 `SCRUB` | root dataset | 0 | scrub state (`ScrubState`, 104 B) |
| 28 `SCRUBERR` | root dataset | sequence | text of an unrepairable error |
| 29 `DEDUPPOS` | every dataset | 0 | generation up to which the dataset is deduplicated |

## Compression

- **Chunk = 32 logical blocks (128 KiB)** aligned to the logical offset.
  A chunk is stored compressed only when it saves at least 1/8 (at least
  one block) - photos, video and archives stay uncompressed and reading
  does not pay for unpacking.
- **Checksum over the data on disk** (one per chunk): scrub and
  deduplication do not unpack.
- **LZ4** (block format, lz4_flex) and **Zstd** (frame, ruzstd, level
  ~zstd -1); both pure Rust without `std`. The algorithm number is in the
  extent, so a better encoder or level can be added without a format change.
- **A chunk is indivisible:** writing to any of its blocks takes the whole
  chunk into the write (unpacks it) and frees the old extent; truncating
  in the middle of a chunk rewrites the kept blocks.
- **Reading:** the chunk is read whole, verified, unpacked and kept in a
  small cache (8 chunks), so sequential 4 KiB reads do not unpack it again.
- The setting applies to **newly written** data; old data stays as it is.
  A new dataset inherits the parent's setting.

> **Risk (consciously accepted):** compression runs on one thread inside
> the commit. On fast NVMe (3-7 GB/s) it therefore slows a long contiguous
> write several times, and a random 4 KiB overwrite rewrites a whole
> 128 KiB chunk - that is why it is off by default. Parallel compression is
> in the [Performance](fs-v3-design.md#performance) chapter.

## Automatic holes

A block of all zeros is not stored when written: the file gets a hole
(reads as zeros) and the earlier data at that place is freed. With
compression, zeros inside a chunk are compressed with it and a hole is
made only from a whole zero chunk. Zeros are not written into preallocated
space (it already reads as zeros).

## Preallocation

`fallocate(path, offset, length, keep_size)` fills the holes in the range
with **unwritten extents** (contiguous blocks, read as zeros, without
checksums). Blocks that already hold data stay. With data copies the
second copy is allocated as well.

- **In-place write:** the first write into an unwritten extent goes
  directly into its blocks (no new allocation, the file stays contiguous)
  if no snapshot and no sharing hold the extent. The extent is then split
  into unwritten - written - unwritten; the birth generation stays.
- **Crash before the commit:** the saved state still sees the extent as
  unwritten, so it reads zeros - never someone else's old blocks.
- **fsync:** the intent log records extents written in place too; replay
  recognises the same blocks under an unwritten extent and only marks them
  as written (no new allocation).
- An extent held by a snapshot is written the classic copy-on-write way
  (the snapshot keeps reading zeros).
- Neither compression nor deduplication is used for preallocated files.

## Instant copies (reflink)

`clone_path(src, dst)` copies a file, a link or a whole folder,
`clone_range(src, offset, dst, offset, length)` a range (offsets and
length in 4 KiB units, the length may end at the end of the source). The
copy points to the same blocks:

- the copy's extent gets `SHARED` and the generation of this transaction,
  the sharing table adds a reference,
- an unwritten extent = a hole in the copy; a compressed chunk is shared
  only whole, a part of a chunk is copied,
- the source is **committed** before the copy: blocks from the unfinished
  transaction could otherwise be freed immediately even though the copy
  already points to them,
- works across the whole volume, including between datasets; datasets
  inside a copied folder are skipped (they have their own history); copying
  a folder into itself = `INVALID_ARGS`,
- **restoring from a snapshot** (`restore_copy`, file versions, merge) now
  shares blocks as well - it is instant and takes no extra space.

Rights: read the source (for a folder list it and read every file),
create or write the destination.

## Sharing table

A record (`SHARE`) covers a contiguous physical range with a reference
count. A range without a record has a single owner (the original rules).
Every reference (an extent or a deadlist entry) is **finally** freed once
(deferred or from a deadlist); such a freeing of a data range goes into the
`unref` queue, which the commit processes in the root dataset:

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Freeing a shared range">
<defs><marker id="fe2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="50" width="150" height="50" rx="6"/><text class="t" x="95" y="72" text-anchor="middle">reference freed</text><text class="m" x="95" y="88" text-anchor="middle">extent / deadlist</text>
<rect class="acc" x="210" y="50" width="140" height="50" rx="6"/><text class="t" x="280" y="72" text-anchor="middle">unref queue</text><text class="m" x="280" y="88" text-anchor="middle">before the commit</text>
<rect class="box" x="390" y="20" width="160" height="44" rx="6"/><text class="t" x="470" y="40" text-anchor="middle">in the table</text><text class="m" x="470" y="56" text-anchor="middle">references - 1</text>
<rect class="box" x="390" y="90" width="160" height="44" rx="6"/><text class="t" x="470" y="110" text-anchor="middle">not in the table</text><text class="m" x="470" y="126" text-anchor="middle">single owner</text>
<rect class="ok" x="590" y="50" width="150" height="50" rx="6"/><text class="t" x="665" y="72" text-anchor="middle">free the range</text><text class="m" x="665" y="88" text-anchor="middle">with its copy, deferred</text>
<path class="ln" d="M170 75 L205 75" marker-end="url(#fe2)"/>
<path class="ln" d="M350 70 L385 45" marker-end="url(#fe2)"/>
<path class="ln" d="M350 82 L385 108" marker-end="url(#fe2)"/>
<path class="ln" d="M550 42 L585 65" marker-end="url(#fe2)"/>
<text class="m" x="560" y="30">at 0</text>
<path class="ln" d="M550 112 L585 88" marker-end="url(#fe2)"/>
<text class="m" x="20" y="160">While the table has no record, the queue is bypassed (no overhead for volumes without sharing).</text>
</svg>
<figcaption>Freeing a reference to a data range: the table lowers the count, the last reference frees the range.</figcaption>
</figure>

- Records do not overlap and are not longer than an extent (496 blocks). A
  partial decrement splits a record; an indexed record (deduplication)
  loses its index entry when split (the chunk is no longer whole).
- A shared extent is **never freed immediately** (even if it was created in
  this transaction) - the physical blocks may belong to a saved state.
- Everything changes in one transaction with the commit of all datasets,
  so a crash never leaves the reference counts and the extents out of sync.

## Deduplication

Chunk = 32 aligned logical blocks of one extent (physically contiguous),
fingerprint:

- uncompressed: a checksum (the volume algorithm) of the prefix "P" and
  the checksums of all 32 blocks - nothing is read, the checksums are
  already in the extent,
- a compressed extent: a checksum of the prefix "C", the algorithm, the
  lengths and the checksum of the data on disk (the same input compresses
  the same way).

The index (`DEDUP`) maps a fingerprint -> a sharing-table record. On a
match, with **XXH3** the data is compared byte by byte (protection against
a deliberate collision), with **BLAKE3** not. The file's extent then points
to the found chunk (`SHARED`) and its own blocks are freed.

- **In the background** (`dedup_step`, the default): a pass over the live
  trees only in subtrees born after the last finished pass (`DEDUPPOS`) -
  the cost is proportional to new data. Files with unwritten buffered data
  are skipped (they come next time). The service calls it when idle, after
  defragmentation.
- **At write** (`DedupMode::Inline`): the chunk is looked up before
  allocation; a new chunk is indexed at once and its extent gets `SHARED`.
  An `fsync` with a shared extent from this transaction does a commit (the
  intent log does not carry reference counts).
- Memory: the index is on disk, only the tree node cache is in memory.

> **Risk:** only live data is deduplicated; blocks held only by a snapshot
> stay. Deleting a file whose blocks another file shares frees no space -
> free space is thus harder to estimate (`free_blocks` is the truth, file
> sizes are not).

## Metadata 2x and self-healing

Every tree node is written **twice**: the second copy is allocated from the
end of the disk downwards (far from the first, on a disk with several
allocation groups in another group). On a fragmented disk without a
contiguous run for the copy the node is written only once (better without a
copy than a failed commit) - fsck reports it as `nodes_without_copy` (not
an error).

<figure class="diagram">
<svg viewBox="0 0 760 160" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Reading with self-healing">
<defs><marker id="fe3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="50" width="130" height="44" rx="6"/><text class="t" x="85" y="77" text-anchor="middle">read copy 1</text>
<rect class="ok" x="200" y="10" width="130" height="40" rx="6"/><text class="t" x="265" y="35" text-anchor="middle">checksum ok</text>
<rect class="box" x="200" y="95" width="130" height="44" rx="6"/><text class="t" x="265" y="114" text-anchor="middle">read copy 2</text><text class="m" x="265" y="130" text-anchor="middle">bad / I/O error</text>
<rect class="acc" x="390" y="95" width="160" height="44" rx="6"/><text class="t" x="470" y="114" text-anchor="middle">ok: rewrite copy 1</text><text class="m" x="470" y="130" text-anchor="middle">self-healing</text>
<rect class="warn" x="590" y="95" width="150" height="44" rx="6"/><text class="t" x="665" y="114" text-anchor="middle">copy 2 bad too</text><text class="m" x="665" y="130" text-anchor="middle">CORRUPTED</text>
<path class="ln" d="M150 65 L195 35" marker-end="url(#fe3)"/>
<path class="ln" d="M150 82 L195 112" marker-end="url(#fe3)"/>
<path class="ln" d="M330 117 L385 117" marker-end="url(#fe3)"/>
<path class="ln" d="M550 117 L585 117" marker-end="url(#fe3)"/>
</svg>
<figcaption>Reading a node or data with a copy: a bad first copy is repaired from the second.</figcaption>
</figure>

- The checksum is in the pointer and holds for both copies, so rewriting a
  bad copy in place breaks nothing (the content is given by the checksum).
- During mounting (before it is clear whether the volume is writable)
  repairs are only noted and written once mounting decides; a read-only
  mount writes nothing.
- A damaged newest root thus heals without falling back a generation; the
  fallback ([Mounting](file-system.md#mount-and-generation-fallback))
  happens only when both copies are damaged.

**Data 2x** (`copies: 2` in the dataset settings): an extent gets a second
copy of the same length (from the end of the disk). Reading and scrub
repair a bad block from the copy, just as for nodes.

> **Risk:** both copies are on **one disk** - they protect against bad
> sectors and silent corruption, not against the failure of the whole disk
> (that is what a mirror is for; the format is ready: the device number in
> the pointer). **Space logs** (free-space metadata) are not 2x yet: a
> damaged space log is detected by its checksum and mounting falls back a
> generation; when even that fails, the volume cannot be mounted.
> Rebuilding free space by walking the trees is a follow-up.

## Scrub

`scrub_start` / `scrub_stop` / `scrub_status` / `scrub_step`. The pass:
datasets by number, in each the snapshots from the oldest and then the
saved live version. For each tree, subtrees and extents born up to the
generation of the previous snapshot are skipped - the pass over the
previous tree already verified them (a clone also skips everything up to
its origin generation). The cost is proportional to used space, not to the
number of versions.

<figure class="diagram">
<svg viewBox="0 0 760 150" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Scrub pass">
<defs><marker id="fe4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="40" width="150" height="50" rx="6"/><text class="t" x="95" y="62" text-anchor="middle">snapshot S1</text><text class="m" x="95" y="78" text-anchor="middle">everything</text>
<rect class="box" x="210" y="40" width="150" height="50" rx="6"/><text class="t" x="285" y="62" text-anchor="middle">snapshot S2</text><text class="m" x="285" y="78" text-anchor="middle">only birth &gt; S1</text>
<rect class="box" x="400" y="40" width="150" height="50" rx="6"/><text class="t" x="475" y="62" text-anchor="middle">live version</text><text class="m" x="475" y="78" text-anchor="middle">only birth &gt; S2</text>
<rect class="acc" x="590" y="40" width="150" height="50" rx="6"/><text class="t" x="665" y="62" text-anchor="middle">next dataset</text><text class="m" x="665" y="78" text-anchor="middle">by number</text>
<path class="ln" d="M170 65 L205 65" marker-end="url(#fe4)"/>
<path class="ln" d="M360 65 L395 65" marker-end="url(#fe4)"/>
<path class="ln" d="M550 65 L585 65" marker-end="url(#fe4)"/>
<text class="m" x="20" y="125">Both copies are read for every node and extent; the position (dataset, snapshot, key) is in the tree and survives a restart.</text>
</svg>
<figcaption>Scrub verifies every block once, even when several versions share it.</figcaption>
</figure>

- For every node and extent **both** copies are read: a bad one is
  overwritten with the good one. When there is no good copy, an error with
  the file path is recorded (`SCRUBERR`, at most 64 records per run).
  Unwritten extents are not read.
- A step verifies the **last saved state** (roots by the superblock) and
  commits nothing; the position is saved with the next commit. A tree
  change between steps does no harm - a step starts over the current saved
  state from the saved key.
- The fsd service starts a scrub once a week (from the end of the last
  finished run) and takes 1 MiB steps between client requests (requests
  have priority).

## Consistency check

fsck (`check`) additionally verifies:

- node copies lie inside the volume and nothing else uses their blocks,
- extents including copies and compressed lengths lie inside the volume,
- a block used more than once in the live state must be in the sharing
  table, a shared extent of the live version must have a record,
- table records do not overlap and nobody uses them "extra" (a record
  without used blocks = a lost reference),
- the deduplication index matches the indexed table records (and vice
  versa),
- with `scrub = true` it reads the data of all extents (unpacking
  compressed ones).

## Protocol (operations 110-117)

`fsproto::data` (87-92 belong to nested datasets, 100 is `SIMULATE_POWER_LOSS`):

| Op | Name | Request | Reply | Rights |
|---|---|---|---|---|
| 110 | `CLONE` | source, destination | shared bytes u64 | read the source, create the destination |
| 111 | `CLONE_RANGE` | source, offset, destination, offset, length | - | read the source, write the destination |
| 112 | `FALLOCATE` | path, offset, length, keep size u8 | - | write |
| 113 | `DATA_SETTINGS` | dataset | compression, copies, deduplication | read |
| 114 | `SET_DATA_SETTINGS` | dataset, settings | - | `MANAGE` |
| 115 | `SCRUB_START` | - | - | `MANAGE` on the volume root |
| 116 | `SCRUB_STOP` | - | - | `MANAGE` on the volume root |
| 117 | `SCRUB_STATUS` | - | state, times, counts, errors | anyone (disk state, not content) |

## Risks and follow-ups

- compression on one thread (see above) - a parallel commit is planned,
- space logs without a second copy; rebuilding free space by walking the
  trees,
- both copies on one disk (a mirror only with several disks),
- background deduplication does not walk data held only by snapshots,
- deduplication at write splits uncompressed extents into 128 KiB (more
  extent items, not fragmentation on disk),
- an instant copy commits the source (a short pause with a large
  unfinished change).
