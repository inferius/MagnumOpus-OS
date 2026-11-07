# File system history (phase 2)

Phase 2 of format v3 ("versions", see the [FS v3 design](fs-v3-design.md#implementation-order))
adds **history** to the datasets, snapshots and clones of phase 1: a
change journal, trash, undo of operations, file versions, automatic
snapshots, versioned commits with a logical ID, branches and merge, "try
changes aside", extended attributes and file expiry. The format and the
general algorithms are described in [file-system.md](file-system.md); the
format changes are summarised in
[Format versions](fs-versions.md#phase-2-history-ro_compat_history).

> **Change from v3 (phase 1):** a new chapter. A volume with history has the
> bit `RO_COMPAT_HISTORY`; new tree items 13-23, a fixed trash object
> `TRASH_OID` = 2, intent log records 14-18, the error `JOURNAL_GAP`,
> protocol operations 54-86.
> **Why:** history must hold for every write and survive a crash, so it
> belongs in the FS; the decisions (when to take a snapshot, when to empty
> the trash) are in the service, according to the dataset settings.

What is where (the boundary from the design - invariant in the FS,
settings as data, decisions in the service):

| What | Layer | Code |
|---|---|---|
| change journal, sequence numbers, publishing after confirmation | invariant (FS) | `libs/fs/src/fs/journal.rs` |
| trash (move, restore, purge) | invariant (FS) | `libs/fs/src/fs/trash.rs` |
| extended attributes, expiry index | invariant (FS) | `libs/fs/src/fs/xattr.rs` |
| file versions, undo of operations, automatic snapshots | mechanism (FS) | `libs/fs/src/fs/history.rs` |
| commits, logical ID, branches | invariant (FS) | `libs/fs/src/fs/commit.rs` |
| tree merge, "try aside" | mechanism (FS) | `libs/fs/src/fs/merge.rs`, `libs/fs/src/merge3.rs` |
| journal and trash retention, snapshot rules | settings (dataset data) | `records::Settings` |
| when to run the snapshot step, expiry, trash cleanup | decisions (service) | `user/fsd` (every minute) |
| records on disk | format | `libs/fs/src/records.rs` |

## Quick example

```rust
// Journal: an indexer continues exactly where the view ended.
let v = fs.view("/home/alice")?;                       // temporary snapshot + position
index_all(&mut fs, "/home/alice", &v.snapshot)?;       // walk the view
fs.release_view("/home/alice", &v.snapshot)?;
let mut next = v.seq + 1;
loop {
    let page = match fs.journal_read("/home/alice", next, 256) {
        Ok(p) => p,
        Err(Error::JournalGap) => break,               // retention: start again from VIEW
        Err(e) => return Err(e),
    };
    for (seq, e) in &page.entries {
        apply_to_index(e);
        next = seq + 1;
    }
}

// Trash and undo of an operation.
let id = fs.trash("/home/alice/report.docx")?;          // nothing is copied
fs.trash_restore("/home/alice", id, None)?;             // back to the original place
fs.undo("/home/alice", seq_rename)?;                    // move back

// File versions and restore.
for v in fs.file_versions("/home/alice/report.docx")? {
    println!("{} {} B", v.snapshot, v.size);
}
fs.restore_version("/home/alice/report.docx", "auto-2026-11-07-140000",
                   RestoreMode::Copy("/home/alice/report (version 14:00).docx"))?;

// Commits, a branch and merge.
fs.commit_version("/project", "Working login")?;
fs.branch("/project", "refactor", "/project-refactor")?;
/* ... changes in /project-refactor ... */
fs.commit_version("/project-refactor", "Refactor")?;
let r = fs.merge("/project", "/project-refactor")?;    // three-way
assert!(r.conflicts.is_empty());

// Try changes aside (the agent gets a capability to the clone only).
fs.aside_begin("/project", "/project-try")?;
/* ... the agent works in /project-try ... */
let changes = fs.diff_clone("/project-try")?;
fs.aside_apply("/project-try", Some(&["/src".into()]))?;   // selected only
fs.aside_discard("/project-try")?;
```

Through the fsd service the same is done by `FsClient` (`fsproto::hist`),
see [Protocol](#protocol-operations-54-86).

## Change journal

<figure class="diagram">
<svg viewBox="0 0 760 190" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Change journal">
<defs><marker id="hj1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="warn" x="20" y="40" width="80" height="40" rx="6"/><text class="t" x="60" y="65" text-anchor="middle">discarded</text>
<rect class="box" x="110" y="40" width="70" height="40" rx="6"/><text class="t mono" x="145" y="65" text-anchor="middle">first</text>
<rect class="box" x="190" y="40" width="70" height="40" rx="6"/><text class="t mono" x="225" y="65" text-anchor="middle">...</text>
<rect class="ok" x="270" y="40" width="90" height="40" rx="6"/><text class="t mono" x="315" y="65" text-anchor="middle">VIEW.seq</text>
<rect class="box" x="370" y="40" width="70" height="40" rx="6"/><text class="t mono" x="405" y="65" text-anchor="middle">...</text>
<rect class="acc" x="450" y="40" width="100" height="40" rx="6"/><text class="t mono" x="500" y="65" text-anchor="middle">published</text>
<rect class="box" x="560" y="40" width="170" height="40" rx="6"/><text class="t" x="645" y="58" text-anchor="middle">unconfirmed</text><text class="m" x="645" y="74" text-anchor="middle">readers do not see them</text>
<path class="ln" d="M315 85 L315 120" marker-end="url(#hj1)"/>
<text class="m" x="315" y="138" text-anchor="middle">the view has all up to VIEW.seq</text>
<path class="ln" d="M500 85 L500 120" marker-end="url(#hj1)"/>
<text class="m" x="500" y="138" text-anchor="middle">commit or fsync moves the limit</text>
<text class="m" x="60" y="105" text-anchor="middle">read = JOURNAL_GAP</text>
<text class="m" x="20" y="175">Sequence numbers start at 1 and never repeat for anyone who has seen them.</text>
</svg>
<figcaption>The dataset journal: retention discards the oldest, readers see only confirmed records.</figcaption>
</figure>

Every dataset has its own journal - an ordered stream of records:

| Field | Meaning |
|---|---|
| sequence number | u64 from 1, the offset of the `JOURNAL` item |
| operation | `CREATE`, `REMOVE`, `RENAME`, `CONTENT`, `ATTR`, `TRASH`, `RESTORE`, `SNAPSHOT` |
| kind, oid, generation | object identity (the oid is stable within the dataset) |
| parent, name / new parent, new name | where the object was and where it went |
| size, extra | the new size; for `ATTR` what changed (times, permissions, owner, attributes), for the trash the entry number, for a snapshot its number |
| time, who, application | time of the operation, the user; application is 0 for now (reserved) |

Rules:

- **A record is created atomically with the change** - it is an item of the
  dataset tree, stored by the same commit. A snapshot also captures the
  journal up to its creation (`SnapshotInfo::journal_seq`).
- **Published only after confirmation:** a reader gets only records up to
  `published` (the last commit or `fsync`). A change confirmed by `fsync` is
  replayed from the intent log after a crash and its records are created
  again **with the same numbers** - the same order, time and user (the log
  record `Actor`). Unconfirmed changes disappear after a crash together with
  their numbers, but nobody has seen those.
- **Content writes are merged:** one `CONTENT` per object per commit or
  `fsync` (it gets its number at the end of the batch). A database or a log
  with thousands of writes per second thus does not flood the journal; the
  reader reads the current content anyway.
- **Retention** (dataset setting): by default 1 million records or 30 days,
  whichever comes first. A commit discards at most the 4096 oldest (a commit
  must not take unbounded time). A number older than the oldest stored one =
  **`JOURNAL_GAP`**, never a silent skip.
- **A new baseline together with a position:** `view(dataset)` takes a
  temporary snapshot (kind `VIEW`, not shown in the snapshot listing) and
  returns the number of the last record it contains. The client walks the
  view and continues by subscribing from `seq + 1` - it skips nothing, not
  even a directory rename during the walk. A view is released with
  `release_view`; views from before a crash are deleted on mount.
- **Permissions:** reading the journal and a view require `MANAGE` on the
  dataset root (the journal shows the names of all objects).
- **Not recorded:** writes to an orphan (a removed open file) - it no
  longer belongs to the namespace.

### Watching changes (`JOURNAL_WATCH`)

The client sends the number from which it wants records and a time limit
(at most 60 s). When there are records, the reply comes at once; otherwise
the service defers it and replies after the next confirmation (the automatic
commit every 5 s or a client's `fsync`), or with an empty page after the
limit. Waiting does not block other sessions.

⚠️ A session with a pending `WATCH` does not process another request until
it gets the reply (the protocol is synchronous). If the client sends another
request anyway, the service first answers the wait (possibly empty) - the
order of replies is kept. Watching is therefore best done in its own session
(`CONNECT`).

## Trash

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Trash">
<defs><marker id="hk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="30" width="200" height="50" rx="6"/><text class="t" x="120" y="52" text-anchor="middle">/documents/report.docx</text><text class="m" x="120" y="70" text-anchor="middle">oid 812</text>
<path class="ln" d="M220 55 L330 55" marker-end="url(#hk1)"/><text class="m" x="275" y="45" text-anchor="middle">trash</text>
<rect class="warn" x="332" y="20" width="200" height="70" rx="6"/><text class="t" x="432" y="42" text-anchor="middle">TRASH_OID (oid 2)</text><text class="m" x="432" y="60" text-anchor="middle">entry "17" -> oid 812</text><text class="m" x="432" y="76" text-anchor="middle">parent 0, outside the root</text>
<rect class="acc" x="560" y="20" width="180" height="70" rx="6"/><text class="t" x="650" y="42" text-anchor="middle">TRASH record 17</text><text class="m" x="650" y="60" text-anchor="middle">original parent, name</text><text class="m" x="650" y="76" text-anchor="middle">time, who, size</text>
<text class="m" x="20" y="125">Nothing is copied: the object is only moved. Open handles keep working.</text>
<text class="m" x="20" y="147">Restore = move back; purge = delete the subtree (one REMOVE record).</text>
</svg>
<figcaption>The trash is a hidden dataset directory with a fixed oid; the record remembers where the object came from.</figcaption>
</figure>

- **`trash(path)`** moves a file or a whole folder into the trash of its
  dataset (the `DELETE` right, like `remove`). Plain `remove` stays
  permanent - compilers and temporary files do not fill the trash. A
  dataset cannot be trashed (`WRONG_TYPE`).
- **`trash_list(dataset)`** - newest first; everyone sees only their own
  entries and entries of objects they own, an administrator (`MANAGE`) sees
  everything.
- **`trash_restore(dataset, id, dest)`** - to the original place
  (`dest = None`) or elsewhere in the same dataset. Name taken =
  `ALREADY_EXISTS`, the original folder is gone = `NOT_FOUND`.
- **`trash_purge(dataset, id)`** - permanent deletion of the subtree; before
  the first change it checks that it contains no dataset (the operation is
  all or nothing).
- **`trash_tick(now)`** (service, every minute) purges entries older than
  `trash_max_age` (30 days by default) and, when free space is below 10 %,
  also further oldest ones, at most 64 per step.
- The trash has its own subtree summary - it does not count toward the size
  of the root.
- All trash operations go through the intent log (`Trash`, `Restore`,
  `Purge`), so `fsync` preserves them across a crash.

## Extended attributes and expiry

Attributes are generic data stored atomically with the object (the `XATTR`
item, offset = hash of the name; several attributes with the same hash share
the value). The FS only stores them and does not interpret them - settings
of services and applications belong here, not in format bits.

| Namespace | Who changes it | Example |
|---|---|---|
| `user.*` | applications and the user (`READ` reads, `WRITE` changes) | `user.color`, `user.label` |
| `sys.*` | only the system or the FS's own API | `sys.expires` |

Limits: name up to 255 B, value up to 4 KiB, the attribute count is in the
object record (fsck checks it).

**Expiry** (`set_expires(path, Some(time))`, the `WRITE` right): after the
date, `expire_tick` (service, every minute) moves the object into the trash
and deletes the attribute. So that the step does not have to walk the tree,
the dataset has an **index by time** (the `EXPIRY` item: key = seconds,
value = a list of (oid, generation)); the attribute and the index always
change together and fsck checks that they match. An object restored from
the trash no longer has an expiry.

## File versions and undo of operations

- **`file_versions(path)`** - versions of a file in the snapshots of its
  dataset, oldest first. The object is looked up in a snapshot **by oid**, so
  a rename or move in between does not matter. Each version (the content
  change counter `Revision::version`) appears once, with the oldest snapshot
  that contains it.
- **`restore_version(path, snapshot, mode)`** - `Copy(target)` = a copy next
  to the original; `Replace` = the restored copy gets the original's name
  and the current version goes **to the trash** (nothing is lost; the
  restored file has a new oid).
- **`undo(dataset, seq)`** reverts a journal write operation:

| Operation | Undo |
|---|---|
| `RENAME` | move back (the object must be where it went; otherwise `CONFLICT`) |
| `TRASH` | restore from the trash to the original place |
| `CREATE`, `RESTORE` | object to the trash |
| `REMOVE` | a copy from the last snapshot before the operation, to the original place |
| `CONTENT` | content from the last snapshot before the operation; the overwritten version goes to the trash |
| `ATTR`, `SNAPSHOT` | `NOT_SUPPORTED` |

  "Snapshot before the operation" = the newest snapshot whose `journal_seq`
  is lower than the operation number and which contains the object; without
  one, `NOT_FOUND`. That is why a user's home should have automatic
  snapshots.

## Automatic snapshots

The rules are dataset settings (`Settings::rules`, at most 8): "keep the
newest snapshot in each of the last `keep` intervals of length `interval`"
(interval at least 60 s). For example:

```rust
fs.set_settings("/home/alice", &Settings {
    rules: vec![
        AutoRule { interval: 300, keep: 12 },        // 5 minutes, 1 hour
        AutoRule { interval: 3600, keep: 24 },       // hourly, 1 day
        AutoRule { interval: 86_400, keep: 30 },     // daily, 30 days
        AutoRule { interval: 7 * 86_400, keep: 26 }, // weekly, half a year
    ],
    ..Settings::default()
})?;
```

`auto_snapshot_tick(now)` (service, every minute) is a deterministic step:

1. a new snapshot `auto-YYYY-MM-DD-HHMMSS` (kind `AUTO`) when the shortest
   interval has passed **and the dataset has changed** (since the last
   automatic snapshot the journal has a record other than `SNAPSHOT`, or
   there are unconfirmed changes),
2. pruning: deletes automatic snapshots that no rule keeps (the newest
   always stays). It does not delete manual snapshots and commits; a
   snapshot with a clone is only hidden.

Default: **no rules** (they are set by the home installation or the system
settings). Rules for a "point before an attack" (5 minutes / 1 hour) are
proposed by [ransomware protection](fs-v3-design.md#ransomware-protection-behaviour-detection).

## Commits and logical ID

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Commits and branches">
<defs><marker id="hc1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<circle class="acc" cx="60" cy="60" r="18"/><text class="t" x="60" y="65" text-anchor="middle">C1</text>
<circle class="acc" cx="200" cy="60" r="18"/><text class="t" x="200" y="65" text-anchor="middle">C2</text>
<circle class="box" cx="200" cy="140" r="18"/><text class="t" x="200" y="145" text-anchor="middle">B1</text>
<circle class="ok" cx="360" cy="60" r="18"/><text class="t" x="360" y="65" text-anchor="middle">M</text>
<path class="ln" d="M182 60 L80 60" marker-end="url(#hc1)"/>
<path class="ln" d="M184 130 L76 70" marker-end="url(#hc1)"/>
<path class="ln" d="M342 60 L220 60" marker-end="url(#hc1)"/>
<path class="ln" d="M344 70 L216 132" marker-end="url(#hc1)"/>
<rect class="box" x="430" y="30" width="310" height="44" rx="6"/><text class="t" x="585" y="50" text-anchor="middle">/project: HEAD -> M</text><text class="m" x="585" y="66" text-anchor="middle">commits = snapshots of kind COMMIT</text>
<rect class="warn" x="430" y="118" width="310" height="44" rx="6"/><text class="t" x="585" y="138" text-anchor="middle">/project-refactor = clone of C1 (branch)</text><text class="m" x="585" y="154" text-anchor="middle">own dataset, commits B1...</text>
<text class="m" x="20" y="190">Arrows point to parents; M has two (merge). Merge base = common ancestor C1.</text>
</svg>
<figcaption>History is a DAG of commits; a branch is a writable clone, merge is three-way.</figcaption>
</figure>

A **commit** (`commit_version(dataset, message)`, the `MANAGE` right) = a
snapshot of kind `COMMIT` + author, message (up to 4 KiB), time, parents
(the dataset's last commit and possibly the second parent of a merge in
progress) and a **logical ID**. The snapshot name is `c-` + the first 12 hex
characters of the ID. `head(dataset)` and `log(dataset, max)` read the
history (log = all ancestors, newest first; a commit whose snapshot no
longer exists ends that branch of the history).

The **logical ID** = BLAKE3 of an unambiguous representation of the logical
tree:

```text
file     = H("f" | size | H(content) | metadata)
link     = H("l" | target | metadata)
folder   = H("d" | count | (name, kind, child hash)* sorted by name bytes | metadata)
metadata = mtime, btime, owner, permissions (canonical, without descriptor number), extended attributes
commit   = H("osfs-commit-v1" | tree | parents | author | time | message)
```

It does not depend on block placement, oids, generations, ctime or atime -
the same tree on another volume has the same ID (verified by the test
`logical_id_is_layout_independent`). A nested dataset counts only by its
name (it has its own history).

**Cost:** ordinary snapshots and the commits every 5 s do not compute the
ID, only a versioned commit does. The hash of a file's content is
remembered (the `LHASH` item: content version, generation, hash) and is
recomputed only after a change; folders are walked in full (cost by object
count, not data). The walk has no recursion (the service has a small
stack).

## Branches and merge

- **`branch(dataset, name, path)`** - a writable clone of the last commit at
  `path` (its own dataset, instant); the source dataset remembers the branch
  (the `BRANCH` item). The dataset must have a commit (`BAD_STATE`).
  `branches`, `delete_branch` (also deletes the clone).
- **`merge(target, source)`** - merges the source's last commit into the
  live version of the target, three-way against the **common ancestor** in
  the commit DAG:

| Situation (base B, ours O, theirs T) | Result |
|---|---|
| change only in T | taken over |
| change only in O | stays |
| the same change in O and T | once |
| text file changed in O and T | line-based merge (diff3); on conflict the markers `<<<<<<< nase` / `=======` / `>>>>>>> branch` |
| binary file changed in O and T | ours stays, theirs next to it as `name (konflikt branch).ext` |
| deleted in T, unchanged in O | to the trash |
| deleted in T, changed in O | conflict, ours stays |
| deleted in O, changed in T | conflict, their version is restored |
| move in T | move (also into new folders); both elsewhere = conflict, our place |
| new name on both sides | same content = nothing, otherwise the second gets "(konflikt ...)"; a new folder on both sides is merged |
| folder deleted in T, something left in it in O | conflict, stays |

  Objects are paired by **oid and generation**, but only those that existed
  in B (a clone continues with the same oids). New objects on both sides may
  have the same oid and still be different objects - they are paired by
  path. Text = valid UTF-8 without NUL, up to 4 MiB.

  Without conflicts a **merge commit** with two parents is made at once.
  With conflicts the changes stay in the live version and `HEAD` remembers
  the second parent; the next `commit_version` (after resolving) gets it.
  `merge_abort` cancels a merge in progress (the changes stay).

**Procedure:** first the whole plan is computed **without a single change**
(indexes of the three trees, simulation of names in the target folders, a
cycle check for moves, text merging, a space estimate), then it is carried
out: trash, new objects, moves through a helper folder (name swaps are
fine), content; finally the commit. An error before the first change puts
nothing at risk; an error in the middle (e.g. a bad block) switches the
volume to the error state like every all-or-nothing operation - after the
next mount the state before the merge holds.

## Try changes aside

```rust
fs.aside_begin("/project", "/project-try")?;        // ASIDE snapshot + clone
// the application (agent, installer) gets a capability to the clone only
let changes = fs.diff_clone("/project-try")?;       // what was added, removed, changed
let r = fs.aside_apply("/project-try", Some(&["/src".into()]))?;
for c in &r.conflicts { println!("not taken over: {}", c.path); }
fs.aside_discard("/project-try")?;                  // clone and snapshot in one step
```

- `aside_apply(clone, selection)` takes over everything (`None`) or only the
  selected paths (inside the dataset, including the subtree). It is the same
  three-way merge (base = the `ASIDE` snapshot, ours = the original, theirs
  = the live clone), but **a conflict is not taken over at all** - it is
  only reported; the file manager then offers a merge with markers or saving
  alongside. Taking over is atomic (a commit).
- Taking over can be repeated (e.g. `/src` first, then the rest).
- Deleted files go to the original's trash.

## On disk

| Kind | Key | Value |
|---|---|---|
| 13 `JOURNAL` | (META, 13, number) | journal record: operation, kind, time, who, application, oid, generation, parents, size, extra, names |
| 14 `JSTATE` | (META, 14, 0) | next number, oldest stored |
| 15 `SNAPMETA` | (META, 15, snapshot) | kind (manual, automatic, commit, view, aside), time, journal position, logical ID, author, parents (ID + dataset + snapshot), message |
| 16 `SETTINGS` | (META, 16, 0) | journal retention (count, age), trash age, snapshot rules |
| 17 `TRASH` | (META, 17, 0) | counter of the next trash number |
| 17 `TRASH` | (META, 17, number) | oid, generation, original parent, time, kind, who, size, name |
| 18 `XATTR` | (oid, 18, name hash) | list of (name, value) |
| 19 `EXPIRY` | (META, 19, seconds ^ 2^63) | list of (oid, generation) |
| 20 `LHASH` | (oid, 20, 0) | content version, generation, BLAKE3 of the content |
| 21 `BRANCH` | (META, 21, name hash) | list of (dataset, name) |
| 22 `HEAD` | (META, 22, 0) | last commit (ID, dataset, snapshot), which dataset it applies to, merge in progress |
| 23 `DSLINK` | (META, 23, 0) in the dataset's own tree | where the dataset is mounted: parent dataset, folder, name (the dataset path without walking the tree; updated on creation, clone and move) |

The fixed object **`TRASH_OID` = 2**: the trash directory (parent 0). A
clone inherits the origin's `HEAD` in its tree - the record has a "for
dataset" field, and when it does not match, the clone's last commit is its
origin snapshot.

**The `RO_COMPAT_HISTORY` flag** (ro_compat bit 1): new volumes have it from
formatting, an older volume gets it on the first writable mount. An older
driver may only read such a volume - when writing it would not maintain the
journal or the indexes.

**Intent log:** new records 14 `Actor` (who performed the following
operations - the journal comes out the same after replay), 15 `Trash`,
16 `Restore`, 17 `Purge`, 18 `SetXattr`. Operations on commits, branches,
merge and settings commit immediately.

**fsck** additionally checks: a contiguous journal sequence `first..next`
and its state, metadata only for existing snapshots, trash record <-> entry
in the trash directory and the counter, the attribute count in the object
record, that the expiry index matches the attributes (oid + generation),
the hash cache only for files, and that settings, branches and `HEAD` are
readable.

## Protocol (operations 54-86)

Module `fsproto::hist`; paths are session paths, the FS checks rights by the
session identity. Time on the wire = seconds i64 + nanoseconds u32.

| op | Name | Parameters | Result |
|---|---|---|---|
| 54 | `JOURNAL_READ` | dataset, from u64, at most u32 | oldest u64, last u64, count u32 + records |
| 55 | `JOURNAL_WATCH` | dataset, from, at most, limit ms u32 | like 54; waits for a record or the limit |
| 56 | `VIEW` | dataset | snapshot, id u64, last record u64 |
| 57 | `VIEW_RELEASE` | dataset, snapshot | - |
| 58 | `TRASH` | path | entry number u64 |
| 59 | `TRASH_LIST` | dataset | count + (id, kind, oid, time, who, size, name, original folder) |
| 60 | `TRASH_RESTORE` | dataset, id, target (empty = original) | path |
| 61 | `TRASH_PURGE` | dataset, id | - |
| 62 | `UNDO` | dataset, record number | kind u8 (1 move back, 2 from trash, 3 to trash, 4 from snapshot), path, snapshot, trash number |
| 63 | `FILE_VERSIONS` | path | count + (snapshot, id, time, size, mtime, version) |
| 64 | `RESTORE_VERSION` | path, snapshot, replace u8, target | path |
| 65 / 66 | `SETTINGS` / `SET_SETTINGS` | dataset [, settings] | settings: 3x u64, count + (interval u64, keep u32) |
| 67 | `COMMIT_VERSION` | dataset, message | commit |
| 68 | `LOG` | dataset, at most | count + commits |
| 69 | `BRANCH` | dataset, name, path | branch dataset u64 |
| 70 | `BRANCHES` | dataset | count + (name, dataset, path) |
| 71 | `DELETE_BRANCH` | dataset, name | - |
| 72 | `MERGE` | target, source | merge report |
| 73 | `MERGE_ABORT` | target | - |
| 74 | `ASIDE_BEGIN` | dataset, clone path | clone dataset u64 |
| 75 | `ASIDE_APPLY` | clone, all u8, count + paths | merge report |
| 76 | `ASIDE_DISCARD` | clone | - |
| 77 | `ASIDE_CHANGES` | clone | count + (kind u8, path, old path, oid, folder u8) |
| 78-81 | `XATTR_GET` / `SET` / `REMOVE` / `LIST` | path, name [, value = rest of the message] | value / - / - / names |
| 82 / 83 | `SET_EXPIRES` / `EXPIRES` | path [, has u8, time] | has u8, time |
| 84 | `HEAD` | dataset | has u8 + commit |
| 85 | `DATASET_CREATE` | path | dataset number u64 (a new folder with its own history; `CREATE` right in the parent) |
| 86 | `DATASET_DELETE` | path | - (including snapshots; `MANAGE` + `DELETE`; not with nested datasets or clones) |

A commit on the wire: ID (32 B), snapshot, dataset u64, snapshot id u64,
time, author u64, message, count u32 + parent IDs. A merge report: paths
taken over, conflicts (path, kind u8: 1 content, 2 ours changed / theirs
deleted, 3 ours deleted / theirs changed, 4 added on both sides, 5 moved on
both sides, 6 folder missing, 7 folder not empty, 8 cycle; where their
version was saved) and optionally the merge commit. Lists have at most 4096
items.

**The fsd service** calls `auto_snapshot_tick`, `expire_tick` and
`trash_tick` every minute (as the system) - not on a read-only volume.

## Risks and limitations

- ⚠️ **Undoing content changes and deletions needs a snapshot before the
  operation.** Without automatic snapshots `undo` of `REMOVE`/`CONTENT`
  returns only `NOT_FOUND` - the journal remembers *what* happened, not the
  old content.
- ⚠️ **Journal retention is a trade-off:** a subscriber that has not been
  connected for a long time gets `JOURNAL_GAP` and must walk the dataset
  again (`VIEW`). A large limit costs space (~100-200 B per record).
- ⚠️ **The logical hash walks all folders** on every versioned commit (file
  contents only after a change). For a dataset with millions of objects a
  commit takes time proportional to the object count. An incremental hash
  in folders is a possible later optimisation.
- ⚠️ **A clone of a dataset with nested datasets** would contain a second
  path to the same datasets: `branch` and `aside_begin` refuse such a
  dataset (`NOT_SUPPORTED`).
- **Version restore and merge copy data** (instant copies via shared blocks
  come with phase 3).
- **Merge** pairs new objects by path; the common ancestor is the first one
  found in the DAG (for criss-cross merges it may not be the best).
  Permission and attribute conflicts are not merged - new objects get the
  caller's default owner.
- **`WATCH` waits for confirmation** - latency up to 5 s (the commit
  interval) unless the client calls `fsync`.
- **The application in a journal record** is 0 for now (application identity
  in the session comes with the session manager).

## Tests

| Test | What it verifies |
|---|---|
| `tests/journal.rs` | records of operations, merging of writes, publishing after confirmation, **the same numbers and users after a crash and log replay**, retention and `JOURNAL_GAP`, a view with a position and cleanup after a crash, rights, datasets and snapshots in the journal |
| `tests/trash.rs` | trash: move without a copy, restore (also to another path), purge and freeing space, an open handle, through the intent log after a crash, cleanup by age, visibility and rights; attributes and namespaces; expiry (index, cancelling, crash) |
| `tests/history.rs` | file versions across a rename, restore as a copy and by replacing, `undo` of all kinds, automatic snapshots and pruning, settings; diff3 (also a property test) |
| `tests/commits.rs` | commits and log, **logical ID independent of block placement**, a branch and a clean merge, text and binary file conflicts, "aside" with a selection and a conflict, new folders and name swaps |
| init `tests/fs_hist.rs` | in the system through fsd: journal and `JOURNAL_WATCH` with a limit, view, trash, commit, branch and merge, "aside", attributes and expiry, fsck |
