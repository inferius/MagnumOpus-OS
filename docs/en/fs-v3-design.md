# Design: file system v3

**Status: agreed, introduced in phases** (done changes: [Format versions](fs-versions.md#transition-v2---v3)). Describes what is added and changed
compared to format v2 ([v2 archive](fs-v2.md)). It is implemented in phases
(see [Implementation order](#implementation-order)); the finished parts are
described in the current reference [file-system.md](file-system.md) (which
distinguishes what the format contains and what the driver already uses).

Principles (agreed):

- **Do things right, not the Windows or Linux way.** Translation for other
  systems is handled by drivers (Linux via FUSE, Windows via WinFsp on top
  of `libs/fs`).
- **Completely simple by default** for the user; stricter or looser
  behaviour can be chosen in the system settings (profiles).
- **The format may change for free now**, so it immediately reserves room
  for everything below, including things implemented later (feature flags).
- **What belongs to the FS and what to services** (three layers):

| Layer | Criterion | Examples |
|---|---|---|
| **invariant** - FS code | must hold for every write, survive a crash or be stored atomically with the file | block release, snapshots and clones, stable identity, snapshot holds, quotas, permissions, encryption, the journal with sequence numbers, decoys and freezing an application's writes |
| **settings and state** - stored in the FS, but as generic data | must survive a crash or travel with the file/dataset; the FS stores it atomically but does not interpret it | "keep history 30 days" as a dataset property, "always offline" as a file attribute, sync state, file origin |
| **decisions** - services on top of the FS | when and how to act according to the settings | when to run history cleanup, when to empty the trash, content-based ransomware detection, sync, indexes, audit (storing and display) |

  The benefit: tight integration stays (settings are stored atomically with
  the file), yet service behaviour can change with experience without a
  format change. So there are no bits on disk tied to one policy - only
  generic properties and attributes (namespaces `sys.*`, `sync.*`, ...).

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
| Applications | - | **file and range locks, preallocation, lossless change watching, memory mapping**, per-dataset case sensitivity, disk health |
| Security | - | **audit with a hash chain, file origin, ransomware detection, decoys**, file expiry, true times, no execution on data, per-application quotas |
| Cloud | - | **encrypted backup, files on demand, sync between devices, sharing** (format reserved, see [Cloud and network](#cloud-and-network)) |
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
- **Who keeps blocks alive (clones).** A dataset's own deadlist alone does
  not decide when a block may be returned to the allocator: two writable
  clones can come from snapshot S, a file is overwritten in the first one,
  S is deleted - and the second clone must still read the original data.
  Rules (first version, the ZFS model):
  - the rule is about **reachability**: a clone takes over the blocks
    **reachable from the root of S** and the `origin` link (explicit, in the
    clone's record) keeps that root alive. An older block need not belong
    to S (it may belong to another dataset or no longer be in S's root at
    all) - "born up to the generation of S" would be too broad,
  - the birth generation then only **helps within this link**: a block born
    after the clone was created is owned by the clone (its own overwrite
    frees it); a block reachable from S that the clone overwrites goes onto
    the clone's deadlist, but only processing S's deadlist may free it,
  - **deleting S while a clone depends on it is deferred:** S disappears
    from the user's view ("hidden") but keeps its blocks until the last
    clone is gone; then it is deleted normally (deadlists in commits, so
    atomically even on power loss),
  - space accounting: a clone = its own blocks, shared ones count to S; the
    space overview shows how much deleting the clone and S frees.
  - Tests: the scenario above, the same with power loss during deletion,
    and fsck checking space accounting too (own + shared = used).
  - More general per-block counters (the sharing table) only for instant
    file copies - for whole trees they would be needlessly expensive.
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

- **Commit** = a dataset snapshot + author, message, time, parents.
- **Two separate identities:**
  - the **physical root checksum** (the checksum in the pointer): local
    integrity checks and quickly skipping shared nodes in diff. Not
    portable - nodes contain block numbers, so the same logical tree has a
    different checksum on another volume;
  - the **logical commit ID**: BLAKE3 of a versioned unambiguous
    representation of the logical tree (names, kinds, content, metadata) +
    parents and commit metadata - a content-addressed DAG independent of
    block placement. Used for send/receive (resuming, deduplication), sync
    (common ancestor) and the git layer.
  - **Cost:** ordinary commits (every few seconds) do not compute the
    logical ID; only versioned commits do. It is incremental: every
    directory keeps its logical hash, a change recomputes only the path to
    the root and the content of changed files (BLAKE3 ~1-2 GB/s per core;
    free with BLAKE3 deduplication - an XXH3 checksum is not enough for
    identity, a collision can be crafted).
  - **Acceptance test:** export and import onto a volume with a different
    block placement keeps the commit ID.
- **Branches and tags** = named references to a commit. **Working state** =
  a writable clone of the last commit; `checkout` is instant.
- **Diff of two versions:** a parallel walk of both trees; a physically
  identical node (the same block) is skipped entirely. The cost matches the
  change, not the size. Result: a list of changed, new, deleted and moved
  objects (the oid is stable, so a move is detected exactly, not
  heuristically as in git).
- **Merge:** three-way at the file and directory level (common ancestor
  from the DAG); text files line by line, conflicts as marked files.
- **Change journal** of a dataset: an ordered stream (sequence number,
  object, operation, application, time, commit) - for watching changes,
  indexing, synchronization and `status` without walking the disk.
  - A **persistent sequence number** within the dataset (u64, never
    repeats).
  - A record is **published only after the change is confirmed** (commit or
    fsync) - a subscriber never sees a change that disappears after power
    loss.
  - **Retention** (days or space): older records are dropped. A
    subscription is "lossless" only within retention: whoever asks for a
    number older than the oldest stored gets **`JOURNAL_GAP`** - never a
    silent skip.
  - **A new baseline together with its position:** `VIEW(dataset)` returns
    a consistent read-only view (a temporary snapshot) **and the sequence
    number of the last record it contains**. The client (e.g. an indexer)
    walks the whole view and then continues subscribing from number + 1. If
    it asked for the number only after finishing the listing, it could miss
    a change (say a directory rename) made during the listing.
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

## Multiple disks and union folders (agreed 2026-11-09)

All **[designed]**.

- **Every disk is a separate volume** (default). Another disk with our FS,
  a foreign FS (NTFS, FAT, ext4 - dual boot) and a removable disk appear
  under **`/svazky/<name>`** and in the file manager sidebar under Devices.
  Foreign FS: read right away, write only explicitly (`diskguard`).
  Nothing joins by itself; there are no drive letters.
- **Volume name, description and identity**: every volume has
  - a **short name** for paths and the terminal (`/svazky/data`): lower
    case letters, digits, `-`, no spaces, unique; the default is derived
    from the description or the disk model and can be changed;
  - a **description** for people ("4 TB data disk in the drawer") and a
    free-form **note**;
  - automatic **details**: model, size, type (NVMe, SATA SSD, HDD, USB,
    SD), connection, FS, serial number, health;
  - a stable **identity (UUID)**: pinned folders, union folder sources
    and settings bind to it, so renaming the short name breaks nothing and
    the disk is recognized in another port.
  The terminal uses the short name and shows the description as a hint in
  completion; `ls /svazky` has columns name, description, type, size,
  free; the file manager shows mainly the description with the short name
  in smaller type.
- **A pool is a deliberate action on a disk, not a setting**: "Mirror the
  system disk" (the same data twice) or "Extend" (more space; with a
  warning that losing one disk loses the data of the whole pool and that
  fast and slow disks get mixed). The format is ready for it (device
  number in pointers).
- **Union folder** (like Plan 9 union directories or Windows libraries):
  one folder, e.g. `~/Hudba`, with folders from several disks pinned into
  it. The listing shows the content of all sources together, but each file
  physically stays on its disk - losing a disk loses only its files, not
  the whole folder.
  - **Where a new file goes**: to the primary source or (without a
    primary) automatically to the source with the most free space. Move
    and delete work on the real source.
  - **The same name in two sources**: both entries visible with a disk
    tag; opening without qualification takes the primary source.
  - **A disconnected disk**: its entries stay greyed out with a "disk
    disconnected" tag (from the index) and cannot be opened.
  - The file manager shows that a folder is a union and, for each entry,
    which disk it lives on; sources can be added and removed.
  - Implemented by the namespace service (a process's view of the tree),
    not the on-disk format; the FS of each disk does not change.
- **The system installs only on our FS** (versions, journal, permissions,
  atomic writes, vaults). FAT only where firmware requires it (ESP) or for
  exchange with Windows (MO-LOGY); NTFS and ext4 for data only.

## System and updates (agreed 2026-11-09)

1. **Now (development)**: the system = kernel + initfs on the ESP.
   Network updates into two A/B slots on the ESP, switch on restart, a
   boot attempt counter and a return to the previous slot after 3 failed
   starts. Source `cargo xtask serve-update` on the local network, the
   `update` command on the machine. No signatures (development mode, off by
   default), only SHA-256 against corruption. **[designed]**
2. **Once the system outgrows the initfs** (apps after the Rust std port,
   libraries): the system as an **immutable read-only dataset with
   versions**. An update creates a new version (only changed blocks are
   downloaded and written, the old version shares the unchanged ones), the
   loader picks the version at start, rollback = starting the previous
   version (e.g. 3 kept). Settings, logs and data live in separate
   writable datasets - a system rollback does not erase them;
   configuration has its own history. The ESP holds only the loader, the
   kernel and a small initfs that mounts the system dataset.
   **[designed]**
3. **With signatures**: the root hash of the system dataset (Merkle tree)
   is signed; the loader and kernel verify the whole system with one
   signature (sealed system), building on Secure Boot with own keys.
   **[designed]**

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
- **An application** gets by default only its own directory and what the
  user just opened or picked for it (Open/Save dialog, drag and drop, "Open
  with" - the powerbox). Picking a folder can be remembered ("this
  application may always use Projects"). **The user** has convenient access
  to all their files through the file manager and the system dialog - user
  convenience does not require broad application access.
- ⚠️ Old programs (mainly through the Windows layer) expect access to the
  whole Documents folder: a one-time prompt "The application wants access to
  Documents", or the Relaxed profile for that application only.
- **Profiles in the system settings:**

| Profile | For whom | Behaviour |
|---|---|---|
| **Standard** (default) | everyday use | the user accesses their files without restrictions or prompts; applications only their own directory + what the user picked (can be remembered); not system or other users' files |
| **Strict** | sensitive data, companies | applications sandboxed (picker dialog only), vaults lock on sleep, audit of changes and reads, ransomware detection by entropy (see [Defaults and profiles](#defaults-and-profiles)) |
| **Relaxed** | developers | applications see the whole user home, fewer confirmations |

## Encrypted vaults

> **Change from the v3 design (phase 4, detailed design):** previously only
> bullet points. Newly decided: without the key **only the pointers** stay
> readable (block locations and their checksums, like ZFS), **the client
> derives the key from a password and FIDO2**, the FS only gets a 32-byte
> wrapping key, **both ciphers right away** (XChaCha20-Poly1305 and
> AES-256-GCM), **the recovery key is a wrap** of the master key (like
> BitLocker and LUKS). **Why:** scrub, self-healing, deletion and backups
> must work for a locked vault too; fsd must not stall for half a second on
> Argon2id for one client; a leaked recovery key must be replaceable
> without re-encrypting the data.

**Status:** [designed] - the whole chapter; none of it is implemented yet.

A vault = **a dataset with encryption**: file contents, names, metadata
(sizes, times, permissions, attributes), the journal, the trash and its
snapshots and clones. The cryptography comes from RustCrypto
(`chacha20poly1305`, `aes-gcm`, `hkdf`, `argon2`, `zeroize`), none of our
own.

### Keys

```text
protector (password / recovery key / FIDO2 / service / TPM)
   │ the client derives the wrapping key KEK (32 B): Argon2id, hmac-secret, HKDF
   ▼
vault keyring (not encrypted, in the root dataset)
   │ AEAD(KEK) wrap ──> master key MK (256 bits, random, only in memory)
   ▼
HKDF(MK, purpose) ──> data key, metadata key, deduplication key,
                      intent log key (a separate key per purpose)
```

- **The master key** is random, created with the vault and never changes
  (replacing it = a new vault and a copy). Changing protectors does not
  re-encrypt the data, it only changes the keyring.
- **The wrapping key (KEK)** comes from the client: the FS never sees the
  password. The derivation parameters (salt, Argon2id m/t/p, FIDO2
  credential id) are stored by the FS with the protector so the client
  knows them on another computer too.
- **Clones and snapshots of a vault** share the master key (shared blocks
  must be readable). A clone outside a vault with the same key is not
  possible.

### Protectors

| Kind | KEK | Note |
|---|---|---|
| password | Argon2id(password, salt, m, t, p) in the client | default m = 256 MiB, t = 3, p = 1 (like accountd); parameters stored |
| **recovery key** | HKDF(256 random bits) | **mandatory**: a vault cannot be created without one and the last one cannot be removed; shown to the user only once; format below |
| FIDO2 | hmac-secret(salt) from the security key | the credential id and salt are stored; the client drives the key through accountproto/ctap |
| service | HKDF(key from the service) | the user's home: accountd supplies the key through init; accountd has its own wraps for every sign-in method and its own recovery key, so for a home vault the FS does not enforce a recovery protector |
| TPM | - | only an interface for now (`NOT_SUPPORTED`); never the only protector |

**Recovery key notation** [designed]: 256 random bits in base32
(Crockford: digits and letters without I, L, O, U; input is
case-insensitive and O/I/L are read as 0/1/1). Base32 carries 5 bits per
character, so 256 bits need ⌈256 / 5⌉ = **52 data characters** (260 bits,
the last 4 bits are zero padding). They are followed by **4 check
characters** = 20 bits of checksum (the first 20 bits of BLAKE3 over the
32 key bytes): a typo is caught with probability 1 - 2^-20 and the UI shows
which group it most likely is in. In total **56 characters** in **14
groups of 4** separated by dashes = 69 characters on a line, e.g.
`7K3M-...-QX2P` (14 groups). The entropy is not reduced: 256 bits matches
the master key, a shorter key would be the vault's weakest point.

Only someone who has just unlocked the vault (the FS knows MK) and has
the *Manage* right to the vault root may add, remove or rename a
protector.

### What is encrypted and what is not

| Visible without the key | Encrypted |
|---|---|
| that the dataset is a vault, its name in the parent folder, the protectors (kind, name, parameters) | file and folder names, directory structure |
| total space used by the vault | sizes, times, permissions, attributes, number of files |
| block locations, extent lengths and their checksums (for scrub and self-healing) | file contents (also small ones in a leaf), symlinks |
| number of tree nodes (rough amount of metadata) | change journal, trash, snapshot names and metadata |

⚠️ **Knowingly accepted leak:** extent lengths and the node count reveal
the rough size distribution of large files and the amount of metadata
(same as ZFS). Not the names, the contents or the exact number of files.
Compression before encryption reveals compressibility (the compressed
length); in a vault compression is therefore off by default like
everywhere, and turning it on in a vault shows a warning.

### On disk

- **A vault tree node:** the header is not encrypted (magic, level,
  generation, an "encrypted" flag, nonce, a 16 B authentication tag). An
  internal node has **only the child pointers** in the clear (location,
  length, checksum, copy, generation); the child keys (oid, item kind) are
  encrypted. A leaf has a plaintext **pointer table** (every block it
  refers to: a data extent with block checksums, a snapshot root, a node
  of a shared tree) and an encrypted part with the items, which refer to
  the table by index.
- **A data extent:** every block is a separate AEAD (reading 4 KiB does
  not need to decrypt the whole extent). The 16 B tag per block is in the
  encrypted part of the leaf, the checksum of the encrypted block
  (XXH3/BLAKE3) in the plaintext table. Overhead 32 B per 4 KiB block =
  0.8 %.
- **Checksums cover the encrypted data**, so scrub, self-healing from the
  copy, moving blocks (defragmentation) and send work without the key. The
  tag also catches a deliberate change the keyless checksum cannot (an
  attacker can recompute XXH3): a changed ciphertext = `CORRUPTED`, never
  forged data.
- **Binding to the tree:** a node's AEAD takes as additional data (AAD)
  the plaintext header, the vault id and the **checksums, lengths and
  generations** from the pointer table (not the location). A subtree
  therefore cannot be swapped for another or an older one, but a block can
  be moved (defragmentation, self-healing) without the key - only the
  location changes. Same for the data block tags: the table with checksums
  is in the leaf's AAD. ⚠️ Rolling the **whole** vault back to an older
  valid state (a forged dataset record in the root dataset) cannot be
  detected without the key - same as ZFS; only the verified system (root
  signature) prevents that.
- **Nonces:**
  - XChaCha20-Poly1305: a random 192-bit base per extent or node, block
    `i` = base XOR `i` (stateless random nonces, negligible collisions),
  - AES-256-GCM: 96 bits = **epoch** (32 bits) + an in-memory counter (64
    bits). The epoch is in the keyring and is increased and saved by a
    commit on every unlock, **before anything is encrypted**. After a crash
    the counter does not repeat (a new epoch), so nonce reuse cannot happen
    even after a power cut. A random 96-bit nonce would not be enough for
    billions of blocks.
  - The cipher is chosen when the vault is created: AES-256-GCM when the
    CPU has hardware AES (AES-NI, ARMv8 Crypto), otherwise
    XChaCha20-Poly1305; stored in the keyring.
- **The keyring** (item `KEYRING` in the root dataset + copy
  `KEYRING_COPY`; with metadata 2x = 4 copies on disk): version, cipher,
  epoch, MK fingerprint (HKDF, not MK itself), protectors (id, kind, name,
  parameters, nonce, wrapped MK + tag). Changing protectors = one commit
  (both items at once): after a crash the whole old or the whole new
  keyring is valid.
- **Intent log:** in a vault `fsync` does a **commit** instead of a log
  write (the log is replayed on mount, when the vault is not unlocked yet).
  Cost: `fsync` in a vault ~25-50 ms instead of ~5 ms in QEMU.
- Feature flag `INCOMPAT_VAULT` (an older driver does not mount a volume
  with a vault - it would write unencrypted).

### A locked vault

- In the parent folder's listing it is an entry of kind `Dataset` with
  a **locked** flag; `stat` of the root, a listing and opening anything
  inside return `LOCKED`. The content size and number of files are not
  reported (the vault's total used space is visible only in disk
  management).
- **Locking** (`VAULT_LOCK`, sign-out, sleep depending on the profile):
  a commit of the vault's changes, dropping decrypted nodes and data from
  the cache, zeroing MK and the derived keys (`zeroize`). Open handles then
  return `LOCKED` (clients' persistent handles go through `REOPEN` after an
  unlock).
- **An fsd crash** = all vaults locked (the keys were only in memory).
  accountd unlocks the home again: it keeps the home key for the duration
  of the session and after an fsd restart (`Happened::Restarted`) sends it
  through init again. Other vaults must be unlocked by the user.
- Scrub, self-healing, defragmentation, backups (send) and **deleting the
  whole vault** work while locked (only through pointers). Deleting a
  snapshot or a file inside works only when unlocked.

### Instant secure deletion

`VAULT_DESTROY` first deletes the keyring (a commit) - from then on the
data is unreadable even for someone who knows the password or the
recovery key - and then frees the blocks by walking the pointers (without
the key, in the background). ⚠️ Copies of MK may survive in the vault's
clones (shared key) and in an exported keyring backup; the deletion says
so explicitly. On an SSD the blocks physically disappear only after TRIM.

### Compression and deduplication

- Compression **before** encryption (encrypted data does not compress).
- Deduplication **only inside the vault and its clones**: the index uses
  a keyed hash (HMAC with the deduplication key from MK) instead of the
  content hash, so the index does not reveal a match with data outside
  the vault. An instant copy (reflink) between a vault and the outside = an
  ordinary copy (decrypt and encrypt with another key).

### Protocol (fsproto, new operations)

| Operation | Who | Meaning |
|---|---|---|
| `VAULT_CREATE` | *Create* right in the parent | path, cipher (auto), protectors (kind, name, parameters, KEK); without a recovery protector (or a service kind) `INVALID_ARGS`; returns the vault id |
| `VAULT_UNLOCK` | anyone who can see the vault | protector id, KEK; a wrong KEK = `ACCESS_DENIED` (slowed down after failures); unlocking applies system-wide, access is still governed by permissions |
| `VAULT_LOCK` | *Manage* or whoever unlocked it | see locking |
| `VAULT_STATUS` | anyone who can see it | `locked` / `unlocked`, cipher, protectors without wraps (for the client: salt, parameters) |
| `VAULT_ADD_PROTECTOR` / `VAULT_REMOVE_PROTECTOR` | unlocked + *Manage* | the last recovery protector cannot be removed |
| `VAULT_EXPORT_KEYRING` | unlocked + *Manage* | the keyring as a file (a backup against a damaged keyring; useless without a protector) |
| `VAULT_DESTROY` | *Manage* + *Delete* | instant secure deletion |

The user's home (accountd, agreed with the accounts thread):

- the home key = 32 B from a CSPRNG (not derived from the password); the
  service protector's KEK = HKDF(home key, `"mo-vault-service-v1"` ||
  dataset id) - bound to the specific vault. accountd keeps the home key
  wraps for the sign-in methods and the home's recovery key,
- `ENCRYPT_HOME` (only a new or empty home, otherwise `NOT_SUPPORTED` until
  conversion exists) or `CREATE_HOME` with a key -> init `VAULT_CREATE`
  with a service-kind protector,
- `UNLOCK_HOME { uid, key }` -> init `VAULT_UNLOCK`; init zeroes the key
  after passing it on,
- locking the screen does **not** lock the vault (processes keep running);
  `VAULT_LOCK` only when the user's last session signs out (the strict
  profile's "lock on sleep" goes the same way + a new authentication),
- fsd crash: accountd keeps the home key (`Zeroizing`) while the user has
  at least one session; init sends it the event `FS_RESTARTED`
  (accountproto op 61) and accountd sends `UNLOCK_HOME` again. If accountd
  crashed first (keys lost), the home stays locked and the screen asks for
  a new authentication (without signing out).

### ⚠️ Risk of data loss

| Situation | Consequence | Protection |
|---|---|---|
| forgotten password without a recovery key | **the vault's data is lost for good** - nobody can recover it (that is the point of encryption) | the recovery key is mandatory; creation shows it and has it written down and verified (typed back); settings warn about a vault with a single protector besides recovery |
| damaged keyring | neither the password nor the recovery key helps | the keyring 2x as an item and with metadata 2x = 4 copies; self-healing from a copy; keyring export (`VAULT_EXPORT_KEYRING`) - the backup is worthless without a protector and may go to the cloud |
| power cut while changing protectors | - | the change is one commit: the whole old or the whole new keyring; a new protector is added before the old one is removed (the UI does not allow the opposite order) |
| power cut while unlocking (AES-GCM epoch) | - | the epoch is saved by a commit before anything is encrypted |
| fsd crash, power cut | the vault is locked, unsaved changes as everywhere (since the last commit) | `fsync` in a vault = a commit |
| key in memory | readable from an fsd memory dump or from swap | keys only in `Zeroizing`, zeroed on lock; there is no swap yet (when there is, it must be encrypted); fsd memory dumps only for administration |
| deleting a vault by mistake | `VAULT_DESTROY` is irreversible at once | the UI asks for confirmation with the vault's name; immutable snapshots of the parent dataset do not protect the vault (it is a separate dataset), so the file manager first offers a keyring export and a backup |

### Tests

- model: random operations in a vault against a model, locking and
  unlocking, snapshots and clones, consistency check,
- power cuts: during creation, protector changes, unlocking (epoch),
  a commit, locking and vault deletion,
- wrong key: `ACCESS_DENIED`, no data, slow-down,
- tampering: flipping a bit in an encrypted block and node with a
  **correctly recomputed checksum** must end in `CORRUPTED`; so must
  swapping two blocks or subtrees of the same vault and a subtree from an
  older snapshot (AAD); moving a block without the key (defragmentation) on
  the other hand must pass,
- leaks: a vault file's name or content must not be found in the raw disk
  image; a locked vault reports neither size nor count,
- scrub and self-healing of a locked vault, deleting a locked vault frees
  the space,
- nonces: all nonces of a test run unique (AES-GCM also after a power cut),
- QEMU: a vault through fsd, locking with open handles, an fsd crash.

### Implementation order (phase 4a)

1. keyring, protectors, protocol, locked view (data not encrypted yet -
   only the skeleton and interface tests),
2. data encryption (extents, small files in a leaf),
3. metadata encryption (nodes with a pointer table), journal, trash,
   snapshots,
4. deduplication in a vault, keyless scrub and deletion, `fsync` = commit,
5. accountd integration (home), QEMU tests.

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
- **Trial boot after an update including service data** (ties in with A/B
  and the verified system): the candidate system version is bound to a
  snapshot of the configuration and the data of those services that change
  their data format on first start (the service declares the list). It is
  marked good only after the required services have started; a boot attempt
  counter (like `boot-complete` in systemd). On failure the old system
  **and the compatible state of the affected services** come back. **User
  documents are not part of the group** - a failed update must not roll
  back their work. The FS provides the mechanism (an atomic snapshot of a
  group of datasets, rolling back the group), the decisions are in the
  update service.
- **Pinning a version for the duration of work:** whoever opens an old
  version, exports a snapshot (backup, send) or needs stable sources (a
  build) gets a **handle to a specific root**. Automatic history cleanup may
  hide it, but its blocks are freed only after the last handle is closed
  (the same rule as a clone's `origin`: a reference keeps the root alive).
  For an export that must survive a restart, a **persistent pin with a
  reason** (a record in the dataset: who, why, until when); the space
  overview then shows "this version is held by a backup in progress".
  Immutable snapshots (holds) are a special case of a persistent pin.
- **Online volume resize** (grow and shrink).
- **Access audit** - see [Security and audit](#security-and-audit).
- **Space overview** including how much deleting each snapshot frees.

## Applications and compatibility

**Status: agreed.** Things other systems have and applications rely on.

- **File and range locking** (phase 1e, protocol v2): a shared lock
  (read) and an exclusive lock (write) on a whole file or a byte range. A
  lock belongs to a handle; closing the handle or a process crash releases
  it automatically.
  - **Advisory** locks by default (like POSIX `fcntl`): whoever does not
    check them may still write.
  - **Mandatory** locks only for the Windows layer (`LockFileEx`, the
    `CreateFile` share modes), because Windows applications depend on them.
  - Waiting for a lock has a timeout and the service detects wait cycles (a
    deadlock is reported as an error, nothing hangs).
- **Space preallocation** (phase 3): `allocate(file, from, length)` reserves
  contiguous blocks in advance (databases, downloads, video). The extent has
  an **unwritten** flag - it reads as zeros until written; the first write
  only clears the flag. The format is reserved already (extent flags).
- **Change watching** (phase 2): an application subscribes to a directory
  or dataset and receives events (created, changed, removed, moved) over a
  channel. The source is the change journal, so events are **never lost**,
  not even on queue overflow: the application asks "since number N"
  (unlike inotify, where overflow = lost events). After retention expires
  it gets `JOURNAL_GAP` and reads the state again (see the change journal).
- **Memory-mapped files** (phase 5 + kernel): a memory object (VMO) backed
  by a file, pages are loaded on access and one page cache is shared by the
  FS and mappings. Program loading (PE/ELF) and databases need it. It is
  mostly kernel work (demand paging).
- **Case sensitivity per dataset** (phase 1c): a dataset option for git
  repositories and Linux sources where `Foo` and `foo` exist side by side.
  The default stays case-insensitive; it can be changed only on an empty
  dataset.
- **Disk health** (phase 4): a service reads SMART (SATA) and the health log
  (NVMe), tracks SSD wear, remapped sectors and scrub errors and warns
  before the disk fails. The result is in the disk status overview.
- **No hard links:** every object has one parent (that is what makes
  subtree summaries and visibility work). Instant copies (shared blocks)
  replace them; the Linux and Windows drivers report hard links as
  unsupported.

## Security and audit

**Status: agreed, phase 4** (except where noted).

### Access log (audit)

| What | How |
|---|---|
| Events | open, read, write, create, remove, move, permission change, **denied attempt** |
| Who | application (identity from its signature), user, process, handle |
| Levels | security events (default, always) / + changes / + reads - configurable per dataset, see [profiles](#defaults-and-profiles) |
| Volume | repeated reads of the same file by the same application in one session = one record |
| Storage | a special append-only dataset, retention by days or space |
| Tamper resistance | records form a **hash chain** (each contains the hash of the previous one) and the head of the chain is in the dataset record, i.e. in the commit's Merkle tree. The log can only be deleted as a whole, which is visible; it cannot be edited silently. |

- **"Who read my file"** right in the file properties: the last N accesses
  from the audit, in plain words ("Photos - read by Viewer, yesterday
  18:02").
- Reading the audit itself requires the *Manage* right on the dataset and
  is logged too.

### File origin

- Every new file gets the extended attribute **`sys.origin`**: where it came
  from (download URL, the application that created it, copy source,
  removable disk). Copy and move carry it over; rename does not change it.
- **An executable with origin "internet" or "removable disk"** asks before
  its first run (like Mark of the Web, but it cannot be bypassed by renaming
  or copying inside the system).
- Only the FS and system services may set the attribute, not applications.

### Ransomware protection (behaviour detection)

- **Split (only cheap and deterministic things in the FS):**

| Where | What |
|---|---|
| **FS** | per-application counters of rewrites and renames of existing files (O(1) per write), **decoys** (touching a marked object = an immediate event), **freezing an application's writes**, a hard safeguard "more than N existing files rewritten per minute = freeze and ask" |
| **service** | entropy over a sample of blocks, thresholds and exemptions, the dialog with the user, selective rollback; reads the journal asynchronously, so it does not slow writes |

  Why: heuristics (entropy) in the path of every write cost performance,
  mislead (JPG, ZIP, DOCX have high entropy by themselves) and a false alarm
  is a user interface decision - it does not belong in the most critical
  component. **The strongest defence is not detection but mechanisms:**
  capabilities (an application can encrypt at most what the user let it
  touch), frequent snapshots with holds and rollback via the journal.

- A service watches the change journal: when one application rewrites or
  renames many files within a short time and the new content has high
  entropy (looks encrypted), the FS:
  1. immediately takes a **snapshot** of the affected datasets,
  2. **pauses** the application (its writes wait),
  3. asks the user: continue / stop and roll the changes back from the
     snapshot.
- **Decoys:** hidden files in the user's home that nothing legitimate
  touches. A write to them triggers the same immediately.
- Applications that do this legitimately (archivers, encryption, video
  export) can be exempted after confirmation.
- **The point before the attack matters more than the snapshot at
  detection** (that one already captures the incident). Hence short-term
  automatic snapshots of the home (e.g. every 5 minutes, kept for 1 hour -
  copy-on-write is cheap) and restoring from the last snapshot before the
  first suspicious write.
- **Selective rollback:** only the suspect application's changes are
  rolled back (the journal knows who changed what); changes of other
  applications in the same dataset stay. When another application
  legitimately wrote to the same file in the meantime, both versions stay
  and the user chooses.

### More safeguards

- **File expiry date** (phase 2): the `sys.expires` attribute - after the
  date the file moves to the trash (or is archived). For privacy and
  temporary files.
- **True times:** the creation time and generation are set only by the FS.
  An application may change the displayed modification time (for
  compatibility), but the true times stay in the audit and the object
  record - forged times are visible.
- **No execution on a dataset** (phase 1d, permissions): data datasets
  (downloads, documents, removable disks) have the *Execute* right off by
  default; programs run only from application datasets.
- **Per-application quotas** (with quotas): an application cannot fill the
  disk (not even with logs by mistake); past the limit it gets `NO_SPACE`,
  the user and the system still have space.
- **Unlocking vaults - several "protectors" of the same key:** vault data
  is encrypted by a random master key, and that key is stored several
  times, each time wrapped differently: by a password (Argon2id), by a
  **recovery key** (always, mandatory), optionally by a FIDO2 key and
  **optionally by the TPM** (convenient unlocking without a password on
  this machine with an intact boot loader, possibly with a PIN).
  - The TPM is **only an add-on, never the sole protector** - off by
    default. A disk moved to another machine (or a new CPU with a different
    integrated TPM) is unlocked with the password or recovery key and the
    system offers to add the new machine's TPM; the old entry is removed.
  - Adding or removing a protector does not re-encrypt the data, it only
    changes a small record of wrapped keys.

### Defaults and profiles

Whatever slows things down or gets in the way is off or in a cheap variant
in the default (Standard) profile; the Strict profile turns it on, and each
item can also be enabled on its own in the settings (for one dataset or the
whole system).

| Feature | Cost | Standard (default) | Strict | Relaxed |
|---|---|---|---|---|
| audit of security events (permission change, denied attempt, vault deletion) | negligible | on | on | on |
| audit of file changes | small (a record per write) | option | on | option |
| audit of reads | medium (a record per open) | option | sensitive datasets | option |
| "who read my file" | depends on the read audit | only with the read audit | on | option |
| file origin | negligible (an attribute at creation) | on | on | on |
| prompt before running a file from the internet | none | on | on | option |
| ransomware decoys | none | on | on | option |
| ransomware detection by rewrite count | small (counters) | on | on | option |
| ransomware detection by content entropy | medium (computed over a sample of blocks) | option | on | option |
| no execution on data datasets | none | on | on | off |
| per-application quotas | negligible | on (generous) | on (tight) | off |
| vaults lock on sleep | convenience | option | on | option |
| inline deduplication on write | medium to large | off (background only) | off | option |
| checksum verification on every read | small | on (cannot be disabled) | on | on |
| new input device verification - keyboard, mouse (against BadUSB, [USB](usb.md#verifying-a-new-input-device-against-badusb)) | convenience (a code on attach) | option | on | off |

Data integrity (checksums, the Merkle tree, metadata twice) does not change
with profiles - it protects against corruption, not against an attacker,
and costs little.

### Risks (knowingly accepted)

- **Auditing reads costs performance and is sensitive.** The log itself
  reveals who read what - so by default only security events are logged, the log is
  protected by the *Manage* right and reading it is logged.
- **Ransomware detection has false alarms** (compression, encryption, video
  export). Hence pause and ask, never a silent block; exemptions after
  confirmation. A clever attacker may encrypt slowly - immutable snapshots
  are the main defence against that, detection is only an early warning.
- **The origin is lost** when copying through a foreign FS (FAT on a flash
  drive). The Windows driver carries it as `Zone.Identifier` where possible.
- **TPM:** a firmware update or a hardware change alters the measurements
  and the TPM will not unlock the vault. That is why the TPM is only an
  optional add-on and the password and **recovery key** always work.
- **Mandatory locks** can block other applications (DoS). Hence only for the
  Windows layer, bound to a handle and with a wait timeout.

## Working with versions in the file manager

**Status: agreed.** Features built on clones, diff and capabilities.

- **Restore as a copy** (part of the first demo): in the file properties
  you pick a version and choose *Open read-only*, *Restore as a copy* (next
  to the current one, e.g. "Report (version 3 Nov 14:02).docx") or *Replace
  current*. One paragraph or a config can be pulled from history without
  rolling back the whole folder. **In the first demo the file is simply
  copied from the snapshot** (verifying version selection and restoring
  next to the original); an instant copy via shared blocks comes later as an
  optimization together with instant file copies (phase 3), so that the
  first milestone does not depend on block sharing.
- **Try changes on the side** for a whole project folder: the file manager
  creates a temporary writable clone and opens it in an editor, an
  installer or hands it to an agent. **The application gets a capability to
  the clone only**, it does not see the original. When done: an overview of
  what was added, removed and changed (a diff of the clone against its
  origin snapshot) and taking selected changes over into the original. If
  the same file changed in the original in the meantime, taking it over
  shows a conflict (three-way comparison: origin snapshot, original,
  clone). An untaken clone is discarded in one step. It shows best what
  clones, diff and capabilities do together (e.g. trying a bigger refactor).
- **A temporary sub-dataset for builds and caches:** `target/` or render
  output looks like an ordinary folder in the project, but it is its own
  dataset with its own space limit and retention rules (default: no
  history). A project snapshot does not include it (it captures sources and
  configuration) and it can be cleaned separately. A snapshot listing shows
  clearly which subfolders have their own history (they are datasets).

## Cloud and network

**Status: agreed, the format is reserved in phase 1, implementation in
phase 6 (after the network stack).**

Principles:

- **The truth is local.** The cloud is another storage, not a replacement of
  the FS: without network everything local keeps working and sync catches
  up later.
- **Client-side (end-to-end) encryption.** The provider only sees encrypted
  packs of equal size; names, structure and content never reach it.
- **Provider independence.** Storage is attached through an adapter: S3
  compatible (AWS, Backblaze, MinIO, own server), WebDAV, another of our
  devices on the network. An adapter only does `put/get/list/delete` of a
  pack.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Cloud and network">
<defs><marker id="cl1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="150" height="60" rx="6"/><text class="t b" x="85" y="56" text-anchor="middle">application</text><text class="m" x="85" y="75" text-anchor="middle">reads a file</text>
<rect class="acc" x="200" y="30" width="150" height="60" rx="6"/><text class="t b" x="275" y="56" text-anchor="middle">fsd + libs/fs</text><text class="m" x="275" y="75" text-anchor="middle">datasets, versions</text>
<rect class="ok" x="390" y="30" width="170" height="60" rx="6"/><text class="t b" x="475" y="56" text-anchor="middle">sync service</text><text class="m" x="475" y="75" text-anchor="middle">packs, encryption</text>
<rect class="warn" x="600" y="30" width="150" height="60" rx="6"/><text class="t b" x="675" y="56" text-anchor="middle">adapter</text><text class="m" x="675" y="75" text-anchor="middle">S3 / WebDAV / device</text>
<path class="ln" d="M160 60 L198 60" marker-end="url(#cl1)"/>
<path class="ln" d="M350 60 L388 60" marker-end="url(#cl1)"/>
<path class="ln" d="M560 60 L598 60" marker-end="url(#cl1)"/>
<rect class="box" x="200" y="130" width="360" height="70" rx="6"/><text class="t b" x="380" y="155" text-anchor="middle">network stack (service, milestone 6)</text><text class="m" x="380" y="175" text-anchor="middle">virtio-net, TCP/IP, DNS, TLS 1.3, HTTP</text>
<path class="ln" d="M475 90 L475 128" marker-end="url(#cl1)"/>
<path class="ln" d="M675 90 L560 150" marker-end="url(#cl1)"/>
<text class="m" x="10" y="220">File on demand: fsd sees the REMOTE flag, asks the sync service, which downloads, verifies and decrypts the pack.</text>
</svg>
<figcaption>The path of data to the cloud: the FS knows nothing about the network, the sync service and the adapter do everything.</figcaption>
</figure>

### 1. Encrypted backup

- Built on **send/receive**: the difference of two snapshots is split into
  packs of ~4 MiB, each encrypted (XChaCha20-Poly1305) and named by a
  **keyed BLAKE3 hash** of its content. Same content = same name =
  **deduplication across backups and devices**, without the provider
  learning what is inside.
- The backup index (also encrypted) can **restore a single file** without
  downloading the whole backup.
- **Retention rules** (e.g. 7 daily, 4 weekly, 12 monthly) and **backup
  verification**: a random sample is downloaded regularly and its checksums
  verified (a backup that cannot be restored is not a backup).
- **Immutable backups:** on storage with object lock (S3) the packs are
  locked for the retention time - neither ransomware nor a stolen account
  password can delete them.

### 2. Files on demand (placeholders)

- An object with the **REMOTE** flag: the record, name, size and thumbnail
  are local, the data only in the cloud (extents with the pointer flag
  `PTR_REMOTE`).
- On read, fsd asks the sync service to download it; the application just
  waits. The **"always keep offline"** option (PINNED flag) keeps the data
  local.
- **Automatic space freeing:** when space runs low, unshared, long unused
  files with a copy in the cloud become placeholders.
- The file manager shows the state (local / cloud only / syncing).

### 3. Sync between devices

- A **dataset** bound to a storage is synchronized. Every device makes
  local commits (phase 2) and sync exchanges them like git: non-overlapping
  changes merge by themselves.
- A **conflict** (the same file changed in two places) never loses data:
  both versions remain ("Report (conflict - laptop).docx") and a
  notification appears. Deletion goes through the trash, so it can be
  undone on the other device too.
- **Selective folders** (NOSYNC flag), bandwidth limits and a pause on
  metered (mobile) networks.

### 4. Sharing

- Every dataset has its own key. Sharing a folder = handing over the key
  encrypted with the recipient's public key; revoking = a new key (rotation)
  for further changes.
- A read link (with expiry and password) through an own server or device.

### 5. Third-party clouds

- OneDrive, Google Drive, Dropbox through their APIs as **separate
  adapters** in file mode (without client-side encryption, because the files
  are meant to be visible on the provider's web too). The settings label
  this clearly.

### What is reserved now (phase 1)

| What | Where | Meaning |
|---|---|---|
| object flag `INODE_REMOTE` (1) | object record, `flags` | data only in the cloud |
| attribute `sync.pinned` | extended attribute of a file | always keep local (a sync service setting, not a format bit) |
| attribute `sync.exclude` | extended attribute of a directory | do not sync (inherited) |
| pointer flag `PTR_REMOTE` (8) | `BlockPtr.flags` | the block is not on disk, only in the cloud |
| attribute `sync.state` | extended attribute | sync state of an object: remote version, tag, time |
| feature `INCOMPAT_REMOTE` (bit 0) | superblock | the volume has placeholders; an older driver must not mount it |
| dataset binding to a storage | dataset record (phase 1c) | storage ID, key ID, rules |

### Network

The network was not in the milestones yet; it was added to **milestone 6**
as userspace services: a virtio-net driver, a TCP/IP stack, DNS, TLS 1.3,
an HTTP client. Applications get network access only through a capability
(no capability, no socket), so the firewall is naturally per application.

### Risks (knowingly accepted, handled by the design)

- **Losing the key = losing the backup.** Client-side encryption means the
  provider cannot restore the data. So enabling backup creates a **recovery
  key** (print it / store it elsewhere) and backup does not start until it
  is confirmed.
- **A placeholder cannot be read without network.** The read returns a new
  error `UNAVAILABLE` (added to the ABI in phase 6); the UI shows in advance
  what is cloud only, and important folders can be pinned.
- **Traffic metadata:** even an encrypted cloud sees when and how much is
  uploaded. Packs have equal size (padding), but timing cannot be hidden.
- **Third-party clouds** (point 5) are not encrypted - the provider sees
  the content.

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
- **Lifetime of an open file:**
  - a handle keeps the object alive: after **removing its last name** or an
    **atomic replace** the object becomes an orphan (recorded in a
    persistent orphan list, like the ext4 orphan list); whoever has it open
    keeps reading and writing; it disappears with the last handle. After a
    crash orphans are cleaned up at mount;
  - a **service restart** invalidates handles (the channels die), the
    client reopens by path; durable handles across restarts maybe later;
  - a test with two clients: A opens `report`, B atomically replaces it, A
    keeps reading the original content from its handle, a new `OPEN` gets
    the new object.
- **NT layer - two separate things:**
  - the **share mode** is checked at open (the requested access against
    `FILE_SHARE_READ/WRITE/DELETE` of all open handles);
    `FILE_SHARE_DELETE` also decides renaming and deleting an open file;
  - **range locks** (`LockFileEx`) have their own overlap rules; writes
    through a mapped view do not check them and a mapped view blocks
    truncation (`STATUS_USER_MAPPED_FILE`).
- **Symlinks and directory capabilities:** walking inside a directory
  handle never escapes the granted subtree (like `RESOLVE_BENEATH` in
  Linux): an absolute target or `..` above the capability root =
  `ACCESS_DENIED`.
- **Conditional save** (first in v2): `OPEN` also returns the file's
  **revision** (oid + generation + sequence number of the last content
  change). `REPLACE(file, new data, expected revision)` checks in one
  operation that the revision still matches and atomically replaces the
  content; otherwise it returns a **conflict** (`CONFLICT`) and changes
  nothing - the new data stays with the client (the editor offers to save
  alongside or compare). It prevents silently overwriting someone else's
  change when the user, an editor, an agent and sync take turns on one
  project. First version: one operation on one file (like a git reference
  update with an expected old value); taking over changes from a clone then
  uses the same mechanism.
- **Write: accepted vs. durable.** A write reply says **how many bytes
  were accepted** (they are in the buffer, reads see them at once, the next
  commit stores them), not whether they are durable - only `FSYNC`
  confirms that. After an error the client may consider exactly the
  reported count accepted (in version 1 of the library: the whole call or
  nothing). **A write split into several messages** = each message is a
  separate write with its own result; the client does not continue after
  the first error and knows exactly how many earlier messages were accepted.
- **Replace by rename** (atomic save), `O_EXCL` creation.
- **Multiple clients:** each client has its own channel (`OPEN_SESSION`),
  request IDs, paged directory listing (cursor), timeouts; the service never
  dies because of one client.
- Later: shared memory and request queues (asynchronous I/O).

## Implementation order

**A vertical slice as the first goal:** instead of finishing all of phase
1 in breadth, the first demo is a **versioned project folder** in Windows
Explorer: take a snapshot, open a writable clone, make changes, show the
diff, **restore one file as a copy** - and the same with a crash in the middle of a
write. "Try changes on the side" (see [Working with versions in the file
manager](#working-with-versions-in-the-file-manager)) comes right after it.
Order:

1. **F1b** - review fixes (in progress),
2. **datasets and a minimum of versioning** - the dataset tree, dataset
   snapshots, writable clones with blocks held by their origin snapshot,
   diff, restoring a file; tests with power loss and space accounting
   checks,
3. **a handle layer** on top of `libs/fs` - open files, orphans, replace by
   rename, share modes, delete on close; used by the driver and by fsd
   (protocol v2),
4. **the Windows driver and the `osfs` tool** - see
   [Windows driver](#windows-driver),
5. permissions (F1d) and protocol v2 in fsd (F1e), then phases 2-6 below.

| Phase | Contents |
|---|---|
| 1 - format | 16 KiB nodes, superblock/pointer/record v3, feature flags, datasets and the dataset tree (with case sensitivity), names and symlinks, review fixes, protocol v2 with handles and locks, permissions (capabilities, grants, visibility, profiles, no execution on data) |
| 2 - versions (**done**, [FS history](fs-history.md)) | commits, branches, clones, diff, merge, change journal, automatic snapshots, trash, version history, undo, change watching, file expiry |
| 3 - data (**done**, [FS data](fs-data.md)) | compression, instant copies, deduplication, automatic holes, metadata twice and self-healing, background scrub, preallocation |
| 4 - protection | encrypted vaults (key protectors, optional TPM), immutable snapshots, quotas also for applications, audit with a hash chain, file origin, ransomware detection and decoys, disk health, verified system |
| 5 - other | indexes and tags, send/receive, transactions, ARC and prefetching, resize, git layer, memory-mapped files (with the kernel) |
| 6 - cloud (after network) | sync service, adapters (S3, WebDAV, device), encrypted backup, files on demand, sync between devices, sharing, third-party clouds |

Each phase ends with tests (unit, model, power loss, in the system),
documentation in cs and en, and a review.

## Windows driver

Goal: a disk (flash drive, test partition, image file) formatted with our
FS mounts in Windows as an ordinary drive (e.g. `F:`) and can be used in
Explorer. **For trying out only, not for real data.**

- **WinFsp** (a Windows kernel driver for user-mode file systems,
  winfsp.dev) - the user installs it themselves (it needs administrator
  rights). The binding to its DLL is our own and small (no binding
  generator, no clang).
- **The `osfs` tool** (command line; later a simple window on top of it with
  a list of disks and Format / Mount / Unmount buttons):

```text
osfs disks                         list disks and partitions (model, size, removable?)
osfs format <disk|image> [--label X] [--blake3]
osfs mount <disk|image> F:         mount as drive F:
osfs unmount F:
osfs check <disk|image> [--scrub]
osfs snapshot F: <name>  /  osfs diff ...  /  osfs restore ...
```

- **Start: an image file** (`osfs format image.img`, `osfs mount image.img
  F:`), a real disk only after that.
- **Names the first version exposes** (explicitly, so that every displayed
  name opens exactly the object that Explorer listed):

| Name in our FS | In Windows |
|---|---|
| valid in Windows too | unchanged |
| contains a forbidden character `: \ * ? " < > \|` | the character is shown as its counterpart in the Unicode private use area (U+F000 + code, like WSL and Cygwin) |
| already contains a character from U+F000-F0FF itself | **hidden** (displaying it would collide with a translated name); `osfs check` lists it |
| a reserved name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`, also with an extension), a trailing dot or space | **hidden**, `osfs check` lists it |
| longer than 255 UTF-16 characters | **hidden** (the Windows API mostly cannot open it) |
| a pair differing only in case (a case-sensitive dataset) | the first version **does not mount** case-sensitive datasets (later: the second name hidden with a report) |

  Writing from Windows: a name with a character from U+F000-F0FF is stored
  with the original forbidden character; a name our FS forbids (text
  direction controls) is refused. **Acceptance test:** for random names (a
  property test) every name in the listing opens exactly the object it
  came from, and no two entries have the same displayed name.
- Other translation: times in 100 ns units, the file ID from oid +
  generation; case is not distinguished, just as in Windows.
- **Concurrency:** WinFsp calls from several threads, `Fs` is
  single-threaded - one lock around the whole `Fs` to start with.

**Risks (stated up front):**

- ⚠️ **Formatting a real disk is irreversible** and needs administrator
  rights. `osfs format` refuses the system disk and disks with mounted
  Windows partitions, prints the model and size and erases only after the
  disk number is typed by hand as confirmation. An image file is the safest
  start.
- ⚠️ **Windows does not recognise our format:** after inserting the flash
  drive it offers "You need to format the disk" - **do not click it**, mount
  it with `osfs mount`.
- The FS is young: for trying out only, with your own backup elsewhere.

**A native Windows kernel driver (later, maybe):** WinFsp adds a kernel ->
our process -> kernel round trip to every operation. Our own kernel driver
would be faster, would integrate fully with the Windows cache manager
(mapped files), Windows would recognise the format by itself when a disk is
inserted (a file system recognizer) and it could even boot from it. The
cost: a huge scope (correct IRPs, oplocks, caching - WinBtrfs has over
100,000 lines), a bug = a blue screen and data corruption across the whole
system, and a driver signature from Microsoft (without it only the test
signing mode, which lowers system security). So WinFsp first; a native
driver only when performance or booting from the disk is really needed.
The FS core (`libs/fs`, Rust, `no_std`) can be used in both.

## Open questions

- Multiple disks (mirror, RAID): the format is ready, implementation after phase 5.
