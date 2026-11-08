# Disks: NVMe, AHCI and foreign disk protection

Three userspace drivers handle disks. All of them speak the same
[`blockproto`](file-system.md) protocol to their client (the `fsd` file
system, init, tests):

| driver | device | where | started by |
|---|---|---|---|
| `virtio-blk` | virtio 1.x virtual disk | QEMU | `devmgr` by virtio device ID |
| `nvme` | NVMe controller (PCI class 01:08:02) | VMware, real PC, QEMU `-device nvme` | `devmgr` by PCI class |
| `ahci` | SATA AHCI controller (PCI class 01:06:01) | VMware (SATA), real PC, QEMU `-device ahci` / q35 | `devmgr` by PCI class |

Request handling is shared in `blockproto::server`: range checks,
translation to our GPT partition and the write ban on foreign disks live in
one place for all three drivers. A driver only provides read, write, flush
and TRIM in device sectors (the `Disks` trait).

<figure class="diagram">
<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Disk driver start-up and foreign disk protection">
<defs><marker id="dk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="76" rx="6"/><text class="t b" x="85" y="44" text-anchor="middle">PCI Express</text><text class="m" x="85" y="63" text-anchor="middle">01:08:02 NVMe</text><text class="m" x="85" y="80" text-anchor="middle">01:06:01 AHCI</text>
<rect class="box" x="210" y="20" width="150" height="76" rx="6"/><text class="t b" x="285" y="44" text-anchor="middle">devmgr</text><text class="m" x="285" y="63" text-anchor="middle">restricted Resource</text><text class="m" x="285" y="80" text-anchor="middle">BAR, MSI(-X), DMA</text>
<path class="ln" d="M160 58 L208 58" marker-end="url(#dk1)"/>
<rect class="acc" x="410" y="12" width="190" height="92" rx="6"/><text class="t b" x="505" y="36" text-anchor="middle">nvme / ahci</text><text class="m" x="505" y="55" text-anchor="middle">queues, commands, interrupts</text><text class="m" x="505" y="71" text-anchor="middle">DMA only through the IOMMU</text><text class="m" x="505" y="87" text-anchor="middle">one channel per disk</text>
<path class="ln" d="M360 58 L408 58" marker-end="url(#dk1)"/><text class="m" x="364" y="50">pci, bar</text>
<rect class="acc" x="410" y="140" width="190" height="80" rx="6"/><text class="t b" x="505" y="164" text-anchor="middle">diskguard</text><text class="m" x="505" y="183" text-anchor="middle">reads MBR and GPT</text><text class="m" x="505" y="199" text-anchor="middle">decides: ours / foreign</text>
<path class="ln" d="M505 104 L505 138" marker-end="url(#dk1)"/>
<rect class="ok" x="640" y="120" width="110" height="56" rx="6"/><text class="t b" x="695" y="143" text-anchor="middle">our partition</text><text class="m" x="695" y="161" text-anchor="middle">partition, writable</text>
<rect class="warn" x="640" y="190" width="110" height="56" rx="6"/><text class="t b" x="695" y="213" text-anchor="middle">foreign disk</text><text class="m" x="695" y="231" text-anchor="middle">whole, read-only</text>
<path class="ln" d="M600 170 L638 150" marker-end="url(#dk1)"/><path class="ln" d="M600 195 L638 215" marker-end="url(#dk1)"/>
<rect class="box" x="210" y="160" width="150" height="60" rx="6"/><text class="t b" x="285" y="185" text-anchor="middle">init</text><text class="m" x="285" y="204" text-anchor="middle">disk with FS -&gt; fsd</text>
<path class="ln dash" d="M408 80 L362 175" marker-end="url(#dk1)"/><text class="m" x="300" y="130">block + channel, done</text>
<rect class="box" x="10" y="160" width="150" height="60" rx="6"/><text class="t b" x="85" y="185" text-anchor="middle">fsd</text><text class="m" x="85" y="204" text-anchor="middle">blockproto</text>
<path class="ln" d="M210 190 L162 190" marker-end="url(#dk1)"/>
<text class="m" x="10" y="270">The protection is in the driver, not in fsd: a faulty client (fsd, test, tool) cannot write outside our partition.</text>
<text class="m" x="10" y="290">Write and TRIM on a foreign disk return READ_ONLY; FEATURES reports READ_ONLY, so the FS is mounted read-only.</text>
<text class="m" x="10" y="310">A client of our partition sees sector 0 = the first sector of the partition; not the partition table or other partitions.</text>
<text class="m" x="10" y="330">On any doubt (damaged GPT, overlapping partitions, read error) the disk is treated as foreign.</text>
</svg>
<figcaption>Disk driver start-up and the decision what the client sees.</figcaption>
</figure>

## Risk and foreign disk protection

> **Risk.** On VMware with a physical disk attached and above all on a real
> computer, the NVMe or AHCI driver also sees disks that are not ours:
> Windows, Linux, backups. A single wrong write (formatting, FS log
> replay, a test writing to a "test" disk) to the wrong disk destroys
> foreign data and may leave another system unbootable. It does not take
> malice: init picking the wrong disk, or a test recognising its test disk
> by size alone, is enough.

So **before serving the client** the driver examines the disk
(`libs/diskguard`) and limits what the client sees and may do:

| disk | client sees | write and TRIM |
|---|---|---|
| GPT with **our partition**: type `C123ADA0-51BA-48C5-A536-C2CE6B2B3581` or partition name exactly `MagnumOpus` | only this partition (sector 0 = first sector of the partition) | allowed, only inside the partition; the partition attribute "read-only" (bit 60) forbids it |
| whole disk with **our FS** and no partition table (`OSFS` superblock in sectors 8..72, sector 0 without an MBR signature) | whole disk | allowed (QEMU data disk, older layout) |
| GPT without our partition | whole disk | **forbidden** (`READ_ONLY`) |
| our partition is invalid (beyond the disk, outside the usable area, **overlapping another partition**) | whole disk | **forbidden** |
| MBR (no GPT), protective MBR with damaged GPT headers | whole disk | **forbidden** |
| empty or unknown disk, read error | whole disk | **forbidden** |
| device reports write protection (NVMe write protect, virtio `RO`) | per the table | **forbidden** |

Details:

- **Foreign disks are read-only by default.** There is no switch yet that
  would allow writing to a foreign disk. When one is needed (e.g.
  installing to an empty disk), it must be an explicit user decision for one
  specific disk, not a global option.
- The whole GPT is verified: signature, header and table CRC, header
  location, usable area. If the primary header is damaged the backup at the
  end of the disk is used. If neither passes, the disk is foreign.
- The **first valid** partition of ours is used (in table order). The
  client does not see further partitions of ours yet (TODO: a channel per
  partition).
- The protection lives in the driver process, not only in `fsd`. A bug in
  `fsd`, init or a test cannot write outside our partition. The range the
  client sees is checked by one function (`Decision::translate`, including
  LBA overflow).
- A file system on a foreign disk is mounted read-only: the driver sets
  `READ_ONLY` and clears `DISCARD` in the `FEATURES` reply.

Residual risks (not covered by the protection):

- **Without an IOMMU** (`--iommu none`, an older computer, IOMMU disabled
  in firmware) a faulty or compromised driver can DMA into any memory and so
  bypass the disk protection too (e.g. overwrite another driver's memory).
  With an IOMMU a device may only write to its own driver's DMA memory
  (test `IOMMU chrani pamet mimo DMA`).
- The protection trusts the partition table. If the user marks a foreign
  partition with our type or the name `MagnumOpus` by mistake, the driver
  will write to it.
- The driver itself sends commands to the controller. A bug in driver code
  outside `blockproto::server` (e.g. a wrong LBA in a command) bypasses the
  protection; that is why request handling is shared and drivers only
  translate to commands.
- Disks with a logical sector other than 512 B (NVMe formatted to 4 KiB,
  SATA 4Kn) are skipped and logged for now. Their GPT lives in different
  sectors; without changes the examination would be wrong.

## NVMe driver

Per NVM Express Base 1.4. The driver is synchronous (one command at a
time), so it needs:

- controller reset (`CC.EN = 0`, wait for `CSTS.RDY`), the admin queue,
  enable (`CC`: 4 KiB page, 64/16 B entries),
- one MSI-X interrupt (vector 0) for both the admin and the I/O completion
  queue,
- Set Features "Number of Queues" and one I/O queue pair (16 entries),
- IDENTIFY of the controller (`MDTS`, `ONCS`, `VWC`) and active namespaces;
  each namespace with 512 B sectors and no metadata is one disk (up to 16).

| `blockproto` request | NVMe command |
|---|---|
| `READ`, `WRITE` | Read 02h / Write 01h, data via PRP (PRP1 + PRP list); split by `MDTS` |
| `FLUSH` | Flush 00h; without a volatile cache (`VWC = 0`) there is nothing to flush and `WRITE_THROUGH` is reported |
| `DISCARD` | Dataset Management 09h with the Deallocate attribute, up to 256 ranges per command |
| `FEATURES` | `FLUSH`, `WRITE_THROUGH` (no VWC), `DISCARD` (ONCS bit 2), `READ_ONLY` (write protect) |

When the driver ends (clients closed their channels) it performs a normal
shutdown (`CC.SHN`), so the controller saves its cache. When the controller
does not answer within 10 s, or returns an entry with a foreign command ID,
the driver disables it (`CC.EN = 0`, it stops touching DMA memory) and
further requests return `IO_ERROR`.

## AHCI driver

Per Serial ATA AHCI 1.3.1:

1. take the controller over from firmware (BIOS/OS handoff, `BOHC`),
   controller reset (`GHC.HR`) and AHCI mode (`GHC.AE`),
2. MSI interrupt (AHCI controllers usually have no MSI-X); without MSI the
   driver polls the controller,
3. for each implemented port (`PI`): stop, command list, received FIS
   area, spin-up (`SUD`); then one shared wait for the links (`DET = 3`,
   at most 1 s, an empty port is given up after 200 ms), ready disk, SATA
   disk signature (`0x101`; ATAPI is skipped),
4. IDENTIFY DEVICE: LBA48 capacity, write cache, FLUSH CACHE (EXT),
   TRIM (word 169), logical sector size (word 106).

| `blockproto` request | ATA command |
|---|---|
| `READ`, `WRITE` | READ/WRITE DMA EXT (25h/35h), one PRDT entry |
| `FLUSH` | FLUSH CACHE EXT (EAh) or FLUSH CACHE (E7h); with the cache disabled `WRITE_THROUGH` |
| `DISCARD` | DATA SET MANAGEMENT (06h) with TRIM, 8 B ranges (48-bit LBA, 16-bit count) |

A disk with the write cache enabled that cannot FLUSH reports no
durability guarantee, and the FS is mounted read-only on it. After a command
error (TFES) the driver restarts the port; a port that does not answer
within 20 s is stopped.

## Disk reports and start-up

`devmgr` sends the driver `pci <bus> <device> <function>` and `bar
<address> <size>` (NVMe BAR0, AHCI BAR5). For each disk the driver works
out what the client will see, creates a channel and sends `block` with the
channel end, then `done`. `devmgr` waits at most 20 s and forwards the
`block` messages to init.

Driver resources (least privilege):

| driver | interrupts | DMA |
|---|---|---|
| `nvme` | 2 | 1 MiB (queues, PRP list, 32 KiB buffer) |
| `ahci` | 2 | 4 MiB (up to 32 ports, at most 64 KiB each) |

## Tests and running in QEMU

`cargo xtask test` adds an NVMe and an AHCI controller with test disks
(`xtask/src/testdisks.rs`; the GPT is written by the same code the driver
reads it with):

| disk | controller | client sees |
|---|---|---|
| `scratch` | virtio-blk | our 1 MiB partition (virtio-blk tests) |
| `nvme-test` | NVMe | our 2 MiB partition, a marker in sector 1 |
| `ahci-test` | AHCI port 0 | our 3 MiB partition, a marker in sector 1 |
| `foreign` | AHCI port 1 | the whole foreign GPT disk (Linux partition), read-only |

The `disky` suite checks read, write (several requests, last sector of the
partition), flush, TRIM, partition bounds, the IOMMU and, for the foreign
disk, `READ_ONLY` for write and TRIM and unchanged foreign data. The GPT
examination is tested by `cargo test -p diskguard` (damaged header and
table, overlap, read-only attribute, MBR, empty disk).

The data disk on another controller (it gets a GPT with our partition):

```bash
cargo xtask run --data-disk nvme
```

```bash
cargo xtask test --data-disk ahci
```

Your own disk via NVMe (a disk without our partition will be read-only):

```bash
cargo xtask run -- -drive if=none,id=nv,format=raw,file=disk.img -device nvme,drive=nv,serial=nv0
```

## VMware and a real computer

- **VMware:** an NVMe or SATA (AHCI) disk controller. Convert the QEMU data
  disk (`OS_KEEP_DATA=1 cargo xtask run --data-disk nvme`) with
  `qemu-img convert -O vmdk target/data-gpt-<pid>.img data.vmdk`.
- **Real computer:** create our partition with a partitioning tool using
  type `C123ADA0-51BA-48C5-A536-C2CE6B2B3581` (e.g. `gdisk`, type code
  entered as a GUID), or any type with the name `MagnumOpus`. Formatting
  the partition from the system is not there yet (TODO); write an FS image
  from the [osfs](osfs.md) tool to the partition outside the system.
- The system sees the computer's other disks, but read-only (see the table
  above).
