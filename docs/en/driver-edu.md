# Driver edu

The sample driver for the QEMU educational device `edu` (PCI `1234:11e8`,
`-device edu`). It is entirely on the driverkit and it is the first driver
in the [guide](drivers.md#the-first-driver-step-by-step): registers in
BAR0, a factorial computed with an MSI interrupt and a DMA transfer there
and back.

- code: `user/edu/src/lib.rs` (the generic driver), `src/main.rs` (start
  on the kernel), `src/model.rs` (device model for the simulation),
  `src/tests.rs`,
- manifest: `user/edu/edu.driver.toml` (BAR0, 1 interrupt, 16 KiB DMA),
- device facts: QEMU `docs/specs/edu.rst`; own code (MIT OR Apache-2.0),
  nothing taken from QEMU (GPL).

## Device

| Register | Offset | Meaning |
|---|---|---|
| `ID` | `0x00` | `0xRRrr00ed`: major and minor version |
| `LIVENESS` | `0x04` | reads back the written value with bits inverted |
| `FACTORIAL` | `0x08` | a write starts the computation, a read returns the result (32 bits) |
| `STATUS` | `0x20` | bit 0 computing, bit 7 interrupt after the computation |
| `IRQ_STATUS` | `0x24` | interrupt reason: bit 0 factorial, bit 8 DMA |
| `IRQ_ACK` | `0x64` | a write clears the written `IRQ_STATUS` bits |
| `DMA_SRC`, `DMA_DST`, `DMA_COUNT` | `0x80`, `0x88`, `0x90` | transfer addresses and length |
| `DMA_CMD` | `0x98` | bit 0 start, bit 1 direction to memory, bit 2 interrupt |

The device's internal DMA buffer is at `0x40000` (as the device sees it),
4 KiB. A transfer takes about 100 ms in QEMU. xtask adds the device with
`dma_mask=0xffffffffffffffff` (the default mask is 28 bits and the device
would clip DMA addresses above 256 MiB). A running edu transfer cannot be
cancelled; after the driver ends, devmgr stops it by disabling bus master.

## Protocol

| Message | Reply |
|---|---|
| `fact <n>` | `fact <n> <n!>` (32 bits, overflows like the hardware) |
| `dma <text>` | `dma <text>`: the text goes through DMA into the device and back (1-4096 B) |
| `sg <text>` | `sg <text>`: the same through a buffer in pages (`alloc_dma_pages`, [dma.md](dma.md)); the text lies across a page boundary, without an IOMMU the transfer goes in chunks (`device_chunks`) |
| `stale` | `stale <iova>`: the driver unmaps a buffer and lets the device write to its old address; the IOMMU blocks the write and the kernel logs it. Without an IOMMU it refuses (`chyba stale: bez IOMMU ...`), the device would write into freed memory |
| another command | `chyba <description>` (error) |
| `dk:ping`, `dk:info`, `dk:restart`, `dk:crash` | common driverkit messages |

While computing or transferring, the driver answers another command with
`chyba zarizeni je zaneprazdnene` (device busy).

## Tests

- in the simulation (`cargo test -p edu`): the factorial with an interrupt,
  DMA there and back without access outside the allocated memory, bad
  commands, the DMA quota from the manifest,
- in QEMU (`cargo xtask test --driver edu`, the `ovladace z manifestu`
  suite): ping, the factorial through MSI, 1000 B through DMA and the
  IOMMU, 3000 B across non-contiguous pages (`sg`), a write after unmapping
  blocked and in the kernel log (`stale`, only with an IOMMU), recovery
  after `dk:crash` and a restart after `dk:restart`. On
  x86_64 (VT-d) and aarch64 (SMMUv3).
