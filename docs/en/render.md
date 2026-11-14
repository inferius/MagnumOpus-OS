# The render class and the gpud service

The **render class** is the common interface between a 3D client (an
application with Vulkan, the compositor, a test program) and a GPU driver.
It consists of:

- the `libs/renderproto` protocol,
- kernel fences ([fences.md](fences.md)),
- the buffer description ([shared-buffers.md](shared-buffers.md)).

Render nodes are handed out by the **gpud** service, not by the display
owner. These are steps 6a-3 and 6a-4 of the foundations for graphics
([gpu-amd-nvidia.md](gpu-amd-nvidia.md#impact-on-the-foundations),
item 8).

## Status

| Part | What | Status |
|---|---|---|
| `renderproto` | common envelope: `INFO`, `QUERY`, `CREATE_CONTEXT`, `CREATE_BUFFER`, `IMPORT_BUFFER`, `DESTROY`, `BIND`, `SUBMIT` with kernel fences | **[verified in QEMU]** Venus in WSL (suites `vulkan`, `gui`); encoding by host tests |
| Venus | `SUBMIT` on a queue = `ring_idx`; completion only through kernel fences; waiting for a Vulkan queue (`Venus::wait_queue`) instead of polling `vkGetFenceStatus` | **[verified in QEMU]** WSL (`cargo xtask test --qemu-wsl --gpu venus`) |
| hang deadline | `HANG_TIMEOUT` 10 s: output fences `FENCE_FAILED`, log `hang` | **[implemented]** (no test: the renderer in QEMU does not hang) |
| gpud | a supervised service; the driver hands out the provider once (`OPEN_PROVIDER`), init keeps the end, gpud a duplicate | **[verified in QEMU]** (Venus in WSL) |
| `IMPORT_BUFFER` for Venus | a client buffer (VMO with the `DMA` right + `bufdesc`) as a `GUEST` blob over `DMA_MAP` | **[designed]** step 6a-4 (video into compd without a copy) and NVK; `NOT_SUPPORTED` today |
| application permissions | manifest `[gpu] render = true`, per-client quotas, connection through tickets from launchd | **[designed]** (today only init opens render nodes: compositor, tests) |
| recovery of gpud clients | after a gpud crash clients reconnect through tickets (`libs/svcclient`) | **[designed]**; today the supervisor restarts gpud and init is its only client |

## Why a service and not the display

Up to step 6a-2 a render node was opened by the display owner: init, then
the compositor (`OPEN_RENDER` on the display channel). The compositor thus
decided about access to 3D, which has nothing to do with the display, and it
would have had to know application manifests.

Now init asks **once** on the display channel for a provider channel
(`OPEN_PROVIDER`; another attempt returns `BAD_STATE`). It does so before
the compositor gets the display, and hands the provider to the **gpud**
service.

<figure>
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Render nodes through gpud">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
</style>
<defs><marker id="rn1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L10,5 L0,10 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/>
<text class="t b" x="110" y="44" text-anchor="middle">init</text>
<text class="m" x="110" y="62" text-anchor="middle">OPEN_PROVIDER (once)</text>
<text class="m" x="110" y="78" text-anchor="middle">keeps the provider end</text>
<rect class="acc" x="280" y="20" width="200" height="70" rx="6"/>
<text class="t b" x="380" y="44" text-anchor="middle">gpud</text>
<text class="m" x="380" y="62" text-anchor="middle">provider duplicate</text>
<text class="m" x="380" y="78" text-anchor="middle">OPEN -> render node</text>
<rect class="box" x="550" y="20" width="200" height="70" rx="6"/>
<text class="t b" x="650" y="44" text-anchor="middle">virtio-gpu</text>
<text class="m" x="650" y="62" text-anchor="middle">provider + render nodes</text>
<text class="m" x="650" y="78" text-anchor="middle">SUBMIT, kernel fences</text>
<path class="ln" d="M210 55 L278 55" marker-end="url(#rn1)"/>
<path class="ln" d="M480 55 L548 55" marker-end="url(#rn1)"/>
<rect class="box" x="10" y="160" width="200" height="60" rx="6"/>
<text class="t b" x="110" y="184" text-anchor="middle">compositor, vk-probe</text>
<text class="m" x="110" y="202" text-anchor="middle">render node from init</text>
<path class="ln" d="M210 190 L640 190 L640 92" marker-end="url(#rn1)"/>
<text class="m" x="420" y="182" text-anchor="middle">renderproto directly to the driver (a gpud crash does not cut it)</text>
</svg>
<figcaption>init keeps the provider, gpud hands out render nodes, the client talks to the driver directly.</figcaption>
</figure>

- **A gpud crash** does not interrupt render nodes: they go directly to the
  driver. The supervisor restarts gpud and the new instance gets another
  duplicate of the provider from init (`Kept`). The new instance discards
  replies the provider sent to the old one.
- **A GPU driver crash** closes the provider: no new render nodes until
  display driver recovery is solved.
- **Clients today:** init opens a render node (`init::render::open_render`).
  The compositor gets it with the `backend gpu` command, the Vulkan test
  program in its startup message. Applications with a manifest
  `[gpu] render = true` will come with launchd **[designed]**.

## The `renderproto` protocol

One channel = one client = at most one context (like a DRM render node
file). Closing the channel destroys the context and all buffers of the
client; unfinished commands end with `FENCE_FAILED`.

| Op | Fields | Reply | Venus (virtio-gpu) |
|---|---|---|---|
| `INFO` | - | driver class, uAPIs (id, version, size), shared memory size | uAPI = capsets |
| `QUERY` | key, a, b | data | `CAPSET` (a = id, b = version): capset contents |
| `CREATE_CONTEXT` | uAPI | `CONTEXT` | context with the capset |
| `CREATE_BUFFER` | memory, flags (`MAPPABLE`...), driver id, size | `BUFFER` (id, size, `map_info`) + VMO with `MAPPABLE` | `HOST3D` blob, driver id = `VkDeviceMemory` |
| `IMPORT_BUFFER` | `bufdesc` + VMO in the handles | `BUFFER` | `NOT_SUPPORTED` (step 6a-4 and NVK) |
| `DESTROY` | buffer | `DONE` | revoke the window, destroy the blob |
| `BIND` | driver data (`VM_BIND`) | `DONE` | nothing (virtio-gpu has no GPU address space) |
| `SUBMIT` | queue, values of fences to wait for and to signal, data (at most 60 KiB, multiple of 4) | `SUBMITTED` at once | queue = `ring_idx` (0 = CPU timeline, 1..63 = Vulkan queues) |

### `SUBMIT` and fences

The fences travel in the message handles: first the fences to wait for (the
`WAIT` right), then the fences to signal (the `SIGNAL` right), at most 16 of
each. The message data holds only their values. The driver:

1. accepts the command and replies `SUBMITTED`; completion is reported
   only by fences,
2. registers the wait fences in its port (`FENCE_WAIT_ASYNC`). It sends the
   command to the device once all of them are met. Order within a queue is
   kept: a command does not overtake an earlier command of the same queue
   that is still waiting,
3. after the device fence (virtio: a fence on the `ring_idx` timeline)
   advances the signal fences. With empty data the command still goes
   through the device: a queue fence is met only after all work of the
   queue sent before it.

Errors that end a command:

| What happens | Result |
|---|---|
| a wait fence ended with an error or was abandoned | the command is not sent; the signal fences get `FENCE_FAILED` |
| the device returns an error | the signal fences get `FENCE_FAILED`, a log line with the error |
| waiting plus running takes longer than `HANG_TIMEOUT` (10 s) | the signal fences get `FENCE_FAILED`, a log line `hang` |

`HANG_TIMEOUT` is a class constant (`renderproto::HANG_TIMEOUT`):

- Windows (TDR) has 2 s, Linux (amdgpu, i915) about 10 s.
- 10 s leaves room for long computations and slow emulation (lavapipe in
  WSL).
- Nobody waits forever, even when a client sends a cycle of fences.

A client may have at most 16 unfinished commands, then it gets
`SHOULD_WAIT`.

### Venus on kernel fences

`libs/venus` (`Venus`) has one kernel fence per connection:

- **`call` and `send`** submit a stream on queue 0 (the CPU timeline) with
  the next fence value and wait for it with a deadline.
- **`wait_queue(ring_idx)`** sends an empty `SUBMIT` on the queue bound to
  that timeline (`VkDeviceQueueTimelineInfoMESA`; vk-probe and vkcomp use
  timeline 1). The fence is met when the Vulkan queue finishes the work
  sent before it. Polling `vkGetFenceStatus` is gone.
- **A fence error or a timeout** = the connection is broken
  (`Error::Broken`).

## What changed against `gpuproto`

> **Change against `gpuproto` (6a-3): what and why.** `gpuproto` was a
> Venus-only protocol.
>
> - **New envelope.** `GET_CAPSET` is now `QUERY CAPSET`, `CREATE_BLOB` is
>   `CREATE_BUFFER` and `DESTROY_BLOB` is `DESTROY`. The envelope is shared
>   with further drivers (NVK, AMD).
> - **`SUBMIT` with a fence.** It used to reply only after processing and
>   knew only timeline 0, so waiting for the GPU meant polling. Now it
>   replies at once and kernel fences report completion on any queue.
> - **`SCANOUT_BLOB` is gone.** It was a step 4 experiment in which a
>   render client overwrote the compositor's image. Zero-copy scanout
>   belongs to the display owner (compd) and comes with the display class
>   v2.
> - **`OPEN_RENDER` on the display is gone.** It was replaced by the
>   one-time `OPEN_PROVIDER` and gpud.

## Tests

| Test | Where | What it verifies |
|---|---|---|
| `cargo test -p renderproto` | host | encoding of all messages, limits (fence count, data size), provider |
| `cargo test -p bufdesc` | host | buffer description (see [shared-buffers.md](shared-buffers.md)) |
| suite `vulkan` | QEMU in WSL (`--qemu-wsl --gpu venus`) | render node from gpud, vk-probe: device, image with waiting through a queue fence, compositing |
| suite `gui` (`backend gpu`) | QEMU in WSL | compositor with a render node from gpud, same pixels as the CPU |
| without Venus | QEMU on Windows (default) | gpud does not run or returns `NOT_SUPPORTED`, the tests are skipped, the rest is unchanged |
