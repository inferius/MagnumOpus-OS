# Syscalls

The interface between a process and the kernel. **Programs should use
libos** ([libos.md](libos.md)) - syscall numbers and exact arguments are not
a stable interface and change together with libos. This document describes
what the kernel does so behavior can be traced.

## ABI stability

Until version 1.0 the kernel interface may change (programs are built
together with the system). From 1.0 on:

- **Programs call the kernel through the system library** (libos, later
  libc and the Windows layer's ntdll), not directly with the `syscall`
  instruction. The library interface is stable; syscall numbers are an
  internal matter of the library and the kernel (as in Windows, macOS or
  Fuchsia).
- Syscall numbers are **never removed or reused**; a behaviour change = a
  new syscall (e.g. `..._V2`), the old one stays.
- Structures passed to the kernel carry a length or version; new fields are
  only appended.
- New error codes and signals must not change the meaning of existing ones.
- The process startup message carries a version; the kernel and the library
  support older ones too.

## Conventions (x86_64)

- `syscall` instruction, number in `rax`, arguments in `rdi`, `rsi`, `rdx`,
  `r10`, `r8`, `r9`.
- Result in `rax`: non-negative = success, negative = error code
  ([errors.md](errors.md)). All other registers except `rcx` and `r11`
  (clobbered by the instruction) are preserved.
- A handle is a 32-bit value valid only in the caller's handle table.
- Pointers point into the caller's memory. The kernel validates the whole
  range (mapped, and writable for outputs), otherwise returns `BAD_ADDRESS`
  and does nothing.
- Times and deadlines are nanoseconds since boot (`CLOCK_MONOTONIC`).
  `DEADLINE_INFINITE` = `u64::MAX` = wait forever.
- Numbers and limits: `libs/abi/src/syscall.rs`. Rights: `libs/abi/src/object.rs`.

Failed syscalls are logged at Debug level (except `SHOULD_WAIT` and
`TIMED_OUT`, which are routine).

## Logging and time

### `DEBUG_LOG`

`(ptr, len) -> 0`. Writes text (UTF-8, max 1024 B, invalid UTF-8 replaced)
to the kernel log prefixed with the process `[name id]`. Until a log service
exists this is the only program output. Errors: `BAD_ADDRESS`,
`INVALID_ARGS` (text too long). The `libos::sys::debug_log` wrapper (and
`println!`) splits longer text into 1024 B pieces, preferably after a line
end (`abi::syscall::debug_log_chunks`), so a long message is never lost whole.

### `CLOCK_MONOTONIC`

`() -> ns`. Monotonic time in nanoseconds since kernel start (TSC
calibrated against the HPET). Cannot fail.

### `CLOCK_REALTIME`

`(out_ptr) -> 0`. Real ("wall clock") time for file times and display.
Writes 16 B (little endian) to `out_ptr`:

| Offset | Type | Meaning |
|---|---|---|
| 0 | i64 | seconds since 1970-01-01 00:00:00 UTC (negative = earlier) |
| 8 | u32 | nanoseconds within the second (0..10⁹) |
| 12 | u16 | source: 1 UEFI, 2 CMOS RTC, 3 set at run time, 4 synchronized over the network (`CLOCK_SET` with `NETWORK` or `CLOCK_ADJUST` with an offset) |
| 14 | u16 | flags: bit 0 = time zone not given, time taken as UTC |

At boot the kernel stores the real time at moment 0 of the monotonic clock;
the result is that origin + `CLOCK_MONOTONIC`. The origin is accurate to
1 s (the resolution of firmware and the RTC). Unlike `CLOCK_MONOTONIC` it
is **not monotonic**: it jumps when the clock is set (`CLOCK_SET`). For measuring and deadlines always use `CLOCK_MONOTONIC`. Wrapper:
`libos::time::wall()`, `wall_reading()`. Errors: `NOT_SUPPORTED` (the
kernel does not know the real time), `BAD_ADDRESS`.
Details: [Real time](time.md).

### `CLOCK_SET`

`(resource, in_ptr, flags) -> 0 | 1`. Sets the real time. Requires a
Resource handle with the `SET_CLOCK` right. Only the root Resource has it,
and `init` gives it only to the time service `timed` (without `MANAGE`).
`in_ptr` points to 16 B (little endian, `abi::time::SetRequest`):

| Offset | Type | Meaning |
|---|---|---|
| 0 | i64 | new time: seconds since 1970-01-01 00:00:00 UTC (years 1970–9999, else `INVALID_ARGS`) |
| 8 | u32 | nanoseconds (0..10⁹, else `INVALID_ARGS`) |
| 12 | i32 | `rtc_offset`: seconds added to UTC when writing the hardware clock (RTC in local time = zone offset, else 0); at most ±18 h, else `INVALID_ARGS` |

`flags`: bit 0 `WRITE_HARDWARE` = also write the time to the hardware clock
(x86: CMOS RTC), bit 1 `NETWORK` = the time comes from network
synchronization (source 4). Other bits = `INVALID_ARGS`. Returns 1 when the hardware
clock was written. Returns 0 without `WRITE_HARDWARE`, when there is no
clock (aarch64, ACPI reports no RTC) or when the year does not fit its
range. With `WRITE_HARDWARE` the call can sleep up to 1 s: the kernel writes
the RTC in the middle of a second so the clock ticks on second boundaries.
The system time is set in every case. Afterwards `CLOCK_REALTIME`
reports source 3 (set at run time; with `NETWORK` source 4) and no flags. Wait deadlines do not
change; they come from `CLOCK_MONOTONIC`. Wrapper: `libos::time::set`,
`Resource::set_clock`. Errors: `ACCESS_DENIED` (no `SET_CLOCK`),
`WRONG_TYPE`, `BAD_HANDLE`, `INVALID_ARGS`, `BAD_ADDRESS`. Details:
[Setting the clock](time.md#setting-the-clock-clock_set).

### `CLOCK_ADJUST`

`(resource, offset_ns i64, freq_ppb i64, flags, out_ptr) -> 0`. Smooth
correction of real time without a step. Requires a Resource with the
`SET_CLOCK` right. Flags (`abi::time::adjust`):

- `OFFSET` (1): start slewing away the offset `offset_ns`, replacing the
  rest of the previous one. The rate is 500 ppm (`MAX_SLEW_PPB`), so an
  offset of 1 s takes 2000 s. At most 1 hour (`MAX_OFFSET_NS`).
- `FREQUENCY` (2): a permanent frequency correction `freq_ppb`, at most
  ±500 ppm (`MAX_FREQ_PPB`).
- no flags: status only.

At `out_ptr` (0 = nothing) it writes `abi::time::AdjustStatus` (24 B):
remaining offset (ns), frequency (ppb), current slew rate (ppb). Real time
never goes backwards. With `OFFSET`, `CLOCK_REALTIME` reports source 4
(synchronized over the network). `CLOCK_SET` cancels the slew and keeps the
frequency. Wrapper: `libos::time::adjust`. Errors: `ACCESS_DENIED` (no
`SET_CLOCK`), `INVALID_ARGS` (limits, unknown flags), `BAD_STATE` (real
time is not known), `WRONG_TYPE`, `BAD_HANDLE`, `BAD_ADDRESS`. Details:
[Smooth correction](time.md#smooth-correction-clock_adjust).

## Handles

### `HANDLE_CLOSE`

`(handle) -> 0`. Closes the handle. The object is destroyed once no handle
or internal reference points to it (e.g. closing the last end of a channel
sets `PEER_CLOSED` on the other end). Errors: `BAD_HANDLE`.

### `HANDLE_DUPLICATE`

`(handle, rights) -> handle`. A new handle to the same object. `rights`
must be a subset of the original rights; `u32::MAX` keeps them. Requires
`DUPLICATE`. Errors: `BAD_HANDLE`, `ACCESS_DENIED` (missing `DUPLICATE` or
rights would be added), `NO_RESOURCES`.

## Waiting and events

### `OBJECT_WAIT_ONE`

`(handle, mask, deadline, out_signals_ptr) -> 0`. Waits until the object's
signals intersect `mask` or the deadline passes. Writes the object's current
signals to `out_signals_ptr` (if non-zero) - also on `TIMED_OUT`. Requires
`WAIT`. Errors: `BAD_HANDLE`, `ACCESS_DENIED`, `TIMED_OUT`, `CANCELED`,
`BAD_ADDRESS`.

### `OBJECT_WAIT_MANY`

`(items_ptr, count, deadline) -> index`. Waits on several objects at once
(max 64). `items` is an array of `WaitItem { handle: u32, mask: u32, observed: u32 }`;
the kernel returns the index of the first satisfied object and writes the
current signals of every item to `observed`. `count` = 0 is a sleep until
the deadline (ends with `TIMED_OUT`). Requires `WAIT` on all handles. Errors:
as `OBJECT_WAIT_ONE` plus `INVALID_ARGS` (count > 64).

### `OBJECT_SIGNAL`

`(handle, clear, set) -> 0`. Changes an event's signals: clears `clear`,
then sets `set`. A process may change `SIGNALED` and eight user bits
`USER_0` to `USER_7` (bits 24 to 31, `Signals::USER_ALL`) **[verified in
QEMU]** (suite "udalosti a nahoda"); other signals are kernel-controlled.
User bits carry state that a service shows to its clients: the service
keeps a handle with `SIGNAL` and gives the client a duplicate with `WAIT`
only. For example `netd` reports socket readiness with them (see
[Network](network.md#socket-readiness)). They are waited for like other
signals (`OBJECT_WAIT_*`, ports). Requires `SIGNAL`. Errors:
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE` (not an event), `INVALID_ARGS`
(a bit other than `SIGNALED` and `USER_*`).

### `EVENT_CREATE`

`() -> handle`. A new (unsignaled) event with rights `DUPLICATE`,
`TRANSFER`, `WAIT`, `SIGNAL`. Errors: `NO_RESOURCES`.

## Ports

A port is an event queue for waiting on **any number of objects**
(services with hundreds of channels, e.g. `netd`). `OBJECT_WAIT_MANY` is
limited to 64 objects and registers the thread with all of them on every
wait. A port instead holds **one-shot registrations**: an object whose
signals intersect the mask puts a packet (key + signals) into the port
and the registration ends. The program picks the packet up and registers
the object again after servicing it. The cost of an event does not depend
on the number of watched objects.

- **Room for the packet is reserved at registration**, so delivery (even
  from an interrupt handler) does not allocate and a packet is never lost.
  Every registration and unpicked packet costs 64 B of the kernel-memory
  budget of the job of the process that created the port
  ([Jobs](#jobs-resource-budgets)); there is no fixed count. The charge
  is returned when a packet is picked up, the object is destroyed or the
  port is closed. Packets live in 4 KiB blocks (no large contiguous
  allocation), an empty port returns spare blocks.
- **A destroyed object** (its last handle closed) returns the reservation
  of its undelivered registrations. **A closed port**: objects drop its
  registrations.
- A port has the `READABLE` signal while it holds a packet, so it can also
  be waited on with `OBJECT_WAIT_ONE`/`OBJECT_WAIT_MANY`.
- Packets arrive in the order the conditions occurred.
- Wrappers: `libos::Port` (`create`, `wait_async`, `wait`).

```rust
let port = libos::Port::create()?;
port.wait_async(ch.handle(), 1, Signals::READABLE)?;     // key 1
port.wait_async(ch.handle(), 2, Signals::PEER_CLOSED)?;  // key 2
loop {
    let p = port.wait(None)?;
    match p.key {
        1 => { /* read the message */ port.wait_async(ch.handle(), 1, Signals::READABLE)?; }
        2 => break, // the other side closed
        _ => {}
    }
}
```

### `PORT_CREATE`

`() -> handle`. A new empty port (charged to the job budget) with rights `DUPLICATE`, `TRANSFER`,
`WAIT`, `READ`, `WRITE`. Errors: `NO_RESOURCES`.

### `OBJECT_WAIT_ASYNC`

`(handle, port, key, mask) -> 0`. A one-shot registration: when the
signals of the object `handle` intersect `mask`, the port gets a packet
with `key` (u64, meaning chosen by the program). If the condition already
holds, the packet arrives at once. Requires `WAIT` on the object and
`WRITE` on the port. Registering on a port itself is not possible.
Errors: `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE` (`port` is not a
port), `NOT_SUPPORTED` (the object is a port), `INVALID_ARGS` (empty
mask), `NO_RESOURCES` (the kernel budget of the job is used up), `NO_MEMORY`.

### `PORT_WAIT`

`(port, deadline, out_ptr) -> 0`. Removes the oldest packet and writes it
to `out_ptr` (16 B, `abi::object::PortPacket`): key u64, the object's
signals when the condition held u32, reserved u32. An empty port waits
until the deadline (`deadline` = 0: just try). Requires `READ`. Errors:
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `TIMED_OUT`, `CANCELED`,
`BAD_ADDRESS` (the packet is then lost).

## Fences

A fence is a `u64` timeline for explicit synchronisation between processes
(the producer advances, the consumer waits). Model, abandonment and wait
rules: [fences.md](fences.md). **[verified in QEMU]** x86_64 and aarch64
(`ploty` suite, kernel self-tests `plot: ...`). Wrappers: `libos::Fence`,
`libos::fence::{wait_any, wait_all}`.

### `FENCE_CREATE`

`(initial, flags) -> handle`. A new fence with value `initial` and rights
`DUPLICATE`, `TRANSFER`, `WAIT`, `SIGNAL`; `flags` must be 0. Consumers get
a duplicate without `SIGNAL`. Errors: `INVALID_ARGS`, `NO_RESOURCES` (job
kernel budget), `NO_MEMORY`.

### `FENCE_SIGNAL`

`(fence, value, flags) -> 0`. Without flags it advances the value to
`value` (must be >= the reached value; equal = no-op). With
`fence::signal::FAILED` it marks point `value` (greater than the reached
value) as failed: a sticky error, waits on an unreached value end with
`FENCE_FAILED`. Wakes the waiters whose value is decided and sets
`SIGNALED` (value >= 1 or an error). Requires `SIGNAL`. Errors:
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS` (value going
back, error on a reached point, unknown flag), `BAD_STATE` (the fence has
already failed).

### `FENCE_WAIT`

`(items_ptr, count, flags, deadline, out_index_ptr) -> index`. Waits on
items `fence::WaitItem { handle: u32, _reserved: u32, value: u64 }` (1 to
64). Without flags one reached value is enough and the result is its
index; with `fence::wait::ALL` it waits for all and returns 0. Items are
checked in order; a fence error (`FENCE_FAILED`, `FENCE_ABANDONED`) ends
the wait and the index of the deciding item is written to `out_index_ptr`
(u32, 0 = do not write). `deadline` = 0: just try. Requires `WAIT` on all
handles. Errors: also `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE` (the
handle is not a fence), `INVALID_ARGS` (count 0 or > 64, unknown flag),
`TIMED_OUT`, `CANCELED`, `BAD_ADDRESS`, `NO_MEMORY`.

### `FENCE_WAIT_ASYNC`

`(fence, value, port, key) -> 0`. One-shot registration: once value
`value` is decided (reached, error or abandonment), the port receives a
packet with `key` and the fence's signals; `FENCE_QUERY` tells the result.
A decided value = the packet right away. Slot and budget as with
`OBJECT_WAIT_ASYNC`. Requires `WAIT` on the fence and `WRITE` on the port.
Errors: `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `NO_RESOURCES`,
`NO_MEMORY`.

### `FENCE_QUERY`

`(fence, out_ptr) -> 0`. Writes `fence::Info` (24 B): the reached value
u64, the first failed point u64 (0 = no error), status i32 (0,
`FENCE_FAILED` or `FENCE_ABANDONED`), reserved u32. Requires `WAIT`.
Errors: `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `BAD_ADDRESS`.

## Channels

A message = data (max 64 KiB) + handles (max 64). Each end has its own queue
of incoming messages (max 1024). Signals of an end: `READABLE`,
`PEER_CLOSED`.

### `CHANNEL_CREATE`

`(out_handles_ptr) -> 0`. Creates a connected pair of ends and writes two
handles (u32) to `out_handles_ptr`. Rights: `DUPLICATE`, `TRANSFER`, `WAIT`,
`READ`, `WRITE`. Errors: `NO_RESOURCES`, `BAD_ADDRESS`.

### `CHANNEL_WRITE`

`(handle, data_ptr, data_len, handles_ptr, handles_count) -> 0`. Sends a
message to the other end. Transferred handles need `TRANSFER`, must not
repeat and must not be the end being written to nor its peer (the message
would end up in the peer's own queue, which would then hold itself). A
channel end whose queue already holds another channel end cannot be sent
(`BAD_STATE`): this way no cycle of ends holding each other in their queues
(and never being freed) can ever form. Transferred handles are
**always removed from the sender's table, even if the write fails** - a
handle is never held by both sender and receiver. Requires `WRITE`. Errors:
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`, `PEER_CLOSED`,
`SHOULD_WAIT` (queue full), `BAD_ADDRESS`.

### `CHANNEL_READ`

`(handle, data_ptr, data_cap, handles_ptr, handles_cap, out_actual_ptr) -> 0`.
Takes the next message: copies data to `data_ptr`, inserts handles into the
process table and writes their values to `handles_ptr`, writes two u32
(data length, handle count) to `out_actual_ptr`. Non-blocking. Requires
`READ`. Errors: `SHOULD_WAIT` (empty queue), `PEER_CLOSED` (empty queue and
other end closed), `BUFFER_TOO_SMALL` (message stays queued, required sizes
are written to `out_actual_ptr`), `BAD_HANDLE`, `ACCESS_DENIED`,
`WRONG_TYPE`, `BAD_ADDRESS`.

## Memory

### `VMO_CREATE`

`(size, process) -> handle`. A memory object of `size` bytes (rounded up
to 4 KiB, max 1 TiB), zero-filled. The whole size is committed at once
(the job's user-memory budget and the global ceiling of program memory);
frames come into existence on the first write
([memory-management.md](memory-management.md)). It is charged to the
user-memory budget of the job of `process` (a handle
with `WRITE`; 0 = the caller). A parent thus creates a program's memory
(segments, stack) on the child's account. Rights: `DUPLICATE`,
`TRANSFER`, `WAIT`, `READ`, `WRITE`, `MAP`, `EXECUTE`. Errors:
`INVALID_ARGS` (0 or too large), `NO_MEMORY`, `NO_RESOURCES` (job
budget), `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE` (`process`).
`NO_MEMORY` also means that so much memory cannot be committed (ceiling =
free RAM minus the kernel reserve minus pages already committed).

### `VMO_CLONE`

`(vmo, offset, len, process) -> handle`. A copy-on-write clone of `len` bytes
of the object from `offset`: the new object shares the source's pages until
one of them writes (the first write makes a copy; source and clone never
affect each other). The clone commits its whole size, the source one extra
page per newly shared frame, so a copy on write never runs out of memory;
it is charged to the budget of the job of `process` (a handle with `WRITE`;
0 = the caller). Pinned pages (DMA, `VMO_PHYS_ADDR`) are copied into the
clone right away. The clone may see writes to the source made during the
call. Rights: source `READ`; the clone has full object rights. Regular
objects only. Errors: `INVALID_ARGS` (zero length, unaligned offset or
length), `OUT_OF_RANGE`, `NOT_SUPPORTED` (contiguous, DMA or physical
object), `NO_MEMORY`, `NO_RESOURCES`, `BAD_HANDLE`, `ACCESS_DENIED`,
`WRONG_TYPE`.

### `VMO_SEAL`

`(vmo) -> 0`. Seals the memory object: its content never changes again.
`VMO_WRITE`, mapping for writing and `VMO_PHYS_ADDR` then return
`ACCESS_DENIED`; mappings get maximum rights without write. A clone
(`VMO_CLONE`) of a sealed object is a regular object. Irreversible; sealing
again does nothing. Without the JIT permission only sealed memory can be
executed (see [memory-management.md](memory-management.md)). Requires
`WRITE`. Errors: `BAD_STATE` (the object has a mapping that may write -
even only as a maximum right), `NOT_SUPPORTED` (contiguous, DMA or physical
object), `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `VMO_READ`

`(handle, offset, buf_ptr, len) -> 0`. Reads object data (max 1 MiB per
call). Requires `READ`. Errors: `OUT_OF_RANGE`, `BAD_ADDRESS`, `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`.

### `VMO_WRITE`

`(handle, offset, buf_ptr, len) -> 0`. Writes data into the object (max
1 MiB). Requires `WRITE`. Errors: as `VMO_READ`.

### `VM_MAP`

`(process, vmo, vmo_offset, len, flags, addr) -> addr`. Maps `len` bytes of
the object starting at `vmo_offset` into the address space of `process`
(own or another one, e.g. when starting a program). `flags`: `MAP_READ`
(required), `MAP_WRITE`, `MAP_EXECUTE` (not together with `MAP_WRITE` -
W^X), `MAP_FIXED` (exactly at `addr`; otherwise `addr` is a hint and 0 lets
the kernel choose). Address, length and offset must be aligned to the
process page size (`VM_PAGE_SIZE`, 4 KiB today), range
within `0x10000..0x7FFF_FFFF_F000` (`USER_MIN`..`USER_MAX`; the last page
of the lower half always stays unmapped). Rights: process `WRITE`; object `MAP` +
`READ`, plus `WRITE`/`EXECUTE` per flags. A mapping keeps the object alive
after its handle is closed. Errors: `INVALID_ARGS`, `OUT_OF_RANGE`,
`BAD_STATE` (`MAP_FIXED` over an existing mapping; the process has finished
and its address space is gone; without `MAP_IN_RESERVATION` over a
reservation; with it the range is not fully within reservations),
`NO_RESOURCES` (no free range), `NO_MEMORY`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `VM_UNMAP`

`(process, addr, len, flags) -> 0`. Removes regions in the range (mappings
and reservations), also parts of them - a region on the boundary is split.
The range must be fully covered by regions without holes. `flags`: 0, or
`UNMAP_DECOMMIT` = mappings turn into reservations (memory returned,
addresses stay taken; `MEM_DECOMMIT` on Windows), reservations stay. On
return no CPU holds a stale TLB entry. Requires `WRITE` on the process.
Errors: `NOT_FOUND` (a hole in the range), `INVALID_ARGS`, `NO_RESOURCES`
(kernel budget for splitting a region), `BAD_HANDLE`, `ACCESS_DENIED`.

### `VM_RESERVE`

`(process, addr, len, flags) -> addr`. Takes a range of addresses without
memory (a reservation). `flags`: `MAP_FIXED` (exactly at `addr`, otherwise
a free range and `addr` is a hint), `RESERVE_GUARD` (a guard region: an
access ends the process with a stack overflow message). It costs only a
region record from the job's kernel budget (no page tables, no memory), so
even tens of GiB can be reserved (Wine). An access into a reservation is a
process error. Only `VM_MAP` with `MAP_FIXED | MAP_IN_RESERVATION` maps
into it. `VM_UNMAP` removes it. Requires `WRITE` on the process. Errors:
`INVALID_ARGS`, `BAD_STATE` (`MAP_FIXED` over an existing region; the
process has finished), `NO_RESOURCES`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `VM_PROTECT`

`(process, addr, len, prot) -> 0`. Changes the rights of mappings in the
range (also parts; the region is split). `prot` is a subset of `MAP_READ |
MAP_WRITE | MAP_EXECUTE`, 0 = no access. Never write and execute together
(W^X) and never above the rights at `VM_MAP` (which checked the object
handle's rights). Mapped pages are unmapped and a fault fills them in with
the new rights. Requires `WRITE` on the process. **`PROT_SEAL`** in `prot`
also **seals** the regions (like Linux `mseal`, irreversibly until the
process ends): the rights are set to `prot` and the `max_flags` ceiling is
lowered to them; a sealed region can then not be unmapped (`VM_UNMAP` and
`UNMAP_DECOMMIT` = `ACCESS_DENIED`), its rights cannot change
(`ACCESS_DENIED`), a sealed reservation cannot be filled and nothing can be
mapped over it (`BAD_STATE`); `VM_QUERY` reports `region::ATTR_SEALED`. With
the seal the range may contain reservations too (they are only sealed). Use:
RELRO after relocation, library code
([dynamic-linking.md](dynamic-linking.md)). Errors: `INVALID_ARGS` (unknown
bit, W+X, alignment), `ACCESS_DENIED` (above the original rights, a sealed
region), `NOT_FOUND` (a hole), `BAD_STATE` (a reservation in the range
without the seal), `NO_RESOURCES`, `BAD_HANDLE`.

### `VM_QUERY`

`(process, addr, out_ptr) -> 0`. Writes `abi::vm::RegionInfo` (40 B) of the
region at `addr`: start, length, kind (`region::FREE` / `RESERVED` /
`MAPPED`), current and maximum rights, attributes (`ATTR_GUARD`,
`ATTR_IN_RESERVATION`) and the offset in the object. For a free range, the
start and length of the gap between neighbouring regions. For
`VirtualQuery`, a process memory listing and debugging. Requires `READ` on
the process. Errors: `INVALID_ARGS` (`addr` outside `USER_MIN..USER_MAX`),
`BAD_ADDRESS`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `VM_PAGE_SIZE`

`(process) -> bytes`. The page size of the process's address space: a power
of two from `MIN_PAGE_SIZE` (4 KiB) to `MAX_PAGE_SIZE` (64 KiB) in
`abi::vm`. Addresses, lengths and offsets for `VM_MAP` and `VM_UNMAP` must
be aligned to it. It is not an ABI constant: it is 4 KiB everywhere today,
but on aarch64 the kernel may give native processes 16 KiB and Windows
layer processes 4 KiB (each address space has its own granule). A parent
asks about its child when loading a program (`libos::vm::page_size_of`), a
program about itself (`libos::vm::page_size`, queried once and cached).
Any process handle is enough (no rights). Errors: `BAD_HANDLE`,
`WRONG_TYPE`.

## Processes and threads

Starting a program (libos `spawn` does this): `PROCESS_CREATE` ->
`VMO_CREATE`/`VMO_WRITE`/`VM_MAP` of segments and stack -> `CHANNEL_CREATE`
and writing the startup message -> `THREAD_CREATE` -> `PROCESS_START`.

### `PROCESS_CREATE`

`(job, name_ptr, name_len) -> handle`. An empty process with no memory or
threads in job `job` (right `MANAGE`); everything the process costs is
charged to the job budget. Name: UTF-8, max 64 B (for logs). Rights:
`DUPLICATE`, `TRANSFER`, `WAIT`, `MANAGE`, `WRITE`, `READ`. Errors:
`INVALID_ARGS`, `NO_MEMORY`, `NO_RESOURCES` (job budget), `BAD_ADDRESS`,
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `PROCESS_START`

`(process, thread, entry, stack, bootstrap_handle) -> 0`. Starts the
process's first thread at `entry` with stack `stack`. `bootstrap_handle`
(typically a channel end carrying the startup message) is moved into the new
process's table and the thread receives its value in `rdi`. The thread must
belong to the process and the process must not be started yet. Requires
`MANAGE` on the process and the thread, `TRANSFER` on the bootstrap handle.
`entry` must lie in the process memory (`USER_MIN` to `USER_MAX`), `stack`
at most at `USER_MAX`, otherwise `INVALID_ARGS`. Errors: `BAD_STATE`,
`INVALID_ARGS`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `PROCESS_EXIT`

`(code)` - does not return. Terminates the current process with `code`: all
its threads end (waiting ones with `CANCELED`), the address space is torn
down and handles are closed (memory and job budgets come back at once, not
when the process handle is closed), then the process gets the `TERMINATED`
signal. A handle to a finished process holds only a tombstone (code, CPU
time, name) - see [kernel.md](kernel.md#processes-and-threads).

### `PROCESS_KILL`

`(process) -> 0`. Terminates another process with code -1,000,000. Threads
running on other CPUs are interrupted. The process's memory and handles are
released as with `PROCESS_EXIT` once the last thread ends. Requires
`MANAGE`.

### `PROCESS_EXIT_CODE`

`(process, out_code_ptr) -> 0`. Writes the exit code (i64) of a finished
process. Requires `READ`. Errors: `BAD_STATE` (still running - wait for
`TERMINATED` first).

### `THREAD_CREATE`

`(process, name_ptr, name_len) -> handle`. A new (not started) thread of the
process. Requires `MANAGE` on the process. Errors: `BAD_STATE` (process is
terminating), `NO_MEMORY`, `INVALID_ARGS`.

### `THREAD_START`

`(thread, entry, stack, arg0, arg1) -> 0`. Starts another thread of an
already running process; it receives `arg0`, `arg1` in `rdi`, `rsi`. The
stack must be aligned as after a `call` instruction (address = 8 mod 16) if
the entry function follows the System V ABI. `entry` and `stack` follow the
same rules as for `PROCESS_START`. Requires `MANAGE`. Errors: `BAD_STATE`,
`INVALID_ARGS`.

### `THREAD_EXIT`

`()` - does not return. Ends the current thread. When the last thread ends,
the process ends (code -1,000,002 unless `PROCESS_EXIT` was called before).

### `THREAD_SUSPEND`

`(thread) -> token`. Suspends a process thread. The thread stops at the
boundary of returning to userspace and gets the `SUSPENDED` signal; a thread
sleeping in a syscall right now counts as suspended at once (its user
context is saved) and after waking it stops before returning. A running
thread is interrupted (IPI). Closing the token resumes the thread; with
several tokens the thread stays stopped while at least one lives (if a
debugger dies, the thread runs again by itself). Kill and `JOB_KILL` always
win: a suspended thread ends. Requires `MANAGE` on the thread. Errors:
`WRONG_TYPE` (kernel thread), `BAD_STATE` (the thread has ended),
`NO_RESOURCES`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `THREAD_STATE`

`(thread, op, kind, ptr, len) -> 0`. Reads (`thread::op::READ`, right
`READ`) or writes (`WRITE`, right `WRITE`) the context of a suspended
thread. `kind`: `thread::kind::GENERAL` - general registers, PC, SP, flags
and the TLS base (`abi::thread::X64General` 160 B, `A64General` 280 B);
`FP` - FPU/SIMD: the header `abi::thread::fp::Header` (64 B: format, data
length, x86_64 XCR0) and the data - on x86_64 an XSAVE image in the
standard format for the parts the kernel enabled (size depends on the CPU,
CPUID 0xD; without XSAVE an FXSAVE image of 512 B), on aarch64 V0-V31 +
FPCR + FPSR, 528 B. A program gets the size from
`libos::exception::fp_state_size()` (or from the header). `len` at least
the state size (a larger buffer is used only up to it, a smaller one =
`BUFFER_TOO_SMALL`; reading the FPU with a buffer of at least the header
size, the kernel writes the needed length into it). A read waits (at most about a
tenth of a second) until the suspended thread really stands in the kernel.
The kernel validates a write and the thread applies it right before
returning to userspace (a read already sees it); for a thread sleeping in a
syscall it also overwrites the return value (rax / x0). Write validation:
PC, SP and the TLS base inside the process memory (canonical on x86_64,
below `USER_MAX`), only user bits of the flags (x86_64 RFLAGS without IOPL,
IF always 1; aarch64 only NZCV), for the FPU the header of this CPU
(format, length, XCR0) and zero reserved bits (MXCSR, FPCR, FPSR; in the
XSAVE header XSTATE_BV only parts from XCR0, XCOMP_BV 0 - the compacted
format is not accepted). Errors: `BUFFER_TOO_SMALL` (`len` below the
state size), `INVALID_ARGS` (unknown `op`/`kind`, invalid context), `BAD_STATE` (the thread is not suspended), `WRONG_TYPE`,
`BAD_ADDRESS`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `EXCEPTION_CHANNEL`

`(task, flags) -> channel`. An exception channel of a process or a thread
(`task`, right `MANAGE`). `flags`: 0, `exception::channel::DEBUGGER`
(process only: the debugger channel, first and second chance), or
`HANDLER_THREAD` (thread only: marks it as a handler thread - its
exceptions skip the thread and process channels; no channel is created,
returns 0). At most one live channel per slot (a closed one is replaced).
A CPU exception in userspace that the kernel does not resolve itself
(demand paging) goes to handlers in order: debugger (first chance),
thread, process, debugger (second chance); nobody = the process ends as
before. Message: `exception::Report` (56 B: kind, stage, access, process,
thread, PC, address, architecture code) and handles of the thread (`READ`,
`WRITE`, `MANAGE`) and of the exception token. The thread stands meanwhile
and its context can be read and written (`THREAD_STATE`). Errors:
`INVALID_ARGS` (a flag for the other type), `ALREADY_EXISTS`,
`WRONG_TYPE`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `EXCEPTION_RESOLVE`

`(token, action) -> 0`. The handler's decision (`exception::action`):
`RESUME` = the thread continues (with the context changed through
`THREAD_STATE`), `NEXT` = the next handler in order, `KILL` = the process
ends as with an unhandled exception. Closing the token without a decision
= `NEXT` (also when the handler dies). Breakpoint: on x86_64 the PC is
already past `int3`, on aarch64 at `brk` (the handler advances it by 4).
Errors: `INVALID_ARGS` (unknown action), `BAD_STATE` (already decided or
the thread has ended - kill always wins), `WRONG_TYPE`, `BAD_HANDLE`.

### `EXCEPTION_HANDLER`

`(op, a0, a1) -> ...`. Exception delivery right in the thread (the fast
path for SEH in the Windows layer and signals in `cstd`). Operations
`exception::handler`:

- `SET_ENTRY(process, entry)`: the process's handler entry (0 = clear;
  right `WRITE` on the process). `entry` must lie in an executable mapping
  - sealed or with the JIT permission (W^X), so a handler cannot bypass
  the executable memory rules. Otherwise `INVALID_ARGS`.
- `SET_STACK(base, size)`: an alternate stack of the current thread (like
  `sigaltstack`, 0 = none; at least a frame + 256 B), needed to handle a
  stack overflow.
- `RETURN(frame, disposition)`: return from the handler. `RESUME` restores
  the context from the frame (the same validation as `THREAD_STATE`: PC,
  SP in userspace, user flags, FPU without reserved bits; the TLS base is
  not restored) and the thread continues; `NEXT` restores the context and
  offers the exception to the next handlers (thread channel, process
  channel, debugger, end of the process) - the kernel takes the exception
  kind and address from its own copy, not from the frame. A successful
  return does not return to the caller. Errors: `INVALID_ARGS` (invalid
  frame, unknown action), `BAD_STATE` (no handler running), `BAD_ADDRESS`.

Handler order: debugger (first chance), in-thread handler, thread channel,
process channel, debugger (second chance). The in-thread handler gets the
`exception::frame` (aligned to 64 B: `Report`, general registers and
FPU/SIMD) on the alternate stack (if set and the thread is not on it yet),
otherwise below SP (x86_64 below the 128 B red zone); x86_64: `rdi` =
frame, `rsp % 16 == 8`, return address 0; aarch64: `x0` = frame, `x30` =
0. If the frame does not fit (a guard region on stack overflow), the
handler is skipped and the exception goes on - never recursion. Nesting
(an exception inside the handler) is limited to `handler::MAX_DEPTH` (8);
the next one goes straight on.

### `PROCESS_THREADS`

`(process, out_ptr, max) -> count`. Writes up to `max` (at most 4096)
handles (`u32`) of the process's live threads to `out_ptr` (for a
debugger: suspend them all). Handle rights follow the process handle:
`READ` = read the context only, with `WRITE` also write it and suspend
(`MANAGE`). Returns the total thread count (also above `max`). Requires
`READ`. Errors: `INVALID_ARGS` (`max` over 4096), `BAD_ADDRESS`,
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

Whoever holds a process handle with `WRITE` (or a thread handle with
`WRITE`) has **full control** over the process: just like with `VM_MAP`
they can overwrite its code or registers. A parent is thus naturally its
child's debugger; a process handle with `WRITE` is given to nobody else.

### `THREAD_YIELD`

`() -> 0`. Gives the CPU to the next ready thread of the same or higher
class.

### `THREAD_SET_POINTER`

`(value) -> 0`, number 56. Sets the thread pointer register of the current thread for
TLS (x86_64 `FS_BASE`, aarch64 `TPIDR_EL0`). The kernel stores the value in
the thread and loads it on every switch; a new thread starts with 0, so the
value never leaks into another thread or process (on aarch64 a program may
write `TPIDR_EL0` itself, so the kernel saves it on a switch). Errors:
`INVALID_ARGS` (`value` outside user space). In libos
`sys::thread_set_pointer`; used by the `std` runtime ([Rust std](rust-std.md)).

## Futex

Waiting for a change of a `u32` in process memory, the base of locks and
condition variables in user space (`std::sync`, `libos::sync::Mutex`). The
key is a virtual address in the own process; sharing between processes is
not supported yet. Checking the value and queueing happen under the
process queue lock, which `WAKE` takes too, so a wake-up between the
check and going to sleep is never lost.

### `FUTEX`

`(op, addr, value, deadline) -> result`. Number 55.

| `op` | `value` | meaning |
|---|---|---|
| 0 `WAIT` | expected value | if the `u32` at `addr` (4-byte aligned) equals `value`, the thread sleeps until `WAKE` on the same address wakes it or `deadline` passes (ns since boot, `DEADLINE_INFINITE` = no limit); a different value = 0 right away; spurious wake-ups are possible, the caller re-checks its condition; returns 0 |
| 1 `WAKE` | most threads to wake (`u32::MAX` = all) | wakes threads waiting on `addr` in arrival order, returns the number woken; `deadline` is unused |

Errors: `TIMED_OUT` (`WAIT`), `INVALID_ARGS` (unknown operation, unaligned
address, address outside user space), `INVALID_ADDRESS` (not mapped),
`CANCELED` (process termination). In libos `sys::futex_wait`,
`sys::futex_wake`.

## Jobs (resource budgets)

Every process belongs to one **job**. Jobs form a tree: init gets the
root from the kernel (no limit, all rights) and divides it among
services. Everything a process costs is charged to the budget of its job
**and of all ancestors**, so a job limit applies to its whole subtree.

Two budgets in bytes (`abi::job::pool`):

| Budget | What is charged |
|---|---|
| `KERNEL` (0) - kernel memory | the process (structure + root page table), objects (event, channel, port, memory object, interrupt, resource, job), handle slots, port registrations (64 B), messages in channel queues (charged to the sender), mappings (record + upper estimate of page tables), threads (structure + 64 KiB kernel stack; the stack is returned right when the thread ends), page lists of memory objects |
| `USER` (1) - user memory | pages of memory objects (`VMO_CREATE`, `VMO_CREATE_CONTIGUOUS`, DMA memory too) |

- A charge is returned automatically when the thing holding it goes away.
- Exceeding a budget = `NO_RESOURCES`. Running out of physical memory of
  the system is still `NO_MEMORY` (kernel reserve below).
- A child gets its job **without the `WRITE` right**: it may start
  processes and create sub-jobs in it (`MANAGE`) but cannot change the
  limit. Even if it raised the limit of a sub-job, the ancestor limit
  still applies.
- A limit below current usage is allowed: nothing is taken away, only
  further charges fail.
- Races: two concurrent charges just below the limit may both fail; the
  limit is never exceeded.
- Wrappers: `libos::job` (`Job::create`, `set_limit`, `info`, `current`),
  `libos::spawn_in` (start a program in a given job; `spawn` = the same
  job as the parent). The startup message carries the job as
  `bootstrap::kind::JOB`.

```rust
use libos::job::{Job, pool};
let job = Job::create(&libos::job::current()?)?;
job.set_limit(pool::USER, 64 << 20)?;   // 64 MiB of user memory
job.set_limit(pool::KERNEL, 4 << 20)?;  // 4 MiB of kernel memory
let (proc, ch) = libos::spawn_in(&job, "service", image, handles)?;
let info = job.info()?;                 // usage, limits, remaining
```

> **Change from the previous version:** limits were fixed counts per
> process (65,536 handles, 1024 threads, 65,536 mappings, 65,536 port
> registrations per port and per process). A large server (hundreds of
> thousands of connections) would hit them, and they did not protect what
> matters - memory: 65,536 memory objects of 1 GiB each would not exceed
> the limit. Byte budgets protect memory directly, the parent sets them
> and they can be graded along the tree.

### `JOB_CREATE`

`(parent) -> handle`. A sub-job with no own limit (ancestor limits
apply). `parent` needs `MANAGE`. The object is charged to the kernel
budget of `parent`. Rights: `DUPLICATE`, `TRANSFER`, `WAIT`, `READ`,
`WRITE`, `MANAGE`. Errors: `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`,
`NO_RESOURCES`.

### `JOB_SET_LIMIT`

`(job, pool, bytes) -> 0`. Sets the own limit of budget `pool`
(`KERNEL` = 0, `USER` = 1) in bytes; `u64::MAX` (`job::UNLIMITED`) = no
own limit. With `pool` = `policy::JIT` (2) it sets the JIT policy: the
value is the bits `jit::RUN` (processes of the job may execute unsealed
memory) and `jit::GRANT` (the job may grant JIT to sub-jobs); only someone
whose parent job may grant can turn them on, turning off always works (and
applies to descendants at once). A new job has neither, the root job only `GRANT`.
Requires `WRITE`. Errors: `INVALID_ARGS` (unknown `pool`, unknown bit),
`ACCESS_DENIED` (the parent does not grant JIT), `BAD_HANDLE`,
`WRONG_TYPE`.

### `JOB_INFO`

`(job, out_ptr) -> 0`. Writes `abi::job::JobInfo` (56 B, little endian):
for kernel memory and then user memory always `used` (charged to the job
including descendants), `limit` (own limit) and `available` (how much
can still be charged - the smallest free room over the job and all
ancestors; `u64::MAX` = no limit anywhere), finally `id` (the job number,
unique until reboot). Requires `READ`. Errors:
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `BAD_ADDRESS`.

### `JOB_PROCESSES`

`(job, out_ptr, max) -> count`. Processes of the job and all its
descendants (task overview: also a process that did not register itself).
Writes at most `max` (up to 4096) `abi::job::ProcEntry` records (64 B,
little endian) to `out_ptr`: `pid`, `job` (the process's job number),
`cpu_ns` (CPU time of all threads, added by the scheduler on a switch),
`threads` (live threads), `flags` (`proc_flags::FINISHED` = exited; only a
tombstone without memory remains while someone holds a handle;
`proc_flags::JIT_USED` = the process got executable unsealed memory - the
JIT audit), `name` (32 B UTF-8, zero padded). Returns the total
count (also above `max`). Requires `READ`. Errors: `INVALID_ARGS` (`max`
> 4096), `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `BAD_ADDRESS`.

### `JOB_KILL`

`(job, code) -> 0`. **[implemented]** Ends all processes of the job and
all its descendants with exit code `code` and **closes** the job: no new
process (`PROCESS_CREATE`) or sub-job (`JOB_CREATE`) can be created in it
or its descendants any more (`BAD_STATE`). Requires `MANAGE`. Init uses it
at sign-out (the session job, see [User accounts](accounts.md#sessions)).

- **A caller inside the killed job** ends like the others: the syscall
  returns 0, its threads get a termination request and end on the return
  to user space (code `code`).
- **Race with creating a process or sub-job:** the kernel first sets the
  job's flag, then walks the process registry. A new process registers
  first and then checks the flag of its job and the ancestors (both
  `SeqCst`): either `JOB_KILL` sees it in the registry and ends it, or its
  own check refuses it. A sub-job is checked the same way after creation.
  No process slips through; an unstarted process ended while loading fails
  `PROCESS_START` (`BAD_STATE`).
- Processes end asynchronously (as after `PROCESS_KILL`): wait for the end
  via `TERMINATED` on the process handle or `JOB_PROCESSES` (`FINISHED`).

Errors: `BAD_HANDLE`, `ACCESS_DENIED` (no `MANAGE`), `WRONG_TYPE`. In
libos `Job::kill`. Tests in the `joby` suite: `job: JOB_KILL ukonci
podstrom a job uzavre`, `job: JOB_KILL vlastniho jobu ukonci i
volajiciho`, `job: JOB_KILL behem spousteni nic neproklouzne`.

### `FB_CONTROL`

`(vmo, op, out_ptr) -> 0`. The firmware framebuffer: `vmo` must be exactly
the object from the bootstrap `kind::FRAMEBUFFER` (any other memory object
= `WRONG_TYPE`). Operations (`abi::syscall::fb_op`):

- `INFO` (0): writes `abi::syscall::FbInfo` (16 B: width, height, pixels
  per line, format 1 = RGB, 2 = BGR; all u32). Requires `READ`.
- `CLAIM` (1): **explicit takeover** - the kernel stops drawing the log
  into the framebuffer, the log keeps going to the serial line. Requires
  `WRITE`.
- Handover: when another live process holds the screen, `CLAIM` returns
  `BAD_STATE`. The new owner gets it once the old one gives it back
  (`RELEASE`) or exits (the kernel then gives it back by itself). The
  owner of the display drives the handover (the init supervisor: the
  fallback console after a GUI crash).
- `RELEASE` (2): give back - the kernel clears the screen and keeps
  drawing the log (with the boot screen, `bootstrap::flags::SPLASH`, it
  leaves the picture unchanged for the next owner, see
  [Boot screen](boot-screen.md)). Requires `WRITE`. Only the owner may (the process that
  last called `CLAIM`), otherwise `BAD_STATE`: a late give-back of the
  previous owner does not clear the new owner's picture (the console after
  a GUI crash). The end of the process that took the
  framebuffer over gives it back by itself.

A kernel panic always draws into the framebuffer. Errors: `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`, `BAD_ADDRESS`, `NOT_FOUND`
(no framebuffer).

## Structural limits

Limits given by data formats or calls, not policy:

| Limit | Value | When exceeded | Why |
|---|---|---|---|
| kernel reserve: memory for programs (memory objects, thread stacks, message data, page tables, objects and handles) is not allocated if less free memory would remain | 1/32 of RAM, at least 16 MiB | `NO_MEMORY` | the kernel must survive memory exhaustion by a program |
| handles in a process table | 1,048,575 | `NO_RESOURCES` | a handle has 20 index bits (the rest is the slot generation) |
| size of a memory object | 1 TiB | `INVALID_ARGS` | the page list is a two-level array |
| messages queued at one channel end | 1024 | `SHOULD_WAIT` | flow control (memory is guarded by the budget) |
| data bytes queued at one channel end | 4 MiB | `SHOULD_WAIT` | flow control |
| one channel message | 64 KiB, 64 handles | `INVALID_ARGS` | copied in one call; larger data go in a memory object |
| objects in `OBJECT_WAIT_MANY` | 64 | `INVALID_ARGS` | array on the kernel stack; use a port for more |
| port registrations on one object | ~100,000 | `NO_MEMORY` | the waiter list of an object is a contiguous array (no limit across objects) |
| `VMO_READ` / `VMO_WRITE`, `RANDOM_GET` | 1 MiB, 4 KiB per call | `INVALID_ARGS` | copied through a kernel buffer; libos splits into parts |

Page tables, the heap and kernel stacks may also use the reserve, so
memory exhaustion by a program cannot bring the kernel down.

## Hardware (drivers)

These require a handle to a **Resource** with the `MANAGE` right (hardware
access). The kernel creates a single unrestricted root Resource and gives it
to `init` (startup message, kind `ROOT_RESOURCE`); init passes it to the
device manager. The device manager creates a **restricted** Resource for
each driver (`RESOURCE_CREATE`): only the memory BARs of its device and the
page of its PCI configuration space, at most 8 interrupts and 4 MiB of DMA
memory. A driver bug therefore cannot overwrite another device's registers.
With an IOMMU the device may moreover DMA only into memory created through
its resource (see [kernel.md](kernel.md#iommu-dma-protection)); without an
IOMMU it can write anywhere.

### Firmware framebuffer

Not a system call but a handle kind in init's start message
(`abi::bootstrap::kind::FRAMEBUFFER` = 7): the physical memory object of
the GOP framebuffer, rights `READ | WRITE | MAP`. It can be read and
written only through a mapping (`VMO_READ`/`VMO_WRITE` = `NOT_SUPPORTED`,
as a syscall buffer `BAD_ADDRESS`). Geometry and taking over the screen
go through [`FB_CONTROL`](#fb_control).

The object is mapped **write-combining** (a physical object with cache
type `WRITE_COMBINING`, the same as with
[`VMO_CREATE_PHYSICAL`](#vmo_create_physical); x86_64: PAT entry 1 = WC,
set by the kernel on every CPU; aarch64: Normal Non-cacheable): writes are
merged, reads are slow. The kernel does not send the handle when the
framebuffer overlaps RAM used by the allocator, does not start on a page
boundary or is smaller than its geometry (a warning in the log, the kernel
console keeps using it). Init passes it to the owner of the screen: the
`fbdisplay` driver when there is no virtio-gpu (see
[GUI](gui.md#displays-virtio-gpu-and-fbdisplay)), otherwise the rescue
console `user/console`.

### `VMO_CREATE_CONTIGUOUS`

`(resource, size) -> handle`. A physically contiguous memory object for DMA
(max 4 MiB), zero-filled. It is mapped into the process as DMA memory: on
a coherent platform (x86_64, QEMU virt) normally cached, on a non-coherent
one (e.g. Raspberry Pi 4) uncached, so the device and the driver see the
same data without manual cache maintenance. The driver must order writes
towards the device with barriers (`libos::barrier`). Only from a resource
with a device identity (`RESOURCE_CREATE` with `device`): when the driver
ends, the kernel stops the device and releases the memory only afterwards
(the DMA quarantine, see [kernel.md](kernel.md#iommu-dma-protection)).
Errors: `INVALID_ARGS` (0 or over 4 MiB), `NO_MEMORY` (no contiguous
block), `ACCESS_DENIED` (also a resource without a device identity),
`WRONG_TYPE`.

### `VMO_CREATE_PHYSICAL`

`(resource, phys, size, cache) -> handle`. A window onto a physical range
outside RAM (device registers, BAR). The range must not lie in RAM or
firmware memory (otherwise `ACCESS_DENIED`). `cache` (`abi::vm::cache`)
sets the cache type of every mapping of the object: `UNCACHED` (0,
registers), `WRITE_COMBINING` (1, x86_64 PAT WC, aarch64 Normal
Non-cacheable) or `CACHED` (2, host memory mapped by the device, e.g. the
virtio-gpu shared memory). It cannot be read through `VMO_READ`/`VMO_WRITE`
(`NOT_SUPPORTED`, reading registers has side effects) and cannot be used
as a syscall buffer (`BAD_ADDRESS`). A restricted resource
(`RESOURCE_CREATE`) may use only its ranges. The resource holder can
revoke it (`VMO_REVOKE`). Errors: `INVALID_ARGS` (unaligned address,
unknown cache type), `ACCESS_DENIED`.


**ACPI memory:** the only exception to "outside RAM" is the firmware
memory holding the ACPI tables (`AcpiReclaimable` and `AcpiNvs` regions of
the memory map, which the kernel never hands to the allocator): a range
lying **entirely** inside it always gets a `CACHED` object (the same
attributes as the kernel's direct map; `cache` is ignored) - the ACPI
service needs it (`acpid`, see [ACPI](acpi.md)). A range only partly
overlapping RAM is always `ACCESS_DENIED`.

### `VMO_REVOKE`

`(resource, vmo)`. Revokes a physical memory object: removes its mappings
in all processes (including those it was handed to) and further `VM_MAP`
calls fail with `ACCESS_DENIED`. Access through an old mapping = page
fault. A driver can thus lend a client a window into device memory and
safely take it back (the virtio-gpu render node after a blob is
destroyed). The resource must allow the object's range. Errors:
`NOT_SUPPORTED` (not a physical object), `ACCESS_DENIED`.

### `VMO_PHYS_ADDR`

`(resource, vmo, offset) -> phys`. Physical address of byte `offset` of the
memory object (for programming DMA). For a contiguous object it holds for the
whole range, for a regular one only within one page. For a regular object
it creates and pins the page (the address stays valid while the object
lives). Errors: `OUT_OF_RANGE`, `ACCESS_DENIED`, `NO_MEMORY`.

### `INTERRUPT_CREATE`

`(resource, out_msi_ptr) -> handle`. An interrupt object with a free CPU
vector. Writes the MSI message `{ address: u64, data: u32, reserved: u32 }`
to `out_msi_ptr`; the driver writes it into the device (MSI-X table). When
the device raises the interrupt, the object gets `SIGNALED`. On aarch64
(GICv3 ITS) the resource must be restricted to a device (`RESOURCE_CREATE`
with `device`), otherwise `INVALID_ARGS`; x86_64 does not need it. Errors:
`NO_RESOURCES` (out of vectors), `INVALID_ARGS`, `ACCESS_DENIED`,
`BAD_ADDRESS`.

### `RESOURCE_CREATE`

`(resource, ranges_ptr, ranges_count, irq_max, dma_max, device) -> handle`. A
narrower resource for a driver. `ranges_ptr` points to `ranges_count`
(max. 16) u64 pairs (physical start, length; both page-aligned, otherwise
`INVALID_ARGS`). The new resource may create
windows (`VMO_CREATE_PHYSICAL`) only inside these ranges, at most `irq_max`
interrupt objects and at most `dma_max` bytes of DMA memory
(`VMO_CREATE_CONTIGUOUS`, the really allocated block of 2^n pages counts;
the limits apply for the resource's whole lifetime, `NO_RESOURCES` once
exhausted, a failed attempt consumes nothing). Quotas are charged along the
whole parent chain: children together never exceed the parent's limit. If the parent is
itself restricted, the ranges must lie within its ranges and the limits must
not be higher (otherwise `ACCESS_DENIED`). Requires `MANAGE`. Errors:
`INVALID_ARGS` (more than 16 ranges, zero length, overflow),
`ACCESS_DENIED`, `BAD_ADDRESS`, `BAD_HANDLE`, `WRONG_TYPE`.

`device` = the device the resource belongs to: 0 = none, otherwise
`1 << 32 | segment << 16 | bus << 8 | device << 3 | function`
(`abi::syscall::pci_device`). With an IOMMU the device gets a domain and
memory from `VMO_CREATE_CONTIGUOUS` through this resource (and its
children) is mapped into it. A child inherits the parent's device and may
not choose another (`ACCESS_DENIED`). Without an IOMMU `device` is only
recorded.

**Ports and interrupt lines.** The top two bits of a range start give its
kind (`abi::syscall::RANGE_KIND_MASK`): 0 = physical memory (above),
`RANGE_IO_PORTS` (`1 << 62`) = I/O ports from `start & 0xFFFF`, length =
number of ports (the end at most 0x10000), `RANGE_IRQ_LINES` (`2 << 62`) =
fixed interrupt lines from `start`, length = count (lines 0 to
`abi::syscall::IRQ_LINES_MAX - 1` = 1023: x86_64 ISA IRQs, aarch64 GIC SPI
interrupt numbers 32-1019).
Ports and lines need no alignment. A child may only have its parent's ports
and lines. The root resource may use all of them. Any other kind is
`INVALID_ARGS`. In libos: `hw::io_ports(first, count)` and
`hw::irq_lines(first, count)`. Example (the PS/2 driver):

```rust
let ranges = [io_ports(0x60, 1), io_ports(0x64, 1), irq_lines(1, 1), irq_lines(12, 1)];
let ps2 = root.restrict(&ranges, 2, 0)?;
```

### `IOPORT_READ`

`(resource, port, width) -> value`. Reads an I/O port of `width` bytes
(1, 2 or 4). x86_64 only, elsewhere `NOT_SUPPORTED`. The resource must
allow the ports `port..port + width` (`RESOURCE_CREATE` with
`RANGE_IO_PORTS`), otherwise `ACCESS_DENIED`. It is a syscall rather than
an `in` instruction in userspace: slower, but every access passes the
permission check, and it is enough for slow devices (PS/2). Errors:
`INVALID_ARGS` (width, port above 0xFFFF), `ACCESS_DENIED`,
`NOT_SUPPORTED`, `BAD_HANDLE`, `WRONG_TYPE`. In libos
`Resource::port_read8`.

### `IOPORT_WRITE`

`(resource, port, width, value) -> 0`. Writes an I/O port. Conditions and
errors as for `IOPORT_READ`; also `INVALID_ARGS` when `value` does not fit
the width (e.g. 0x100 for 1 byte). In libos `Resource::port_write8`.

### `INTERRUPT_CREATE_LINE`

`(resource, line, flags) -> handle`. An interrupt object on a fixed line.
x86_64: ISA IRQ 0-15 through the IOAPIC (PS/2 keyboard 1, mouse 12, ACPI
SCI usually 9); the kernel maps the line to a GSI using the ACPI MADT
overrides (polarity, trigger mode), routes it to a free vector on the boot
CPU and ignores `flags`. aarch64: SPI 32-1019 through the GIC distributor
(on-board devices without MSI, e.g. the ACPI Generic Event Device); the
trigger mode comes from `flags` (`abi::syscall::line_flags::EDGE` = edge,
otherwise level; the value from the device's `_CRS`), and the interrupt
goes to the boot CPU. An unknown bit in `flags` = `INVALID_ARGS`. On an interrupt the object
gets `SIGNALED` as with `INTERRUPT_CREATE`, and it is acknowledged with
`INTERRUPT_ACK`. The kernel masks a **level**-triggered line after the
interrupt and unmasks it on acknowledgement (otherwise the interrupt would
repeat endlessly). An **edge**-triggered line is not masked, so after each
interrupt the driver should service everything the device has, and check
the device now and then even without an interrupt. The resource must
allow the line (`RANGE_IRQ_LINES`), and the object counts towards
`irq_max`. A line can have only one object (`ALREADY_EXISTS`). When the
last handle closes, the line is masked and released. Errors:
`INVALID_ARGS` (line above 15), `ACCESS_DENIED`, `ALREADY_EXISTS`,
`NO_RESOURCES`, `NOT_SUPPORTED` (a platform without an IOAPIC),
`NOT_FOUND` (the line is not an IOAPIC input); on aarch64 also
`INVALID_ARGS` for a number outside the SPIs the distributor has. In libos
`Resource::interrupt_line` and `Resource::interrupt_line_with` (with
`flags`). Older calls without `flags` (libos passes 0) behave as before.

### `INTERRUPT_ACK`

`(interrupt) -> 0`. Acknowledges the interrupt (clears `SIGNALED`). A
driver should acknowledge **before** servicing the device so an interrupt
arriving during servicing is not lost. Requires `WRITE`.

### `SYSTEM_POWER`

`(resource, op) -> does not return`. Powers the computer off
(`abi::syscall::power_op::OFF` = 0) or resets it (`RESET` = 1) through the
firmware. Requires a Resource with the `POWER` right: only the root
resource has it (restricted resources from `RESOURCE_CREATE` never do),
and init hands it only to the ACPI service (`acpid`) as a `BASIC | POWER`
duplicate without `MANAGE`. The kernel logs which process asked.

| Architecture | `OFF` | `RESET` |
|---|---|---|
| aarch64 | PSCI `SYSTEM_OFF` | PSCI `SYSTEM_RESET` |
| x86_64 | `NOT_SUPPORTED` - `acpid` powers off by writing the S5 sleep type into the PM1 register (only the service knows AML) | Reset Control register 0xCF9, then the keyboard controller (0x64 = 0xFE) |

It is normally called only after the services have stopped cleanly (init,
see [ACPI](acpi.md)). On success the call does not return. Errors:
`ACCESS_DENIED` (no `POWER` right), `INVALID_ARGS` (unknown operation;
checked before anything is powered off), `NOT_SUPPORTED` (no PSCI in the
firmware, or power-off on x86), `BAD_HANDLE`, `WRONG_TYPE`. In libos
`Resource::system_power`.

### `SCHED_CONTROL`

`(op, ...) -> result`. Scheduling: thread QoS class, job ceiling, Realtime
budget and CPU information. Operations (`abi::sched::op`):

| Operation | Arguments | Right | Result |
|---|---|---|---|
| `THREAD_SET` (0) | `thread, class, budget_us, period_us` | `MANAGE` on the thread (0 = calling thread) | 0 |
| `PROCESS_SET` (1) | `process, class, budget_us, period_us` | `MANAGE` on the process | 0 |
| `JOB_SET_MAX` (2) | `job, class` | `WRITE` on the job | 0 |
| `THREAD_GET` (3) | `thread, out_ptr` | `READ` (0 = self) | `QosInfo` (16 B) at `out_ptr` |
| `CPU_INFO` (4) | `cpu_index, out_ptr` | - | CPU count; `CpuInfo` (64 B) at `out_ptr` |
| `CPU_HINT` (5) | `resource, cpu_index, capacity` | Resource with `POWER` | 0 |

Class (`abi::sched::class`): `REALTIME` 0, `INTERACTIVE` 1, `NORMAL` 2,
`BACKGROUND` 3. Asking above the job ceiling (including ancestors) =
`ACCESS_DENIED`; lowering the class is always allowed. Realtime:
`budget_us` and `period_us` (0 = default 1 ms / 10 ms), period 500 µs to
1 s, budget at least 50 µs and at most half the period; all Realtime
threads together at most 50 % of CPU capacity, otherwise `NO_RESOURCES`.
A job ceiling cannot be set above the parent's (`ACCESS_DENIED`).
`CPU_HINT` with capacity 0 restores the CPUID/MIDR estimate. Other errors:
`INVALID_ARGS` (unknown operation, class, CPU index, Realtime parameters),
`BAD_HANDLE`, `WRONG_TYPE`, `INVALID_ADDRESS`. Behaviour:
[Scheduler](scheduler.md). In libos: module `sched`.

## Diagnostics

The syscalls below require a Resource with the `DIAG` right: only the root
resource has it and init hands it only to the log service (`logd`) as a
`BASIC | DIAG` duplicate without `MANAGE` (the resource cannot touch
hardware). See [Live USB](live-usb.md#diagnostics).

### `LOG_READ`

`(resource, cursor, buf_ptr, buf_len, out_start_ptr) -> bytes`. Reads the
kernel log (it also holds all process messages from `DEBUG_LOG`) as a byte
stream: `cursor` is the position in the stream (how many bytes the kernel
ever wrote). The ring buffer has 512 KiB; when older data has been
overwritten, reading starts at the oldest available byte and the real start
is written to `out_start_ptr` (u64) - so the reader sees the loss. New
the second u64 after it is the total number of lines the kernel
overwrote before the reader read them ("N lines lost"). New position =
start + returned length, 0 = nothing new. At most 64 KiB per call. Errors: `ACCESS_DENIED` (no `DIAG`), `INVALID_ARGS` (`buf_len` over
64 KiB), `INVALID_ADDRESS`, `BAD_HANDLE`, `WRONG_TYPE`. In libos
`Resource::log_read`.

### `CRASH_RECORD`

`(resource, op, buf_ptr, buf_len) -> bytes`. The kernel crash record from
the previous run (pstore, format `libs/crashrec`). The loader reserves
64 KiB of RAM at a fixed address (kind `Pstore` in the memory map); on a
panic or fatal exception the kernel writes a header and the end of the
kernel log there, and a warm reset does not erase it.

| `op` | meaning |
|---|---|
| 0 `READ` | copies a valid record (header + text) into the buffer, returns the length; 0 = none; a small buffer = `BUFFER_TOO_SMALL` |
| 1 `CLEAR` | erases the record (the log service saved it to disk) |
| 2 `TEST_PANIC` | test kernel only (`qemu-test`): causes a panic, the kernel writes the record and reboots right away; otherwise `NOT_SUPPORTED` |
| 3 `CAPACITY` | size of the area (0 = the loader did not get one) |

Errors: `ACCESS_DENIED` (no `DIAG`), `INVALID_ARGS` (unknown operation),
`INVALID_ADDRESS`, `BAD_HANDLE`, `WRONG_TYPE`. In libos
`Resource::crash_record`.

### `LOG_CONTROL`

`(resource, op, buf_ptr, buf_len) -> bytes`. Kernel log levels at run
time (shell command `loglevel`, file `/system/log/urovne.conf`).

| `op` | buffer | meaning |
|---|---|---|
| 0 `SET` | `module=level` | level for a kernel module (`sched`, `arch::x86_64::apic`; applies to submodules too) or for a process (its `DEBUG_LOG`); level `error`, `warn`, `info`, `debug`, `trace` |
| 1 `CLEAR` | `module` | removes the override (the global level applies) |
| 2 `LIST` | output | `module=level` per line and `potlaceno=N` (suppressed repeats) |

Default suppressed sections (only `info` and above): `syscall`,
`object::process`, `smp::clock_sync`. Independently of the levels, the
kernel writes at most 10 records per second from one place (file and line,
for `DEBUG_LOG` the process and the text without digits); further ones are
only counted and the first message in a new second is preceded by "...
previous message suppressed N times". At most 32 overrides, a name at most
40 B. Errors: `ACCESS_DENIED` (no `DIAG`), `INVALID_ARGS` (unknown level or
operation, long name, full table), `BUFFER_TOO_SMALL`, `INVALID_ADDRESS`.
In libos `Resource::log_control`.

## Random numbers

The kernel has a cryptographic generator (`kernel/src/random.rs`, crate
`libs/csprng`):

- **The entropy pool** mixes inputs with BLAKE2s. Each input has a credit
  = an estimate of real randomness: RDSEED/RDRAND 32 bits out of 64, data
  from the virtio-rng driver as the driver says (4 bits per byte), counter
  jitter, firmware time and interrupt timing 0.
- **The generator is ChaCha20.** Reseed: key = BLAKE2s(old key, pool), at
  the latest after 10 s or when at least 256 bits of credit are waiting.
  After every request the key is overwritten by the first block of the
  stream (fast key erasure), so a later leak of the kernel state does not
  reveal earlier output.
- **Seeded** is the generator after a reseed with at least 256 bits of
  credit in total. Before seeding the credit adds up over all reseeds (the
  entropy stays in the key), so a source that delivers in small batches
  is enough. In QEMU RDRAND is enough (`-cpu max`), on a machine
  without it the virtio-rng driver.

⚠️ Without a hardware source and without virtio-rng the generator will not
be seeded (output only from time and jitter) and the kernel logs it. Take
keys (TLS) with `REQUIRE_SEEDED`.

### `RANDOM_GET`

`(buf_ptr, len, flags) -> 0 | 1`. Fills the buffer (max `RANDOM_MAX` =
4096 B) with random bytes. Returns 1 when the generator is seeded, 0 when
not yet (the data is weak then). `flags`: bit 0 `REQUIRE_SEEDED` = an
unseeded generator returns `SHOULD_WAIT` and writes nothing. Needs no
handle. Wrappers: `libos::random::fill`, `fill_seeded`, `u64`. Errors:
`INVALID_ARGS` (length, unknown flag), `SHOULD_WAIT`, `BAD_ADDRESS`.

### `RANDOM_ADD`

`(resource, ptr, len, credit_bits) -> 0`. Mixes data (max 4096 B) into the
entropy pool with credit `credit_bits` (the kernel limits it to 8 bits per
byte). Requires a Resource with the `MANAGE` right (a device driver).
Mixing never lowers the quality of the generator, but **the credit trusts
the caller**: a faulty driver can declare the generator seeded before it
really is. Wrapper: `libos::random::add`. Used by the `virtio-rng`
driver. Errors: `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`,
`INVALID_ARGS`, `BAD_ADDRESS`.
