# Init and the supervisor

`init` is the first process in userspace. It starts the system services
and then stays on as the **supervisor**: it hands over devices attached at
run time and watches the services. In a test run it instead runs the
userspace tests and exits with their result. Only in a test run does it
switch `fsd` to debug mode (power-loss simulation for the FS tests).

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Init and the supervisor">
<defs><marker id="in9" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="56" rx="6"/><text class="t b" x="85" y="43" text-anchor="middle">kernel</text><text class="m" x="85" y="62" text-anchor="middle">starts init</text>
<rect class="acc" x="210" y="20" width="200" height="90" rx="6"/><text class="t b" x="310" y="45" text-anchor="middle">init: start-up</text><text class="m" x="310" y="64" text-anchor="middle">timed, devmgr (drivers),</text><text class="m" x="310" y="80" text-anchor="middle">fsd, inputd</text><text class="m" x="310" y="96" text-anchor="middle">settled</text>
<path class="ln" d="M160 48 L208 48" marker-end="url(#in9)"/>
<rect class="ok" x="470" y="20" width="280" height="90" rx="6"/><text class="t b" x="610" y="45" text-anchor="middle">supervisor (normal run)</text><text class="m" x="610" y="64" text-anchor="middle">devices at run time -> inputd</text><text class="m" x="610" y="80" text-anchor="middle">service exits -> log, restart</text><text class="m" x="610" y="96" text-anchor="middle">never exits</text>
<rect class="box" x="470" y="140" width="280" height="70" rx="6"/><text class="t b" x="610" y="165" text-anchor="middle">tests (TEST_RUN)</text><text class="m" x="610" y="184" text-anchor="middle">init/src/tests, tests drive the supervisor</text><text class="m" x="610" y="200" text-anchor="middle">exit = result, the kernel ends QEMU</text>
<path class="ln" d="M410 55 L468 55" marker-end="url(#in9)"/>
<path class="ln" d="M410 90 L468 165" marker-end="url(#in9)"/>
<text class="m" x="10" y="250">The kernel built with qemu-test (cargo xtask test) puts the TEST_RUN flag in the SELF_PROCESS argument of the start message.</text>
<text class="m" x="10" y="270">cargo xtask run and real machines: no flag, init does not run the tests and runs as the supervisor.</text>
</svg>
<figcaption>Init after starting the services: the supervisor, or the tests in a test run.</figcaption>
</figure>

## Start-up

1. `timed` (the time service, the only one allowed to set the clock),
   right after it `acpid` (ACPI: power, buttons, sensors; the only one with
   the `POWER` right, see [ACPI](acpi.md)) - ACPICA initialisation runs in
   parallel with the next steps,
2. `devmgr`: finds devices and starts drivers. It reports disks (`block`)
   and input devices (`input`), and then `done` once the USB devices
   present at boot are ready too; while waiting long for a disk driver it
   sends `busy` (init waits at most 30 s for each message),
3. `fsd` on the disk with the file system; init prepares the root
   certificate store `/system/config/certs` (see
   [TLS and HTTP](tls-http.md#root-certificate-store)),
4. `inputd`: init hands it the input devices, its own focus channel and the
   `secure` channel, and sends `settled` (devices attached now are
   trusted),
5. `netd` on the first network card (with a channel for `dnsd`), `dnsd`
   (encrypted DNS and DNSSEC, see
   [Network](network.md#encrypted-dns-and-dnssec-dnsd)) and `fwd` (firewall, configuration on
   the FS), both in jobs by their manifests (see [Network](network.md),
   [Firewall](firewall.md)), then `ntpd` (time synchronization, see
   [Real time](time.md#synchronization-over-the-network-ntpd)) and `revocd`
   (certificate revocation, see [TLS and HTTP](tls-http.md#certificate-revocation)),
6. all these processes are handed to the supervisor to watch,
7. `taskd` (task overview, see [Architecture](architecture.md#task-overview-agreed)):
   init holds its control channel and is the only one to mint client and
   producer channels with an identity; fsd gets a producer channel, the
   supervisor reports service state after every round (at most 2 s),
   applications with a window are reported together with their job,
8. in the normal run the GUI (`compd`, the windows `gui-term` and
   `gui-demo`), without a GUI the fallback console (see
   [Terminal](#terminal)).

init always sends the `fsd` configuration (the first message of the control
channel): `debug` in the test run, `normal` otherwise. fsd waits for it
before mounting the disk.

## Power-off and reboot

The power button (an event from `acpid`) or the shell command `shutdown` /
`reboot` = a **clean power-off**: init stops termd and the fallback
console, then the services in reverse registration order (clients before
services), `fsd` and finally `devmgr` and `timed` (waiting at most 1.5 s for
each), and only then asks `acpid` for S5 or a reboot. Details in
[ACPI](acpi.md#power-off-and-reboot).

## The supervisor

The loop waits on the device manager's channel and on the end of the
watched processes:

| Event | What the supervisor does |
|---|---|
| `devmgr` sends `input` with a channel | passes it to the input service (`device`). This is how USB devices attached at run time get into the system |
| `devmgr` sends `dev <name>` with a channel | a manifest driver, new or recovered after a crash: init stores the channel (`Devices::drivers`) and replaces the old one ([Drivers](drivers.md#life-cycle-and-crash-recovery)) |
| `devmgr` sends `block` | only logs it for now (disks at run time come with multiple disks in `fsd`) |
| `devmgr` closes its channel | logs it, hotplug stops working |
| a service with restart (`timed`) exits | logs the code and starts it again after 100 ms, 200 ms, 400 ms ... (at most 5 s). It does not block waiting for the new service to confirm: its channel is one more item of the loop. If it does not answer within 5 s (or answers with an error), it kills it and the attempt counts as another crash. More than 5 restarts within 60 s = it gives up and the service stays stopped |
| a terminal window sends `connect`, `termd` exits | starts `termd`, serves the session registry (see [Terminal](#terminal)) |
| `compd` exits | logs it and starts the fallback console |
| a service with an adapter in the registry exits | as with `timed`, plus in dependency order: services with `Dep::restart` on the crashed service are stopped and started after it, services with `Dep::notify` get a notification; clients reconnect through leases (see [Service recovery](service-recovery.md)) |
| a service that is only watched exits (`devmgr`, `fsd`, `inputd`, `netd`, `fwd`, until their threads wire them up) | logs the code. No recovery yet (see below) |

Log when a service crashes:

```text
init: sluzba timed skoncila: ukoncen jinym procesem (PROCESS_KILL)
init: sluzba timed znovu spustena
...
init: sluzba timed spadla 6x za 60 s, vzdavam to
```

The code is in `user/init/src/supervisor/` (`mod.rs` the loop, `lifecycle.rs` restarts and dependencies, `leases.rs` leases, `service.rs` the adapter). One round is
`Supervisor::step(startup, deadline)`. It returns what happened
(`DeviceAdded`, `Exited`, `Restarted`, `GaveUp`, `Stopped`, `Notified`, `Reconnected`, `DevmgrGone`), so the
tests drive it the same way as a normal run, only with a deadline. A new
service is handed to the supervisor with `watch(name, process, restart)`,
where `restart` is `Restart { start, ready }` (`None` = only watch it).
`start` starts a new process and returns `Started { process, ready }`:
the process handle and optionally a channel on which the service confirms
the start. Once the channel is readable, the supervisor calls `ready`
(reads the confirmation) and only then reports `Restarted`:

```rust
let mut sup = supervisor::SUPERVISOR.lock();
sup.watch("timed", &t.process, Some(system::TIME_RESTART));
sup.watch("fsd", &fsd_process, None);
sup.run(&startup); // never returns
```

## Terminal

In the normal run the terminal is the `gui-term` window (GUI) and without
a GUI the fallback console `console` on the firmware framebuffer. Both draw
the interactive terminal `term::ui`; the shell runs in the session service
`termd` and sessions in their own `shsess` processes (see
[Shell](shell-design.md)). The code is in `user/init/src/terminal.rs`.

After 1 s a running command shows "running 12 s · Ctrl+C to interrupt"
below it and Ctrl+C interrupts it. What you type meanwhile waits visibly in
a queue ("queued: ...", Esc clears the queue, the up arrow brings the last
line back for editing) and runs after it. Ctrl+Z moves a running command
to the background and the prompt comes back at once; `command &` starts it
there right away. `jobs` lists background jobs, `fg N` brings a job back to
the foreground, `kill N` ends it, and the end of a job is announced by a
line in its block. **[implemented]** Details in the shell design
([Lines in threads](shell-design.md#lines-in-threads-ctrlz-and-background)).

```text
gui-term ──connect──▶ init (supervisor) ──starts──▶ termd ──▶ shsess (session)
    ▲                      │ the window's session registry │
    └── channel to termd ──┘ (spare session ends) ◀────────┘
```

- The window gets a **connect channel** from init (the second handle of the
  start message, after the window channel). On the message `connect` init
  starts `termd` (its own fsd session, the initfs image, a registry
  channel) and answers `ok` with a channel to it, or `err <CODE>`.
- **termd crash:** the window sees the closed channel and asks again. The
  new `termd` takes over the sessions from the registry (`shsess` keeps
  running), so the window attaches to the same session including the block
  history. Every window has its own registry, so a new termd of one window
  does not take over the sessions of another. At most 10 termd starts within
  60 s per window, then init refuses (`err NO_RESOURCES`) and the window
  retries with a growing delay (at most 5 s).
- **Fallback console:** when the GUI does not come up (no virtio-gpu) or the
  compositor crashes (the supervisor sees `compd` exit), init starts
  `console`, hands it the firmware framebuffer (`display`) and the focus
  from `inputd` (`focus`), and it connects the same way as the window.
- **Welcome text:** a new session starts with a block of help (`help`, a
  few examples, keys).

⚠️ **Limits:** after a compositor crash the application windows are gone
(no `compd` restart yet). Whether the firmware framebuffer is still visible
after the virtio-gpu driver took over the display depends on the device
(QEMU q35 with VGA: yes).

## Recovering `inputd`, `fsd`, `netd`, `compd` and `devmgr`

The common protocol (a service registry with dependencies, leases, the
driver endpoint keeper, rules for client libraries) is described on the
[Service recovery](service-recovery.md) page. The registry
(`register(Spec { name, deps, service }, process)`) and leases are done in
the supervisor; the adapters of the individual services are wired up by
their threads. Until then these services are only watched with
`watch(name, process, None)`.

## Test run

A kernel with the `qemu-test` feature (`cargo xtask test`) sets the
`abi::bootstrap::flags::TEST_RUN` flag in init's start message (the
argument of the `SELF_PROCESS` entry; `Startup::flags` in libos). Init
then:

- runs the userspace tests (`user/init/src/tests`) and exits with code 0
  or 1,
- the kernel passes the result to QEMU (isa-debug-exit).

Tests that need the supervisor (USB hotplug, service restart) drive it
through `Supervisor::step`. Without the flag (`cargo xtask run`, a real
machine) init does not run the tests.

Tests of the `supervisor` suite:

| Test | What it checks |
|---|---|
| time service crash: restart and reply | `timed` terminated with `PROCESS_KILL`: the supervisor notices, starts it again and the new service answers |
| repeated crashes: the supervisor gives up | after 5 restarts within a minute the next crash = `GaveUp`, the service is not running; `revive` starts it again |
| restart without confirmation: the supervisor kills the service | its own supervisor (`Supervisor::for_tests`, 300 ms limit) with a service that never confirms the start: the supervisor waits without blocking, kills it after the limit (`PROCESS_KILL`) and does not report `Restarted` |
