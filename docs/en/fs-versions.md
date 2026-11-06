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
| Intent log | records 1-6 | + 7 symlink, generation on create, 8 dataset marker | a symlink survives `fsync`; the generation stays the original after replay; one batch for several datasets |
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

### Remaining in v3

Datasets, versioning (clones, diff, restore), the handle layer and the
Windows driver, permissions, protocol v2 - order in the
[design](fs-v3-design.md#implementation-order).
