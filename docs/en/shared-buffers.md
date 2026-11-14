# Shared buffers

A shared buffer (a window image, a video frame, GPU output) is **a VMO plus
a description** from the `libs/bufdesc` library. It is not a new kernel
object. The 6a decision is in
[gpu-amd-nvidia.md](gpu-amd-nvidia.md#impact-on-the-foundations), item 2.

## Status

| Part | What | Status |
|---|---|---|
| `libs/bufdesc` | description (fourcc, modifier, planes, width, height, placement, cache), validation against the VMO, 72 B encoding | **[implemented]** host tests (`cargo test -p bufdesc`) |
| `renderproto` `IMPORT_BUFFER` | description + VMO for a GPU driver | **[implemented]** in the protocol; Venus `NOT_SUPPORTED` ([render.md](render.md)) |
| window protocol | `BUFFER_CREATE_DESC` (NV12, I420, BGRA), `SYNC_POINTS` with acquire and release fences, YUV conversion in the compositor (`libs/yuv`, SIMD) | **[verified in QEMU]** step 6a-4 x86_64 and aarch64 ([gui.md](gui.md#buffers-with-a-description-and-explicit-synchronisation-video)) |
| YUV conversion on the GPU | `spirv::yuv` shader in `vkcomp`, bit-exact with `libs/yuv` | **[verified in QEMU]** WSL (vk-probe); the compositor does not use it yet **[designed]** |

## Description

| Field | What |
|---|---|
| `fourcc` | format as in DRM: `AR24`/`XR24` (B G R A), `AB24`/`XB24` (R G B A), `NV12`, `YU12` (I420), `P010` |
| `modifier` | layout as in DRM: `LINEAR` (0), driver tiling, `INVALID` = error |
| `width`, `height` | 1 to 32768 |
| planes | 1 to 4 × `{offset, pitch}` in format order |
| placement | system memory / device memory (a physical VMO the driver can revoke with `VMO_REVOKE`) |
| cache | `UNCACHED`, `WRITE_COMBINING`, `CACHED` (values of `abi::vm::cache`) |

The fourcc and modifier values are the same as in Linux, so they can be
passed to Mesa and drivers unchanged.

**Who may do what** is decided by the VMO handle rights, not by the
description:

- a consumer gets `READ | MAP`,
- a writer also `WRITE`,
- a device only with the `DMA` right ([dma.md](dma.md)).

**Synchronisation** is explicit only. Acquire and release fences travel in
protocol messages ([fences.md](fences.md)). There is no implicit `dma_resv`
as in Linux.

## Validation (untrusted input)

The description comes from another process. The receiver therefore calls
`BufDesc::validate(VMO size)` before it reads the buffer or passes it to a
device:

- **A known format with `LINEAR`:** the plane count matches the format and
  the row pitch is at least one pixel row. The last row of every plane must
  end inside the VMO (`offset + pitch × (rows - 1) + row`).
- **A tiling modifier:** only the driver knows the layout, so only the rough
  bound `offset + pitch × height` inside the VMO is checked.
- **Arithmetic** is overflow-checked everywhere: a hostile offset
  `u64::MAX - 10` ends with an error, it does not wrap.
- **The encoding** has a single form (72 B): unused planes are zero, the
  reserved byte is 0. Two different encodings of the same description could
  be abused to get around comparisons.

```rust
use bufdesc::{BufDesc, format};
// producer: NV12 1920x1080, rows aligned to 64 B
let d = BufDesc::linear(format::NV12, 1920, 1080, 64)?;
let vmo = Vmo::create(d.min_size()?)?;
// receiver
let d = BufDesc::decode(&msg)?;
d.validate(vmo.size()?)?;
```
