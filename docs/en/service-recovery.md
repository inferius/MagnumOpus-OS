# Service recovery after a crash

When a system service crashes (`fsd`, `inputd`, `netd`, `compd`, ...), the
init supervisor starts it again and **clients reconnect on their own**: the
client library reports "the service is restarting", waits for the new
instance and restores the session with **the same identity**. Only **the
supervisor** hands out the new channel to the service. A request in flight
is repeated only when that is safe; otherwise the client gets a clear
error. **It never hangs.**

This page is the common protocol for all services. Code: the supervisor
`user/init/src/supervisor/`, the client library `libs/svcclient`, the test
service `user/svctest`, the tests `user/init/src/tests/recovery.rs`.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Recovering a service after a crash">
<defs><marker id="ob1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="ob1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="200" height="110" rx="6"/><text class="t b" x="110" y="55" text-anchor="middle">client (application)</text><text class="m" x="110" y="76" text-anchor="middle">svcclient::Conn</text><text class="m" x="110" y="94" text-anchor="middle">session channel + lease</text><text class="m" x="110" y="112" text-anchor="middle">instance generation</text>
<rect class="err" x="300" y="20" width="170" height="60" rx="6"/><text class="t b" x="385" y="45" text-anchor="middle">service gen 1</text><text class="m" x="385" y="64" text-anchor="middle">crashed</text>
<rect class="ok" x="300" y="110" width="170" height="60" rx="6"/><text class="t b" x="385" y="135" text-anchor="middle">service gen 2</text><text class="m" x="385" y="154" text-anchor="middle">new instance</text>
<rect class="acc" x="560" y="30" width="190" height="200" rx="6"/><text class="t b" x="655" y="55" text-anchor="middle">supervisor (init)</text><text class="m" x="655" y="78" text-anchor="middle">service registry</text><text class="m" x="655" y="96" text-anchor="middle">dependencies, order</text><text class="m" x="655" y="114" text-anchor="middle">backoff, attempt limit</text><text class="m" x="655" y="132" text-anchor="middle">session recipes (leases)</text><text class="m" x="655" y="150" text-anchor="middle">driver endpoint keeper</text><text class="m" x="655" y="168" text-anchor="middle">instance generations</text><text class="m" x="655" y="186" text-anchor="middle">state for the task view</text>
<path class="ln dash" d="M210 60 L298 50" marker-end="url(#ob1)"/><text class="m" x="225" y="44">1. PEER_CLOSED</text>
<path class="ln-acc" d="M210 125 C 380 230, 520 230, 558 200" marker-end="url(#ob1a)"/><text class="m" x="250" y="250">2. RECONNECT on the lease (session was from gen 1)</text>
<path class="ln" d="M558 140 L472 140" marker-end="url(#ob1)"/><text class="m" x="480" y="128">3. starts it,</text><text class="m" x="480" y="160">creates a session</text>
<path class="ln-acc" d="M300 160 L212 120" marker-end="url(#ob1a)"/><text class="m" x="218" y="170">4. new channel, gen 2</text>
<rect class="box" x="10" y="270" width="740" height="50" rx="6"/><text class="m" x="20" y="290">5. the library repeats the request in flight only if it is idempotent; otherwise SERVICE_RESTARTED.</text><text class="m" x="20" y="308">The service does not come up within 10 s or the supervisor gives up: SERVICE_UNAVAILABLE. The client never hangs.</text>
</svg>
<figcaption>A service crash: the client asks the supervisor for a new session through its lease.</figcaption>
</figure>

## Terms

| Term | Meaning |
|---|---|
| **instance** | one running process of the service |
| **generation** | sequence number of the instance (`u64`, first start = 1, every start +1). The client uses it to tell it is talking to a new instance |
| **recipe** | bytes from which the supervisor can create **the same session** on a new instance: for `fsd` the user, groups and mounted paths, for `netd` the network rules, for `compd` the window identity. The format belongs to the service; the supervisor only stores it |
| **lease** | the client's channel to the supervisor, bound to one recipe. Whoever holds the lease may ask for a new session **exactly by the recipe** (never a wider one). A lease can be narrowed and passed on |
| **adapter** | the service's code in init (`Service`): starts an instance, confirms the start, creates a session from a recipe, narrows a recipe |
| **endpoint keeper** | the supervisor keeps a duplicate of the endpoint of a driver channel it handed to a service. The driver therefore does not notice the service crash and the new instance gets the same endpoint |

## The service registry in the supervisor

Every service is in the registry as a **specification**:

```rust
sup.register(Spec {
    name: "netd",
    deps: &[Dep::restart("devmgr")],
    service: Some(Box::new(NetdService::new(...))),
}, Some(&process));
```

- `name`: the name in the log, in the task view and in leases,
- `deps`: who the service depends on and what happens to it when the
  dependency crashes:
  - `Dep::restart(x)`: when `x` crashes, the service is stopped and started
    again **after** `x` starts again (dependency order). For services that
    hold a channel directly to the instance of `x` without a lease (`fwd`
    and `dnsd` to `netd`),
  - `Dep::notify(x)`: the service keeps running; after `x` starts again the
    supervisor calls its `dependency_restarted(x, generation)` (e.g. it restores
    state the dependency forgot in its restart),
- `service`: the adapter (`trait Service`):

```rust
pub trait Service {
    /// Starts a new instance (does not block). ctx: generation, restart?
    fn start(&mut self, ctx: &StartCtx) -> Result<Started>;
    /// The confirmation channel is readable: Ok(true) = running, Ok(false) = not yet.
    fn ready(&mut self) -> Result<bool> { Ok(true) }
    /// A session by the recipe on the running instance.
    fn connect(&mut self, recipe: &[u8]) -> Result<Channel>;
    /// A narrowed recipe; must never give more rights than `parent`.
    fn narrow(&self, parent: &[u8], narrowing: &[u8]) -> Result<Vec<u8>>;
    /// A Dep::notify dependency runs in a new generation.
    fn dependency_restarted(&mut self, dep: &str, generation: u64) {}
    /// The instance ended: drop its channels.
    fn stopped(&mut self) {}
}
```

A service without an adapter (`watch(name, process, None)`) is only
watched: the crash is logged and shows in the task view, there is no
restart.

### Service states

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Service states in the supervisor">
<defs><marker id="ob2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="70" width="120" height="50" rx="6"/><text class="t b" x="70" y="100" text-anchor="middle">Running</text>
<rect class="warn" x="200" y="70" width="120" height="50" rx="6"/><text class="t b" x="260" y="100" text-anchor="middle">Backoff</text>
<rect class="box" x="390" y="10" width="150" height="50" rx="6"/><text class="t b" x="465" y="40" text-anchor="middle">WaitingDeps</text>
<rect class="acc" x="390" y="130" width="150" height="50" rx="6"/><text class="t b" x="465" y="160" text-anchor="middle">Starting</text>
<rect class="err" x="610" y="70" width="140" height="50" rx="6"/><text class="t b" x="680" y="100" text-anchor="middle">GaveUp</text>
<path class="ln" d="M130 95 L198 95" marker-end="url(#ob2)"/><text class="m" x="138" y="88">crash</text>
<path class="ln" d="M320 85 L388 40" marker-end="url(#ob2)"/><text class="m" x="300" y="55">delay, deps</text>
<path class="ln" d="M465 60 L465 128" marker-end="url(#ob2)"/><text class="m" x="472" y="100">deps run</text>
<path class="ln" d="M390 160 C 200 200, 80 170, 70 122" marker-end="url(#ob2)"/><text class="m" x="150" y="200">ready</text>
<path class="ln" d="M540 155 L640 122" marker-end="url(#ob2)"/><text class="m" x="560" y="150">6th crash in 60 s</text>
<path class="ln" d="M320 100 L608 100" marker-end="url(#ob2)"/><text class="m" x="420" y="92">attempt limit</text>
</svg>
<figcaption>Service states. A start without confirmation within 5 s = a crash.</figcaption>
</figure>

| State | Meaning | Lease `RECONNECT` |
|---|---|---|
| `Running` | the instance runs and confirmed the start | a newer generation right away, otherwise see below |
| `Backoff` | after a crash, waits for the delay (100 ms, 200 ms, ... at most 5 s) | waits |
| `WaitingDeps` | waits for its dependencies to run (`Dep::restart`) | waits |
| `Starting` | the new instance has not confirmed the start yet (5 s limit) | waits |
| `GaveUp` | more than 5 restarts within 60 s, or a dependency gave up | `SERVICE_UNAVAILABLE` right away |

A dependency crash does not count towards the attempt limit of a service
with `Dep::restart` (it did not crash itself). `revive(name)` gives a
service that was given up on a new chance (manual intervention, tests),
and with it the services that depend on it.

### Order when `devmgr` crashes

`devmgr` is the root: the other services hold endpoints of channels to its
drivers. A `devmgr` crash (target state, see
[Implementation status](#implementation-status)):

1. the supervisor kills the drivers `devmgr` started (`devmgr` sends it a
   duplicate of the handle of every driver process in a `driver` message),
2. starts `devmgr` again; the new drivers report devices as at boot,
3. services with `Dep::restart("devmgr")` (`fsd`, `inputd`, `netd`,
   `compd`) start again on top of the new endpoints, in dependency order
   (`fsd` before `netd`, `netd` before `fwd` and `dnsd`),
4. clients of all these services reconnect through their leases.

## The lease protocol

A lease is a channel; messages are encoded with `libs/wire`
(little-endian), like the other protocols. There is at most one request on
a lease at a time.

**Request:** `u32 op`, `u32 id` (request number; the reply returns it),
then depending on the operation:

| `op` | Name | Fields | Reply (handles) |
|---|---|---|---|
| 1 | `RECONNECT` | `u32 wait_ms` | channel of the new session |
| 2 | `NARROW` | `bytes narrowing` (service format) | channel of the narrowed session, a new lease |
| 3 | `STATUS` | - | - |

**Reply:** `u32 op`, `u32 id`, `i32 status` (0 = ok, otherwise an error
code), `u64 generation`, `u8 service_state` (0 `Running`, 1 `Restarting`,
2 `GaveUp`, 3 `Stopped`).

`RECONNECT` rules:

- the supervisor remembers for each lease the generation it issued the
  session on, and replies with a session on a **newer** instance. A client
  that saw the crash before the supervisor did therefore does not get a
  session on the dying instance. The client does not need to know the
  generation (it learns it from the reply),
- when the service is not running, the reply waits for the new instance,
  at most `wait_ms` (upper bound 10 min). Then `SERVICE_UNAVAILABLE`,
- when the supervisor gives up (`GaveUp`), it replies
  `SERVICE_UNAVAILABLE` right away,
- **deliberate close**: when the instance the session was issued on is
  still running after 200 ms, the service closed the session itself
  (rights revoked, logged out). The reply is `PEER_CLOSED` and no new
  session is issued: revoking rights cannot be bypassed by the library
  reconnecting,
- a reply with a different `id` than the client expects is a late reply to
  a request that timed out. The client **takes over** a successful late
  `RECONNECT` reply (it is the session the supervisor issued last) and
  drops the others. The client waits for the reply at most `wait_ms` + 5 s.

`NARROW` rules:

- the service adapter computes a new recipe from the lease's recipe and
  the narrowing. A narrowing may only **remove** rights (a subset of the
  mounted paths, narrower network rules), otherwise `ACCESS_DENIED`,
- the supervisor creates the narrowed session right away (the service must
  be running, otherwise `SERVICE_UNAVAILABLE`) and returns it with a new
  lease. That one is passed to the child together with the session,
- a session the client narrowed **directly with the service**
  (`Net::restrict`, `connect_as` from a session other than the control
  one) has no lease and does not survive a restart: the child gets
  `SERVICE_UNAVAILABLE` and has to ask its parent. If the library restored
  it through the parent's lease, the child would get **higher** rights.

A lease ends when the client closes its endpoint. The recipe goes with it.

## Handing over the new instance

- the adapter gets `StartCtx { generation, restart }`. A service that needs
  to behave differently after a restart than after system start learns
  that in its start message (e.g. `fsd` spares orphans, see below),
- **endpoints of channels to drivers** are held by the supervisor (the
  endpoint keeper): the service gets a duplicate. When it crashes, the
  endpoint at the driver stays open and the driver notices nothing. The new
  instance gets another duplicate of the same endpoint and **must first
  drop old messages** (late replies to requests of the old instance): it
  sends the driver a synchronisation request and reads until it gets its
  reply. The synchronisation is defined by the driver protocol
  (`blockproto`, `inputproto::device`, `netproto::link`,
  `guiproto::display`),
- **drivers do not exit when they lose their service**: a closed channel
  to the service (when the supervisor does not keep the endpoint) and a
  closed channel to `devmgr` are only logged; the driver waits until the
  supervisor kills it or it gets a new channel.

## The client library (`libs/svcclient`)

`Conn` holds the session channel, the lease, the generation and the state.
Service libraries (`FsClient`, the `netproto` client, `inputproto`,
`guiproto`) call services through it:

```rust
let conn = Conn::new("fsd", session, Some(lease));
conn.set_wait(Duration::from_secs(30));   // a batch job waits longer
conn.on_event(|service, e| println!("{service}: {e:?}"));
let reply = conn.call(Kind::Idempotent, &request, Vec::new())?;
```

### Rules for client libraries

1. **Every operation has a kind** (`Kind`):
   - `Idempotent`: repeating it changes nothing (read, `stat`, listing,
     `info`, a DNS query, status). After a crash the library **repeats it
     on its own** on the new instance,
   - `Effect`: has a side effect (write, rename, delete, conditional save,
     lock, sending data, a window action). After a crash it returns
     **`SERVICE_RESTARTED`** and does not repeat it: we do not know whether
     the old instance carried it out. The application decides (a write at
     an offset can be repeated, a conditional save checks the revision
     again),
   - a request with **handles** is always `Effect` (sending handed the
     handles to the old instance and they died with it).
2. **Waiting for the new instance**: 10 s by default (`DEFAULT_WAIT`), the
   client may set its own (`set_wait`). When the supervisor gives up on the
   service, the wait ends right away. Then `SERVICE_UNAVAILABLE`; the next
   call tries to recover again.
3. **It never hangs**: losing the channel (`PEER_CLOSED` on write or read)
   always leads to recovery or an error. Waiting for a lease reply has a
   deadline.
4. **Events for the UI**: `Restarting` (show "the service is
   restarting"), `Restored { generation }`, `Unavailable(error)`.
5. **Streams and push channels** (a `compd` window, a
   `netd` socket; not the `inputd` focus, the endpoint keeper holds it): the library waits on its channel; on `PEER_CLOSED` it
   calls `conn.recover()` and then restores the state itself (the window
   sends its layer tree again).
6. **Derived sessions for children** only through `conn.narrow(narrowing)`
   (a lease with a recipe). Never hand the parent's lease to a child with a
   narrower session.
7. **Without a lease** (`Conn::new(name, channel, None)`) the library works
   as before: losing the channel = `SERVICE_UNAVAILABLE`.

## What survives a crash of each service

| Service | What is restored | What returns an error | Wired up by |
|---|---|---|---|
| `fsd` | sessions (identity, mounted paths) from the recipe; open handles by a durable handle (dataset, oid, generation, rights) - the library opens them again (`REOPEN`), rights are checked again. After a restart (not after system start) `fsd` keeps orphans (deleted but open files) for 30 s so clients can reopen them | locks, a conditional save in progress, an unconfirmed write: `SERVICE_RESTARTED`, the client repeats it | FS thread |
| `inputd` | drivers (endpoint keeper + `ISYN` synchronisation), the focus and the `secure` channel (also in the endpoint keeper: `compd` does nothing, its channel stays open), layout, verification option, `settled`; devices attached at run time | pressed keys are treated as released (`Focus(false)`); secure mode after a restart, the system ends it with `release`; synthetic input (`inject`) gets `PEER_CLOSED` | input thread |
| `netd` | program sessions (network rules) from the recipe, addresses (DHCP again), DNS, firewall (`fwd` and `dnsd` with `Dep::restart`) | sockets: `CONNECTION_RESET` (connection lost), listening sockets have to be opened again | network thread |
| `compd` | windows: the application reconnects through its lease (recipe = identity from the manifest) and sends its layer tree again; the display from the endpoint keeper; the focus from `inputd` | a window transaction in progress | GUI thread |
| `devmgr` | drivers and all services on top of them (see the order above) | as with the individual services | recovery coordinator (init) |
| `termd`, `taskd` | terminal sessions, the task list (subscription from the last number) | a shell command running in the crashed session | shell thread |

## Task view

The supervisor is a producer for the task view (`taskproto`): every
service is a task of origin `Service` with the state from the table above,
the generation, the number of restarts and the last exit code. A service
given up on shows in the view as an error with the reason ("crashed 6
times in 60 s"). Until `taskd` is connected (shell thread) the state is in
`Supervisor::status()`.

## "Kill the service" tests

Every service has a test in QEMU: kill it in the middle of work and check
that the client carries on or gets a clear error - it never hangs. The
kit is in `user/init/src/tests/recovery_kit.rs`, the protocol tests in
`user/init/src/tests/recovery.rs` (suite `obnova sluzeb`) with the test
service `user/svctest`:

| Test | What it checks |
|---|---|
| an idempotent request survives a crash | the client is in the middle of a request, we kill the service; the library repeats it on generation 2, same identity |
| a request with an effect returns an error | the same with `Effect`: `SERVICE_RESTARTED`, the next call works |
| a narrowed lease stays narrowed | a child with a narrowed session reconnects after the crash with the narrowed identity, not the parent's |
| a service given up on | the client gets `SERVICE_UNAVAILABLE` right away, it does not wait for its whole limit |
| a slow restart | client limit 300 ms, the service takes longer to start: `SERVICE_UNAVAILABLE` within the limit |
| a deliberate close | the service closes the session itself: `PEER_CLOSED`, no new session is issued |
| dependencies | a crash stops the services with `Dep::restart` and starts them after the service; `Dep::notify` gets `dependency_restarted` |

A test for your own service: the client runs in **another process** than
the supervisor (in a test, init drives the supervisor through `step`; a
client in init would wait for itself). The kit starts `svctest` as a
client with a session channel and a lease and sends it commands:

```rust
let mut h = Harness::new(startup);          // the test's own supervisor
h.add("svctest", &[], "normal")?;           // adapter + start, generation 1
let c = h.client("svctest", "alice")?;      // client (another process) with a lease
h.send(c, "call idem hold")?;               // a request the service holds
h.wait_held("svctest")?;
h.kill("svctest")?;                          // crash in the middle of work
let (reply, ms) = h.reply(c)?;              // the supervisor runs, the client does not hang
check_eq!(reply, String::from("ok gen=2 who=alice"), "repeated");
```

A service with its own adapter: `h.add_service(name, deps,
Box::new(MyAdapter))`; the client is the service's own test program (the
same commands `call`, `narrow`, `wait`, `gen` as `svctest client`).

## Implementation status

| Part | Status |
|---|---|
| lease protocol, `svcclient::Conn`, retry rules | done, tests on the host and in QEMU |
| registry, dependencies (`Dep::restart`, `Dep::notify`), generations, backoff, giving up, `revive`, `status()` | done, suite `obnova sluzeb` |
| endpoint keeper (`Kept`) | done in the supervisor; used by the service adapters |
| `inputd` + input drivers (virtio-input, PS/2, USB HID) | done: the `InputdService` adapter (`input.rs`), `ISYN` synchronisation, drivers do not exit when the service goes away; test "kill inputd in the middle of work" in the `vstup` suite, see [Input](input.md#recovery-after-a-crash) |
| `fsd`, `netd` + `fwd`/`dnsd`, `compd`, `termd`/`taskd` | wired up by the service threads following this page |
| restarting `devmgr` (`driver` message, killing drivers) | next step after the services are wired up: without them a `devmgr` restart would only detach the devices |
| task view (`taskproto` producer) | once `taskd` runs (shell thread) |

## Risks and limitations

- **A stuck service is not detected.** The supervisor reacts to a process
  ending, not to a service that does not answer. The library waits for a
  reply without a deadline as before; liveness checking (a watchdog) is a
  separate step.
- **Whoever did not see the crash has old data.** A write the old `fsd`
  instance accepted but that was not confirmed by `FSYNC` is lost after a
  crash (just like after a power loss). The library does not report it;
  applications that care call `fsync`.
- **Revoking rights by the service** (`PEER_CLOSED`) applies only to the
  running instance. After another restart a client with the old lease
  restores the session. A permanent revocation must be reported to the
  supervisor by the service (cancelling the lease) - not there yet.
- **A lease is one more channel** per recoverable session (kernel memory).
- **The supervisor is single-threaded**: a long `start` or `connect` of an
  adapter delays everybody. Adapters must not block; waiting belongs in
  `ready`.
- **An init crash** cannot be recovered (init is the root of identities).
  Init must stay small.
- **Drivers started by a crashed `devmgr`**: the supervisor kills only
  those `devmgr` managed to report with `driver`. The kernel cannot kill a
  whole job yet.
