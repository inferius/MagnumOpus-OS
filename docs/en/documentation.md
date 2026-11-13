# Writing documentation

Rules that keep the documentation in line with the code: the goal clearly
separated from the current state, one page as the source of truth for
each thing, claims backed by tests and measurements with their
conditions.

## Status marks

Every property where the goal and the current state can differ carries a
mark right next to it (in the text or in a table cell):

| Mark | Meaning | What justifies it |
|---|---|---|
| **[designed]** | a described goal, no code yet | the design is agreed |
| **[implemented]** | the code is in `main`, an automated test is missing or runs only on the host | a commit in `main` |
| **[verified in QEMU]** | a test in `cargo xtask test` passes (x86_64 and aarch64 where both apply) | the test name next to the claim |
| **[verified on HW]** | tried on a real machine | machine, build revision, date |

A mark applies to the nearest claim, not to the whole chapter. When the
code changes, the mark changes in the same commit. The Czech version uses
`[navrženo]`, `[implementováno]`, `[ověřeno v QEMU]`, `[ověřeno na HW]`.

Example:

```markdown
- `MemoryObject`: pages are allocated up front **[implemented]**;
  copy-on-write clones **[designed]**.
- The compositor survives a display driver crash **[verified in QEMU]**
  (test `gui: compd: pad ovladace displeje`).
```

## One source of truth

Each thing has **one owning page** (the display driver
[Display configuration](display.md), Venus [Vulkan over Venus](venus.md),
the scheduler [Scheduler](scheduler.md) and so on). Only there are the
details, numbers and status.

- **Summary tables** (architecture, GUI status, indexes) give one sentence,
  a status mark and a **link** to the owning page. They do not repeat
  details: those go stale first.
- **Numbers are not repeated.** Syscalls, rights, limits and sizes live in
  [Syscalls](syscalls.md) or next to the code; elsewhere only a link.
- **Milestone status** is only in [Architecture](architecture.md)
  (chapter Current deviations from the design) and on the part's page.
- Whoever changes behaviour updates the owning page and goes through the
  links to it (`grep` the page name in `docs/`) to check the summaries
  still hold.

## Claims and tests

An important claim (security, crash recovery, performance, on-disk
format) links to the test that verifies it: the test name in
`cargo xtask test` or `cargo test -p <package> <test>`. A claim without a
test is at most **[implemented]**.

## Measurements

Every measured number states:

- the **build revision** (the commit from the `sestaveni <commit> (<date>)`
  line in the kernel log), `*` means uncommitted changes,
- the **profile** (debug, release),
- the **environment**: host, QEMU with KVM/WHPX or without acceleration
  (TCG), VMware, a real machine (model),
- the **workload size** (resolution, number of files, data size).

Example: "compositing 1280x800: 2.1 ms (release, `b24c01f`, QEMU + KVM,
Ryzen 7 5800X)". Numbers without conditions are not compared.

## Check before merging

- status marks match the code in this branch,
- the owning page is current, summaries only link,
- new claims have a test, new numbers have conditions,
- the cs and en versions say the same,
- the folder's `CLAUDE.md` (agent journal) matches the change: update it when the folder's invariants or key files change,
- `cargo xtask docs` passes. Besides links and anchors it checks the status
  marks (an unknown mark is an error) and prints warnings: a measurement
  without a build revision, cs and en versions with a different number of
  chapters. The audit after each merge batch gradually removes the
  warnings left in older pages.
