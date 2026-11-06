# Design: file system v3

**Status: proposal for approval.** Describes what is added and changed
compared to the current format v2 ([file-system.md](file-system.md)). After
approval it is implemented in phases (see [Implementation order](#implementation-order))
and finished parts move into the main FS chapter.

Principles (agreed):

- **Do things right, not the Windows or Linux way.** Translation for other
  systems is handled by drivers (Linux via FUSE, Windows via WinFsp on top
  of `libs/fs`).
- **Completely simple by default** for the user; stricter or looser
  behaviour can be chosen in the system settings (profiles).
- **The format may change for free now**, so it immediately reserves room
  for everything below, including things implemented later (feature flags).

## Summary of changes

| Area | v2 | v3 |
|---|---|---|
| Tree node | 4 KiB | **16 KiB** (longer names, shallower tree, more extents per leaf) |
| Versioning unit | the whole volume | **dataset (subvolume)** = repository, vault, system |
| Versions | read-only snapshots | **commits, branches, clones, diff, merge, change journal, git layer** |
| Names | 255 bytes | **1024 characters** (max. 4 KiB UTF-8), bidi control characters forbidden too |
| Object record | 64 B | **128 B** (version, flags, creation time, generation, owner, permissions) |
| Object types | file, directory | + **symbolic link**, dataset link |
| Data | extents | + **compression**, **shared blocks** (instant copies, deduplication), automatic holes |
| Resilience | Merkle tree, generation fallback | + **metadata twice** and self-healing, background scrub, format ready for multiple disks |
| Permissions | none | **capabilities + grants**, visibility, profiles |
| Encryption | none | **encrypted vaults** (datasets) |
| Snapshots | manual | + **automatic**, **trash**, **immutable** with a configurable lock |
| Search | - | **indexes**: names, tags, metadata, content hash; full text as a service |
| Other | - | quotas, transactions for applications, send/receive, verified system, online resize |
| Protocol | v1: paths, one client | **v2: handles with rights, multiple clients, paging** |

## On-disk format

- **Block 4 KiB** (data, space metadata, log), **tree node 16 KiB** (4
  contiguous blocks). An item value up to ~8 KiB.
- **Superblock v3** adds: feature flags in three groups (like ext4:
  *compat* - an older driver may mount; *ro_compat* - read-only; *incompat*
  - must not), volume UUID, a pointer to the dataset tree, the list of the
  volume's devices, settings (compression, deduplication), a root signature
  (verified system).
- **Pointer v3** (40 B): device (16 bits) + block (48 bits), generation of
  creation, 128-bit checksum, length in blocks, flags (compressed,
  encrypted, shared). The device number is ready for multiple disks.
- **Object record (128 B):** kind, flags, size, times (content change,
  metadata change, creation, optionally access), parent, generation (for
  handles), subtree summaries, permission descriptor number, owner, number
  of extended attribute references, reserve.

## Datasets (subvolumes)

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Datasets">
<defs><marker id="ds1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="80" width="130" height="44" rx="6"/><text class="t b" x="75" y="107" text-anchor="middle">superblock</text>
<path class="ln" d="M140 102 L188 102" marker-end="url(#ds1)"/>
<rect class="box" x="190" y="70" width="170" height="64" rx="6"/><text class="t b" x="275" y="95" text-anchor="middle">dataset tree</text><text class="m" x="275" y="114" text-anchor="middle">records, commits, branches</text>
<path class="ln" d="M360 90 L428 40" marker-end="url(#ds1)"/><path class="ln" d="M360 102 L428 102" marker-end="url(#ds1)"/><path class="ln" d="M360 114 L428 164" marker-end="url(#ds1)"/>
<rect class="ok" x="430" y="15" width="320" height="46" rx="6"/><text class="t" x="590" y="36" text-anchor="middle">system (read-only, A/B, signed root)</text><text class="m" x="590" y="52" text-anchor="middle">its own metadata tree</text>
<rect class="box" x="430" y="79" width="320" height="46" rx="6"/><text class="t" x="590" y="100" text-anchor="middle">user home, projects (repositories)</text><text class="m" x="590" y="116" text-anchor="middle">own tree, history, branches</text>
<rect class="warn" x="430" y="143" width="320" height="46" rx="6"/><text class="t" x="590" y="164" text-anchor="middle">vault (encrypted, locked)</text><text class="m" x="590" y="180" text-anchor="middle">key only in memory after unlocking</text>
</svg>
<figcaption>Each dataset has its own tree; a snapshot, branch or clone is just another root.</figcaption>
</figure>

- **Dataset tree** (like the btrfs root tree): for each dataset a record
  with the root of its tree, its origin (which snapshot it was cloned
  from), the generation of its last snapshot, its own deadlist, settings
  (compression, quota, automatic snapshots, encryption) and a name.
- In the directory tree a dataset is an **entry of kind "dataset"**: a path
  simply crosses into it. For the user it is an ordinary folder (with a
  repository or lock icon).
- **Snapshot, clone, branch:** a new record pointing to an existing root -
  instant, without copying. Each dataset has its own block ownership
  boundary and its own deadlists (the ZFS dataset model), so writable
  clones and independent histories work.
- **Converting a directory into a dataset** ("make this folder a
  repository/vault"): tree items move, data stays in place. For a vault the
  data is re-encrypted during conversion (in the background, with progress).

## Native versioning

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Commit history">
<defs><marker id="vc1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<circle class="acc" cx="60" cy="70" r="16"/><text class="t" x="60" y="75" text-anchor="middle">A</text>
<circle class="acc" cx="180" cy="70" r="16"/><text class="t" x="180" y="75" text-anchor="middle">B</text>
<circle class="acc" cx="300" cy="40" r="16"/><text class="t" x="300" y="45" text-anchor="middle">C</text>
<circle class="acc" cx="300" cy="110" r="16"/><text class="t" x="300" y="115" text-anchor="middle">D</text>
<circle class="ok" cx="440" cy="70" r="16"/><text class="t" x="440" y="75" text-anchor="middle">M</text>
<path class="ln" d="M164 70 L78 70" marker-end="url(#vc1)"/><path class="ln" d="M285 46 L196 66" marker-end="url(#vc1)"/><path class="ln" d="M285 104 L196 74" marker-end="url(#vc1)"/>
<path class="ln" d="M424 64 L316 44" marker-end="url(#vc1)"/><path class="ln" d="M424 76 L316 106" marker-end="url(#vc1)"/>
<rect class="box" x="520" y="20" width="230" height="40" rx="6"/><text class="t" x="635" y="45" text-anchor="middle">branch main → M</text>
<rect class="warn" x="520" y="80" width="230" height="40" rx="6"/><text class="t" x="635" y="105" text-anchor="middle">working state = clone of M</text>
<text class="m" x="10" y="155">Commit = immutable root + author, message, time, parents; ID = hash. Arrows point to parents.</text>
</svg>
<figcaption>History is a DAG of commits like in git; the working state is a writable clone.</figcaption>
</figure>

- **Commit** = a dataset snapshot + author, message, time, parents. The
  commit ID = hash (BLAKE3 or XXH3 depending on the volume) of the root
  checksum, parents and metadata - a content-addressed DAG.
- **Branches and tags** = named references to a commit. **Working state** =
  a writable clone of the last commit; `checkout` is instant.
- **Diff of two versions:** a parallel walk of both trees; a physically
  identical node (the same block) is skipped entirely. The cost matches the
  change, not the size. Result: a list of changed, new, deleted and moved
  objects (the oid is stable, so a move is detected exactly, not
  heuristically as in git).
- **Merge:** three-way at the file and directory level (common ancestor
  from the DAG); text files line by line, conflicts as marked files.
- **Change journal** of a dataset: an ordered stream (object, operation,
  time, commit) with optional retention - for watching changes, indexing,
  synchronization and `status` without walking the disk.
- **Git layer** (a service on top of the FS): (1) git object IDs (SHA-1 and
  SHA-256) are computed lazily and stored as an extended attribute;
  repository import and export; (2) push/pull over the network once there
  is TCP/IP. For tools, an object storage adapter (e.g. a libgit2 backend).

## Names, types, sizes

- A name up to **1024 Unicode characters** (max. 4 KiB UTF-8), NFC,
  case-insensitive comparison (unchanged from v2).
- Forbidden: `/`, control characters, `.` and `..`, **bidi control
  characters** (U+202A-202E, U+2066-2069; otherwise `invoice[U+202E]fdp.exe`
  can be displayed as `invoiceexe.pdf`).
- **Symbolic links** (target as text, max. 4 KiB) - git repositories need them.
- A file at most **2^63−1 B**; all arithmetic checked, directory summaries
  saturating (sparse files can have a huge logical size).

## Data: compression, shared blocks, holes

- **Compression per extent:** LZ4 (fast, default) or Zstd (better ratio),
  set per dataset. An extent that does not shrink by at least 1/8 stays
  uncompressed (photos, video). The checksum covers the on-disk data, so a
  scrub does not need to decompress.
- **Instant copies (reflink):** a copy points to the same blocks. Shared
  blocks have a counter in a **sharing table** (only shared blocks, like
  the ZFS BRT). A write to the copy goes to a new block as always; the old
  block's counter just decreases. A block is freed at zero. All within one
  atomic commit.
- **Deduplication:** an index "content hash -> block". With XXH3 the blocks
  are compared byte by byte on a match (protection against a deliberate
  collision), with BLAKE3 not. Default **in the background** (does not block
  writes, little memory); inline on write as a dataset option. In vaults
  only within the vault (otherwise content could be guessed).
- **Automatic holes:** all-zero blocks are not stored.

## Resilience

- **Metadata twice** at different places on the disk (default; data
  optionally). On a checksum error the FS reads the copy and rewrites the
  damaged place (self-healing).
- **Background scrub** with low I/O priority; the result in the disk health
  overview.
- **Multiple disks:** the format (device number in pointers, device list
  in the superblock) is ready for a mirror and later parity RAID.
- **Fixes from the review:** generation fallback only on `CORRUPTED` and
  invalidation of newer slots; an unreadable superblock slot = an invalid
  slot; a writable volume with a durability guarantee only on a device with
  a real flush (otherwise read-only); validation of all on-disk values
  (ranges, tree depth, parent cycles, lengths); an operation that fails
  midway is rolled back entirely.
- **fsync and commit atomic:** a log batch is removed only after the write
  is confirmed; after an uncertain write the instance confirms nothing more
  until recovery runs.

## Snapshots, trash, ransomware protection

- **Automatic snapshots** per the dataset's schedule (e.g. hourly for
  24 h, daily for 30 days, weekly for 6 months) - default for the user's home.
- **Trash:** deleting copies nothing, the object moves into the dataset's
  trash; emptied by age or when space runs low.
- **File version history** and **undo of operations** (delete, move,
  rename) from automatic snapshots and the change journal.
- **Immutable snapshots** - cannot be deleted even by an administrator from
  the running system. Modes (configurable, combinable):

| Mode | Behaviour |
|---|---|
| time lock | cannot be deleted before N days |
| space limit | at most X % of the disk is protected; above it the system warns and the oldest protected snapshots can be deleted only with confirmation (below) |
| count | the last N snapshots are protected |
| confirmation at boot | early deletion happens only after a reboot and confirmation in the boot loader (out of malware's reach) |
| security key | early deletion after touching a FIDO2 key |

  Default: a 7-day time lock + a 15 % disk limit + confirmation at boot for
  early deletion. Space is therefore never blocked forever.

## Permissions and visibility

- **Capabilities as the base:** a process accesses only what it was given
  (a directory or file handle with rights). Rights are checked once at
  open, the handle carries them; later operations check nothing.
- **Grants for sharing between users:** an owner + a list "user or group
  -> level", inheritance into subdirectories, **allow only** (no denies).
  Permission descriptors are stored once and objects reference them by
  number (thousands of files with the same rights cost nothing extra).
- **Rights:** *List* (names in a directory), *Traverse* (to an entry with a
  known name), *Read*, *Write*, *Create*, *Delete*, *Execute*, *Manage*
  (change permissions). User-facing levels: **View**, **Edit**, **Run**,
  **Manage**.
- **Visibility:** an entry shows up in a listing only when the user has at
  least some right to it; other users' files are not visible even by name
  and do not count into the user's size summaries.
- **Applications:** their own directory + files the user picks in a
  dialog (powerbox) - without permission prompts.
- **Profiles in the system settings:**

| Profile | For whom | Behaviour |
|---|---|---|
| **Standard** (default) | everyday use | own files without restrictions or prompts; applications see the user's documents, not system or other users' files |
| **Strict** | sensitive data, companies | applications sandboxed (picker dialog only), vaults lock on sleep, access audit |
| **Relaxed** | developers | applications see the whole user home, fewer confirmations |

## Encrypted vaults

- A vault = **a dataset with encryption**: data, names, metadata and its
  snapshots (XChaCha20-Poly1305 or AES-256-GCM depending on the CPU; the
  key from a password via Argon2id, or from biometrics/a key through a
  system service).
- Checksums in pointers cover the encrypted data + the cipher's
  authentication tag, so scrub and self-healing work without the key.
- **A locked vault** shows up as a folder with a lock; neither the content
  nor the number of files is visible. The key is only in memory after
  unlocking.
- **Instant secure deletion:** deleting the key = the data is irreversibly
  unreadable.
- Order: compression before encryption; deduplication only within the vault.

## Indexes, tags, search

- **Indexes in the tree in one place**, updated in the same commit as the
  data (never stale): name -> objects, **tag -> objects**, kind/time/size ->
  objects, **content hash -> objects** (also the deduplication table:
  "which files have the same content" instantly).
- **Tags and custom metadata** (colour, rating, note) as extended attributes.
- **Full text in content** as a service on top of the change journal (not
  in the FS core).

## Other features

- **Quotas** per dataset and user (space and number of objects).
- **Transactions for applications:** several writes and renames atomically
  (all or nothing).
- **Send/receive:** the difference of two snapshots as a data stream -
  backups and synchronization between one's own devices without walking files.
- **Verified system:** the root of the system dataset is signed; the boot
  loader verifies it at startup (like dm-verity) - ties in with A/B updates.
- **Online volume resize** (grow and shrink).
- **Optional audit** of access to sensitive datasets.
- **Space overview** including how much deleting each snapshot frees.

## Performance

- **Adaptive cache (ARC)** for nodes and data instead of "clear everything
  when full".
- **Prefetching:** sequential reads are detected and read ahead; at system
  startup what is read gets recorded and prefetched next time.
- **Parallel commit:** checksums, compression and encryption on multiple cores.
- **Direct I/O** for databases and huge files (no cache).
- **I/O priorities:** reads for the UI overtake background work (scrub,
  deduplication, defragmentation).

## Protocol fsproto v2

- **Handles instead of paths:** `OPEN(directory, name, rights) -> handle`;
  the handle carries the oid + generation and rights, so an open file
  survives a rename and operations do not re-evaluate the path. A directory
  is a capability: whoever holds its handle may only go below it.
- **Replace by rename** (atomic save), `O_EXCL` creation.
- **Multiple clients:** each client has its own channel (`OPEN_SESSION`),
  request IDs, paged directory listing (cursor), timeouts; the service never
  dies because of one client.
- Later: shared memory and request queues (asynchronous I/O).

## Implementation order

| Phase | Contents |
|---|---|
| 1 - format | 16 KiB nodes, superblock/pointer/record v3, feature flags, datasets and the dataset tree, names and symlinks, review fixes, protocol v2 with handles, permissions (capabilities, grants, visibility, profiles) |
| 2 - versions | commits, branches, clones, diff, merge, change journal, automatic snapshots, trash, version history, undo |
| 3 - data | compression, instant copies, deduplication, automatic holes, metadata twice and self-healing, background scrub |
| 4 - protection | encrypted vaults, immutable snapshots, quotas, audit, verified system |
| 5 - other | indexes and tags, send/receive, transactions, ARC and prefetching, resize, git layer |

Each phase ends with tests (unit, model, power loss, in the system),
documentation in cs and en, and a review.

## Open questions

- Cloud backups (encrypted, via send/receive) - once there is a network.
- Multiple disks (mirror, RAID): the format is ready, implementation after phase 5.
