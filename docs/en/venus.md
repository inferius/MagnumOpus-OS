# Vulkan over Venus

Our own minimal **Venus** driver (Vulkan over virtio-gpu) in Rust, without
Mesa. An application in the system serializes Vulkan calls into a command
stream following the venus-protocol format; the renderer on the host
(virglrenderer with the render server) decodes it and calls the host's
real Vulkan (software **lavapipe** in WSL, the GPU driver on Linux with
a GPU).

Status: **steps 1-4 done** (step 4: the compositor can compose on the GPU, the default stays CPU) - a QEMU test prints the name of the host's
Vulkan physical device, creates a logical device, fills an image with a
color (`vkCmdClearColorImage`) and reads the pixels back through a blob
of a `VkDeviceMemory`. Next steps: see [Plan](#plan).

<figure class="diagram">
<svg viewBox="0 0 780 420" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Venus layers">
<defs><marker id="v1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="20" width="230" height="64" rx="6"/><text class="t b" x="135" y="44" text-anchor="middle">vk-probe (application)</text><text class="m" x="135" y="64" text-anchor="middle">vkCreateInstance, vkEnumerate...</text>
<rect class="acc" x="20" y="110" width="230" height="110" rx="6"/><text class="t b" x="135" y="134" text-anchor="middle">libs/venus</text><text class="m" x="135" y="154" text-anchor="middle">vk: types and commands (generated)</text><text class="m" x="135" y="172" text-anchor="middle">cs: stream encoding</text><text class="m" x="135" y="190" text-anchor="middle">client: context, reply blob,</text><text class="m" x="135" y="206" text-anchor="middle">fenced calls</text>
<rect class="box" x="20" y="250" width="230" height="64" rx="6"/><text class="t b" x="135" y="274" text-anchor="middle">init / compositor</text><text class="m" x="135" y="294" text-anchor="middle">render node from gpud</text>
<path class="ln" d="M135 84 L135 108" marker-end="url(#v1)"/>
<path class="ln dash" d="M135 250 L135 222" marker-end="url(#v1)"/><text class="m" x="142" y="241">render node channel</text>
<rect class="acc" x="300" y="110" width="210" height="140" rx="6"/><text class="t b" x="405" y="134" text-anchor="middle">user/virtio-gpu</text><text class="m" x="405" y="154" text-anchor="middle">render node (renderproto)</text><text class="m" x="405" y="172" text-anchor="middle">CTX_CREATE with capset 4</text><text class="m" x="405" y="190" text-anchor="middle">RESOURCE_CREATE_BLOB</text><text class="m" x="405" y="208" text-anchor="middle">RESOURCE_MAP_BLOB</text><text class="m" x="405" y="226" text-anchor="middle">fenced SUBMIT_3D</text>
<path class="ln" d="M250 160 L298 160" marker-end="url(#v1)"/><text class="m" x="256" y="152">SUBMIT</text>
<path class="ln dash" d="M298 200 L252 200" marker-end="url(#v1)"/><text class="m" x="256" y="216">blob VMO</text>
<rect class="box" x="560" y="110" width="200" height="140" rx="6"/><text class="t b" x="660" y="134" text-anchor="middle">QEMU virtio-gpu</text><text class="m" x="660" y="154" text-anchor="middle">venus=on, blob=on</text><text class="m" x="660" y="172" text-anchor="middle">hostmem=256M: BAR with</text><text class="m" x="660" y="190" text-anchor="middle">shared memory</text><text class="m" x="660" y="208" text-anchor="middle">(capability type 8, id 1)</text>
<path class="ln" d="M510 180 L558 180" marker-end="url(#v1)"/><text class="m" x="514" y="172">virtqueue</text>
<rect class="ok" x="560" y="290" width="200" height="110" rx="6"/><text class="t b" x="660" y="314" text-anchor="middle">virglrenderer</text><text class="m" x="660" y="334" text-anchor="middle">render server (vkr)</text><text class="m" x="660" y="352" text-anchor="middle">decodes the stream, calls</text><text class="m" x="660" y="370" text-anchor="middle">host Vulkan (lavapipe)</text>
<path class="ln" d="M660 250 L660 288" marker-end="url(#v1)"/>
<rect class="warn" x="300" y="300" width="210" height="90" rx="6"/><text class="t b" x="405" y="324" text-anchor="middle">reply blob</text><text class="m" x="405" y="344" text-anchor="middle">host shared memory</text><text class="m" x="405" y="362" text-anchor="middle">mapped in the BAR;</text><text class="m" x="405" y="380" text-anchor="middle">renderer writes, client reads</text>
<path class="ln dash" d="M558 345 L512 345" marker-end="url(#v1)"/>
<path class="ln dash" d="M300 345 L180 222" marker-end="url(#v1)"/>
</svg>
<figcaption>Layers: application, the venus library, the driver's render node, the QEMU device, the host renderer. Dashed: shared memory.</figcaption>
</figure>

## How it runs

```bash
cargo xtask test --qemu-wsl --gpu venus
```

The `vulkan` test in init (before the `gui` suite, while init still owns
the display):

1. `INFO` on the display: does the driver report the `VENUS` capability?
   If not, the test is **skipped** - the default `cargo xtask test` on the
   Windows QEMU (no Venus) passes on x86_64 and `--arch aarch64`
   unchanged.
2. init asks the **gpud** service for a render node ([render.md](render.md))
   and starts `vk-probe` with that channel.
3. `vk-probe` uses `libs/venus` to create an instance, enumerate physical
   devices, read their properties and send the line
   `ok <count>|<name>|<type>|<api>|<vendor:device>` (first test).
4. Then (`user/vk-probe/src/clear.rs`, second test): a logical device
   with a queue on timeline 1, a 64x64 `R8G8B8A8_UNORM` image in device
   memory, a buffer in `HOST_VISIBLE | HOST_COHERENT` memory, a command
   buffer (barrier, `vkCmdClearColorImage`, barrier,
   `vkCmdCopyImageToBuffer`, host barrier), `vkQueueSubmit` with a fence,
   waiting through a kernel fence on queue 1 (`Venus::wait_queue`). Pixels are read straight from the blob
   mapped from the buffer's `VkDeviceMemory`; all 4096 must have the fill
   color.

Output of a WSL run (lavapipe):

```text
virtio-gpu 00:06.0: 3D: virgl=1 blob=1 context_init=1 sdilena pamet=256 MiB capsety: virgl(v1, 308 B) virgl2(v2, 1408 B) venus(v0, 160 B)
vk-probe: renderer Venus, vk.xml 1.4.357, format proudu 1
vk-probe: zarizeni 0: llvmpipe (LLVM 20.1.2, 256 bits) (CPU), Vulkan 1.4.318, 10005:0000
vk-probe: rodin front 1, typu pameti 2, hald 1
init: test vulkan: fyzicke zarizeni Vulkan: llvmpipe (LLVM 20.1.2, 256 bits) (...)
[ok] vulkan: venus: jmeno fyzickeho zarizeni Vulkan od hostitele
vk-probe: obrazek: ok 64x64 4080bfff map_info=1 276 ms
[ok] vulkan: venus: obrazek vyplneny barvou a precteny zpet (pamet, command buffer)
```

`map_info=1` = cached memory (`MAP_CACHE_CACHED`): the shared-memory
window is mapped as normal memory (cache type of a physical VMO, see
[Syscalls](syscalls.md), `VMO_CREATE_PHYSICAL`).

In WSL the network tests also fail (the helper servers run on Windows and
are not reachable from WSL) and so does the `gui` suite (the GTK display
under Xvfb does not enable an output, the driver reports "zadny vystup").
That does not affect Venus; the full suite runs on the Windows QEMU.

## Connecting to the renderer (`venus::client`)

| Step | Message / command | Why |
|---|---|---|
| 1 | `INFO` (render node) | Venus capset (4) and shared memory size |
| 2 | `GET_CAPSET 4 v0` | `wire_format_version` = 1 and `supports_blob_id_0` |
| 3 | `CREATE_CONTEXT 4` | the driver sends `CTX_CREATE` with `context_init` = 4 |
| 4 | `CREATE_BLOB HOST3D, MAPPABLE, blob_id 0, 64 KiB` | the renderer creates shared memory (shm), the driver maps it into the BAR and sends the client a VMO of the window |
| 5 | `vkSetReplyCommandStreamMESA` | the renderer writes replies into that blob |
| call | `vkSeekReplyCommandStreamMESA(0)` + commands with `GENERATE_REPLY` | fenced `SUBMIT` on timeline 0; replies are read in chunks (4 KiB, then 4x more) |

**Synchronization:** the render server decodes the stream asynchronously;
the `SUBMIT_3D` response could arrive before the replies are written. The
driver therefore sends `SUBMIT_3D` with `FLAG_FENCE | FLAG_INFO_RING_IDX`
and `ring_idx` 0: timeline 0 is Venus's CPU timeline, the fence signals
once the renderer has processed the commands before it
(`vkr_context_submit_fence`). QEMU answers the command only after the
fence signals. A shared-memory ring (`vkCreateRingMESA`) is not needed yet.

Example (from `user/vk-probe`):

```rust
let mut v = Venus::connect(render_channel)?;
let instance = vk::VkInstance(v.new_id()); // handles are assigned by the client
let app = vk::VkApplicationInfo { p_application_name: Some("vk-probe"), api_version: api_version(1, 1, 0), ..Default::default() };
let info = vk::VkInstanceCreateInfo { p_application_info: Some(&app), ..Default::default() };
let r = v.call(|e| cmd::create_instance(e, REPLY, &info, instance), cmd::decode_create_instance_reply)?;
check(r.ret)?;
```

## Stream encoding (`venus::cs`)

Rules from `vn_protocol_driver_types.h` (venus-protocol):

| What | On the wire |
|---|---|
| command header | `VkCommandTypeEXT` (i32), `VkCommandFlagsEXT` (u32) |
| scalar, enum, bitmask | little-endian, always 4-byte aligned; `uint8_t`/`uint16_t` take 4 bytes |
| `size_t`, `VkDeviceSize`, handle | 8 bytes; a handle is an object id assigned by the **client** (also for outputs like `pInstance`) |
| pointer to one value | array size 0/1 (u64), then the value |
| array (fixed or with a length) | size u64, then elements; arrays of scalars < 4 bytes are packed and padded to 4 |
| string | size `strlen + 1`, bytes with NUL, padding to 4 |
| struct with `sType` | sType, pNext chain (pointer, sType, rest of chain, members), members |
| output struct in a request | "partial": only sType, NULL pNext, handle ids and fixed array sizes |
| reply | command type, return value (unless `void`), output parameters |

The decoder is safe on arbitrary data (reading past the end or huge
arrays = error, never a panic); the encoder records errors (an array
shorter than its length, NUL in a string, a struct not allowed in pNext)
and returns them from `finish`.

## Generator (`cargo xtask venus-gen`)

Inputs in the repository: `libs/venus/xml/vk.xml` (the Vulkan registry),
`VK_EXT_command_serialization.xml` (command numbers) and
`VK_MESA_venus_protocol.xml` (transport commands) from venus-protocol
(licenses in `libs/venus/xml/README.md`), and the list
`libs/venus/subset.txt`:

```text
command vkCreateInstance          # a command (types are added automatically)
chain VkPhysicalDeviceIDProperties  # a struct allowed in pNext chains
```

Output `libs/venus/src/vk/` (files of up to ~1000 lines): types and
constants as in C (`VK_SUCCESS`, `VkFormat` = `i32`, handle =
`struct VkInstance(u64)`), structs (`encode`, `encode_partial`, `decode`)
and commands in module `cmd` (`cmd::create_instance`,
`cmd::CreateInstanceReply`, `cmd::decode_create_instance_reply`). Input
structs hold references (`Option<&'a T>`, `&'a [T]`, `Option<&'a str>`);
counts stay as in C (`enabled_layer_count`).

`cargo xtask test` runs `venus-check`: the generated code must match the
inputs (input fingerprint in `target/venus-check.stamp`; without changes
the XML is not parsed again). The generator follows the rules of `vn_protocol.py`: a non-const
pointer parameter is an output, an output that is the length of another
parameter is both input and output (`pPhysicalDeviceCount`), output
handles and structs are "partial".

Unions (input only) are enums with a variant per member; the encoding is
the member index (u32) and the member, and the renderer accepts any
(`VkClearColorValue::Float32([..])`). pNext chain structs (`chain` in
`subset.txt`, e.g. `VkDeviceQueueTimelineInfoMESA`) are variants of the
`Next` enum and the parent's `p_next` field.

Not supported yet (generation error, never silently wrong encoding):
unions in outputs, pNext chains in outputs, multidimensional arrays,
pointers in output structs.

## Render node (`renderproto`)

The render node channel is now opened by the **gpud** service and the
protocol is the common render class (`renderproto`). The message table,
fences in `SUBMIT` and the hang deadline are in [render.md](render.md). For
Venus:

- uAPI = the Venus capset (4), capset contents through `QUERY CAPSET`,
- buffer = a `HOST3D` blob, driver id = `VkDeviceMemory` id (0 = new shared
  memory for replies),
- `SUBMIT` queue = `ring_idx`: 0 is the CPU timeline (fence after the stream
  is processed), 1..63 Vulkan queues bound with
  `VkDeviceQueueTimelineInfoMESA`.

Closing the channel: the driver revokes the shared-memory windows
(`VMO_REVOKE`), unmaps and frees the blobs, sets the output fences of
unfinished commands to `FENCE_FAILED` and destroys the context.

**Asynchronous queue.** The driver has 32 slots (each 72 KiB of DMA
memory for the command and response, a pair of descriptors). Synchronous
commands (display, blobs, contexts) wait only for their own slot. A
`SUBMIT` with fences is only sent; the main loop advances the fences once
the device returns the slot (the interrupt is in `wait_many` together with
the channels and the fence port). A slow renderer of one client thus blocks
neither the display nor other clients. Two slots are always kept for
synchronous commands; a client may have at most 16 pending `SUBMIT`s (then
`SHOULD_WAIT`).

## Risks

Risks from step 1 that are now solved:

- **Revocation.** The shared-memory window is a physical VMO and the
  driver revokes it on `DESTROY_BLOB` and when the client ends
  (`VMO_REVOKE`): the mapping disappears in the client process too and
  access through an old mapping faults. If revocation fails, the range is
  never lent again (quarantine).
- **Cache type.** The physical VMO gets a cache type from the device's
  `map_info` (QEMU reports `CACHED`); x86_64 PAT has a WC entry, aarch64
  uses Normal-NC.
- **Synchronous waiting.** See the asynchronous queue above; the client
  waits 40 s and then marks the connection as desynchronized
  (`Error::Broken`).

- **Vulkan queues** (step 6a-3) **[verified in QEMU]** (WSL): `ring_idx`
  1..63 works through kernel fences (`Venus::wait_queue`), polling
  `vkGetFenceStatus` is gone. A fence on a timeline the client did not bind
  to a queue never signals; the driver ends it with an error after
  `HANG_TIMEOUT` (10 s) ([render.md](render.md#submit-and-fences)).

Venus is not tested without KVM or under TCG (aarch64 in WSL); `vk.xml`
is 3.3 MB (a fraction of that compressed in git).

## Plan

1. ✅ Physical device name (`vkCreateInstance`, `vkEnumeratePhysicalDevices`,
   `vkGetPhysicalDeviceProperties`, queue families and memory types).
2. ✅ Memory: `vkCreateDevice`, `vkGetDeviceQueue2` with a timeline,
   `vkAllocateMemory`, a blob with `blob_id` = `VkDeviceMemory` id mapped
   into the client.
3. ✅ Image and command buffer: `vkCreateImage`, `vkCmdClearColorImage`
   (unions in the generator), `vkCmdCopyImageToBuffer`, `vkQueueSubmit`,
   a fence, reading pixels back from the blob.
4. ✅ Compositor composition over Vulkan - see [Compositor on the GPU](#compositor-on-the-gpu-step-4).

### Experiment: zero-copy scanout (`SET_SCANOUT_BLOB`)

The ideal for the compositor: the GPU composes the image into its memory
and that memory is shown directly, without a CPU copy. virtio-gpu has
`SET_SCANOUT_BLOB` for this; with virgl/Venus the blob must be backed by a
dmabuf and the QEMU display imports it through EGL.

Experiment (steps 4 to 6a-2; `vk-probe`, the step 3 buffer on output 0 for 0.5 s). It was removed in step 6a-3: scanout belongs to the display owner (compd), not to a render client, and will come with the display class v2 ([render.md](render.md#what-changed-against-gpuproto)).

| Part | Result in WSL |
|---|---|
| exporting `VkDeviceMemory` as a dmabuf (lavapipe `VK_EXT_external_memory_dma_buf`) | works |
| `SET_SCANOUT_BLOB` in the device, restoring the original image | accepted, OK response |
| drawing in the GTK display under Xvfb | **fails**: `eglMakeCurrent failed` exactly when showing and restoring; software EGL under Xvfb does not import the dmabuf (WSL has neither `/dev/dri` nor `/dev/udmabuf`) |

Conclusion: the zero-copy path is right on the system side but cannot be
verified on screen in WSL. The compositor over Vulkan will therefore have
both paths: zero-copy (Linux with a real GPU) and reading the frame back
into a buffer plus today's 2D path (WSL and everywhere else).

## Compositor on the GPU (step 4)

The compositor can compose the frame on the GPU: the control command
`backend gpu` (`backend cpu` switches back; **the default stays CPU**,
also without Venus or with `--gpu 2d`). Without Venus `backend gpu`
returns `NOT_SUPPORTED`.

| Part | What it does |
|---|---|
| `libs/spirv` | own SPIR-V assembler (`Builder`: types, constants, variables, instructions, sections in the order the specification requires) and the composition compute shader `blend` |
| `libs/vkcomp` | `plan` (turning an operation into shader parameters, host tests) and `gpu` (device, compute pipeline, surfaces as buffers in host-visible memory with a blob, composition with barriers and a fence) |
| `libs/comp/src/gpu.rs` | instead of drawing into the frame, a list of operations over surfaces (`GpuSink`) |
| `user/compd/src/gpu.rs` | GPU surfaces, uploads, composition, copying finished rectangles into the frame VMO; on a GPU error it falls back to CPU |

**The `blend` shader** (8x8 workgroup, one invocation = one pixel of the
clip): bindings 0 (frame) and 1 (source) as `u32` arrays, 15 push
constants (destination, clip, row pitches, source rectangle for
nearest-neighbour scaling, opacity, copy/blend). The formulas are exactly
those of `gfx::Canvas::draw_image`: `scale(c, k)` = channel
`(c * k + 128) >> 8`, `over(d, s)` = `s + scale(d, 255 - sa)` with
saturation; over an opaque pixel the alpha stays 255. The output passes
`spirv-val` (in WSL `spirv-val target/spirv-blend.spv`).

**The operation list** (bottom to top for every damaged rectangle):

| Surface | Contents | How it is drawn |
|---|---|---|
| `Desktop` | desktop background | CPU, copy |
| `Below(window)` | shadow, frame, title with text and buttons, content background | CPU; copy inside the frame, the shadow blends |
| `Buffer(client, buffer)` | application layer (buffer) | client pixels, blending, scaling |
| `Color(color)` | solid layer | a 1x1 surface stretched over the layer |
| `Above(window)` | buttons and badge without a title bar, "application not responding" | CPU, blending |
| `Top` | panel, dimming and dialog, cursor | CPU; panel copy, the rest blends |

The CPU-drawn parts are made by the same functions as the CPU path
(`render.rs`: `draw_window_below`, `draw_window_above`, `draw_top`) in a
transparent scratch canvas and uploaded into the surface right away. The
result is pixel-identical to the CPU path (RGB; the frame alpha is not
displayed), except for overlays where one CPU part draws several
semi-transparent things on top of each other (the dimming of a window that
does not respond, with text; semi-transparent dialog edges over the
dimming) - there the rounding differs by a few levels of a channel.

**Tests:**

- host: `cargo test -p spirv` (shader reference = `gfx` on 20,000 random
  pixels, module structure), `-p vkcomp` (plan = `gfx` on 3,000 random
  operations), `-p comp` (the operation list executed by the reference
  shader gives the same frame as the CPU path, also after moving a window),
- QEMU in WSL: `vulkan: venus: skladani vrstev na GPU shodne s gfx`
  (vkcomp against `gfx`, all pixels) and `gui: compd: skladani na GPU
  (Venus) dava stejne pixely jako CPU` (63 screen points with the CPU and
  GPU backends).

⚠️ **Performance in WSL:** Vulkan here is lavapipe (on the CPU), application
buffers live in guest memory the GPU cannot see (every used buffer is
copied into a surface every frame) and the finished frame is copied back.
GPU composition is therefore **slower** than the CPU path in WSL. A
speed-up comes with a physical GPU (Linux with a real card), importing
guest memory (`GUEST` blobs) and zero-copy scanout (`SET_SCANOUT_BLOB`, see
the experiment above).

**Output without a display:** in WSL the GTK display under Xvfb reports no
enabled output. The driver then uses output 0 (1280x800), so the compositor
runs (frames, inspection and tests work, the image is just not visible).
