# Vulkan over Venus

Our own minimal **Venus** driver (Vulkan over virtio-gpu) in Rust, without
Mesa. An application in the system serializes Vulkan calls into a command
stream following the venus-protocol format; the renderer on the host
(virglrenderer with the render server) decodes it and calls the host's
real Vulkan (software **lavapipe** in WSL, the GPU driver on Linux with
a GPU).

Status: **step 1 done** - a QEMU test prints the name of the host's
Vulkan physical device. Next steps: see [Plan](#plan).

<figure class="diagram">
<svg viewBox="0 0 780 420" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Venus layers">
<defs><marker id="v1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="20" width="230" height="64" rx="6"/><text class="t b" x="135" y="44" text-anchor="middle">vk-probe (application)</text><text class="m" x="135" y="64" text-anchor="middle">vkCreateInstance, vkEnumerate...</text>
<rect class="acc" x="20" y="110" width="230" height="110" rx="6"/><text class="t b" x="135" y="134" text-anchor="middle">libs/venus</text><text class="m" x="135" y="154" text-anchor="middle">vk: types and commands (generated)</text><text class="m" x="135" y="172" text-anchor="middle">cs: stream encoding</text><text class="m" x="135" y="190" text-anchor="middle">client: context, reply blob,</text><text class="m" x="135" y="206" text-anchor="middle">fenced calls</text>
<rect class="box" x="20" y="250" width="230" height="64" rx="6"/><text class="t b" x="135" y="274" text-anchor="middle">init / compositor</text><text class="m" x="135" y="294" text-anchor="middle">OPEN_RENDER on the display</text>
<path class="ln" d="M135 84 L135 108" marker-end="url(#v1)"/>
<path class="ln dash" d="M135 250 L135 222" marker-end="url(#v1)"/><text class="m" x="142" y="241">render node channel</text>
<rect class="acc" x="300" y="110" width="210" height="140" rx="6"/><text class="t b" x="405" y="134" text-anchor="middle">user/virtio-gpu</text><text class="m" x="405" y="154" text-anchor="middle">render node (gpuproto)</text><text class="m" x="405" y="172" text-anchor="middle">CTX_CREATE with capset 4</text><text class="m" x="405" y="190" text-anchor="middle">RESOURCE_CREATE_BLOB</text><text class="m" x="405" y="208" text-anchor="middle">RESOURCE_MAP_BLOB</text><text class="m" x="405" y="226" text-anchor="middle">fenced SUBMIT_3D</text>
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
2. `OPEN_RENDER` returns a render node channel; init starts `vk-probe`
   with it.
3. `vk-probe` uses `libs/venus` to create an instance, enumerate physical
   devices, read their properties and send the line
   `ok <count>|<name>|<type>|<api>|<vendor:device>`.

Output of a WSL run (lavapipe):

```text
virtio-gpu 00:06.0: 3D: virgl=1 blob=1 context_init=1 sdilena pamet=256 MiB capsety: virgl(v1, 308 B) virgl2(v2, 1408 B) venus(v0, 160 B)
vk-probe: renderer Venus, vk.xml 1.4.357, format proudu 1
vk-probe: zarizeni 0: llvmpipe (LLVM 20.1.2, 256 bits) (CPU), Vulkan 1.4.318, 10005:0000
vk-probe: rodin front 1, typu pameti 2, hald 1
init: test vulkan: fyzicke zarizeni Vulkan: llvmpipe (LLVM 20.1.2, 256 bits) (...)
[ok] vulkan: venus: jmeno fyzickeho zarizeni Vulkan od hostitele
```

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

Not supported yet (generation error, never silently wrong encoding):
unions (`VkClearColorValue` - step 3), pNext chains in outputs,
multidimensional arrays, pointers in output structs.

## Render node (`gpuproto`)

One channel = one client = at most one context. The display owner opens
it (`guiproto::display` `OPEN_RENDER`) and passes it on.

| Op | Fields | Reply |
|---|---|---|
| `INFO` | - | capsets (id, version, size), shared memory size |
| `GET_CAPSET` | id, version | capset data |
| `CREATE_CONTEXT` | capset | `CONTEXT` |
| `CREATE_BLOB` | memory (`HOST3D`), flags (`MAPPABLE`...), blob_id, size | `BLOB` (resource, size, `map_info`) + VMO when `MAPPABLE` |
| `DESTROY_BLOB` | resource | `DONE` |
| `SUBMIT` | ring_idx (only 0 for now), fence, stream (at most 60 KiB, multiple of 4) | `SUBMITTED` (with a fence, after processing) |

Closing the channel: the driver unmaps and frees the blobs and destroys
the context.

## Risks

⚠️ **Shared-memory VMOs cannot be revoked.** A client may keep the BAR
window after `DESTROY_BLOB`; when the driver gives the same range to
another client, it sees that client's data. Until the kernel supports
revocable mappings, only trusted processes may get a render node (init,
the compositor and applications it allows).

⚠️ **Uncached mapping.** The physical VMO is mapped as device memory. On
x86_64 with KVM it behaves like cached memory in practice; on aarch64
unaligned accesses would fault and reads are slow. Needed: mapping by
`map_info` (cached/WC) in the kernel - a small kernel change, for the main
branch.

⚠️ **The driver waits synchronously.** During a fenced `SUBMIT` (at most
30 s) it serves neither the display nor other clients (the first
`vkCreateInstance` with lavapipe can take seconds). If the fence does not
signal, the client gets `TIMED_OUT`, later commands wait for its
response and the driver keeps running; the client waits 40 s and then
marks the connection as desynchronized (`Error::Broken`). A `ring_idx`
other than 0 is rejected (a fence on a missing timeline would never
signal). The compositor over Vulkan will need an asynchronous queue with
fences.

Venus is not tested without KVM or under TCG (aarch64 in WSL); `vk.xml`
is 3.3 MB (a fraction of that compressed in git).

## Plan

1. ✅ Physical device name (`vkCreateInstance`, `vkEnumeratePhysicalDevices`,
   `vkGetPhysicalDeviceProperties`, queue families and memory types).
2. Memory: `vkCreateDevice`, `vkAllocateMemory` with a blob (`blob_id` =
   `VkDeviceMemory` id), mapped into the client.
3. Image and command buffer: `vkCreateImage`, `vkCmdClearColorImage`
   (`VkClearColorValue` union in the generator), `vkQueueSubmit`, reading
   pixels back through a buffer.
4. Compositor composition over Vulkan.
