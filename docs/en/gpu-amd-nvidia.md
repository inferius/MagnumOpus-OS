# NVIDIA and AMD graphics

**Status: [proposed]** (2026-11-14). Nothing on this page is implemented.
It is research and a proposal: which reused path to take, what it gives
us, and **what should change in the system foundations now** so the
driver does not have to be bolted on later (section
[Impact on the foundations](#impact-on-the-foundations)).

Order: **NVIDIA first** (NVIDIA cards are available for testing), AMD as
the second driver in the same layer. Intel has its own Rust driver
([Display configuration](display.md)).

## Summary

| Question | Recommendation |
|---|---|
| NVIDIA driver | **nouveau** from Linux (MIT licence), logic unchanged, in C, as an isolated process |
| NVIDIA generations | Kepler to Blackwell. Turing and newer through GSP-RM firmware (full clocks, Vulkan); Maxwell 2 to Volta display and slow acceleration only (the card stays at boot clocks) |
| NVIDIA Vulkan | **NVK** from Mesa (MIT), same nouveau kernel part |
| AMD driver | **amdgpu** + display core (MIT), same layer, second step |
| Compatibility layer | our own "LinuxKPI" in Rust on driverkit; model: FreeBSD drm-kmod (BSD) |
| Without an IOMMU | **do not start** the native GPU driver, stay on the firmware framebuffer (proposal, question B) |
| Foundations now | our own fence object, shared buffers with a lifecycle, large scatter-gather DMA memory, firmware loading, a "render" class for Vulkan |

## Architecture

<figure class="diagram">
<svg viewBox="0 0 780 420" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="GPU driver architecture">
<defs><marker id="ge1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="230" height="74" rx="6"/><text class="t b" x="125" y="34" text-anchor="middle">application (Vulkan)</text><text class="m" x="125" y="53" text-anchor="middle">Mesa NVK / RADV in the app</text><text class="m" x="125" y="69" text-anchor="middle">process, "libdrm" over a channel</text>
<rect class="acc" x="275" y="10" width="230" height="74" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">render service (gpud)</text><text class="m" x="390" y="53" text-anchor="middle">permissions, contexts, quotas,</text><text class="m" x="390" y="69" text-anchor="middle">hands out render nodes</text>
<rect class="acc" x="540" y="10" width="230" height="74" rx="6"/><text class="t b" x="655" y="34" text-anchor="middle">compd</text><text class="m" x="655" y="53" text-anchor="middle">display class</text><text class="m" x="655" y="69" text-anchor="middle">(protocol as intel-display)</text>
<path class="ln" d="M240 47 L273 47" marker-end="url(#ge1)"/>
<path class="ln dash" d="M125 84 L125 150 L218 150" marker-end="url(#ge1)"/><text class="m" x="132" y="120">render node (channel)</text>
<path class="ln" d="M390 84 L390 128" marker-end="url(#ge1)"/>
<path class="ln" d="M655 84 L655 128" marker-end="url(#ge1)"/>
<rect class="ok" x="220" y="130" width="550" height="200" rx="6"/><text class="t b" x="495" y="152" text-anchor="middle">gpu-nvidia process (isolated, restricted Resource, IOMMU)</text>
<rect class="box" x="235" y="165" width="250" height="70" rx="4"/><text class="t" x="360" y="187" text-anchor="middle">nouveau + DRM core (C, MIT)</text><text class="m" x="360" y="205" text-anchor="middle">KMS, GEM, TTM, scheduler,</text><text class="m" x="360" y="221" text-anchor="middle">GSP-RM, NVK uAPI (VM_BIND)</text>
<rect class="box" x="505" y="165" width="250" height="70" rx="4"/><text class="t" x="630" y="187" text-anchor="middle">class bridge (Rust)</text><text class="m" x="630" y="205" text-anchor="middle">display = KMS atomic,</text><text class="m" x="630" y="221" text-anchor="middle">render = DRM ioctls over a channel</text>
<rect class="warn" x="235" y="250" width="520" height="66" rx="4"/><text class="t" x="495" y="272" text-anchor="middle">LinuxKPI (Rust, ours): memory, DMA, IOMMU, interrupts, locks, workqueues,</text><text class="m" x="495" y="290" text-anchor="middle">timers, firmware, PCI, i2c; dma-fence and dma-buf as our fence and buffer objects</text><text class="m" x="495" y="306" text-anchor="middle">on driverkit and libos (model: FreeBSD linuxkpi)</text>
<path class="ln" d="M495 330 L495 358" marker-end="url(#ge1)"/>
<rect class="box" x="10" y="360" width="760" height="50" rx="6"/><text class="t b" x="390" y="382" text-anchor="middle">kernel</text><text class="m" x="390" y="400" text-anchor="middle">Resource, BAR (WC), MSI-X, scatter-gather DMA, IOMMU domain, fence and buffer objects, firmware from FS</text>
<rect class="box" x="10" y="190" width="190" height="120" rx="6"/><text class="t b" x="105" y="212" text-anchor="middle">firmware</text><text class="m" x="105" y="232" text-anchor="middle">GSP-RM (Turing+),</text><text class="m" x="105" y="248" text-anchor="middle">PMU/SEC2/ACR,</text><text class="m" x="105" y="264" text-anchor="middle">AMD: PSP, SMU,</text><text class="m" x="105" y="280" text-anchor="middle">DMCUB, VCN</text><text class="m" x="105" y="296" text-anchor="middle">firmware service</text>
<path class="ln" d="M200 250 L233 250" marker-end="url(#ge1)"/>
</svg>
<figcaption>An application never gets a capability to the driver: the render service hands it a render node, the image goes through the display class. The reused C runs in one process on our compatibility layer.</figcaption>
</figure>

- **One process per GPU** (`gpu-nvidia`, later `gpu-amd`): reused DRM code
  (DRM core, TTM, scheduler, nouveau or amdgpu) in C with unchanged logic,
  our compatibility layer below it, a bridge to the device classes above.
- **Display class**: the same protocol as `intel-display` and
  `bochs-display` (connectors, modes, vsync flips, hotplug). The bridge
  translates it to KMS atomic commits. The compositor cannot tell which
  driver is underneath.
- **Render class** (new): Vulkan in an application (Mesa NVK/RADV) needs
  DRM ioctls (buffer allocation, `VM_BIND`, `EXEC`, fences). They travel
  as messages over a "render node" channel. The channel is handed out by
  the **render service** (manifest permission, a context per application,
  GPU memory quotas), never by the application itself
  ([Device interfaces](device-interfaces.md)).
- **Crash**: after a mode set the GPU cannot return to the firmware
  framebuffer (GOP no longer scans out). Recovery = PCI function reset
  (FLR or bus reset) and a new instance; the compositor's frame is not
  shown meanwhile. See [Risks](#risks).

## NVIDIA: which path covers which generations

Verified in the Linux and Mesa sources (2026-11-14):

| Path | Licence | Generations | Display (KMS) | Acceleration | For us |
|---|---|---|---|---|---|
| **nouveau** (Linux) | MIT text in headers (`nouveau_drm.c`, `nvkm/...`, `gsp/...`), module "GPL and additional rights" | NV04 to Ada, chip tables also for Blackwell (GB100/GB102, GB202 to GB207) | yes, all | Kepler+; from Turing through GSP-RM, default (`NvGspRm` = true) | **recommended** |
| **NVK** (Mesa) | MIT (Mesa core) | Kepler to Ada, consumer Blackwell (RTX 5000) | – | conformant Vulkan 1.4; needs nouveau from Linux 6.6+ (`VM_BIND`/`EXEC` uAPI) | **Vulkan** |
| **open-gpu-kernel-modules** (NVIDIA) | files MIT, MIT/GPLv2 as a module | Turing+ (only with GSP firmware of the same release) | `nvidia-modeset` | only with NVIDIA's closed userspace | facts only, maybe display only |
| **nova** (Linux, Rust) | GPL-2.0 | GSP (Turing+) | not yet (no display item in its TODO list) | in progress (GSP, FIFO, GR) | **facts only** |

Therefore:

1. **Turing (RTX 20, GTX 16) to Blackwell (RTX 50)**: nouveau + GSP-RM
   firmware (version 570.144 in nouveau). GSP itself manages clocks and
   power, so the card runs at full speed. Vulkan through NVK. **Main target.**
   **Blackwell** (GB202 to GB207, RTX 50) is GSP-only in nouveau: even
   display goes through GSP-RM (`disp/gb202.c`: "GB20x is GSP-only").
   Without GSP firmware there is not even an image.
2. **Maxwell 2 (GTX 900), Pascal (GTX 10), Volta**: display yes,
   acceleration yes through NVIDIA-signed firmware loaded by ACR secure
   boot (`nvidia/gp108/acr`, `.../gr/*_sig.bin` in linux-firmware); NVK
   is enabled for these generations since April 2025. Clocks cannot be
   raised, though (NVIDIA has not released power-management firmware),
   so the card stays at boot clocks. Fine for the desktop, not for games.
3. **Kepler (GTX 600/700), Maxwell 1 (GTX 750)**: display, acceleration,
   manual reclocking, NVK Vulkan. Low priority.
4. **Fermi and older** (e.g. GTX 560 Ti, GF114/GF110): nouveau has display
   and 3D, but power management is unfinished ("it's slow") and NVK does
   not support Fermi. Proposal: not supported, keep the firmware
   framebuffer (`fbdisplay`).

`open-gpu-kernel-modules` supports new features better, but without
NVIDIA's closed userspace it has no Vulkan or OpenGL. Its display part
(`nvidia-modeset`) is MIT and could serve as a fallback for Turing+ if
nouveau fails on a particular card – at the cost of a second huge code
base. Not recommended.

### Test cards and what to verify on them

The user's cards (as of 2026-11-14):

| Card | Chip / generation | Path | What to test on it |
|---|---|---|---|
| **RTX 4090** (active, from USB) | AD102, Ada | nouveau + GSP-RM, NVK | **main card**: stage (a) display (takeover, modes, flips, hotplug, multiple monitors), (b) copy, (c) Vulkan; recovery after a crash (GSP reset) |
| **RTX 5070 Ti** | GB203, Blackwell | nouveau + GSP-RM (display through GSP too), NVK | second Turing+ card: verify it runs purely through GSP including display; newest GSP firmware; NVK Vulkan |
| RTX 2080 Ti (maybe) | TU102, Turing | nouveau + GSP-RM, NVK | oldest GSP generation: firmware version compatibility, otherwise the same tests as the 4090 |
| **GT 1030** (active, from USB) | GP108, Pascal | nouveau without GSP: display by nouveau's own code, acceleration through signed ACR firmware, no reclocking | **the non-GSP path**: stage (a) display (different code than GSP), loading signed firmware (firmware service), (b) copy; Vulkan functionally only (slow) |
| GTX 560 Ti | GF114/GF110, Fermi | not supported | only check that the driver rejects the card and the display stays on `fbdisplay` (safe fallback) |

Order: first the **GT 1030** and the **RTX 4090** together for stage (a)
– both display paths (nouveau's own code and GSP-RM) show at once what
the compatibility layer must provide. Then the 4090 for (b) and (c), the
5070 Ti for Blackwell.

Blackwell in each path: nouveau has GB100/GB102 and GB202 to GB207 (GB203
= RTX 5070 Ti, display `gb202_disp` through GSP-RM), NVK supports consumer
Blackwell (Mesa 25.2), open-gpu-kernel-modules "Turing or later", nova
lists GB20x in its chip enum (`gpu.rs`) but has no display.

## AMD (second step)

| Part | Licence (verified in headers) |
|---|---|
| `amdgpu` (`amdgpu_device.c`) | MIT text |
| display core (`dc/core/dc.c`, `amdgpu_dm.c`) | MIT text, `amdgpu_dm.c` SPDX MIT |
| power management (`pm/swsmu/amdgpu_smu.c`) | MIT text |

amdgpu goes into the same compatibility layer as nouveau (same DRM core,
TTM, scheduler). It additionally needs firmware for almost everything
(PSP, SMU, DMCUB for display, VCN for video, graphics and SDMA microcode).
Vulkan: RADV from Mesa. The generation range (GCN 1.2+ and RDNA by
default, older experimentally) and the firmware licence are to be
verified when AMD work starts (the firmware sources were unreachable
during the research).

## Licences and what not to reuse

The reused DRM code is MIT or dual MIT/GPL with few exceptions. Verified
headers:

| File | Licence | Action |
|---|---|---|
| `drm_gem.c`, `drm_atomic.c`, `drm_syncobj.c`, `scheduler/sched_main.c` | MIT text | reuse |
| `ttm/ttm_bo.c`, `drm_exec.c` | GPL-2.0 OR MIT | reuse under MIT |
| `drm_gpuvm.c` | GPL-2.0-only OR MIT | reuse under MIT |
| `dma-buf/dma-resv.c` | MIT | reuse |
| **`dma-buf/dma-fence.c`** | **GPL-2.0-only** | **do not reuse**: implement the `dma_fence_*` API in the compatibility layer on our fence object |
| `include/linux/*.h` (kernel headers) | mostly GPL | do not reuse: our own LinuxKPI headers (model FreeBSD, BSD-2) |

Rule: **check the licence of every file before reusing it** and replace
GPL-only files with our own implementation (from the API and
documentation only). The import script keeps the list automatically
(SPDX header / MIT text).

## Compatibility layer (LinuxKPI)

A DRM driver expects Linux kernel services. The layer is ours (Rust +
thin C headers), modelled on FreeBSD `sys/compat/linuxkpi` (BSD), from
which code can also be reused where it fits.

| Group | What the driver calls | What it maps to here |
|---|---|---|
| memory | `kmalloc`, `vmalloc`, `alloc_pages`, `kref`, `idr`/`xarray`, lists, rb-tree | process allocator (`cstd`), VMO pages |
| DMA | `dma_alloc_coherent`, `dma_map_sg`, `dma_map_page`, masks | scatter-gather DMA + device IOMMU domain |
| MMIO | `ioremap`, `ioremap_wc`, `readl`/`writel` | `Platform::map_bar`, WC cache type |
| interrupts | `request_irq`, MSI/MSI-X, `tasklet` | `Platform::enable_irq`, interrupt thread |
| concurrency | spinlock, mutex, rwsem, RCU, atomics, `completion`, `wait_queue` | `cstd` locks, futex/event |
| work | `workqueue`, `kthread`, `timer`, `hrtimer`, `jiffies`, `msleep` | process threads, libos timers |
| PCI | configuration, BARs, `pci_enable_device`, D0/D3 power | configuration page + power service |
| firmware | `request_firmware` | **firmware service** |
| i2c/DDC | `i2c_adapter`, `i2c_transfer` | inside the driver (nouveau/amdgpu have their own) |
| synchronisation | `dma_fence`, `sync_file`, `dma_resv`, `dma_buf` | **our fence and buffer objects** |
| debugging | `printk`, `dev_*`, `WARN_ON`, debugfs | driverkit log; no debugfs |

Size estimate (not measured): layer 15–30 thousand lines (FreeBSD
linuxkpi is in that range), reused DRM core + TTM + scheduler tens of
thousands of lines, nouveau hundreds of thousands, amdgpu millions (mostly
generated register headers).

**Upstream updates**: track one Linux LTS version (like drm-kmod), import
by a script (`xtask`): fetch the tag, check licences, apply our patch set
(patches only in the layer and the build, not in driver logic), build,
host tests of the layer, hardware test. Move to a new LTS once a year.

## Stages

| Stage | Content | Prerequisites |
|---|---|---|
| (a) display | display class through KMS: takeover from GOP, modes from EDID, flips, hotplug, multiple monitors | layer (without scheduler and GEM for apps), firmware (Turing+: GSP), IOMMU |
| (b) 2D/copy | buffers in GPU memory for the compositor, copy engine, zero-copy frames | shared buffers, fences |
| (c) Vulkan | NVK/RADV in applications, render class, render service, quotas | Mesa port (languages/libc thread), render node |
| (d) power and thermals | runtime PM (D3cold, Optimus), temperatures and fans into the power service | ACPI `_DSM`/`_PR3` via acpid |

Stage (a) gives a display on every card from Kepler (for Turing+ with
GSP). Compositor acceleration (b) pays off even without Vulkan.

## IOMMU

The GPU is the most powerful DMA device in the computer and the driver is
hundreds of thousands of lines of reused C plus firmware (GSP is a
processor of its own with memory access). **Without an IOMMU a bug in the
driver or firmware can overwrite any memory, including the kernel.**

Proposal:

- with an IOMMU: the device has its own domain (already today), DMA only
  into memory the driver maps for it; after a crash devmgr turns bus
  master off and drops the domain;
- **without an IOMMU do not start the native GPU driver**: the display
  stays on the firmware framebuffer (`fbdisplay`) and the user is told why;
  optionally a manual override with a warning (question B);
- turn interrupt remapping on (off today) before a GPU runs (a DMA device
  could otherwise forge an MSI).

## Testing

QEMU emulates neither NVIDIA nor AMD graphics. Options:

1. **Real machine** (main): image on a USB stick, log over a serial line
   or to a file ([Display configuration](display.md#testing-on-a-real-machine)).
2. **Passing the card to a VM (VFIO)**: needs Linux as the host with an
   IOMMU; on Windows (Hyper-V DDA) only Windows Server. Alternative: a
   second machine with Linux and QEMU + VFIO; faster cycle (no reboot).
3. **driverkit simulation**: host tests of the compatibility layer (locks,
   workqueues, DMA, fences) and replaying MMIO recordings from real runs
   (recorded from our own driver, not from foreign code).

Hardware: see [Test cards](#test-cards-and-what-to-verify-on-them) – an
RTX 4090 (Turing+ with GSP) and a GT 1030 (Pascal without GSP) are
available. For AMD later one RDNA 2 or newer card (the user has none).

## Impact on the foundations

What is worth changing **now** (independently of the driver itself) so
that the GPU does not require changes to finished parts. Size: S = days,
M = 1–2 weeks, L = more.

| # | Area | Change | Why now | Size |
|---|---|---|---|---|
| 1 | **fence object** | kernel object: state (pending, signalled, error), timeline (64-bit value), waitable in `wait_many`, passed by handle, signalled only by the holder of the `SIGNAL` right | `dma-fence.c` is GPL – we need our own; also the basis for Vulkan semaphores, `sync_file`, `drm_syncobj` and explicit sync in the display protocol; Venus has none today either (fences only inside the driver) | M |
| 2 | **shared buffer** | buffer object over a VMO: size, placement (system / device memory), cache type, format + modifier (tiling, compression), stride, who may write; passed by handle between application, compositor and drivers; revoked on a crash | the `dma-buf` equivalent; device interfaces already require "the whole buffer lifecycle" (point 5) – define it once now for display, Venus and future GPUs | M |
| 3 | **display class v2** | optional: a client buffer as a frame without a copy (with acquire and release fences), formats and modifiers, multiple planes (cursor, overlay), atomic commit (mode + buffers at once), full timings in `SET_MODE` | today's protocol copies the shadow; GPU drivers and Intel can scan a buffer out directly – add it now as capabilities so messages do not change later | S–M |
| 4 | **scatter-gather DMA and IOMMU** | large pinned allocations page by page (not only contiguous 4 MiB), mapping a VMO range into the device domain at an IOVA chosen by the driver, unmapping with a synchronous IOMMU TLB flush, DMA quotas in GB | GPUs map hundreds of MB to GB (TTM, GTT); today's `dma_memory` is contiguous and small | M |
| 5 | **interrupt remapping** | enable in the kernel (VT-d IR, AMD-Vi; GICv3 ITS exists) | without it a DMA device can forge MSIs | M |
| 6 | **firmware service** | `request_firmware` as a service: files from `/system/firmware`, checksums, licence metadata, read-only VMO hand-over; image and installer bundle the firmware | GSP-RM (tens of MB), AMD, Wi-Fi, Realtek NICs (`rtl_fw`), future audio; nothing like it exists today | S–M |
| 7 | **GPU memory and budgets** | job budgets also for device memory (VRAM, GTT); objects the driver may evict (TTM) and pin again; VRAM mapped through the BAR as WC (cache type exists) | per-application Vulkan quotas; fairness between applications | M |
| 8 | **render class and render service** | a general render node protocol (from Venus: context, buffer, `VM_BIND`, submit with fences, queries) + a service that hands render nodes out by manifest and enforces quotas | applications must not get a capability to the driver; Venus hands its render node out through init today – unify before another driver appears | M |
| 9 | **runtime for reused C** | threads in the driver process (kthread, workqueue), precise timers, waits with timeouts; `cstd` with threads | the DRM core and nouveau have their own threads and timers; Wi-Fi needs the same | M |
| 10 | **device power** | a protocol for device power states (D0/D3hot/D3cold) through the power service; ACPI `_DSM`/`_PRx` calls from the driver through acpid | Optimus laptops turn the dGPU off; otherwise battery drain | S–M |
| 11 | **GPU recovery after a crash** | devmgr: PCI function reset (FLR / bus reset) before a new instance; the compositor survives without a display (it already does) | after a mode set there is no way back to GOP; recovery only by a reset | S |

I recommend starting points 1, 2, 4 and 8 right away: they touch the
kernel and protocols already used by the display and Venus, and later
they would change in more places.

## Risks

- **Size**: hundreds of thousands of lines of reused C; upstream updates
  are permanent work (hence one LTS version and patches only in the layer).
- **NVIDIA and AMD firmware is closed** (binary, redistribution under the
  licence in linux-firmware – **not verified**, the source was unreachable
  during the research). Turing+ runs nothing without it; verify the
  licence before bundling it into the image.
- **Without an IOMMU** the risk is unacceptable (see above).
- **Optimus laptops**: the integrated Intel drives the display; NVIDIA only
  computes – requires shared buffers between drivers (point 2) and power
  (point 10).
- **Tests on real hardware only**: slower cycle, needs a serial line or a
  log on disk.
- **GSP firmware crashes**: recovery only by a device reset, briefly no image.
- **Licences can change** per file: licence check at every import.

## Questions

See the message to the coordinator; added here after the decision.
