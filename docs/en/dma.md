# Scatter-gather DMA

A driver often has to give a device a large buffer:

- a GPU: hundreds of megabytes (GTT),
- a network card: packet rings,
- a video decoder: an application's frames.

Contiguous DMA memory (`VMO_CREATE_CONTIGUOUS`) is at most 4 MiB and
fragments over time. **Scatter-gather DMA** therefore gives a device
non-contiguous pinned pages instead:

- with an IOMMU the device sees them as **one contiguous IOVA range**,
- without an IOMMU it sees a list of physical segments.

This is step 6a-2 of the foundations for graphics and video
([gpu-amd-nvidia.md](gpu-amd-nvidia.md#impact-on-the-foundations),
item 4).

See also:

- syscall numbers and arguments: [syscalls.md](syscalls.md#scatter-gather-dma),
- the IOMMU and DMA quarantine: [kernel.md](kernel.md#iommu-dma-protection),
- the driver API: [drivers.md](drivers.md).

## Status

| Part | What | Status |
|---|---|---|
| pinned memory | `VMO_CREATE` with the `VMO_PINNED` flag: all pages at once, pinned, in the job's `DMA` budget (mandatory root ceiling) | **[verified in QEMU]** x86_64 and aarch64 (suite `dma`, kernel self-test `DMA: pripnuta pamet ...`) |
| mapping | `DMA_MAP`, `DMA_SEGMENTS`, `DMA_SYNC`; the `DMA` right on the memory object handle; IOVA allocator (`libs/iova`) with a guard page and a hint | **[verified in QEMU]** x86_64 (VT-d), aarch64 (SMMUv3) and without an IOMMU (suites `dma`, `ovladace z manifestu`: edu `sg`) |
| unmapping | closing the handle = unmapping with a synchronous IOTLB flush; a device access to the old address is blocked by the IOMMU and logged by the kernel | **[verified in QEMU]** x86_64 (VT-d) and aarch64 (SMMUv3) (test `edu: pristup po odmapovani zablokovan`) |
| quarantine | after a driver ends, stop the device first, then unmap | **[implemented]** (same path as contiguous memory; not tested separately) |
| driverkit | `Platform::alloc_dma_pages`, `DmaBuffer::device_chunks`, `DmaBuffer::is_protected` | **[verified in QEMU]** (edu driver) |
| AMD-Vi | mapping at an IOVA with direction rights | **[implemented]** (QEMU `--iommu amd` not tested) |
| devices with 32-bit addresses | the IOVA window is above 4 GiB | **[designed]** (contiguous memory only today) |

## Model

<figure>
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Scatter-gather DMA">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.bad{fill:#fbecec;stroke:#a04040;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
</style>
<defs><marker id="dm1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L10,5 L0,10 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/>
<text class="t b" x="110" y="44" text-anchor="middle">VMO_PINNED (3 pages)</text>
<text class="m" x="110" y="62" text-anchor="middle">physical: 0x7a000, 0x31000, 0x5c000</text>
<text class="m" x="110" y="78" text-anchor="middle">job budget USER + DMA</text>
<path class="ln" d="M210 55 L278 55" marker-end="url(#dm1)"/>
<text class="m" x="244" y="46" text-anchor="middle">DMA_MAP</text>
<rect class="acc" x="280" y="10" width="220" height="90" rx="6"/>
<text class="t b" x="390" y="34" text-anchor="middle">device domain (IOMMU)</text>
<text class="m" x="390" y="54" text-anchor="middle">IOVA 0x1_0000_0000 .. +3 pages</text>
<text class="m" x="390" y="70" text-anchor="middle">window above the end of RAM</text>
<text class="m" x="390" y="86" text-anchor="middle">+ guard page after the range</text>
<path class="ln" d="M500 55 L568 55" marker-end="url(#dm1)"/>
<rect class="box" x="570" y="20" width="180" height="70" rx="6"/>
<text class="t b" x="660" y="44" text-anchor="middle">DMA_SEGMENTS</text>
<text class="m" x="660" y="62" text-anchor="middle">[(0x1_0000_0000, 12 KiB)]</text>
<text class="m" x="660" y="78" text-anchor="middle">no IOMMU: 3 physical segments</text>
<rect class="box" x="10" y="150" width="200" height="60" rx="6"/>
<text class="t b" x="110" y="174" text-anchor="middle">closing the handle</text>
<text class="m" x="110" y="192" text-anchor="middle">(or the driver ends)</text>
<path class="ln" d="M210 180 L278 180" marker-end="url(#dm1)"/>
<rect class="acc" x="280" y="140" width="220" height="80" rx="6"/>
<text class="t b" x="390" y="164" text-anchor="middle">unmapping</text>
<text class="m" x="390" y="182" text-anchor="middle">entries gone + IOTLB synchronously</text>
<text class="m" x="390" y="198" text-anchor="middle">then IOVA back to the allocator,</text>
<text class="m" x="390" y="212" text-anchor="middle">then the memory reference</text>
<path class="ln" d="M500 180 L568 180" marker-end="url(#dm1)"/>
<rect class="bad" x="570" y="150" width="180" height="60" rx="6"/>
<text class="t b" x="660" y="174" text-anchor="middle">device on the old IOVA</text>
<text class="m" x="660" y="192" text-anchor="middle">blocked, kernel log</text>
<text class="m" x="10" y="260">Only a driver with a device resource may map; the memory object needs the DMA right (and READ/WRITE by direction).</text>
<text class="m" x="10" y="280">After a driver crash the mapping goes to quarantine: the kernel turns off bus mastering first, then unmaps.</text>
</svg>
<figcaption>Pinned memory, mapping into the device domain and unmapping.</figcaption>
</figure>

### Pinned memory

`VMO_CREATE` with the `abi::vm::VMO_PINNED` flag (`libos::Vmo::create_pinned`)
creates an object whose pages exist at once and are **pinned** for the
object's whole life. The kernel never moves a pinned page, never replaces
it with a copy (CoW) and never evicts it, because a device may be writing
to it. Rules:

- The pages are charged to the **user memory** budget and also to the
  **pinned memory** budget `DMA` (`abi::job::pool::DMA` = 2).
- **The quota is mandatory.** The root job has a ceiling of half of the
  RAM the allocator manages. `JOB_SET_LIMIT` can only lower it, never
  raise it, so pinned memory never takes the whole system. Child jobs
  inherit the limits of their ancestors; the parent may set their own.
- Contiguous DMA memory (`VMO_CREATE_CONTIGUOUS`) counts towards the `DMA`
  budget too: it is pinned as well.
- A pinned object **cannot be sealed** (`VMO_SEAL` = `NOT_SUPPORTED`):
  a device may change its contents.
- A clone (`VMO_CLONE`) is an ordinary object: pinned pages are copied
  into it, not shared.

### Mapping (`DMA_MAP`)

`DMA_MAP(resource, vmo, offset, len, flags, iova_hint)` makes a range of a
pinned object accessible to the device. The result is a mapping handle
(`ObjectType::DmaMap` = 13).

**Checks:**

- **The resource** must have `MANAGE` and a device identity
  (`RESOURCE_CREATE` with `device`, from devmgr).
- **The memory object** must be pinned (`VMO_PINNED` or contiguous DMA
  memory). Otherwise `BAD_STATE`; a register window
  (`VMO_CREATE_PHYSICAL`) does not work either.
- **The object handle** must have the **`DMA`** right (`1 << 12`). The
  default handle has it. Whoever sends a buffer to a driver (an
  application, the compositor) decides, by dropping the right, whether the
  device may access it.
- **The direction** is checked against the handle rights: `DEVICE_READ`
  (the device reads) needs `READ`, `DEVICE_WRITE` (the device writes) needs
  `WRITE`. With an IOMMU the hardware enforces the direction as well;
  SMMUv3 cannot do "write only", so a write there also allows reading.
- **The resource quota** `dma_max` is charged for the length of the
  mapping and returned on unmapping. For contiguous memory it applies for
  the whole life of the resource.
- **Kernel memory** for the page record is charged to the caller's job.

**Addresses for the device:**

- **With an IOMMU** the mapping gets one contiguous IOVA range from the
  **window above the end of RAM**: from RAM rounded up to 1 GiB, at least
  from 4 GiB, up to the translation width of the domain (VT-d 39 or 48
  bits, AMD-Vi 48, SMMUv3 47). The window therefore never overlaps the
  identity mapping of contiguous memory (there the device address is
  physical).
  - The IOVA allocator (`libs/iova`, first-fit) picks the range and leaves
    a free **guard page** after each range. A device that overruns the end
    of a buffer then hits a translation fault, not the data of the
    neighbouring buffer.
  - The driver may choose the address itself (`iova_hint`). The kernel
    checks that it lies in the window and does not overlap another mapping
    (`ALREADY_EXISTS`).
  - All resources of one device (even after a driver restart) share the
    domain and the allocator.
- **Without an IOMMU** the segments are the physical addresses of the
  pages (adjacent ones merged). A hint is not possible (`NOT_SUPPORTED`)
  and nobody enforces the direction. The kernel logs this once per
  resource (`DMA_MAP bez IOMMU: ...`, see
  [messages.md](messages.md#kernel-iommu)). A driver that must not continue
  without protection passes the `REQUIRE_IOMMU` flag and gets
  `NOT_SUPPORTED`.

`DMA_SEGMENTS(map, out, max)` returns the segments
`abi::dma::Segment { addr, len }`. With an IOMMU there is one segment.

### Unmapping

A mapping has no syscall of its own for unmapping: **closing the handle**
is the unmapping. The kernel works in this order:

1. it removes the entries from the domain and **synchronously** flushes
   the IOTLB (VT-d waits for the invalidation to complete, AMD-Vi for
   `COMPLETION_WAIT`, SMMUv3 for `CMD_SYNC`),
2. it returns the IOVA range to the allocator,
3. it drops the reference to the memory object. The pages are freed only
   when the object itself goes away.

A device that then uses the old address hits a translation fault. The
IOMMU blocks the access and the kernel logs it with the device and the
address (`IOMMU: zablokovan zapis zarizeni ... na adrese 0x... (`, on
SMMUv3 `SMMU: zablokovany pristup DMA ... na adresu 0x... (`).

SMMUv3 has no event queue interrupt yet. The kernel drains the queue on
every invalidation and on `DMA_SYNC` with the `FOR_CPU` direction
(`arch::iommu::poll_faults`), so at the latest when the driver reads data
after a transfer.

A mapping **cannot be passed** to another process (the handle has no
`TRANSFER`). It belongs to the driver process so that DMA quarantine finds
it after that process ends: the kernel first turns off the device's bus
mastering and waits for transfers, and only then destroys the mapping
([kernel.md](kernel.md#iommu-dma-protection)).

### Cache maintenance (`DMA_SYNC`)

`DMA_SYNC(map, offset, len, direction)` maintains the cache of a range of
the mapping on non-coherent platforms (Raspberry Pi):

- `FOR_DEVICE` (0) cleans the cache to memory before a transfer to the
  device,
- `FOR_CPU` (1) discards stale lines after a transfer into memory.

On x86_64 and QEMU virt (coherent) it does nothing except collect IOMMU
fault reports.

## Usage

In a driver through driverkit:

```rust
// 12 KiB in pages: contiguous IOVA with an IOMMU, physical segments without
let sg = p.alloc_dma_pages("edu-sg", 3 * 4096)?;
for (addr, len) in sg.device_chunks(off, len)? {
    // one transfer (or one entry of the device's segment list) per chunk
}
p.dma_sync_for_cpu(&sg); // after a transfer into memory
```

Directly on libos:

```rust
use libos::dma::{DmaMap, map, sync};
let buf = Vmo::create_pinned(1 << 20)?;
let m = DmaMap::new(&resource, &buf, 0, 1 << 20, map::DEVICE_WRITE)?;
let segs = m.segments()?; // [(iova, 1 MiB)] with an IOMMU
m.sync(0, 1 << 20, sync::FOR_CPU)?;
drop(m); // unmapping + IOTLB
```

An application buffer: the application creates `Vmo::create_pinned` on its
own account (the `DMA` budget of its job) and sends the service a handle
with the `DMA` right. The service passes it to the driver, which maps it.

## Tests

| Test | Where | What it verifies |
|---|---|---|
| `cargo test -p iova` | host | allocator: first-fit, guard page, hint and overlap, end of window, random model |
| `cargo test -p abi` | host | `Segment` encoding, merging physical pages |
| `cargo test -p driverkit --features sim` | host | `device_chunks`, `DmaSplit`, a dropped buffer = device fault and returned quota |
| `cargo test -p edu` | host | `sg` there and back, `stale` caught as a DMA fault |
| `DMA: ...` | kernel self-test (`objekty`) | IOVA window, `DMA` budget and root ceiling, mapping without an IOMMU (segments, quota, bounds), with an IOMMU (contiguous IOVA, guard page, hint, reuse) |
| suite `dma` | QEMU | pinned memory in the budget, `DMA` and direction rights, bounds, `REQUIRE_IOMMU`, a mapping cannot be passed on |
| suite `ovladace z manifestu` | QEMU | edu `sg`: 3000 B across page boundaries both ways; edu `stale`: a device write after unmapping blocked and in the kernel log (only with an IOMMU) |

## Limitations and risks

- **Without an IOMMU** a device may DMA anywhere. A mapping only hands out
  physical addresses and the kernel logs this. The blocked access test
  therefore does not run without an IOMMU: the device would write into a
  freed page.
- **Pinned memory** cannot be evicted. The root job ceiling (half of RAM)
  protects the system, but not services from each other. A parent may set
  its own `DMA` limit for child jobs; init and launchd do not set one yet
  (applications and services inherit the root ceiling) **[designed]**: a
  limit from the manifest.
- **The IOVA window is above 4 GiB.** Devices with 32-bit DMA addresses
  (old cards, `edu` with its default mask) do not see it and need
  contiguous memory.
- **SMMUv3** reports blocked accesses only on an invalidation or on
  `DMA_SYNC` (no event interrupt).
- **Ordering between device and memory** is not watched by the kernel. A
  driver that closes a mapping in the middle of a transfer gets a fault
  from the device (with an IOMMU) or corrupts someone else's memory
  (without an IOMMU).
