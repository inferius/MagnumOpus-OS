# Error codes

The kernel and userspace share a single error type (`abi::Error`,
`libs/abi/src/error.rs`). Every error has:

- a **stable numeric code** (always negative) - a syscall returns it in
  `rax`, it appears in the log, and it never changes or gets reused,
- a **name** (e.g. `BAD_HANDLE`) - the same in the log, in code and here,
- a **short description** (`Error::description()`, in Czech) for messages.

A successful syscall returns a non-negative number in `rax`, a failed one
returns the negative error code. libos turns it into `Result<_, Error>`.

`cargo test -p abi` (also run by `cargo xtask test`) checks that every error
is documented in this file and in the Czech version.

| Code | Name | In short |
|---:|---|---|
| -1 | `BAD_HANDLE` | handle does not exist or was already closed |
| -2 | `ACCESS_DENIED` | handle lacks the required rights |
| -3 | `WRONG_TYPE` | object has a different type |
| -4 | `TIMED_OUT` | wait deadline expired |
| -5 | `NO_MEMORY` | out of memory |
| -6 | `NO_RESOURCES` | a limit was exhausted |
| -7 | `INVALID_ARGS` | invalid argument |
| -8 | `NOT_FOUND` | the requested thing does not exist |
| -9 | `NOT_SUPPORTED` | operation not supported |
| -10 | `CORRUPTED` | data is corrupted or invalid |
| -11 | `BAD_ADDRESS` | invalid pointer into process memory |
| -12 | `SHOULD_WAIT` | the operation would have to wait |
| -13 | `PEER_CLOSED` | the other end of the channel is closed |
| -14 | `BUFFER_TOO_SMALL` | buffer is too small |
| -15 | `OUT_OF_RANGE` | offset or range outside the object |
| -16 | `BAD_STATE` | object is in a state where the operation makes no sense |
| -17 | `BAD_SYSCALL` | unknown syscall |
| -18 | `CANCELED` | operation interrupted by termination |
| -19 | `ALREADY_EXISTS` | the entry already exists |
| -20 | `NOT_A_DIRECTORY` | a path component is not a directory |
| -21 | `IS_A_DIRECTORY` | it is a directory |
| -22 | `NOT_EMPTY` | the directory is not empty |
| -23 | `NO_SPACE` | no space left on the device |
| -24 | `IO_ERROR` | device read or write error |
| -25 | `READ_ONLY` | read-only |
| -26 | `LOCKED` | the object is open or locked by another handle |
| -27 | `CONFLICT` | the object has changed in the meantime |

## Details

### `BAD_HANDLE` (-1)

**Meaning:** the handle value does not match any open handle in the
process's table.

**When:** the handle was closed (`HANDLE_CLOSE`), transferred to another
process over a channel (transferring removes it from the table), or it is a
bogus value (0 is never a valid handle). Every table slot has a generation,
so an old handle never silently points to a different object after the slot
is reused - it returns this error instead.

**What to do:** almost always a program bug (use after close, double close).
Check who closes or transfers the handle.

### `ACCESS_DENIED` (-2)

**Meaning:** the handle exists but lacks a right the operation requires.

**When:** e.g. `OBJECT_SIGNAL` without `SIGNAL`, `VM_MAP` of a memory object
without `MAP`, `CHANNEL_WRITE` with a handle lacking `TRANSFER`, or a
`HANDLE_DUPLICATE` that would add rights (rights can only be removed). The
rights each syscall needs are listed in [syscalls.md](syscalls.md).

**What to do:** this is the security model working, not a kernel bug. The
process got a restricted handle on purpose; if it needs more, whoever issued
the handle has to grant it.

### `WRONG_TYPE` (-3)

**Meaning:** the object behind the handle has a different type than the
operation expects.

**When:** e.g. `CHANNEL_READ` with an event handle, `VM_MAP` with a handle
that is neither a process nor a memory object.

**What to do:** program bug - handles mixed up.

### `TIMED_OUT` (-4)

**Meaning:** the wait was not satisfied before the deadline.

**When:** `OBJECT_WAIT_ONE` and `OBJECT_WAIT_MANY` with a deadline other
than `DEADLINE_INFINITE`. `OBJECT_WAIT_MANY` with zero objects is a sleep
until the deadline and always ends with this error (libos `time::sleep`
ignores it).

**What to do:** a normal condition, not a failure. The kernel does not log it.

### `NO_MEMORY` (-5)

**Meaning:** the kernel ran out of physical memory for the operation.

**When:** creating a memory object (`VMO_CREATE` allocates pages up front),
page tables during `VM_MAP`, a kernel stack for a new thread, a new process.

**What to do:** free memory (close unneeded objects, end processes).
Repeated occurrences mean a leak or too little RAM.

### `NO_RESOURCES` (-6)

**Meaning:** a limit other than memory was exhausted.

**When:** the process handle table is full (max 1,048,575 handles), a
channel queue is full (`CHANNEL_MAX_QUEUED` = 1024 messages; writes then
return `SHOULD_WAIT`), no free virtual address range for `VM_MAP` without
`MAP_FIXED`.

**What to do:** close unneeded handles, drain channel messages, map smaller
ranges.

### `INVALID_ARGS` (-7)

**Meaning:** an argument is out of range or the combination is forbidden.

**When:** e.g. `VM_MAP` with both write and execute (W^X), without
`MAP_READ`, with an unaligned address or length; `OBJECT_SIGNAL` with any
bit other than `SIGNALED`; a process name over 64 bytes or not UTF-8;
sending a channel end through itself; the same handle twice in one message;
`VMO_CREATE` with size 0 or above 4 GiB.

**What to do:** program bug - fix the arguments per [syscalls.md](syscalls.md).

### `NOT_FOUND` (-8)

**Meaning:** the requested thing does not exist.

**When:** `VM_UNMAP` of a range that is not made up exactly of whole
mappings (partial unmap is not supported yet); in the kernel, a missing ACPI
MADT table or a missing `init` program in initfs.

**What to do:** for `VM_UNMAP`, unmap exactly the address and length that
`VM_MAP` returned.

### `NOT_SUPPORTED` (-9)

**Meaning:** the operation is valid but this version of the system cannot do it.

**When:** no syscall returns it yet; it is reserved for extensions
(unsupported hardware, an option the kernel does not know).

**What to do:** use a different approach or wait for support.

### `CORRUPTED` (-10)

**Meaning:** data is corrupted or not in the expected format.

**When:** an invalid ACPI table (bad checksum or length), a corrupted
initfs, `init` is not a valid ELF; in libos an invalid ELF passed to `spawn`.

**What to do:** for firmware tables the kernel skips the table and continues
(see [messages.md](messages.md)); for files, fix or rebuild the image.

### `BAD_ADDRESS` (-11)

**Meaning:** a pointer the process passed to the kernel does not point into
valid process memory with the required rights.

**When:** a syscall buffer lies (even partly) in unmapped memory, outside
the lower half of the address space, or it is a write into a read-only
mapping (e.g. the program's code). The kernel validates the whole range up
front, so nothing is done halfway.

**What to do:** program bug - invalid pointer or buffer length.

### `SHOULD_WAIT` (-12)

**Meaning:** the operation would have to wait now, so the non-blocking call
did not complete it.

**When:** `CHANNEL_READ` from an empty queue, `CHANNEL_WRITE` to a full
queue.

**What to do:** wait for a signal (`READABLE` for reads) and retry. libos
`Channel::read` does this for you. The kernel does not log this error.

### `PEER_CLOSED` (-13)

**Meaning:** the other end of the channel no longer exists.

**When:** `CHANNEL_WRITE` after the other end closed its last handle;
`CHANNEL_READ` when the queue is empty and the other end is closed
(messages already queued can still be read). The channel end also gets the
`PEER_CLOSED` signal.

**What to do:** the normal end of a conversation - the other side is gone.
`echo`, for example, exits with code 0 on it.

### `BUFFER_TOO_SMALL` (-14)

**Meaning:** the result does not fit into the buffer.

**When:** `CHANNEL_READ` when the message is longer than the data buffer or
carries more handles than there is room for. The message stays queued and
the kernel writes the required sizes to `out_actual_ptr`.

**What to do:** grow the buffers to the returned sizes and read again (libos
does this automatically).

### `OUT_OF_RANGE` (-15)

**Meaning:** an offset or range lies (even partly) outside the object.

**When:** `VMO_READ` / `VMO_WRITE` past the end of a memory object,
`VM_MAP` where `vmo_offset + len` exceeds the object size.

**What to do:** program bug - check the object size.

### `BAD_STATE` (-16)

**Meaning:** the object is in a state where the operation makes no sense.

**When:** `PROCESS_EXIT_CODE` on a process that is still running;
`PROCESS_START` on an already started process; `THREAD_START` on a started
thread or in a process that has not been started yet; `VM_MAP` with
`MAP_FIXED` over an existing mapping; creating a thread in a terminating
process.

**What to do:** follow the order of operations (described per syscall); for
the exit code, wait for the `TERMINATED` signal first.

### `BAD_SYSCALL` (-17)

**Meaning:** the kernel does not know the syscall number.

**When:** a program calls the kernel directly with an invalid number, or was
built for a different interface version. libos also returns it when the
kernel returns an unknown negative code.

**What to do:** use libos (syscall numbers are not a stable interface).

### `CANCELED` (-18)

**Meaning:** the operation was interrupted because the thread or process is
terminating.

**When:** a thread is waiting (e.g. `OBJECT_WAIT_ONE`) and its process gets
terminated (`PROCESS_KILL`, `PROCESS_EXIT` from another thread, another
thread crashing). The wait ends with this error and the thread exits on
return to userspace, so the program usually never sees it.

**What to do:** nothing, the process is ending.

### `ALREADY_EXISTS` (-19)

**Meaning:** an entry with the given name already exists.

**When:** creating a file or directory, renaming to a name that already
exists in the target directory, creating a snapshot with an existing name.
Names are compared case-insensitively after Unicode normalization, so
`Foto.JPG` collides with `foto.jpg`.

**What to do:** choose another name or remove the existing entry first.

### `NOT_A_DIRECTORY` (-20)

**Meaning:** a component of the path is not a directory.

**When:** path `/a/b/c` where `a` or `b` is a file; listing a file as a
directory.

**What to do:** fix the path.

### `IS_A_DIRECTORY` (-21)

**Meaning:** a file operation got a directory.

**When:** reading or writing data of a directory.

**What to do:** fix the path (a directory is read by listing it).

### `NOT_EMPTY` (-22)

**Meaning:** the directory is not empty.

**When:** removing a directory that has entries (removal is deliberately
not recursive).

**What to do:** remove the contents first.

### `NO_SPACE` (-23)

**Meaning:** the device ran out of space.

**When:** writing data or changing metadata with no free blocks. Because of
copy-on-write even overwriting existing data needs free blocks; blocks owned
by snapshots are never freed.

**What to do:** remove unneeded data, or snapshots (once supported).

### `IO_ERROR` (-24)

**Meaning:** the device reported a read or write error.

**When:** a driver or media error; in tools, an error on the image file.

**What to do:** check the device; data not yet committed may be lost, the
stored state stays consistent.

### `READ_ONLY` (-25)

**Meaning:** an attempt to write to a read-only object.

**When:** writing to a snapshot.

**What to do:** write to the live file system.

### `LOCKED` (-26)

**Meaning:** the object is open in an incompatible share mode, or a file
range is locked by another handle.

**When:** opening a file that another handle opened without sharing the
requested access (`FILE_SHARE_*` in the Windows layer); deleting or
renaming an open file without delete sharing; writing into a range with a
mandatory lock; a range lock overlapping someone else's.

**What to do:** wait until the other handle closes the file or releases the
lock, or open with a compatible mode.

### `CONFLICT` (-27)

**Meaning:** a conditional operation was not performed because the object
has changed since the given revision (or a target that was not supposed to
exist exists).

**When:** a conditional save (`rename_replace_if`, `REPLACE_COMMIT` in
protocol v2) when someone else changed the file in the meantime - another
editor, an agent, sync.

**What to do:** nothing changed and the new data stayed with the client;
offer saving alongside, comparing versions, or a deliberate overwrite (the
operation with the new revision).

## Process exit codes

A process ends with the code passed to `PROCESS_EXIT` (0 = success, other
values are defined by the program). When the kernel terminates a process it
uses a code from a reserved range (`libs/abi/src/process.rs`):

| Code | Meaning |
|---:|---|
| 0 | success |
| -1,000,000 | killed by another process (`PROCESS_KILL`) |
| -1,000,001 | the program panicked (libos panic handler) |
| -1,000,002 | the last thread ended without `PROCESS_EXIT` |
| -1,000,100 - vector | CPU exception in the program (crash); e.g. page fault (vector 14) = -1,000,114, divide error (0) = -1,000,100, invalid opcode (6) = -1,000,106, general protection (13) = -1,000,113 |

On a crash the kernel logs the vector, error code, instruction address and,
for page faults, the accessed address (see [messages.md](messages.md)).
