# Fences

A fence is a kernel object for **explicit synchronisation** between
processes: the producer (GPU driver, video decoder) advances a value, the
consumer (compositor, application) waits until the value is reached. It is
step 6a-1 of the foundations for graphics and video
([gpu-amd-nvidia.md](gpu-amd-nvidia.md#impact-on-the-foundations), point 1).
Syscall numbers and arguments are in [syscalls.md](syscalls.md#fences),
error codes in [errors.md](errors.md).

## Status

| Part | What | Status |
|---|---|---|
| 6a-1 | fence object: `u64` timeline, sticky error, abandonment when the last producer goes away, `FENCE_WAIT` (one / all items, deadline), port registration, `SIGNALED` for `OBJECT_WAIT_*`; wrappers `libos::Fence`, `libos::fence::{wait_any, wait_all}` | **[verified in QEMU]** x86_64 and aarch64 (kernel suite `objekty`, tests `plot: ...`; userspace suite `ploty`) |
| 6a-3 | fences in `renderproto` messages (Venus: virtio fence -> kernel fence, `ring_idx > 0`) | **[designed]** |
| 6a-4 | acquire and release fences in the window protocol (zero-copy video) | **[designed]** |
| LinuxKPI | `dma_fence`, `sync_file`, `drm_syncobj` of imported C as this object | **[designed]** |

## Model

<figure>
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Fence states">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.bad{fill:#fbecec;stroke:#a04040;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}.bad{fill:#331f1f;stroke:#d08f8f}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="fe1" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="acc" x="250" y="20" width="260" height="64" rx="6"/><text class="t b" x="380" y="44" text-anchor="middle">running: value v</text><text class="m" x="380" y="64" text-anchor="middle">FENCE_SIGNAL(w &gt;= v) -&gt; v = w</text>
<rect class="bad" x="20" y="140" width="300" height="64" rx="6"/><text class="t b" x="170" y="164" text-anchor="middle">failed (FENCE_FAILED)</text><text class="m" x="170" y="184" text-anchor="middle">FENCE_SIGNAL(f &gt; v, FAILED); nothing more</text>
<rect class="bad" x="440" y="140" width="300" height="64" rx="6"/><text class="t b" x="590" y="164" text-anchor="middle">abandoned (FENCE_ABANDONED)</text><text class="m" x="590" y="184" text-anchor="middle">the last handle with SIGNAL is gone</text>
<path class="ln" d="M330,84 L200,138" marker-end="url(#fe1)"/>
<path class="ln" d="M430,84 L560,138" marker-end="url(#fe1)"/>
<path class="ln" d="M510,40 C580,10 580,80 512,62" marker-end="url(#fe1)"/>
<text class="m" x="20" y="226">Waiting for t: t &lt;= v always succeeds; t &gt; v after an error or abandonment ends with the fence error.</text>
</svg>
<figcaption>Fence states. The reached value stays valid after an error and after abandonment.</figcaption>
</figure>

- **Timeline**: a `u64` value that grows monotonically (advancing to the
  same value does nothing, going back = `INVALID_ARGS`). A binary fence is
  a timeline with target 1; `FENCE_CREATE` takes the initial value.
- **Rights**: only a handle with the `SIGNAL` right may call
  `FENCE_SIGNAL` (the producer). The consumer gets a duplicate with `BASIC`
  only (`Fence::for_waiter`), waits and queries the state. `SIGNAL` cannot
  be added.
- **Sticky error**: `FENCE_SIGNAL(f, FAILED)` marks point `f` (greater
  than the reached value) as failed. The reached value does not change,
  every wait on an unreached value ends with `FENCE_FAILED` and any
  further advance is `BAD_STATE`. `FENCE_QUERY` returns `failed_at = f`.
- **Abandonment**: every handle with the `SIGNAL` right holds its own
  producer wrapper in the kernel (`FenceSignaler`, installed by the handle
  table). The wrapper also lives in an unread channel message, so passing
  the producer to another process does not abandon the fence. When the last
  one is gone (close, exit or crash of the process, a dropped message), the
  fence becomes abandoned: waits on an unreached value end with
  `FENCE_ABANDONED`. Abandonment does not overwrite an error. The analogue
  of a `dma_fence` that the kernel signals after a driver crash.
- **The `SIGNALED` signal** (for `OBJECT_WAIT_*` and `OBJECT_WAIT_ASYNC`)
  holds when the value is at least 1, or the fence failed or was abandoned.
  A fence created with a value >= 1 is `SIGNALED` immediately.

## Waiting

- `FENCE_WAIT` waits on a list of `(fence, value)` (at most 64): one item
  is enough, or all of them (`fence::wait::ALL`). Items are checked in
  order; a fence error ends the wait and the kernel writes the index of the
  deciding item. The check and the waiter's registration happen under the
  fence lock, so an advance between the check and going to sleep does not
  lose the wake-up (a kernel self-test sends the signal at every point,
  `plot: signal od sebe sama v kazdem miste cekani`).
- `FENCE_WAIT_ASYNC` is a one-shot port registration (like
  `OBJECT_WAIT_ASYNC`, the same slot reservation and port job budget). The
  packet arrives once the value is **decided** (reached, error or
  abandonment); `FENCE_QUERY` tells the result. The compositor and drivers
  thus have one port loop for both channels and fences.
- **The kernel does not detect wait cycles between fences** (A waits on
  B's fence and vice versa). Services (compd, drivers) therefore wait on
  another process's fence **always with a deadline** and treat the timeout
  as an error (a coordinator requirement for 6a-3 and 6a-4).

```rust
use libos::fence::{self, Fence};

let fence = Fence::create(0)?;                  // producer (SIGNAL)
let consumer = fence.for_waiter()?;             // wait and query only
channel.write(b"frame", vec![consumer.into_handle()])?;
// ... GPU work ...
fence.signal(1)?;                               // or fence.fail(1)

// consumer (another process):
match fence.wait(1, Some(time::deadline_after(Duration::from_millis(100)))) {
    Ok(()) => { /* the buffer is ready */ }
    Err(Error::FenceFailed | Error::FenceAbandoned) => { /* drop the frame */ }
    Err(e) => return Err(e),                    // TIMED_OUT, ...
}
```

## Tests

- Host: `cargo test -p abi` (`fence_tests.rs`: the state machine
  `abi::fence::Timeline`, encoding of `WaitItem` and `Info`).
- Kernel (suite `objekty`): a signal at every point between the check and
  going to sleep (one and all items), a race with going to sleep from
  another CPU (300 rounds), sticky error and item index, abandonment after
  the last producer wrapper is gone, cleanup of registrations.
- Userspace (suite `ploty`, `user/init/src/tests/fences.rs`, peer `echo`):
  producer in another process (timeline in `FENCE_WAIT` and in a port),
  consumer in another process, `FENCE_FAILED`, abandonment after `kill`
  (port and wait), producer inside a channel message, all items and the
  error index, rights and invalid arguments.
