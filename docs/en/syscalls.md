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
`INVALID_ARGS` (text too long).

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
| 12 | u16 | source: 1 UEFI, 2 CMOS RTC, 3 set at run time |
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
| 0 | i64 | new time: seconds since 1970-01-01 00:00:00 UTC |
| 8 | u32 | nanoseconds (0..10⁹, else `INVALID_ARGS`) |
| 12 | i32 | `rtc_offset`: seconds added to UTC when writing the hardware clock (RTC in local time = zone offset, else 0) |

`flags`: bit 0 `WRITE_HARDWARE` = also write the time to the hardware clock
(x86: CMOS RTC). Other bits = `INVALID_ARGS`. Returns 1 when the hardware
clock was written. Returns 0 without `WRITE_HARDWARE`, when there is no
clock (aarch64, ACPI reports no RTC) or when the year does not fit its
range. The system time is set in every case. Afterwards `CLOCK_REALTIME`
reports source 3 (set at run time) and no flags. Wait deadlines do not
change; they come from `CLOCK_MONOTONIC`. Wrapper: `libos::time::set`,
`Resource::set_clock`. Errors: `ACCESS_DENIED` (no `SET_CLOCK`),
`WRONG_TYPE`, `BAD_HANDLE`, `INVALID_ARGS`, `BAD_ADDRESS`. Details:
[Setting the clock](time.md#setting-the-clock-clock_set).

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
then sets `set`. A process may only change `SIGNALED`; other signals are
kernel-controlled. Requires `SIGNAL`. Errors: `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE` (not an event), `INVALID_ARGS` (bit other
than `SIGNALED`).

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
  A port holds at most `PORT_MAX_PENDING` (65,536) registrations and
  pending packets.
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

`() -> handle`. A new empty port with rights `DUPLICATE`, `TRANSFER`,
`WAIT`, `READ`, `WRITE`. Errors: `NO_RESOURCES`.

### `OBJECT_WAIT_ASYNC`

`(handle, port, key, mask) -> 0`. A one-shot registration: when the
signals of the object `handle` intersect `mask`, the port gets a packet
with `key` (u64, meaning chosen by the program). If the condition already
holds, the packet arrives at once. Requires `WAIT` on the object and
`WRITE` on the port. Registering on a port itself is not possible.
Errors: `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE` (`port` is not a
port), `NOT_SUPPORTED` (the object is a port), `INVALID_ARGS` (empty
mask), `NO_RESOURCES` (the port holds `PORT_MAX_PENDING` registrations and
packets), `NO_MEMORY`.

### `PORT_WAIT`

`(port, deadline, out_ptr) -> 0`. Removes the oldest packet and writes it
to `out_ptr` (16 B, `abi::object::PortPacket`): key u64, the object's
signals when the condition held u32, reserved u32. An empty port waits
until the deadline (`deadline` = 0: just try). Requires `READ`. Errors:
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `TIMED_OUT`, `CANCELED`,
`BAD_ADDRESS` (the packet is then lost).

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

`(size) -> handle`. A memory object of `size` bytes (rounded up to 4 KiB,
max 4 GiB), zero-filled. Pages are allocated up front for now. Rights:
`DUPLICATE`, `TRANSFER`, `WAIT`, `READ`, `WRITE`, `MAP`, `EXECUTE`. Errors:
`INVALID_ARGS` (0 or too large), `NO_MEMORY`, `NO_RESOURCES`.

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
`BAD_STATE` (`MAP_FIXED` over an existing mapping), `NO_RESOURCES` (no free
range), `NO_MEMORY`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `VM_UNMAP`

`(process, addr, len) -> 0`. Removes mappings. The range must cover whole
mappings exactly (no partial unmap yet). On return no CPU holds a stale TLB
entry. Requires `WRITE` on the process. Errors: `NOT_FOUND`,
`INVALID_ARGS`, `BAD_HANDLE`, `ACCESS_DENIED`.

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

`(name_ptr, name_len) -> handle`. An empty process with no memory or
threads. Name: UTF-8, max 64 B (for logs). Rights: `DUPLICATE`, `TRANSFER`,
`WAIT`, `MANAGE`, `WRITE`, `READ`. Errors: `INVALID_ARGS`, `NO_MEMORY`,
`BAD_ADDRESS`.

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
its threads end (waiting ones with `CANCELED`), handles are closed, the
process gets the `TERMINATED` signal.

### `PROCESS_KILL`

`(process) -> 0`. Terminates another process with code -1,000,000. Threads
running on other CPUs are interrupted. Requires `MANAGE`.

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

### `THREAD_YIELD`

`() -> 0`. Gives the CPU to the next ready thread of the same or higher
class.

## Resource limits

A program must not consume kernel memory without bounds (`abi::limits`):

| Limit | Value | When exceeded |
|---|---|---|
| kernel reserve: memory of memory objects (`VMO_CREATE*`), thread stacks, message data in queues, page tables of mappings, new processes and objects (every new handle) is not allocated if less free memory would remain | 1/32 of RAM, at least 16 MiB | `NO_MEMORY` |
| handles in a process table | 65,536 | `NO_RESOURCES` |
| threads of a process (including finished ones somebody holds a handle to) | 1024 | `NO_RESOURCES` |
| mappings in a process address space | 65,536 | `NO_RESOURCES` |
| messages queued at one channel end | 1024 | `SHOULD_WAIT` |
| data bytes queued at one channel end | 4 MiB | `SHOULD_WAIT` |

Memory exhaustion by a program therefore cannot bring the kernel down: page
tables, the heap and kernel stacks may also use the reserve. Planned:
quotas for groups of processes (job) set by the parent.

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

### `VMO_CREATE_CONTIGUOUS`

`(resource, size) -> handle`. A physically contiguous memory object for DMA
(max 4 MiB), zero-filled. It is mapped into the process as DMA memory: on
a coherent platform (x86_64, QEMU virt) normally cached, on a non-coherent
one (e.g. Raspberry Pi 4) uncached, so the device and the driver see the
same data without manual cache maintenance. The driver must order writes
towards the device with barriers (`libos::barrier`). Errors: `INVALID_ARGS`
(0 or over 4 MiB), `NO_MEMORY` (no contiguous block), `ACCESS_DENIED`,
`WRONG_TYPE`.

### `VMO_CREATE_PHYSICAL`

`(resource, phys, size) -> handle`. A window onto device registers (MMIO).
The range must not lie in RAM or firmware memory (otherwise
`ACCESS_DENIED`). It is mapped uncached; it cannot be read through
`VMO_READ`/`VMO_WRITE` (`NOT_SUPPORTED`, reading registers has side
effects) and cannot be used as a syscall buffer (`BAD_ADDRESS`). A
restricted resource (`RESOURCE_CREATE`) may use only its ranges. Errors:
`INVALID_ARGS` (unaligned address), `ACCESS_DENIED`.

### `VMO_PHYS_ADDR`

`(resource, vmo, offset) -> phys`. Physical address of byte `offset` of the
memory object (for programming DMA). For a contiguous object it holds for the
whole range, for a regular one only within one page. Errors:
`OUT_OF_RANGE`, `ACCESS_DENIED`.

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

### `INTERRUPT_ACK`

`(interrupt) -> 0`. Acknowledges the interrupt (clears `SIGNALED`). A
driver should acknowledge **before** servicing the device so an interrupt
arriving during servicing is not lost. Requires `WRITE`.

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
  credit in total. In QEMU RDRAND is enough (`-cpu max`), on a machine
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
