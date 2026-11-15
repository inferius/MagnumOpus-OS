# Exceptions and thread context

How a program, its parent or a debugger handles a CPU exception (page
fault, invalid opcode, breakpoint, divide by zero ...) and how a thread's
context is read and changed. This is step 5 in the order of adopting
third-party software ([architecture.md](architecture.md#security)): the
basis for SEH in the Windows layer, signals (`SIGSEGV`, `SIGFPE`) in
`cstd`, faults in JIT and GC and a future debugger. Syscalls:
[syscalls.md](syscalls.md).

## Status

| Part | What | Status |
|---|---|---|
| 1 | suspending a thread (`THREAD_SUSPEND`), reading and writing its context (`THREAD_STATE`) | **[verified in QEMU]** x86_64 and aarch64 (suite `procesy`) |
| 2 | exception channels (`EXCEPTION_CHANNEL`, `EXCEPTION_RESOLVE`), listing threads (`PROCESS_THREADS`), handler thread | **[verified in QEMU]** x86_64 and aarch64 |
| 3 | delivery right in the thread (`EXCEPTION_HANDLER`), alternate stack, nesting limit | **[verified in QEMU]** x86_64 and aarch64 |
| 4 | typed libos API: `Frame` (registers, PC, SP, FPU, return), `alloc_alt_stack`, `signal_class` | **[verified in QEMU]** x86_64 and aarch64 |
| - | single-step and hardware breakpoints, delivering a Windows `PAGE_GUARD` exception | **[designed]** |

## Handler order

<figure>
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Exception handler order">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.warn{fill:#fbefe6;stroke:#a0602a;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}.warn{fill:#33261c;stroke:#d39a63}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="ex1" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="5" y="20" width="110" height="56" rx="6"/><text class="t b" x="60" y="44" text-anchor="middle">exception</text><text class="m" x="60" y="62" text-anchor="middle">kernel did not resolve</text>
<rect class="acc" x="135" y="20" width="110" height="56" rx="6"/><text class="t b" x="190" y="44" text-anchor="middle">debugger</text><text class="m" x="190" y="62" text-anchor="middle">first chance</text>
<rect class="acc" x="265" y="20" width="110" height="56" rx="6"/><text class="t b" x="320" y="44" text-anchor="middle">in thread</text><text class="m" x="320" y="62" text-anchor="middle">frame on the stack</text>
<rect class="acc" x="395" y="20" width="110" height="56" rx="6"/><text class="t b" x="450" y="44" text-anchor="middle">thread channel</text>
<rect class="acc" x="525" y="20" width="110" height="56" rx="6"/><text class="t b" x="580" y="44" text-anchor="middle">process channel</text>
<rect class="acc" x="525" y="120" width="110" height="56" rx="6"/><text class="t b" x="580" y="144" text-anchor="middle">debugger</text><text class="m" x="580" y="162" text-anchor="middle">second chance</text>
<rect class="warn" x="655" y="120" width="100" height="56" rx="6"/><text class="t b" x="705" y="144" text-anchor="middle">process</text><text class="m" x="705" y="162" text-anchor="middle">ends</text>
<line class="ln" x1="115" y1="48" x2="131" y2="48" marker-end="url(#ex1)"/>
<line class="ln" x1="245" y1="48" x2="261" y2="48" marker-end="url(#ex1)"/>
<line class="ln" x1="375" y1="48" x2="391" y2="48" marker-end="url(#ex1)"/>
<line class="ln" x1="505" y1="48" x2="521" y2="48" marker-end="url(#ex1)"/>
<line class="ln" x1="580" y1="76" x2="580" y2="116" marker-end="url(#ex1)"/>
<line class="ln" x1="635" y1="148" x2="651" y2="148" marker-end="url(#ex1)"/>
<text class="m" x="5" y="200">Each handler: RESUME (continue) / NEXT (next in order) / KILL. A handler thread skips the thread and process.</text>
</svg>
<figcaption>An exception goes to handlers in order until one resolves it.</figcaption>
</figure>

1. **The kernel itself** tries to resolve a page fault (demand paging,
   copy on write - [memory-management.md](memory-management.md)).
2. The process's **debugger** (a channel with `DEBUGGER`), first chance.
3. **The in-thread handler** (`EXCEPTION_HANDLER`): the fast path without a
   context switch.
4. **The thread channel**, then **the process channel**
   (`EXCEPTION_CHANNEL`).
5. **The debugger**, second chance.
6. Nobody: the process ends with code `EXIT_FAULT_BASE - kind` (address,
   registers and a backtrace in the log).

**A handler thread** (`HANDLER_THREAD`) skips steps 3 and 4: the thread
reading the channel would otherwise wait for itself.

## Exception channels

A channel is a regular `Channel`; the kernel holds the other end. The
message `abi::exception::Report` (kind, stage, access kind, process,
thread, PC, address, architecture code) carries handles of the thread and
of a token. The thread stands meanwhile and the handler reads and changes
its context (`THREAD_STATE`) and decides (`EXCEPTION_RESOLVE`): `RESUME`,
`NEXT`, `KILL`. Closing the token without a decision = `NEXT`, so a dying
handler blocks nothing.

```rust
let ch = child.exception_channel(false)?;
let e = libos::exception::receive(&ch, None)?;
if e.report.kind == abi::exception::kind::PAGE_FAULT {
    // map the page, adjust the context ...
    e.resume()?;
}
```

## Delivery in the thread

The handler (`libos::exception::set_handler`) gets the frame
`abi::exception::frame` (aligned to 64 B): `Report`, general registers and
FPU/SIMD at the moment of the exception. The FP part has the header
`abi::thread::fp::Header` (as in `THREAD_STATE`) and the data after it; its
length depends on the CPU (on x86_64 an XSAVE image with AVX/AVX-512), so
**the frame has a variable size** (`libos::exception::frame_size()`,
`abi::exception::frame::size`). Typed access: `Frame::fp_header()`,
`fp()`, `fp_mut()`; the smallest alternate stack
`libos::exception::min_alt_stack()`. It lies on the **alternate
stack** (`set_alt_stack`, like `sigaltstack`) if set and the thread is not
on it yet, otherwise below SP (on x86_64 below the 128 B red zone). The
handler does not return, it ends with `handler_return(frame, RESUME |
NEXT)`.

- **Return validation:** the context from the frame goes through the same
  checks as `THREAD_STATE` (PC and SP in userspace, only user flags, FPU
  without reserved bits). A forged frame thus can do only what the thread
  may do itself.
- **NEXT** takes the exception kind and address from the kernel's copy,
  not from the frame.
- **Stack overflow:** the frame is written in a checked way; if it does not
  fit (a guard region), the handler is skipped and the exception goes on -
  up to the end of the process, never into recursion. With an alternate
  stack the handler is called.
- **Nesting:** an exception inside the handler is delivered again up to a
  depth of 8 (`handler::MAX_DEPTH`); the next one goes straight to the
  channels.
- **The handler entry** must lie in executable memory, i.e. sealed or with
  the JIT permission (W^X, [memory-management.md](memory-management.md)).

### API for signals and SEH

`libos::exception::Frame` is typed access to the frame (the basis for
`ucontext_t` in `cstd` and `CONTEXT` in the Windows layer): `report()`,
`general()` / `set_general()` (`X64General` / `A64General`), `pc()`,
`sp()`, `fp()` and returning with `resume()` / `next()`.
`alloc_alt_stack(size)` allocates and sets the thread's alternate stack.

| Kind (`abi::exception::kind`) | `signal_class` | POSIX | SEH (Wine) |
|---|---|---|---|
| `PAGE_FAULT`, `GENERAL` | `Segv` | `SIGSEGV` | `EXCEPTION_ACCESS_VIOLATION` (`access`: read / write / execute) |
| `ALIGNMENT` | `Bus` | `SIGBUS` | `EXCEPTION_DATATYPE_MISALIGNMENT` |
| `PAGE_FAULT` with `reason` = `BEYOND_END` (past the end of a resizable memory object, [memory management](memory-management.md#resizable-memory-object); `signal_class_of`) | `Bus` | `SIGBUS` | `EXCEPTION_IN_PAGE_ERROR` |
| `DIVIDE` | `Fpe` | `SIGFPE` | `EXCEPTION_INT_DIVIDE_BY_ZERO` |
| `INVALID_OPCODE` | `Ill` | `SIGILL` | `EXCEPTION_ILLEGAL_INSTRUCTION` |
| `BREAKPOINT` | `Trap` | `SIGTRAP` | `EXCEPTION_BREAKPOINT` |

Signal numbers are chosen by `cstd`, `EXCEPTION_*` codes by the Windows
layer.

```rust
extern "C" fn on_fault(f: *mut u8) -> ! {
    let mut fr = unsafe { libos::exception::Frame::from_raw(f) };
    if libos::exception::signal_class(fr.report().kind) == SignalClass::Trap {
        // aarch64: the PC is at brk
        if cfg!(target_arch = "aarch64") { let pc = fr.pc(); fr.set_pc(pc + 4); }
        let _ = fr.resume();
    }
    let _ = fr.next();
    libos::sys::thread_exit()
}
```

## Suspending and context

- `THREAD_SUSPEND` returns a token; closing it resumes the thread. The
  thread stands at the boundary of returning to userspace; one sleeping in
  a syscall is suspended at once.
- `THREAD_STATE` reads and writes general registers
  (`abi::thread::X64General`, `A64General`) and FPU/SIMD (header and an
  XSAVE/FXSAVE image, V0-V31 + FPCR + FPSR) of a suspended thread or a thread in an exception. The
  thread applies a write itself before returning.
- `PROCESS_THREADS` gives a debugger handles of all threads.

## Security

- **Who may:** read the context `READ`, write it `WRITE`, suspend and
  channels `MANAGE`. **`WRITE` on a process = full control** (just like
  `VM_MAP`): a parent is naturally its child's debugger; nobody else gets
  such a handle.
- **The kernel cannot be crashed** by a context: non-canonical or kernel
  PC and SP, IOPL and mode changes, reserved bits of MXCSR/FPCR/FPSR are
  rejected or masked (tests in the `procesy` suite).
- **Kill always wins** over suspension and waiting in an exception.

## Architecture differences

- **aarch64 does not report division by zero** (`udiv`/`sdiv` return 0).
  `cstd` (`SIGFPE`) and Wine (`EXCEPTION_INT_DIVIDE_BY_ZERO`) must provide
  it by checking in code.
- **Breakpoint:** on x86_64 the PC is already past `int3`, on aarch64 at
  `brk` - the handler advances it by 4.
- **TEB through GS** (Windows layer) is a separate task; the context has
  only the FS / `TPIDR_EL0` base for now.

## Key files

| File | What |
|---|---|
| `kernel/src/sched/suspend.rs` | suspending, parking, pending context writes, thread state for exceptions |
| `kernel/src/object/exception.rs` | handler order, channels, token, in-thread delivery, `RETURN` |
| `kernel/src/arch/*/ustate.rs` | register frame and FPU, write validation, preparing the jump into a handler |
| `libs/abi/src/thread.rs`, `exception.rs` | context, message and frame formats |
| `libs/libos/src/exception.rs` | API for handlers |

## Tests

`cargo xtask test --only procesy` (x86_64 and aarch64):
`user/init/src/tests/thread_state.rs`, `exceptions.rs`,
`exceptions_fast.rs` - suspending and redirecting PC, invalid context,
killing a suspended thread, channels and order, breakpoint, handler thread,
in-thread handler (fixing a fault, forged frame, NEXT, overflow with and
without an alternate stack, nesting limit).
