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
2. `devmgr`: finds devices and starts drivers. It reports disks (`block`)
   and input devices (`input`), and then `done` once the USB devices
   present at boot are ready too,
3. `fsd` on the disk with the file system; init prepares the root
   certificate store `/system/config/certs` (see
   [TLS and HTTP](tls-http.md#root-certificate-store)),
4. `inputd`: init hands it the input devices, its own focus channel and the
   `secure` channel, and sends `settled` (devices attached now are
   trusted),
5. `netd` on the first network card and `fwd` (firewall, configuration on
   the FS), both in jobs by their manifests (see [Network](network.md),
   [Firewall](firewall.md)), then `ntpd` (time synchronization, see
   [Real time](time.md#synchronization-over-the-network-ntpd)) and `revocd`
   (certificate revocation, see [TLS and HTTP](tls-http.md#certificate-revocation)),
6. all these processes are handed to the supervisor to watch.

## The supervisor

The loop waits on the device manager's channel and on the end of the
watched processes:

| Event | What the supervisor does |
|---|---|
| `devmgr` sends `input` with a channel | passes it to the input service (`device`). This is how USB devices attached at run time get into the system |
| `devmgr` sends `block` | only logs it for now (disks at run time come with multiple disks in `fsd`) |
| `devmgr` closes its channel | logs it, hotplug stops working |
| a service with restart (`timed`) exits | logs the code and starts it again after 100 ms, 200 ms, 400 ms ... (at most 5 s). It does not block waiting for the new service to confirm: its channel is one more item of the loop. If it does not answer within 5 s (or answers with an error), it kills it and the attempt counts as another crash. More than 5 restarts within 60 s = it gives up and the service stays stopped |
| another service exits (`devmgr`, `fsd`, `inputd`, `netd`, `fwd`) | logs the code. No recovery yet (see below) |

Log when a service crashes:

```text
init: sluzba timed skoncila: ukoncen jinym procesem (PROCESS_KILL)
init: sluzba timed znovu spustena
...
init: sluzba timed spadla 6x za 60 s, vzdavam to
```

The code is in `user/init/src/supervisor.rs`. One round is
`Supervisor::step(startup, deadline)`. It returns what happened
(`DeviceAdded`, `Exited`, `Restarted`, `GaveUp`, `DevmgrGone`), so the
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

## Why `inputd`, `fsd`, `netd` and `devmgr` are not recovered yet

A crashed service closes its channels to its drivers and clients, and they
then exit. An input driver exits when its channel to the service closes.
A new `inputd` would therefore have no devices, and `devmgr` would have to
start the drivers again. For USB, the controller would also have to walk
its ports again. Recovery therefore needs:

- drivers that do not exit when they lose their service and wait for a new
  connection (through `devmgr`),
- restart with dependencies (a `devmgr` crash = restart the services on
  top of the drivers too),
- for `fsd`, mounting the same disk again and restoring client sessions,
- for `netd`, new sessions for programs (every socket is a channel that
  dies with the service) and restarting `fwd` with a new channel to `netd`.

That is the supervisor's next step. The `watch` / `restart` mechanism is
ready for it.

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
