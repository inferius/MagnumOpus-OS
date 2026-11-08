# Network cards

Drivers for the cards of ordinary PCs and VMware, alongside virtio-net.
All of them are our own Rust code.

**Origin:** from the Linux drivers (GPL-2) and the vendors' datasheets
only facts were taken: device IDs, register offsets and bits, the order
of initialization and chip errata. No code from them was translated, so
the code is not a derived work under the GPL. Code from FreeBSD/OpenBSD
(`if_em`, `if_igc`, `if_re`, BSD/ISC licenses) may be taken, but none
has been so far. When it is, its origin and license will be noted in the
file header and in the third-party license file.

| Program | Cards | Verified |
|---|---|---|
| `e1000e` | Intel 82574L/LA, 82583V, I217, I218, I219 (PCH chipset from Lynx Point) | 82574L in QEMU (`-device e1000e`) on x86_64 and aarch64, with and without an IOMMU; host tests on a simulated 82574 and I219 |
| `igc` | Intel I225, I226 (2.5 Gb/s), I220-V, I221-V | **host tests only**, on simulated registers |
| `r8169` | Realtek RTL8111/8168 (IDs `10EC:8168`, `8161`, `8162`) | **host tests only**, on simulated registers |

⚠️ **Not yet verified on real hardware.** QEMU supports neither `igc`
nor `r8169`: QEMU 11.1 has no I225 model (`-device igb` is the 82576, a
different chip), and instead of the RTL8168 it has only the old
`rtl8139`. The simulation in the tests follows documentation and the
Linux drivers, not the behavior of real silicon. On a real card, the
fixes that Linux applies for individual revisions may be missing:

- **I217–I219:** PHY fixes (K1, jumbo frames), disabling ULP through the
  ME firmware, and flushing the rings after a hang (`FLUSH_DESC_REQUIRED`
  on the I219). The driver can only wake the PHY by toggling LANPHYPC.
  When the PHY does not respond and the ME firmware blocks its reset, it
  only reports this.
- **I225/I226:** PHY and EEE setup. The driver relies on the default PHY
  state after reset, which advertises all speeds up to 2.5 Gb/s.
- **RTL8168:** PHY setup and firmware (`rtl_fw`) for dozens of chip
  revisions, ASPM and EEE.

There is no Wi-Fi yet.

| Part | Where | What it does |
|---|---|---|
| chip logic | `libs/nic/src/e1000e.rs`, `igc.rs`, `r8169.rs` | reset, MAC, rings, interrupts, link over the `Hw` interface |
| device IDs | `libs/nic/src/ids.rs` | vendor/device ID → program table; `devmgr` reads it too |
| driver loop | `libs/nic/src/driver.rs` (feature `libos`) | start message, registers and DMA, interrupts, the `netproto::link` protocol |
| programs | `user/e1000e`, `user/igc`, `user/r8169` | only a `main` that calls `nic::driver::run` |
| interrupts | `libs/pci/src/irq.rs`, `msi.rs` | one interrupt through MSI, or MSI-X when MSI is missing |
| device manager | `user/devmgr/src/nic.rs` | finds the card by ID, measures the register window, starts the driver |
| tests | `libs/nic/src/tests/`, `user/init/src/tests/nic.rs` | simulated registers and DMA; in QEMU a second e1000e card |

## Architecture

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Network card driver">
<defs><marker id="nk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">devmgr</text><text class="m" x="95" y="64" text-anchor="middle">ID from nic::ids</text><text class="m" x="95" y="80" text-anchor="middle">measures the register BAR</text>
<path class="ln" d="M180 55 L248 55" marker-end="url(#nk1)"/><text class="m" x="190" y="47">starts</text>
<rect class="acc" x="250" y="20" width="270" height="180" rx="6"/><text class="t b" x="385" y="45" text-anchor="middle">process e1000e / igc / r8169</text>
<rect class="box" x="265" y="60" width="240" height="56" rx="4"/><text class="t" x="385" y="82" text-anchor="middle">nic::driver::run</text><text class="m" x="385" y="100" text-anchor="middle">loop: interrupt, channel, link 1 s</text>
<rect class="box" x="265" y="128" width="240" height="56" rx="4"/><text class="t" x="385" y="150" text-anchor="middle">chip logic (trait Nic)</text><text class="m" x="385" y="168" text-anchor="middle">over the Hw interface</text>
<path class="ln" d="M385 116 L385 126" marker-end="url(#nk1)"/>
<rect class="ok" x="590" y="70" width="160" height="70" rx="6"/><text class="t b" x="670" y="97" text-anchor="middle">netd</text><text class="m" x="670" y="116" text-anchor="middle">first card = interface</text>
<path class="ln" d="M520 95 L588 95" marker-end="url(#nk1)"/><path class="ln" d="M588 110 L522 110" marker-end="url(#nk1)"/><text class="m" x="526" y="88">INFO, LINK</text><text class="m" x="530" y="126">FRAME</text>
<rect class="warn" x="250" y="240" width="270" height="70" rx="6"/><text class="t b" x="385" y="263" text-anchor="middle">card</text><text class="m" x="385" y="282" text-anchor="middle">registers (BAR) · RX/TX rings in DMA</text><text class="m" x="385" y="298" text-anchor="middle">MSI (MSI-X) · DMA in the IOMMU domain</text>
<path class="ln" d="M330 200 L330 238" marker-end="url(#nk1)"/><text class="m" x="200" y="224">MMIO, descriptors</text>
<path class="ln" d="M440 238 L440 202" marker-end="url(#nk1)"/><text class="m" x="448" y="224">interrupt</text>
<rect class="box" x="10" y="240" width="200" height="70" rx="6"/><text class="t b" x="110" y="263" text-anchor="middle">host tests</text><text class="m" x="110" y="282" text-anchor="middle">simulated registers and DMA</text><text class="m" x="110" y="298" text-anchor="middle">instead of Hw from libos</text>
<path class="ln dash" d="M210 270 L263 175" marker-end="url(#nk1)"/>
</svg>
<figcaption>One process per card. The chip logic does not know libos, so the same code runs in the system and in tests on a simulation.</figcaption>
</figure>

**The `Hw` interface** (`libs/nic/src/lib.rs`) has these parts:

- reading and writing registers in 8, 16 and 32 bits (Realtek has 8- and
  16-bit registers at odd offsets, so reading a whole word would clear
  interrupt flags),
- DMA memory (address for the device, reading and writing, the 32-bit
  descriptor status word in a single access),
- three barriers (`dma_wmb`, `dma_rmb`, `mmio_wmb`, see `libos::barrier`),
- waiting.

In the system `driver::Mmio` implements it over `Mapping`; in the tests
the simulation does.

**The `Nic` interface** describes a card after initialization: `link`,
`interrupt`, `receive`, `reclaim`, `send` + `kick`, `stop`, and `notes`
for reporting. The driver loop is the same for all chips.

## Start and loop

1. `devmgr` finds a PCI function with an ID from `nic::ids`
   (`program_for`). The driver gets:
   - a restricted Resource: only the BARs and configuration of this
     function, 4 interrupts, 2 MiB of DMA, and with an IOMMU its own
     domain,
   - the PCI configuration page,
   - a channel to the client; its other end goes to init in a `net`
     message.

   On the bootstrap channel `devmgr` sends `pci <bus> <device> <function>`
   and `regs <address> <size>`, which is the first memory BAR. The manager
   measures the size, so the driver does not rewrite BARs.
2. The driver enables memory and DMA in the PCI command register
   (disabling INTx), maps the registers and creates DMA memory according
   to `Layout`. Then it resets and sets up the card (the chip's `open`).
   It prints any warnings from initialization: a card without a MAC, a
   PHY that does not respond, and so on.
3. It sets up the interrupt only after the card reset: **MSI** first,
   and **MSI-X** when the function has no MSI (`pci::irq::DeviceIrq`).
   The I217–I219 have no MSI-X. One vector is enough, because there is
   only one receive queue and one transmit queue.
4. It sends `INFO` (MAC, MTU 1500, link, speed) and serves:
   - **the interrupt**: acknowledges it before handling, reads the causes
     (`ICR`, `IntrStatus`), takes back sent buffers and passes received
     frames to `netd`,
   - **messages from netd**: queues frames while free descriptors
     remain, then tells the card with a single index write. When no
     descriptor is free, it does not read the channel (backpressure).
   - **every second** it checks the link, in case an interrupt about a
     change was lost. It sends a change as a `LINK` message.
5. End (netd closed the channel, the card disappeared or reports a bus
   error): the driver stops and resets the card, so that it no longer
   uses the process's DMA memory, and disables MSI.

**Which card is the interface:** `netd` for now uses only the first card
init receives. `devmgr` starts card drivers after virtio, so virtio-net
remains the first card in QEMU. On a computer without virtio it is the
first card found. Multiple interfaces and routing between them are not
there yet.

## DMA memory and rings

All three chips have a 16 B descriptor, so the memory layout (`Layout`)
is shared, 1.03 MiB in total:

| Offset | Content |
|---|---|
| 0 | receive ring, 256 descriptors (4 KiB) |
| 4 KiB | transmit ring, 256 descriptors |
| 8 KiB | 256 receive buffers of 2 KiB, then 256 transmit buffers |

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Receive ring">
<defs><marker id="nk2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="t b" x="10" y="24">Intel (e1000e, igc): indexes in registers</text>
<rect class="ok" x="10" y="40" width="60" height="34"/><rect class="ok" x="70" y="40" width="60" height="34"/><rect class="ok" x="130" y="40" width="60" height="34"/>
<rect class="box" x="190" y="40" width="60" height="34"/><rect class="box" x="250" y="40" width="60" height="34"/><rect class="box" x="310" y="40" width="60" height="34"/><rect class="box" x="370" y="40" width="60" height="34"/>
<rect class="warn" x="430" y="40" width="60" height="34"/>
<text class="m" x="100" y="62" text-anchor="middle">done (DD), driver reads</text>
<text class="m" x="310" y="62" text-anchor="middle">empty, owned by the card</text>
<text class="m" x="460" y="62" text-anchor="middle">free</text>
<path class="ln" d="M40 100 L40 78" marker-end="url(#nk2)"/><text class="m" x="10" y="114">next to read</text>
<path class="ln" d="M190 100 L190 78" marker-end="url(#nk2)"/><text class="m" x="160" y="114">RDH (card)</text>
<path class="ln" d="M460 100 L460 78" marker-end="url(#nk2)"/><text class="m" x="430" y="114">RDT (driver)</text>
<text class="t b" x="10" y="150">Realtek (r8169): owner in the descriptor</text>
<rect class="ok" x="10" y="166" width="60" height="34"/><rect class="ok" x="70" y="166" width="60" height="34"/>
<rect class="box" x="130" y="166" width="60" height="34"/><rect class="box" x="190" y="166" width="60" height="34"/><rect class="box" x="250" y="166" width="60" height="34"/><rect class="box" x="310" y="166" width="60" height="34"/><rect class="box" x="370" y="166" width="60" height="34"/><rect class="acc" x="430" y="166" width="60" height="34"/>
<text class="m" x="70" y="188" text-anchor="middle">OWN = 0</text><text class="m" x="280" y="188" text-anchor="middle">OWN = 1 (owned by the card)</text><text class="m" x="460" y="188" text-anchor="middle">RingEnd</text>
<text class="m" x="10" y="230">Intel: one descriptor stays free (RDT = the last one handed over), so a full ring is not mistaken for an empty one.</text>
<text class="m" x="10" y="246">Realtek: the card has its own index from reset and follows the OWN bits; RingEnd returns it to the start.</text>
</svg>
<figcaption>The receive ring. Transmit is the mirror image: the driver fills descriptors and moves the tail (Intel TDT, Realtek TxPoll).</figcaption>
</figure>

- **Receive:** the driver reads descriptors while they are done (Intel
  bit DD, Realtek OWN = 0). It drops and counts a frame with an error
  (CRC, length), a frame split over several buffers (Realtek), or a frame
  outside 14 to 1514 B. Realtek writes the length including the FCS
  (4 B), and the driver subtracts it. Intel strips the FCS itself
  (`RCTL.SECRC`).
- **Transmit:** the frame is copied into the descriptor's buffer. The
  driver pads a short frame with zeros to 60 B, because some Realtek
  revisions cannot. The descriptor has the command "end of frame, add
  FCS, report when sent". A free descriptor is recognized by the card's
  write-back: Intel DD, Realtek OWN = 0.
- **Barriers:** `dma_wmb` and `mmio_wmb` before writing an index to a
  register, and `dma_rmb` after reading a descriptor's status, before
  reading its data. Realtek also writes the OWN bit only after the buffer
  address.

⚠️ **DMA without an IOMMU:** as with virtio-net. With an IOMMU the card
can access only the memory of its domain, without one (`--iommu none`)
anywhere. Emulated Intel cards in QEMU always go through the IOMMU; they
have no `iommu_platform` switch.

## Chips

### e1000e

Legacy descriptors, registers according to the 82574 datasheet.
Initialization:

1. Disable interrupts, stop receive and transmit, `GIO_MASTER_DISABLE`
   (finish DMA in progress).
2. MAC reset (`CTRL.RST`). The PHY is not reset: on the PCH, a PHY reset
   would require reloading the OEM configuration from NVM. On the PCH the
   reset happens under the PHY semaphore (`EXTCNF_CTRL.SWFLAG`), because
   the ME firmware may be using the PHY right then.
3. Wait for the NVM: 82574 `EECD.AUTO_RD`, PCH `STATUS.LAN_INIT_DONE`.
4. **PCH:** try to read the PHY ID through `MDIC` (addresses 1 and 2).
   When the PHY does not respond ("ultra low power" mode, which Windows
   sets when shutting down without a cable), cancel forced SMBus and
   toggle LANPHYPC (PHY power). If the ME blocks the PHY reset
   (`FWSM.RSPCIPHY`), only report it.
5. MAC from `RAL0/RAH0`, where the card loads it from NVM on reset. When
   it is not there: the 82574 reads it from EEPROM (`EERD`); the PCH (NVM
   in the BIOS flash) gets a random local address `02:...`, which the
   driver writes to `RAR0`.
6. `CTRL.SLU`, rings, `RCTL` (receive: 2 KiB buffer, broadcast and
   multicast frames, no FCS), `TCTL` (pad short frames), `TXDCTL` with
   write-back after each descriptor, `ITR` at most ~20,000 interrupts per
   second, `IMS`.

### igc

Advanced descriptors, queue 0. Interrupts in MSI mode through `ICR`,
without MSI-X vectors for queues.

1. `GIO_MASTER_DISABLE`, stop, device reset (`CTRL.DEV_RST`) and wait
   for `EECD.AUTO_RD`.
2. MAC from `RAL0/RAH0`. A card without NVM (`..._BLANK_NVM`) gets a
   local address.
3. Receive:
   - `SRRCTL`: 2 KiB buffer, type "advanced, one buffer", drop when the
     queue is full,
   - `RXDCTL.ENABLE` and wait until it is enabled,
   - only then `RDT`.
4. Transmit: `TXDCTL.ENABLE`, write-back after each descriptor.
5. Speed from `STATUS`: bit 22 together with the 1 Gb/s bit reports
   2.5 Gb/s.

### r8169

8-, 16- and 32-bit registers, configuration lock `Cfg9346`.

1. Disable interrupts, reset `ChipCmd.CmdReset` (the chip clears it
   itself).
2. MAC from `MAC0..5`. When it is not there, the driver writes a local
   address, but only with `Cfg9346` unlocked (upper part first).
3. With `Cfg9346` unlocked:
   - `RxMaxSize` 2 KiB (a longer frame is dropped, not split),
   - `CPlusCmd` without offload and VLAN, `PCIDAC` for DMA above 4 GiB,
   - the ring addresses (upper half first),
   - `ChipCmd` TE and RE, `RxConfig` (broadcast, multicast, own
     address), `TxConfig`, all multicast addresses in `MAR0`.
4. Lock the configuration and set `IntrMask`. An interrupt is
   acknowledged by writing ones to `IntrStatus`.

The driver does not handle the RTL8101/8136 (100 Mb/s only), the RTL8169
(PCI) or the RTL8125/8126 (2.5/5 Gb/s, different registers).

## Tests

**On the host** (`cargo test -p nic`, part of `cargo xtask test`) the
register models in `libs/nic/src/tests/` walk the DMA memory like a real
card. They read descriptors at the addresses the driver wrote into them,
write frames and status, and raise interrupts. An access outside the DMA
memory, a wrong descriptor type, an unpadded short frame or a write to a
locked configuration (Realtek) fails the test.

Shared scenarios run for the 82574, I219, I226 and RTL8168:

- initialization and link,
- sending frames of 14 to 1514 B (short ones padded to 60 B),
- 1000 frames in both directions with varying batch sizes (both rings
  wrap several times),
- receive ring overflow and recovery,
- backpressure (without `kick` the ring does not drain),
- interrupts and a link change,
- a removed card (registers read all ones → `Gone`),
- `stop` halts DMA.

Tests of individual chips:

- the MAC source (NVM, EEPROM, random),
- waking the PHY on the PCH and respecting the ME block,
- igc speeds up to 2.5 Gb/s,
- bad and split frames,
- FCS subtraction on Realtek,
- DMA above 4 GiB (`PCIDAC`).

**In QEMU** (`cargo xtask test`, suite "sitove karty") the second card
is `-device e1000e` (82574L) in its own slirp network. The test talks to
the driver directly with the `netproto::link` protocol:

| Test | What it verifies |
|---|---|
| INFO, MAC from NVM and 1 Gb/s link | the driver read the card's MAC (QEMU `52:54:00:...`, not a local one), the link comes up within 10 s, full duplex |
| ARP to the slirp gateway | a frame goes out through the card and the reply comes back |
| 640 frames there and back | 20 rounds of 32 ARP requests, both rings wrap twice |

The suite runs on x86_64 (Intel VT-d IOMMU) and aarch64 (SMMUv3).
Without an IOMMU it can be run with `--iommu none`.
