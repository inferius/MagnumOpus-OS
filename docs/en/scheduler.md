# Scheduler: QoS, fair sharing and hybrid CPUs

The scheduler decides which thread runs on which CPU and for how long. The
system goals are a **snappy GUI** (compositor and input respond at once even
under full load), **no starvation** (background work always makes progress)
and **proper use of hybrid CPUs** (performance and efficiency cores).

This page describes the design (comparison of approaches and why we chose
this one), the class assignment interface, the behaviour and the measured
numbers. The code is in `kernel/src/sched/` (kernel glue) and in the
`libs/schedcore` library (pure logic testable on the host).

## Previous state (up to version 1) and what was wrong

- Four QoS classes (`Realtime`, `Interactive`, `Normal`, `Background`) in
  **strict priority**: while anything higher ran, a lower class got
  nothing. A compositor stuck in a loop (a bug) would stop disk maintenance
  too.
- User threads were always created as `Normal` and no syscall changed the
  class - the compositor had no precedence over applications.
- A 10 ms quantum for everybody, placement regardless of core type.

## Comparison of approaches

| System | Time sharing | Interactive precedence | IPC inheritance | Hybrid CPUs |
| --- | --- | --- | --- | --- |
| **Linux 6.6+ (EEVDF)** | Weighted virtual time, earliest eligible deadline; weight from `nice` | A short `slice` gives an earlier deadline; no classes | Only futexes (PI); not IPC | CPU `capacity`, EAS from an energy model |
| **macOS (QoS + Mach)** | Decaying priorities, bands per QoS | 5 QoS classes (user-interactive ... background) | **Yes**: a message carries a voucher with QoS, the receiver gets an override | P/E cores: QoS decides the core (background only on E) |
| **Fuchsia / Zircon** | Weighted fair + deadline profiles | Weights and deadline profiles assigned by role | Profiles over channels (inheritance on ports and futexes) | Capacity-based placement, simple |
| **Windows** | 32 priorities, strict priority + temporary boosts | Boost after input and for the foreground window | No (only partly ALPC) | Thread Director (HFI): work classes and EcoQoS |

What we take from them:

- **Strict priority always risks starvation** (Windows fixes it with the
  "balance set manager" that occasionally boosts a starving thread - not
  predictable).
- **Weighted virtual time** (EEVDF, Fuchsia fair) gives a mathematical share
  guarantee and good latency for threads that sleep often.
- **A class as a description of work** (macOS) is easier for applications
  than priority numbers and maps well to the core type.
- **QoS propagation by message** (macOS) solves priority inversion in
  services without services having to report anything.

## Chosen design

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Scheduler hierarchy">
<defs><marker id="sc1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="280" y="10" width="200" height="50" rx="6"/><text class="t b" x="380" y="32" text-anchor="middle">pick on a CPU</text><text class="m" x="380" y="50" text-anchor="middle">Realtime with budget first</text>
<rect class="warn" x="20" y="100" width="190" height="80" rx="6"/><text class="t b" x="115" y="124" text-anchor="middle">Realtime</text><text class="m" x="115" y="144" text-anchor="middle">budget / period (CBS)</text><text class="m" x="115" y="161" text-anchor="middle">EDF by period end</text>
<rect class="box" x="300" y="100" width="440" height="80" rx="6"/><text class="t b" x="520" y="124" text-anchor="middle">fair classes: weighted class virtual time</text><text class="m" x="520" y="146" text-anchor="middle">Interactive 15 : Normal 4 : Background 1</text><text class="m" x="520" y="163" text-anchor="middle">the class with the smallest virtual time wins</text>
<rect class="ok" x="300" y="220" width="135" height="70" rx="6"/><text class="t b" x="367" y="243" text-anchor="middle">Interactive</text><text class="m" x="367" y="262" text-anchor="middle">EEVDF, 3 ms slice</text>
<rect class="box" x="452" y="220" width="135" height="70" rx="6"/><text class="t b" x="519" y="243" text-anchor="middle">Normal</text><text class="m" x="519" y="262" text-anchor="middle">EEVDF, 6 ms slice</text>
<rect class="box" x="604" y="220" width="135" height="70" rx="6"/><text class="t b" x="671" y="243" text-anchor="middle">Background</text><text class="m" x="671" y="262" text-anchor="middle">EEVDF, 12 ms slice</text>
<path class="ln" d="M330 60 L130 98" marker-end="url(#sc1)"/>
<path class="ln" d="M430 60 L520 98" marker-end="url(#sc1)"/>
<path class="ln" d="M440 180 L367 218" marker-end="url(#sc1)"/>
<path class="ln" d="M520 180 L519 218" marker-end="url(#sc1)"/>
<path class="ln" d="M600 180 L671 218" marker-end="url(#sc1)"/>
<path class="ln" d="M115 180 L115 250 L298 250" marker-end="url(#sc1)"/>
<text class="m" x="20" y="205">budget used up:</text><text class="m" x="20" y="221">Interactive until the next period</text>
<text class="m" x="20" y="315">Background gets at least 1/20 of every busy CPU outside Realtime; Realtime at most 50 % of each CPU.</text>
</svg>
<figcaption>Two levels: classes share time by weight, threads inside a class by EEVDF.</figcaption>
</figure>

**Hierarchy (option A of the proposal, approved by the user):**

1. **Realtime** is picked first, but only while it has **budget** left
   (`budget` in every `period`, constant bandwidth server - CBS). Among
   themselves EDF (earliest period end first). When the budget runs out the
   thread **is not stopped**, it runs as `Interactive` until the next period
   - audio with a slightly underestimated budget does not crackle, yet it
   cannot flood the system. Admission: one thread at most 50 % of its
   period, all Realtime threads together at most 50 % of all CPU capacity.
2. **Fair classes** each have their own **virtual time** on every CPU:
   running moves it by `ran * 1024 / weight`. The non-empty class with the
   smallest virtual time wins. Weights **Interactive 15, Normal 4,
   Background 1** - with all three fully loaded they get 75 %, 20 % and
   5 %; Interactive against Background alone 15:1 (94 % and 6 %). A class
   that was empty is placed at the smallest virtual time of the active
   classes: sleeping **does not bank** credit, but it is next right after
   waking up.
3. **EEVDF inside a class**: every thread has a virtual time and a deadline
   `v + slice`; the eligible thread (`v <= average`) with the earliest
   deadline wins. Slice per class: Interactive 3 ms (fast alternation),
   Normal 6 ms, Background 12 ms (fewer switches = more throughput).

Why not the simpler options: flat EEVDF (Linux) gives shares to
**threads**, not classes - a hundred background threads would push the
compositor aside. Priority with aging (Windows) keeps the jitter and a weak
guarantee.

### Short wake-up latency

- A woken thread is enqueued with zero lag - it is eligible at once and
  its deadline is `average + slice`.
- **Wake-up preemption**: a higher class with a smaller virtual time than
  the running class preempts the running thread immediately (IPI to the
  target CPU). Within the same class it preempts if its deadline is earlier
  and the running thread has run at least 0.5 ms (against ping-pong).
- Input: the input driver, `inputd` and `compd` are `Interactive`, so the
  path "interrupt -> driver -> inputd -> compd" overtakes Normal and
  Background at every step.

## Class assignment

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="QoS ceiling from the job">
<defs><marker id="sc2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="20" y="20" width="220" height="56" rx="6"/><text class="t b" x="130" y="43" text-anchor="middle">root job (init)</text><text class="m" x="130" y="62" text-anchor="middle">ceiling Realtime</text>
<rect class="box" x="20" y="120" width="220" height="56" rx="6"/><text class="t b" x="130" y="143" text-anchor="middle">compd, inputd, drivers</text><text class="m" x="130" y="162" text-anchor="middle">Interactive / Realtime</text>
<rect class="box" x="300" y="120" width="200" height="56" rx="6"/><text class="t b" x="400" y="143" text-anchor="middle">application job</text><text class="m" x="400" y="162" text-anchor="middle">ceiling Normal</text>
<rect class="ok" x="540" y="120" width="200" height="56" rx="6"/><text class="t b" x="640" y="143" text-anchor="middle">application</text><text class="m" x="640" y="162" text-anchor="middle">Normal or Background</text>
<path class="ln" d="M130 76 L130 118" marker-end="url(#sc2)"/>
<path class="ln" d="M200 76 L380 118" marker-end="url(#sc2)"/>
<path class="ln" d="M500 148 L538 148" marker-end="url(#sc2)"/>
<text class="m" x="20" y="215">Effective class = the lower of (requested, ceiling of the job and all ancestors); an IPC boost comes only from a higher-class sender.</text>
</svg>
<figcaption>The job inherits the class ceiling from its ancestors; an application cannot make itself Realtime.</figcaption>
</figure>

One syscall `SCHED_CONTROL` (number in [syscalls](syscalls.md)) with
operations:

| Operation | Handle | What it does |
| --- | --- | --- |
| `THREAD_SET` | thread (`MANAGE`), 0 = own thread | thread class and, for Realtime, budget/period (µs) |
| `PROCESS_SET` | process (`MANAGE`) | class of all process threads, also new ones |
| `JOB_SET_MAX` | job (`WRITE`) | job class ceiling; at most the parent's |
| `THREAD_GET` | thread (`READ`), 0 = own | base and effective class |
| `CPU_INFO` | - | core type, capacity, run time per class |
| `CPU_HINT` | resource (`POWER`) | core capacity from ACPI CPPC (`acpid`) |

Rules:

- **Ceiling from the job**: the effective class is never above the ceiling
  of the process's job and all ancestors. Asking above it fails
  (`AccessDenied`); lowering the ceiling applies to all threads right away
  (at their next scheduling).
- A thread may always **lower** its class (e.g. FS maintenance to
  `Background`).
- Why a syscall and not a profile object (Fuchsia)? Rights are already
  carried by handles and jobs; a profile object would be a second path to
  the same check. Fuchsia itself moved from direct profiles to roles
  managed by a service.

Assignment in init and devmgr (`libos::sched::service_default`, `qos` in a
driver manifest overrides it):

| Service | Class |
| --- | --- |
| `compd`, `inputd` | Interactive |
| input drivers (`ps2`, `virtio-input`, `usb-hid`) | Interactive |
| `audiod`, `hda` | Realtime, budget 1 ms / 5 ms |
| FS maintenance (scrub, defragmentation, deduplication in `fsd`) | Background |
| application job | ceiling Normal |

A driver manifest can state its class:

```toml
[service]
qos = "realtime"        # realtime, interactive, normal, background
rt_budget_us = 1000     # realtime only
rt_period_us = 5000
```

## Inheritance over IPC

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="QoS propagation by message">
<defs><marker id="sc3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="20" y="40" width="190" height="60" rx="6"/><text class="t b" x="115" y="64" text-anchor="middle">compd</text><text class="m" x="115" y="83" text-anchor="middle">Interactive</text>
<rect class="box" x="290" y="40" width="190" height="60" rx="6"/><text class="t b" x="385" y="64" text-anchor="middle">message in a channel</text><text class="m" x="385" y="83" text-anchor="middle">carries the sender class</text>
<rect class="acc" x="560" y="40" width="180" height="60" rx="6"/><text class="t b" x="650" y="64" text-anchor="middle">server (fsd)</text><text class="m" x="650" y="83" text-anchor="middle">Normal -> Interactive</text>
<path class="ln" d="M210 70 L288 70" marker-end="url(#sc3)"/>
<path class="ln" d="M480 70 L558 70" marker-end="url(#sc3)"/>
<text class="m" x="20" y="140">Boost: when the waiter is woken (thread and port) and when the message is read.</text>
<text class="m" x="20" y="160">End: reading a lower-class message, or 50 ms without another message. It only raises, never lowers.</text>
<text class="m" x="20" y="180">It chains: the server sends on with its effective (boosted) class.</text>
</svg>
<figcaption>QoS propagation by message (like vouchers in macOS).</figcaption>
</figure>

- Every message carries the **effective class of the sender**.
- On write, the channel boosts the thread waiting on it directly, and a
  port watching it remembers the class with the packet and boosts the
  thread that picks the packet up.
- Reading a message sets the thread's boost to the message class for
  **50 ms** (the window for serving one request). The next message renews
  the window or lowers the class. Without another message the boost
  expires - the thread returns to its own class.
- A boost never lowers, and the job ceiling **does not apply** to it - it
  is a loan from the sender, not the thread's own entitlement (otherwise an
  application with a Normal ceiling serving the compositor would delay the
  frame). It cannot be abused: a boost only comes from a thread that really
  has that class.
- Realtime propagates over IPC as **Interactive** (the budget belongs to
  a particular thread and cannot be lent).

## Hybrid CPUs

- **Core type detection** when every CPU starts:
  - x86_64 Intel hybrid: `CPUID.07H:EDX[15]` (hybrid) and
    `CPUID.1AH:EAX[31:24]` (`0x40` Core = performance, `0x20` Atom =
    efficiency). HFI (`CPUID.06H:EAX[19]`, a performance table in memory
    and a thermal interrupt) not yet - it needs thermal interrupt handling;
    `acpid` can refine the capacity from CPPC.
  - aarch64: `MIDR_EL1` (implementer ARM and part number: A53/A55/A510/A520
    efficiency, A7x/A71x/A72x performance, X1-X4/X925 the fastest). The
    **PPTT** table gives clusters (shared cache).
- **Capacity** (1024 = the fastest core): performance 1024, efficiency 512
  (default estimate), exactly from ACPI CPPC `_CPC` via `CPU_HINT`.
- **Placement by class**: Realtime and Interactive on an idle core with the
  highest capacity, Background on an idle efficiency core (it does not wake
  a performance core; if no efficiency core is free, it stays where it is),
  Normal on its last CPU for the cache, otherwise an idle one.
- **Balancing busy CPUs**: when more work waits on a CPU at the end of a
  slice, the preempted thread is pushed to a CPU that will still be less
  loaded after the move. Load = sum of class weights (Interactive 15,
  Normal 4, Background 1) divided by core capacity: two Background threads
  are not the same work as two Interactive ones. A move happens only when
  the target stays less loaded afterwards (no bouncing). The core type rules
  for the class apply.
- **Balancing idle CPUs**: an idle CPU takes work from other queues; an efficiency
  core takes Interactive only from a queue with at least two waiting
  threads (the performance cores are overloaded). A preempted Interactive
  thread on an efficiency core moves to an idle performance core.
- **Preparation for power management**: the kernel counts run time per
  class on every CPU (`CPU_INFO`); a future governor in `acpid` can pick a
  P-state from it (`_PSS` or CPPC `desired performance`).

Verification: QEMU cannot do a hybrid topology (`-smp` has no core types on
x86_64 nor on aarch64 `virt`). **Placement and balancing on hybrid CPUs are
verified only by simulation** on the host (`libs/schedcore`, tests
`place_tests.rs` and `sim_tests.rs`); decoding of CPUID 0x1A, MIDR and PPTT
only by host tests over bytes. In QEMU the homogeneous path is verified (all
cores capacity 1024).

## Measurements

Kernel tests (`kernel/src/tests/sched_qos.rs`, run in QEMU with 4 CPUs):

- **Input -> frame latency** under full background load: a "driver"
  (Interactive) wakes "inputd", which wakes "compd", which renders a frame
  (200 µs); Background and Normal loops run on all CPUs. Time from the
  event to the frame.
- **Background share** under Interactive load: one Interactive and one
  Background loop per CPU; Background must get at least 4 % (guarantee
  1/16 at weights 15 : 1, minus measurement tolerance).
- **IPC inheritance**: a Normal server woken by a message from Interactive
  runs as Interactive; a port packet carries the class.
- **Realtime**: admission (exactly one 50 % thread per CPU) and the lateness
  of a 2 ms period under Interactive load on all CPUs.
- **Context switch micro-benchmark** (ping-pong over an event).

### Measured values

QEMU TCG (emulation without acceleration), 4 CPUs, x86_64; three test runs
within one `cargo xtask test`. TCG is slow and its timing imprecise - the
numbers are for comparison and regression watching, not real hardware
performance.

| Measurement | Result (3 runs) |
| --- | --- |
| input -> frame latency under 12 Normal and Background loops, median | 728-969 µs (200 µs of it "rendering") |
| the same latency, maximum | ~15 ms |
| Background share under Interactive load (guarantee 1/16 = 62 ‰) | 60-61 ‰ |
| lateness of a 2 ms Realtime period under Interactive load, median / max | 1.0-1.3 ms / 2.7-4.4 ms |
| context switch (ping-pong over an event, one side) | 90-110 µs |
| Background share in version 1 (strict priority) | 0 ‰ (starves) |

The context switch was not compared against the old scheduler to save time
(the same test for version 1 is prepared and only needs a run); the pick is
O(n) in the threads of one class on a CPU, versus O(1) before.

The latency maximum (~15 ms) matches one Background slice (12 ms) on TCG,
where IPIs and the timer are delivered late; the median is below 1 ms.

## Next steps

- **HFI** (Intel Hardware Feedback Interface): core capacities at run time
  by temperature and power - needs thermal interrupt handling.
- **P-state governor** in `acpid` on top of `CPU_INFO` (run time per class)
  and ACPI `_PSS` / CPPC; the kernel prepares the data, no governor yet.
- `acpid` should call `CPU_HINT` with the capacity from `_CPC` (today an
  estimate from CPUID).
- A balanced tree instead of the linear pick if one CPU's queues reach
  hundreds of threads.
- Measurements on real hybrid hardware (today simulation only).
