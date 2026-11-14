# Drivers: the author's guide

How to write a driver for MagnumOpus-OS quickly, safely and with a minimum
of repeated code. All drivers run in userspace as separate processes; the
kernel gives them only what their device needs. The **driverkit**
(`libs/driverkit`) does the common work (registers, DMA, interrupts, event
loop, crash recovery), and the same driver runs in the system as well as
in a hardware simulation on the host.

Quick start:

```bash
cargo xtask new-driver my-driver --pci 1234:5678
cargo test -p my-driver
cargo xtask test --driver my-driver
```

The first command creates a skeleton (code, manifest, device model, test,
documentation page), the second tests it in the simulation without QEMU,
the third builds the system and runs only this driver's tests in QEMU.

## Architecture

<figure class="diagram">
<svg viewBox="0 0 780 380" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Driver architecture">
<defs><marker id="dr1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="300" width="760" height="70" rx="6"/>
<text class="t b" x="390" y="324" text-anchor="middle">kernel</text>
<text class="m" x="390" y="343" text-anchor="middle">Resource (capability to hardware) · MSI/MSI-X interrupts · DMA memory · channels</text>
<text class="m" x="390" y="360" text-anchor="middle">IOMMU: every PCI device has its own domain, may only reach its driver's memory</text>
<rect class="acc" x="10" y="20" width="200" height="90" rx="6"/>
<text class="t b" x="110" y="45" text-anchor="middle">init (supervisor)</text>
<text class="m" x="110" y="64" text-anchor="middle">starts devmgr</text>
<text class="m" x="110" y="80" text-anchor="middle">receives driver channels</text>
<text class="m" x="110" y="96" text-anchor="middle">hands them to services</text>
<rect class="acc" x="290" y="20" width="200" height="90" rx="6"/>
<text class="t b" x="390" y="45" text-anchor="middle">devmgr</text>
<text class="m" x="390" y="64" text-anchor="middle">scans PCI, manifests</text>
<text class="m" x="390" y="80" text-anchor="middle">restricts Resource, spawns</text>
<text class="m" x="390" y="96" text-anchor="middle">recovers after a crash</text>
<rect class="ok" x="570" y="20" width="200" height="90" rx="6"/>
<text class="t b" x="670" y="45" text-anchor="middle">service</text>
<text class="m" x="670" y="64" text-anchor="middle">fsd, inputd, netd ...</text>
<text class="m" x="670" y="80" text-anchor="middle">the driver's client</text>
<text class="m" x="670" y="96" text-anchor="middle">(protocol over a channel)</text>
<rect class="ok" x="290" y="170" width="200" height="90" rx="6"/>
<text class="t b" x="390" y="195" text-anchor="middle">driver</text>
<text class="m" x="390" y="214" text-anchor="middle">only its BARs, quotas</text>
<text class="m" x="390" y="230" text-anchor="middle">interrupts and DMA</text>
<text class="m" x="390" y="246" text-anchor="middle">driverkit</text>
<rect class="box" x="570" y="170" width="200" height="90" rx="6"/>
<text class="t b" x="670" y="195" text-anchor="middle">device</text>
<text class="m" x="670" y="214" text-anchor="middle">registers (BAR)</text>
<text class="m" x="670" y="230" text-anchor="middle">DMA through the IOMMU</text>
<text class="m" x="670" y="246" text-anchor="middle">MSI interrupts</text>
<path class="ln" d="M210 65 L288 65" marker-end="url(#dr1)"/>
<path class="ln" d="M390 110 L390 168" marker-end="url(#dr1)"/>
<text class="m" x="382" y="145" text-anchor="end">Resource, PCI config, channel</text>
<path class="ln dash" d="M290 200 L110 200 L110 112" marker-end="url(#dr1)"/>
<text class="m" x="120" y="190">dev &lt;name&gt; + channel</text>
<path class="ln" d="M110 20 L110 8 L670 8 L670 18" marker-end="url(#dr1)"/>
<path class="ln" d="M570 215 L492 215" marker-end="url(#dr1)"/>
<path class="ln" d="M490 230 L568 230" marker-end="url(#dr1)"/>
<path class="ln" d="M570 95 L530 95 L530 185 L492 185" marker-end="url(#dr1)"/>
<text class="m" x="536" y="150">requests</text>
</svg>
<figcaption>Who starts what and who talks to whom. Solid arrows: handing over rights and messages; dashed: reporting a new driver to init.</figcaption>
</figure>

- The **kernel** gives out hardware only through a **Resource**: a
  capability listing physical ranges (BARs, the PCI configuration page),
  interrupt and DMA memory quotas. Init holds the root Resource and hands
  it to devmgr.
- **devmgr** (the device manager) scans the PCI buses, finds a driver for
  each device (built-in tables in `user/devmgr/src/main.rs`, or a
  [manifest](#manifest)), creates a **restricted Resource** for that
  device only (`restrict_device`: with an IOMMU also its own DMA domain)
  and spawns the driver process.
- The **driver** receives the restricted Resource, its device's
  configuration page and a channel to its client. It sees nothing else in
  the system.
- The **service** (fsd, inputd, netd, init) is the driver's client. Init
  hands it the channel.
- **Recovery**: when a manifest driver crashes, devmgr disables the
  device's bus master (stopping DMA), creates a new Resource and starts it
  again; init gets a new channel ([Life cycle](#life-cycle-and-crash-recovery)).

## Who works on what and the security boundaries

| Part | Runs | May | May not |
|---|---|---|---|
| kernel | ring 0 / EL1 | everything | |
| devmgr | userspace, root Resource without `SET_CLOCK` | the whole PCI configuration, create restricted Resources | set the clock |
| driver | userspace, restricted Resource | its BARs, its configuration page, `irq_max` interrupts, `dma_max` B of DMA memory | other devices, other BARs, other processes' memory, DMA outside its memory (with an IOMMU) |
| device | hardware | DMA into its driver's domain (with an IOMMU) | anything else with an IOMMU |
| service | userspace | the protocol over the channel | hardware |

Risks to know about (and to report when a driver is affected):

- **Without an IOMMU** (`--iommu none`, a machine without VT-d / AMD-Vi /
  SMMU) a device may DMA **anywhere in memory**. A driver bug (a wrong
  address in a descriptor) can then corrupt the kernel. The kernel reports
  it at boot (`IOMMU neni: ...`, see [Log messages](messages.md#kernel-iommu)).
  The driverkit at least protects the driver side: only
  `DmaBuffer::iova_of` hands out device addresses, with bounds checking.
- **Devices can lie.** A faulty or malicious device (Thunderbolt, USB)
  returns a nonsense index, length or status. Every number from a device
  is outside input: the driverkit rings check it (`DescRing::complete_to`
  rejects a head outside the ring) and the driver should return
  `DriverError::Device`, not panic or read outside a buffer.
- **MSI without remapping** (`intremap=off` in QEMU): a DMA-capable device
  could in theory send another device's MSI message. The kernel does not
  enable interrupt remapping yet.
- **After a driver crash** the device may still be finishing DMA. devmgr
  therefore disables bus master before restarting the driver, and the new
  driver starts with a device reset in `probe`.

## What drivers keep repeating

A survey of the existing drivers (`virtio-blk`, `virtio-net`,
`virtio-input`, `virtio-rng`, `virtio-gpu`, `xhci`, `usb-hid`, `ps2`,
`fbdisplay`, `nvme`, `ahci`) showed that a large share of the code is the
same steps over and over:

| What | Where it repeated | What the driverkit does |
|---|---|---|
| taking the start message (Resource, config, channel), the `pci b d f` message | every driver's own `main` (30-50 lines) | `driverkit::os::main::<Driver>(startup, "name")` |
| mapping a BAR (page alignment, offset) | `virtio::Window`, xhci, nvme, ahci | `Platform::map_bar`, a window with bounds checks |
| registers as numbers (`read32(0x98) \| 1 << 2`) | everywhere | typed registers and fields: `registers!`, an error instead of a silent overwrite |
| MSI-X, or MSI when there is no MSI-X | virtio, xhci, nvme, ahci | `Platform::enable_irq` picks by itself |
| DMA memory + physical address + mapping | every driver | `Platform::alloc_dma` -> `DmaBuffer` with bounds |
| descriptor rings (head/tail, phase bit) | nvme, ahci, xhci, virtqueue | `ring::DescRing`, `ring::PhaseRing` |
| a `wait_many` loop over the interrupt and the channel, acking the interrupt | virtio-input, virtio-net, xhci, usb-hid | `Driver` + `run`: interrupt, message, timer, client gone |
| resetting the device on exit (no DMA after the process ends) | each driver by hand, differently | `Driver::stop` always runs, even after an error |
| logs prefixed with `name BB:DD.F:` | by hand in every message | `driverkit::log!` |
| restarting a crashed driver | none (devmgr only started it) | devmgr, per the manifest |
| an entry in devmgr (driver table) and xtask (program list) | every new driver | the `<name>.driver.toml` manifest, xtask finds it itself |

Converted drivers: `virtio-rng` (171 -> 147 lines) and `virtio-input`
(251 -> 203 lines) use `driverkit::os` and keep using the `virtio`
library; taking the start message, the loop and stopping the device are
gone. The sample driver `edu` is entirely on the driverkit and is also
tested in the simulation.

## The driverkit

`libs/driverkit`, `no_std`. Feature `os` = the backend on the kernel (only
for the OS target), feature `sim` = the simulation on the host.

| Module | Contents |
|---|---|
| `regs` | `Reg<W>`, `Field<W>`, the `registers!` macro |
| `dma` | `DmaBuffer`: reads and writes with bounds, `iova_of` |
| `ring` | `DescRing` (the driver produces), `PhaseRing` (the device produces) |
| `Platform` | what a driver needs from the system: BAR, DMA, interrupts, time, channel, logs, barriers |
| `Driver`, `run`, `Runner` | life cycle and the event loop |
| `manifest` | reading and checking `<name>.driver.toml` |
| `config` | the message from devmgr (`pci ... bar ...`) |
| `proto` | `dk:` messages common to all drivers, exit codes |
| `os` | `OsPlatform` on the kernel (feature `os`) |
| `sim` | `Sim`, `DeviceModel`, `RegFile` (feature `sim`) |

### Registers

Registers are described once, from the datasheet. The constant is the
register, the module of the same name holds its fields (`NAME: bit` or
`NAME: from..=to`):

```rust
driverkit::registers! {
    /// Identification `0xRRrr00ed`: RR major version, rr minor.
    pub ID @ 0x00: u32 {
        MAGIC: 0..=7,
        MINOR: 16..=23,
        MAJOR: 24..=31,
    }
    /// Liveness check: reads back the written value with bits inverted.
    pub LIVENESS @ 0x04: u32;
    /// Writing a number starts the factorial, reading returns the result.
    pub FACTORIAL @ 0x08: u32;
    pub STATUS @ 0x20: u32 {
        /// Computation running (read only).
        BUSY: 0,
        /// Raise an interrupt after the computation.
        IRQ_FACT: 7,
    }
```

Usage: `STATUS::IRQ_FACT.set(&regs)` (read-modify-write of one bit),
`FACTORIAL.write(&regs, n)`, `ID::MAJOR.get(id)`. Writing a value that
does not fit the field (`Field::write`, `Field::value`) returns
`DriverError::FieldOverflow` with the register and field names; the
neighbouring bits do not change. Watch out for registers where a read or
a write triggers something (W1C, queues): use `Reg::write` with the whole
value there, not `modify`.

### DMA and rings

`Platform::alloc_dma(name, size)` returns a `DmaBuffer`. Numbers in it are
little endian (`read::<u32>`, `write::<u64>`), every access checks bounds.
The address the driver writes into a register or a descriptor comes
**only** from `iova_of(offset, length)`: it checks that the whole range
lies inside the buffer.

`DescRing` is a ring the driver writes (e1000/igc, r8169, NVMe SQ, AHCI):
`push` returns a slot, `tail` goes into the device register,
`complete_to(head)` returns the completed slots and rejects a nonsense
head from the device. `PhaseRing` is a completion ring with a phase bit
(NVMe CQ, xHCI event ring): `pop` returns a new entry according to the
phase. Barriers: `p.dma_wmb()` between descriptors and the index,
`p.mmio_wmb()` before writing the doorbell register, `p.dma_rmb()` after
reading an index from the device.

### Platform and Driver

A driver is generic over `P: Platform`. The same code runs on the kernel
(`OsPlatform`) and in the simulation (`SimPlatform`):

```rust
impl<P: Platform> Driver<P> for Edu<P> {
    fn probe(p: &P) -> Result<Self> {
        let regs = p.map_bar(0)?;
        let id = ID.read(&regs);
        if ID::MAGIC.get(id) != 0xed {
            return Err(DriverError::Device(format!("neznama identifikace {id:#x}")));
        }
```

| Method | When | What to do |
|---|---|---|
| `probe` | process start | reset the device, check identification, registers, `enable_irq`, `alloc_dma` |
| `start` | after `probe` | introduce itself to the client, first requests |
| `message` | a message from the client | a request; reply with `p.send` |
| `interrupt` | an interrupt (already acknowledged) | read the status, ack it in the device, finish the work |
| `next_timer` / `timer` | a deadline | periodic work (virtio-rng: refilling entropy) |
| `stop` | always at the end | stop the device: no DMA nor interrupts after returning |
| `client_closed` | the client closed the channel | `false` (default) = end; `true` = keep running without a client (`send` returns `Sent::NoClient`) |

A driver whose service is recovered after a crash through the
supervisor's adapter (`inputd`, see [Service recovery](service-recovery.md))
must not end when it loses the service: it returns `true` from
`client_closed` and answers the new instance's synchronisation in
`message` (that is what `virtio-input` does with `ISYN`).

Code that needs something only the kernel has (e.g. the `virtio` library
over `Ecam`) implements `Driver<OsPlatform>` directly and uses
`p.ecam()`, `p.resource()`, `p.adopt_irq(irq)` (that is what `virtio-rng`
and `virtio-input` do). There is no simulation for it then.

### Common `dk:` messages

Client messages starting with `dk:` are handled by the loop itself:

| Message | Reply / effect |
|---|---|
| `dk:ping` | `dk:pong <name>` |
| `dk:info` | `dk:info <name> <PCI address> <vendor>:<device>` |
| `dk:restart` | the driver stops the device and exits with code 75; devmgr starts it again at once (not counted as a crash) |
| `dk:crash` | like a real crash (code 70): the driver neither stops nor releases anything (the kernel stops the device, the DMA quarantine), devmgr recovers it as after a crash; for recovery tests |

## The first driver step by step

The example is the `edu` driver (`user/edu`) for the QEMU educational
device `edu` (PCI `1234:11e8`): registers in BAR0, a factorial computed
with an MSI interrupt and a DMA transfer. It is the smallest driver that
has everything real hardware has. The facts about the device come from
the QEMU specification `docs/specs/edu.rst` (QEMU code is under the GPL,
only the facts about the registers are taken).

### 1. Skeleton

```bash
cargo xtask new-driver edu --pci 1234:11e8
```

Creates `user/edu/` (Cargo.toml, manifest, `src/lib.rs` with the driver,
`src/main.rs`, `src/model.rs` with the device model, `src/tests.rs`), the
pages `docs/cs/ovladac-edu.md` and `docs/en/driver-edu.md`, adds the
workspace member and a row in `docs/README.md`. Nothing else needs to
change: devmgr and xtask find the driver through its manifest.

The logic lives in the library (`src/lib.rs`); the program only starts the
driverkit. On the host the program builds empty so that `cargo test` works:

```rust
#![cfg_attr(target_os = "magnumopus", no_std, no_main)]

#[cfg(target_os = "magnumopus")]
libos::entry!(main);

#[cfg(target_os = "magnumopus")]
fn main(startup: libos::Startup) -> i64 {
    driverkit::os::main::<edu::Edu<_>>(startup, "edu")
}
```

### 2. Manifest

```toml
[match]
pci = ["1234:11e8"]

[resources]
bars = [0]
irq_max = 1
dma_max = "16K"
```

Only what the driver really needs: edu has its registers in BAR0, one
interrupt and two 4 KiB buffers. The full format is in
[Manifest](#manifest).

### 3. Registers and probe

Registers from the specification (see [Registers](#registers)). `probe`
maps the BAR, checks identification and liveness, enables the interrupt
and allocates DMA:

```rust
        // Liveness check: the card returns the bits inverted.
        LIVENESS.write(&regs, 0x1234_5678);
        if LIVENESS.read(&regs) != !0x1234_5678 {
            return Err(DriverError::Device(String::from("karta neprosla kontrolou zivota")));
        }
        let irq = p.enable_irq()?;
        let out = p.alloc_dma("edu-tam", EDU_BUF_SIZE)?;
        let back = p.alloc_dma("edu-zpet", EDU_BUF_SIZE)?;
```

### 4. A request and the interrupt

A message from the client starts the work, the interrupt finishes it. The
driver does not busy-wait: between the message and the interrupt the
loop serves other events.

```rust
        if let Some(n) = text.strip_prefix("fact ") {
            let Ok(n) = n.trim().parse::<u32>() else {
                return reply(p, "chyba fact ocekava cislo");
            };
            STATUS::IRQ_FACT.set(&self.regs);
            FACTORIAL.write(&self.regs, n);
            self.op = Op::Factorial(n);
            return Ok(());
        }
```

```rust
    fn interrupt(&mut self, p: &P, irq: IrqId) -> Result<()> {
        debug_assert_eq!(irq, self.irq);
        let why = IRQ_STATUS.read(&self.regs);
        // Ack before further work: a new interrupt is not lost.
        IRQ_ACK.write(&self.regs, why);
        match self.op.clone() {
            Op::Factorial(n) if IRQ_STATUS::FACT.get(why) != 0 => {
                let v = FACTORIAL.read(&self.regs);
                self.op = Op::Idle;
                reply(p, &format!("fact {n} {v}"))
            }
```

### 5. DMA

Device addresses only from `iova_of`; a barrier before starting the
transfer so that the data in memory is visible before the device starts
reading:

```rust
    fn start_dma(&self, p: &P, src: u64, dst: u64, count: u64, to_ram: bool) -> Result<()> {
        DMA_SRC.write(&self.regs, src);
        DMA_DST.write(&self.regs, dst);
        DMA_COUNT.write(&self.regs, count);
        let cmd = DMA_CMD::START.value(1)? | DMA_CMD::IRQ.value(1)? | DMA_CMD::TO_RAM.value(to_ram as u64)?;
        // Data in memory before the device starts the transfer.
        p.mmio_wmb();
        DMA_CMD.write(&self.regs, cmd);
        Ok(())
    }
```

### 6. Stop

`stop` always runs, even after an error. It must stop the device so that
it no longer touches DMA memory or raises interrupts:

```rust
    fn stop(&mut self, _p: &P) {
        // No more interrupts or DMA: disable the interrupt after the factorial.
        // A running edu transfer cannot be cancelled; it completes into buffers
        // owned by the driver (with an IOMMU the device cannot reach them after
        // the process ends).
        STATUS::IRQ_FACT.clear(&self.regs);
    }
```

### 7. Test in the simulation

The device model (`src/model.rs`) mimics the registers, interrupts and DMA
([Hardware simulation](#hardware-simulation)). The test runs the driver on
the simulation with the manifest's quotas and steps through it in virtual
time:

```rust
fn sim() -> Sim {
    let info = DeviceInfo { bus: 0, device: 4, function: 0, vendor: crate::VENDOR, device_id: crate::DEVICE };
    let manifest = driverkit::manifest::parse(include_str!("../edu.driver.toml")).unwrap();
    Sim::new("edu", info, EduModel::new()).with_manifest(&manifest)
}
```

```rust
#[test]
fn dma_roundtrip_stays_in_iommu() {
    let sim = sim();
    let p = sim.platform();
    let mut r = driverkit::Runner::<Edu<_>>::new(&p).ok().unwrap();
    sim.send(b"dma ahoj z DMA");
    for _ in 0..3 {
        assert_eq!(r.step(&p), None); // message, DMA there, DMA back
    }
    assert_eq!(sim.sent_text(), ["dma ahoj z DMA"]);
```

`cargo test -p edu` takes a fraction of a second; no QEMU, no system
build.

### 8. Test in QEMU

`cargo xtask test --driver edu`: the driver's host tests, the system build
and in QEMU only the `ovladace z manifestu` suite (ping every manifest
driver, the factorial through MSI, DMA there and back through the IOMMU,
recovery after `dk:crash` and `dk:restart`). Without checking the other
architectures' kernels and the documentation, so it is the fastest loop.
The full run is `cargo xtask test` (on `--arch x86_64` and
`--arch aarch64`).

xtask adds `-device edu,dma_mask=0xffffffffffffffff` to QEMU when QEMU has
it (`-device help`); otherwise the edu tests skip with a message. A
`new-driver` skeleton for hardware QEMU does not emulate just skips the
QEMU test (`zarizeni v QEMU neni`); the simulation verifies it.

### 9. Documentation

Fill in the driver page (`docs/en/driver-<name>.md` and the Czech one):
what the device does, where the facts come from, code origin and licence,
protocol, tests. `cargo xtask docs` checks the links.

## Manifest

`user/<name>/<name>.driver.toml`. xtask finds it, builds the program,
packs both the program and the manifest into the initfs and adds the
crate to the host tests. devmgr reads the manifests at start and starts
the driver for devices it has no built-in driver for. A manifest error is
a build error.

| Key | Meaning | Default |
|---|---|---|
| `driver.name` | program name in the initfs = directory name; lowercase letters, digits, `-` | required |
| `driver.description` | description | |
| `driver.license` | licence of the driver code | required |
| `driver.origin` | `vlastni` (own), or where the code was taken from (project, version, licence) | required |
| `match.pci` | `["vvvv:dddd", ...]` (hex) | |
| `match.class` | `["cc.ss.pp", ...]` class, subclass, interface (hex) | |
| `resources.bars` | BARs the driver gets (0-5) | none |
| `resources.irq_max` | how many interrupts it may enable (0-64) | 1 |
| `resources.dma_max` | how much DMA memory it may allocate in its lifetime (`65536`, `"64K"`, at most 256 MiB) | 0 |
| `service.restart` | start again after a crash | `true` |
| `service.max_restarts` | at most this many crashes per system run | 5 |

At least one `match.*` is required. The driver gets only the listed BARs:
trying to map another fails with `ovladac nesmi: mapovat BAR 2 ... ->
pridejte BAR do resources.bars v manifestu ovladace` (the driver may not
map BAR 2; add it to `resources.bars`).

devmgr sends init `dev <name>` with the channel to the driver's client;
init keeps it in `Devices::drivers` and replaces it when the driver is
recovered.

## Hardware simulation

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="A driver on the kernel and in the simulation">
<defs><marker id="dr2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="260" y="10" width="240" height="60" rx="6"/>
<text class="t b" x="380" y="35" text-anchor="middle">driver: Driver&lt;P: Platform&gt;</text>
<text class="m" x="380" y="54" text-anchor="middle">the same code in the system and in a test</text>
<rect class="acc" x="40" y="120" width="300" height="70" rx="6"/>
<text class="t b" x="190" y="144" text-anchor="middle">OsPlatform (feature os)</text>
<text class="m" x="190" y="162" text-anchor="middle">BAR via Resource, MSI/MSI-X, DMA via IOMMU</text>
<text class="m" x="190" y="178" text-anchor="middle">wait_many on interrupts and the channel</text>
<rect class="acc" x="420" y="120" width="300" height="70" rx="6"/>
<text class="t b" x="570" y="144" text-anchor="middle">SimPlatform (feature sim)</text>
<text class="m" x="570" y="162" text-anchor="middle">virtual time, quotas from the manifest</text>
<text class="m" x="570" y="178" text-anchor="middle">guards DMA like an IOMMU, access trace</text>
<rect class="box" x="40" y="210" width="300" height="34" rx="6"/>
<text class="t" x="190" y="232" text-anchor="middle">real device / QEMU</text>
<rect class="box" x="420" y="210" width="300" height="34" rx="6"/>
<text class="t" x="570" y="232" text-anchor="middle">DeviceModel (model from the datasheet)</text>
<path class="ln" d="M330 70 L200 118" marker-end="url(#dr2)"/>
<path class="ln" d="M430 70 L560 118" marker-end="url(#dr2)"/>
<path class="ln" d="M190 190 L190 208" marker-end="url(#dr2)"/>
<path class="ln" d="M570 190 L570 208" marker-end="url(#dr2)"/>
</svg>
<figcaption>One driver, two backends: the kernel and the simulation on the host.</figcaption>
</figure>

The simulation (`driverkit::sim`, feature `sim`) is mainly meant for
hardware QEMU does not emulate (Intel graphics, Realtek and Intel igc
network cards, HDA codecs): a driver can be developed and tested from the
datasheet before a real machine is at hand, and the test stays as a
regression test. What the simulation does:

- **registers**: an access to a BAR calls the model's `DeviceModel::read` /
  `write`; every access is recorded in `Sim::trace` (time, BAR, offset,
  width, value),
- **interrupts**: the model calls `bus.raise_irq(n)`; an interrupt the
  driver did not enable is dropped and logged,
- **DMA**: the model reads and writes memory through `bus.dma_read` /
  `dma_write`; an access outside memory the driver allocated is recorded
  in `Sim::faults` as a `DmaFault` (on a machine the IOMMU would block
  it). There is a one-page gap between buffers, so an overrun past the
  end of a buffer is caught,
- **quotas and rights** from the manifest (`Sim::with_manifest`): a BAR
  the driver did not get, `irq_max`, `dma_max`,
- **time** is virtual: `wait` advances time and calls `DeviceModel::tick`
  (the model finishes ongoing work there). The test is deterministic and
  fast; when the driver waits and nothing happens, `wait` returns an error
  after `Sim::time_limit` (10 s of virtual time by default) instead of
  hanging,
- **client**: `Sim::send`, `Sim::close`, `Sim::sent_text`,
  `Sim::set_client_full` (the client's queue is full).

A model only covers what the driver uses. `RegFile` handles the usual
register mechanics (reset value, read-only bits, W1C):

```rust
            regs: RegFile::new()
                .ro(ID, 0x0100_00ed)
                .rw(LIVENESS, 0)
                .rw(FACTORIAL, 0)
                // BUSY (bit 0) is read only.
                .reg(STATUS, 0, 1, 0)
                .ro(IRQ_STATUS, 0)
```

The model has its own behaviour only where a write triggers something:

```rust
    fn tick(&mut self, bus: &mut Bus) {
        if self.fact_done.is_some_and(|t| bus.now() >= t) {
            self.fact_done = None;
            let n = self.regs.get(FACTORIAL) as u32;
            let v = (1..=n).fold(1u32, |a, k| a.wrapping_mul(k));
            self.regs.set(FACTORIAL, v as u64);
            self.regs.clear_bits(STATUS, 1);
            if self.regs.get(STATUS) & 0x80 != 0 {
                self.raise(bus, 1);
            }
        }
```

For the driver threads (network, audio, native display): the model
belongs in the driver crate (`src/model.rs`, `#[cfg(test)]` only); outside
events (a received packet, a connected monitor, an unplugged cable) are
injected by the test through `Sim::with_model(|m: &mut Model, bus| ...)`.
A model does not prove the driver works on hardware (the model can
misread the datasheet just like the driver); it checks the logic, bounds
and order of steps. Before merging a driver for real hardware, state in
the description what it was tried on.

## Life cycle and crash recovery

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Driver life cycle">
<defs><marker id="dr3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="110" height="50" rx="6"/><text class="t b" x="65" y="50" text-anchor="middle">probe</text>
<rect class="acc" x="160" y="20" width="110" height="50" rx="6"/><text class="t b" x="215" y="50" text-anchor="middle">start</text>
<rect class="ok" x="310" y="10" width="200" height="70" rx="6"/><text class="t b" x="410" y="36" text-anchor="middle">events</text><text class="m" x="410" y="55" text-anchor="middle">interrupt · message · timer</text><text class="m" x="410" y="70" text-anchor="middle">dk:ping, dk:info</text>
<rect class="acc" x="560" y="20" width="110" height="50" rx="6"/><text class="t b" x="615" y="50" text-anchor="middle">stop</text>
<path class="ln" d="M120 45 L158 45" marker-end="url(#dr3)"/>
<path class="ln" d="M270 45 L308 45" marker-end="url(#dr3)"/>
<path class="ln" d="M510 45 L558 45" marker-end="url(#dr3)"/>
<text class="m" x="515" y="35">end</text>
<rect class="box" x="400" y="130" width="350" height="80" rx="6"/>
<text class="t b" x="575" y="154" text-anchor="middle">devmgr</text>
<text class="m" x="575" y="172" text-anchor="middle">code 0: done · 75 (dk:restart): again at once</text>
<text class="m" x="575" y="188" text-anchor="middle">otherwise a crash: disable bus master, new Resource,</text>
<text class="m" x="575" y="204" text-anchor="middle">start again (at most max_restarts), init: new channel</text>
<path class="ln" d="M615 70 L615 128" marker-end="url(#dr3)"/>
<path class="ln dash" d="M400 170 L65 170 L65 72" marker-end="url(#dr3)"/>
<text class="m" x="80" y="160">new process: probe starts with a device reset</text>
</svg>
<figcaption>A driver from start to end, and devmgr recovering it after a crash.</figcaption>
</figure>

Exit codes of a driverkit driver (devmgr prints them in words):

| Code | Meaning | devmgr |
|---|---|---|
| 0 | the client closed the channel | done |
| 2 | bad configuration from devmgr | recovery (crash) |
| 3 | a missing right or quota (manifest) | recovery (crash); the fix is in the manifest |
| 4 | a device or driver error | recovery (crash) |
| 70 | `dk:crash` | recovery (crash) |
| 75 | `dk:restart` | again at once, not counted |
| negative | the kernel ended the process (CPU exception, panic, kill) | recovery (crash) |

A quick restart without restarting the system: the client sends
`dk:restart` (e.g. a test, or a service after a settings change), or the
driver process crashes. Recovery takes tenths of a second. The rule for
authors: **`probe` must not assume any state from before**, it always
starts with a device reset; the client gets a new channel after recovery
(init: `init: ovladac X obnoven (N. start)`).

## Readable errors

`DriverError` carries what happened and advice after `->`. Example
messages (the system's messages are in Czech; the meaning is given here):

| Error | Message |
|---|---|
| a BAR missing from the manifest | `ovladac nesmi: mapovat BAR 2 (spravce zarizeni ho nepredal) -> pridejte BAR do resources.bars v manifestu ovladace` (may not map BAR 2, add it to `resources.bars`) |
| DMA quota | `DMA buffer 'edu-zpet' (4096 B) se nevejde do kvoty -> zvyste resources.dma_max ...` (does not fit the quota, raise `dma_max`) |
| interrupt quota | `preruseni c. 1 nelze zapnout -> zvyste resources.irq_max ...` (raise `irq_max`) |
| DMA outside a buffer (driver) | `pristup mimo DMA buffer 'tx': offset 0x40 + 16 B > velikost 64 B -> chyba v ovladaci ...` (a driver bug: wrong index or length) |
| DMA outside the IOMMU (simulation) | `DMA mimo pridelenou pamet: zarizeni zapsalo 3 B na 0x100003000 -> ...` (the driver gave the device an address not from `DmaBuffer::iova_of`; an IOMMU would block it) |
| register field | `hodnota 0x8 se nevejde do pole CTRL.MODE (3 b)` (does not fit the field) |
| kernel call | `mapovani BARu: ACCESS_DENIED (-2): ... -> ovladac na to nema pravo; ...` (no right; check `resources` in the manifest) |

On a real machine the kernel reports a blocked DMA access:
`IOMMU: zablokovan zapis zarizeni BB:DD.F na adrese ...`
([Log messages](messages.md#kernel-iommu)).

## Source and licence rules

**Do not reinvent, adopt.** A driver for common hardware has already been
written and debugged on thousands of machines; its bug fixes and hardware
workarounds (errata) are worth more than a fresh attempt.

| Origin | How to adopt |
|---|---|
| a small driver under BSD, MIT, ISC, Apache-2.0 (FreeBSD, OpenBSD, NetBSD, Redox, Haiku) | translate to Rust on the driverkit; keep the copyright and licence in the file header |
| code under the GPL (most of Linux) | **do not translate** or copy; take only facts about the hardware (registers, order of steps, errata), preferably from the datasheet |
| a huge driver (DRM graphics AMD/NVIDIA, Wi-Fi, ACPICA) | adopt it in C as an isolated process through the compatibility layer (below); only with a licence that allows it |
| datasheet, specification (PCI-SIG, NVMe, AHCI, xHCI, virtio, HDA) | freely; cite the source in the header |

Every adopted file has in its header: where it comes from (project,
version or commit, path), the original authors' licence and copyright,
and what changed. Adopted code also goes into `THIRD-PARTY.md` in the
repository root (project, version, licence, our path). In the manifest
`driver.origin` and `driver.license` say the same.

## Rust, or adopted C

| Situation | Choice |
|---|---|
| a new or small driver (up to a few thousand lines), a BSD/MIT source or just a datasheet | Rust on the driverkit |
| a driver whose main value is years of debugging (hundreds of thousands of lines, firmware, errata): DRM AMD/NVIDIA, Wi-Fi (iwlwifi, ath), ACPICA | adopt the C without changing the logic, as an isolated process |
| code under the GPL | neither: facts only, write the driver in Rust |

Adopted C runs as a **separate process** with the same rules as a Rust
driver: a restricted Resource, the IOMMU, a manifest, crash recovery. A
bug in the C code (a buffer overflow) stays inside the process. The
compatibility layer maps the services the C code expects from its kernel
(memory allocation, register mapping, DMA, interrupts, locks, timers,
logs) onto the same facilities the driverkit `Platform` provides. The ACPI
thread (ACPICA) introduces the layer; further adopted drivers will use it.
Building C for the OS target and its library belong to the language plan
(own libc, clang).

## Debugging

- **Logs**: every driver message is prefixed `name BB:DD.F:`
  (`driverkit::log!`). The serial output is in `target/serial-*.log` after
  the run ([Building](building.md#output-and-tracking-down-errors)).
  Meaning of messages: [Log messages](messages.md#drivers-and-device-manager).
- **Liveness**: `dk:ping` and `dk:info` check that the driver loop runs
  and which device it belongs to.
- **Simulation**: `Sim::trace()` is a complete record of register
  accesses, `Sim::logs()` the driver's and the simulation's messages,
  `Sim::faults()` DMA outside memory. The fastest way to a bug in the
  order of steps.
- **GDB**: `cargo xtask run --gdb` (QEMU waits for GDB), then the driver
  program's symbols from `target/user/<target>/debug/<name>`
  ([Debugging with GDB](building.md#debugging-with-gdb)).
- **QEMU monitor**: `cargo xtask run -- -monitor stdio` and in it
  `info pci` (BARs, interrupts), `info mtree` (memory map), `info qtree`.
  The tests use QMP (`xtask/src/qmp.rs`) to inject events.
- **Just one driver**: `cargo xtask test --driver <name>`, or only chosen
  suites `cargo xtask test --only "ovladace z manifestu"`, a flaky test
  repeatedly with `--repeat 20`.

## Checklist before merging

- [ ] Manifest: only the BARs needed, `irq_max` and `dma_max` with some
      headroom, not a lot; `driver.origin` and `driver.license` filled in.
- [ ] Code origin: file headers, `THIRD-PARTY.md` for adopted code, no
      translation of GPL code.
- [ ] `probe` starts with a device reset and assumes no state from before.
- [ ] `stop` stops DMA and interrupts; verified by a test (model state
      after `finish`).
- [ ] All device addresses from `iova_of`; no manual IOVA arithmetic.
- [ ] Every number from the device (index, length, status) is checked; a
      faulty device = `DriverError::Device`, not a panic.
- [ ] Barriers: `dma_wmb` between a descriptor and the index, `mmio_wmb`
      before the doorbell, `dma_rmb` after an index from the device
      (aarch64 needs them).
- [ ] Simulation test: the normal path, a device error, empty
      `Sim::faults()`.
- [ ] `cargo xtask test --driver <name>` and the full `cargo xtask test`
      on x86_64 and aarch64.
- [ ] Driver page cs+en, new messages in [Log messages](messages.md),
      `cargo xtask docs`.
- [ ] Comments in Czech, only ASCII punctuation in code, files up to ~1500
      lines, tests in separate files.
- [ ] Serious limitations (no IOMMU, missing reset, errata) stated
      explicitly in the change description and in the documentation.
