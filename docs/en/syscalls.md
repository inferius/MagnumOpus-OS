# Syscalls

The interface between a process and the kernel. **Programs should use
libos** ([libos.md](libos.md)) - syscall numbers and exact arguments are not
a stable interface and change together with libos. This document describes
what the kernel does so behavior can be traced.

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
repeat and must not be the end being written to. Transferred handles are
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
the kernel choose). Address, length and offset must be 4 KiB aligned, range
within `0x10000..0x8000_0000_0000`. Rights: process `WRITE`; object `MAP` +
`READ`, plus `WRITE`/`EXECUTE` per flags. A mapping keeps the object alive
after its handle is closed. Errors: `INVALID_ARGS`, `OUT_OF_RANGE`,
`BAD_STATE` (`MAP_FIXED` over an existing mapping), `NO_RESOURCES` (no free
range), `NO_MEMORY`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `VM_UNMAP`

`(process, addr, len) -> 0`. Removes mappings. The range must cover whole
mappings exactly (no partial unmap yet). On return no CPU holds a stale TLB
entry. Requires `WRITE` on the process. Errors: `NOT_FOUND`,
`INVALID_ARGS`, `BAD_HANDLE`, `ACCESS_DENIED`.

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
Errors: `BAD_STATE`, `INVALID_ARGS`, `BAD_HANDLE`, `ACCESS_DENIED`,
`WRONG_TYPE`.

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
the entry function follows the System V ABI. Requires `MANAGE`. Errors:
`BAD_STATE`.

### `THREAD_EXIT`

`()` - does not return. Ends the current thread. When the last thread ends,
the process ends (code -1,000,002 unless `PROCESS_EXIT` was called before).

### `THREAD_YIELD`

`() -> 0`. Gives the CPU to the next ready thread of the same or higher
class.
