# Process memory management

How the kernel gives memory to processes: memory objects, demand paging,
the zero page, budgets and pins. This is step 3 in the order of adopting
third-party software
([architecture.md](architecture.md), chapter on the order of adopting third-party software):
the basis for `mmap` in the C library, for Wine and for mapping files
from fsd later. The address space layout and protections (SMAP, NX) are
in [kernel.md](kernel.md#memory), syscalls in [syscalls.md](syscalls.md).

## Status by part

| Part | What | Status |
|---|---|---|
| 1 | demand paging, zero page, faults in the kernel, commit and ceiling, pins | **[verified in QEMU]** x86_64 and aarch64 (kernel self-tests `pamet`, the `pamet` suite in init) |
| 2 | reservations (`VM_RESERVE`), partial `VM_UNMAP` and decommit, `VM_PROTECT`, `VM_QUERY`, guard regions | **[verified in QEMU]** x86_64 and aarch64 (self-tests `mm_regions`, suite `pamet`) |
| 3 | `VMO_CLONE` (copy-on-write) and shared program code | **[designed]** |
| 4 | `VMO_SEAL`, W^X with a "may JIT" permission, audit | **[designed]** |
| 5 | libos API (basis of `mmap` in `cstd`), guards below stacks | **[designed]** |

## Demand paging

A memory object (`VMO_CREATE`) **commits** its whole size at creation but
takes physical frames only on the first write:

- **The commit** is charged twice right away: to the job's user memory
  budget (as before) and to the global ceiling of memory for programs
  (`phys::commit`). Ceiling = free RAM minus the kernel reserve minus pages
  already committed. Whatever cannot be committed fails at creation
  (`NO_MEMORY`).
- **Hence never an OOM killer:** a page fault takes its frame from the
  commit, so it never runs out of memory. The only exception is the kernel
  itself: if its heap and page tables used more than their reserve, the
  fault ends the process with a message in the log.
- **Reading** a page that does not exist yet returns zeros and creates no
  frame.
- **Mapping** (`VM_MAP`) maps only pages that already exist. A fault fills
  in the rest.

<figure>
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Page fault">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.warn{fill:#fbefe6;stroke:#a0602a;stroke-width:1.2}
.t{font:14px sans-serif;fill:#1d2333}
.m{font:12px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}.warn{fill:#33261c;stroke:#d39a63}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="mm1" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="56" rx="6"/><text class="t b" x="110" y="44" text-anchor="middle">fault in userspace</text><text class="m" x="110" y="63" text-anchor="middle">address + read/write/exec</text>
<rect class="box" x="280" y="20" width="200" height="56" rx="6"/><text class="t b" x="380" y="44" text-anchor="middle">mapping and rights</text><text class="m" x="380" y="63" text-anchor="middle">under the space lock</text>
<rect class="warn" x="550" y="20" width="200" height="56" rx="6"/><text class="t b" x="650" y="44" text-anchor="middle">process ends</text><text class="m" x="650" y="63" text-anchor="middle">outside a mapping / no right</text>
<rect class="acc" x="130" y="150" width="240" height="70" rx="6"/><text class="t b" x="250" y="174" text-anchor="middle">read / exec</text><text class="m" x="250" y="193" text-anchor="middle">page exists -> its frame</text><text class="m" x="250" y="210" text-anchor="middle">otherwise the read-only zero page</text>
<rect class="acc" x="400" y="150" width="240" height="70" rx="6"/><text class="t b" x="520" y="174" text-anchor="middle">write</text><text class="m" x="520" y="193" text-anchor="middle">frame from the commit (commit_page)</text><text class="m" x="520" y="210" text-anchor="middle">zero page removed (BBM, shootdown)</text>
<rect class="box" x="400" y="245" width="240" height="44" rx="6"/><text class="m" x="520" y="263" text-anchor="middle">unmap the zero page from</text><text class="m" x="520" y="279" text-anchor="middle">other mappings (forget_zero)</text>
<line class="ln" x1="210" y1="48" x2="276" y2="48" marker-end="url(#mm1)"/>
<line class="ln" x1="480" y1="48" x2="546" y2="48" marker-end="url(#mm1)"/>
<line class="ln" x1="350" y1="76" x2="260" y2="146" marker-end="url(#mm1)"/>
<line class="ln" x1="410" y1="76" x2="510" y2="146" marker-end="url(#mm1)"/>
<line class="ln" x1="520" y1="220" x2="520" y2="241" marker-end="url(#mm1)"/>
<text class="m" x="10" y="120">Once resolved, the instruction runs again.</text>
</svg>
<figcaption>A page fault: a read gets the zero page, a write its own frame.</figcaption>
</figure>

## Regions: reservations, parts, rights

The address space consists of **regions**: a mapping of a memory object or
a **reservation** (addresses taken without memory). An operation on a range
inside a region first splits it at the boundaries; each piece has its own
record in the job's kernel budget.

| Operation | What it does | For whom |
|---|---|---|
| `VM_RESERVE` | takes addresses without memory; costs only a record (no page tables) | Wine (`VirtualAlloc(MEM_RESERVE)`), large arenas, stacks |
| `VM_MAP` + `MAP_IN_RESERVATION` | maps an object only into a reservation (with `MAP_FIXED`) | `MEM_COMMIT` |
| `VM_UNMAP` | removes also a part of a region; `UNMAP_DECOMMIT` = mapping back to a reservation | `munmap`, `MEM_DECOMMIT`, `MEM_RELEASE` |
| `VM_PROTECT` | rights in a range, 0 = no access; never W+X, never above the rights at `VM_MAP` | `mprotect`, `VirtualProtect` |
| `VM_QUERY` | the region or free range at an address | `VirtualQuery`, memory listings |

- **A guard region** (`RESERVE_GUARD`): an access ends the process with
  the message "access to a guard region (stack overflow?)". Delivering the
  exception to the program (Windows `PAGE_GUARD`, SEH) comes with step 5 of
  the adoption order. Guards below libos stacks are part 5.
- **Adding execute** (`VM_PROTECT` with X above the original rights) is not
  possible today; it comes with the "may JIT" permission (part 4).
- **Reservations without page tables:** page tables are charged only for
  mappings (an estimate by length at `VM_MAP`), so reserving tens of GiB
  costs hundreds of bytes.

## The zero page

One shared zeroed page (`mm::zero_frame`), always mapped **read-only**
in place of pages that do not exist yet. A program that takes a large
buffer and only reads from it uses no memory.

Once the page comes into existence, nobody may keep reading the old zero
page instead of the written data:

- **a write fault** in the same mapping replaces the zero page with its own
  frame (break-before-make: on aarch64 `unmap_4k` invalidates the TLB of
  all CPUs at once, on x86_64 a shootdown completes before the thread
  returns);
- **any other way the page comes into existence** (a write through another
  mapping or another process, `VMO_WRITE`, a kernel copy into process
  memory, `VMO_PHYS_ADDR`) unmaps the zero page from every process mapping
  the object (`MemoryObject::forget_zero` -> `AddressSpace::drop_zero_ptes`).

**Race:** a read fault sets the object's "a zero page is mapped somewhere"
flag before it checks that the page does not exist; creating the page (a
CAS on the entry) reads the flag only afterwards. Whoever mapped the zero
page before the page was created is found by `forget_zero`, which takes
that process's address space lock. Whoever came after saw the page
already existing.

## Kernel copies

The kernel never touches process memory through process addresses (SMAP
stays on), so no fault happens in the kernel:

- **reading** (`copy_from_user`) a page that does not exist yet reads the
  zero page;
- **writing** (`copy_to_user`) first creates the pages of the range and
  unmaps the zero page from them, and only then copies.

## Pins and DMA

The pin counter array has one entry per RAM frame (`phys::pin`, `unpin`,
`is_pinned`). A pinned frame is used by a device: it must not be moved,
replaced by a copy (future CoW) or evicted (future pager).

- **DMA memory** (`VMO_CREATE_CONTIGUOUS`) is contiguous, fully present
  right away and pinned for the object's whole life. Demand paging and the
  zero page do not apply to it.
- **`VMO_PHYS_ADDR` on a regular object** creates the page and pins it. The
  address a driver gives a device thus stays valid while the object lives.

## Budgets

| What | Charged when | From |
|---|---|---|
| object pages | at creation (whole size) | job user memory + global ceiling |
| a page's frame | on the first write | from the commit (nothing extra) |
| page list, the object | at creation | job kernel memory |
| page tables of a mapping | at `VM_MAP` (estimate by length) | job kernel memory |

The commit comes back when the object goes away (pages that never came
into existence) or with the frame. **Later** (noted as an option): a
`SPARSE` flag for sparse objects (`malloc` arenas in `cstd`) whose commit
would be charged only on a fault.

## Key files

| File | What |
|---|---|
| `kernel/src/mm/phys.rs` | commit and ceiling (`commit`, `uncommit`, `alloc_committed_zeroed`), the pin array |
| `kernel/src/object/vmo.rs` | page entries (0 = does not exist), `commit_page`, `forget_zero`, `expose_phys` |
| `kernel/src/mm/address_space.rs` | mapping only existing pages, kernel copies, `drop_zero_ptes` |
| `kernel/src/mm/address_space/fault.rs` | fault resolution (`resolve_fault`) |
| `kernel/src/arch/*/` | fault decoding (x86_64 error code, aarch64 ESR) -> `trap::user_page_fault` |

## Tests

- Kernel self-tests (suite `pamet`, `kernel/src/tests/mm_lazy.rs`): commit
  without frames, the commit ceiling, read and write faults, rights, zero
  page coherence across mappings and processes, kernel copies, pins.
- Userspace tests (`cargo xtask test --only pamet`,
  `user/init/src/tests/memory_lazy.rs`): a 256 MiB object, the zero page
  against a write through another mapping and `VMO_WRITE`, 4 threads on
  the same pages, a commit above RAM.
