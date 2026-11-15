# The C library (cstd)

MagnumOpus's own C library, written in Rust with a C ABI. It has two
parts: **`cstd-core`** (the base without system services, for `no_std`
services such as ACPICA in `acpid`) and **`cstd`** (a POSIX layer over
[Rust std](rust-std.md) for applications). A C program therefore runs on
the same runtime as a Rust program: one allocator, one TLS, one stdio.
Compiling C (clang, flags, protections) is described in
[Building C code](c-build.md).

> **Status:** part 1 **[verified in QEMU]** on x86_64 and aarch64 (test
> `ctest: stdio, soubory, errno, getenv, atexit`, suite `cstd`): `errno`,
> `FILE` (stdin/stdout/stderr, files), `exit`/`atexit`, `getenv`.
> Part 2a **[verified in QEMU]** on x86_64 and aarch64 (step `posix` of
> the same test): fd numbers, `unistd.h`, `fcntl.h`, `sys/stat.h`,
> `dirent.h`, `FILE` over fd numbers. Part 2b **[verified in QEMU]** (step
> `threads`): `pthread`. Part 2c **[verified in QEMU]** (step `time`):
> time, calendar, `setenv`. Part 2d **[verified in QEMU]** (step `math`):
> `math.h`, floating point in `printf`, `strtod`. Part 2e **[verified in
> QEMU]** (suite `cstd sit`): BSD sockets, `getaddrinfo`, `poll`, `select`,
> `epoll`. Part 2f **[verified in QEMU]** (step `mmap`): `mmap`, `mprotect`,
> `madvise`, `mseal`. Part 2g **[verified in QEMU]** (step `signal`):
> signals, `setjmp`. File locks **[verified in QEMU]** (test `ctest lock`:
> two processes, `EDEADLK`). Part 2i **[verified in QEMU]** (suite
> `cstd porty`): `libc.a`, the C sysroot and the zlib and sqlite ports.
> `libc.so` **[verified in QEMU]** (suite `cstd dyn`: a PIE program in C,
> threads, TLS, `dlopen` of a plugin). Wide characters and locale
> **[verified in QEMU]** (step `wchar`, chapter
> [Wide characters and locale](#wide-characters-and-locale)). POSIX
> additions (`posix_spawn`, `waitpid`, semaphores, `termios`, `getifaddrs`, `statfs`
> ...) **[verified in QEMU]** (step `posix2`, chapter
> [POSIX additions](#posix-additions-for-ports-and-the-windows-layer)). Further parts
> (signals from other processes, `iconv`) **[designed]**, see
> [Continuation](#continuation-a-complete-libc-and-posix-parts-2-8).

## Layers

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Layers of the C library">
<defs><marker id="cs1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="10" width="370" height="50" rx="6"/><text class="t b" x="195" y="32" text-anchor="middle">C application (+ Rust)</text><text class="m" x="195" y="50" text-anchor="middle">user/ctest, later zlib, FFmpeg (process)</text>
<rect class="box" x="400" y="10" width="370" height="50" rx="6"/><text class="t b" x="585" y="32" text-anchor="middle">no_std service with C code</text><text class="m" x="585" y="50" text-anchor="middle">acpid (ACPICA)</text>
<path class="ln" d="M195 60 L195 88" marker-end="url(#cs1)"/>
<rect class="acc" x="10" y="90" width="370" height="60" rx="6"/><text class="t b" x="195" y="113" text-anchor="middle">libs/cstd (POSIX over std)</text><text class="m" x="195" y="131" text-anchor="middle">errno, FILE, exit/atexit, getenv</text>
<path class="ln" d="M195 150 L195 178" marker-end="url(#cs1)"/>
<path class="ln" d="M380 120 L560 178" marker-end="url(#cs1)"/>
<path class="ln" d="M585 60 L585 178" marker-end="url(#cs1)"/>
<rect class="box" x="10" y="180" width="370" height="50" rx="6"/><text class="t b" x="195" y="202" text-anchor="middle">std + osrt</text><text class="m" x="195" y="220" text-anchor="middle">files, stdio, at_exit, TLS</text>
<rect class="acc" x="400" y="180" width="370" height="50" rx="6"/><text class="t b" x="585" y="202" text-anchor="middle">libs/cstd-core (no_std)</text><text class="m" x="585" y="220" text-anchor="middle">string, ctype, malloc, snprintf, SSP</text>
<path class="ln" d="M195 230 L195 258" marker-end="url(#cs1)"/>
<rect class="box" x="10" y="260" width="760" height="34" rx="6"/><text class="t b" x="390" y="282" text-anchor="middle">libos + services (fsd, ...)</text>
</svg>
<figcaption>cstd uses std (one runtime with Rust) and shares the cstd-core base with no_std services.</figcaption>
</figure>

- **`libs/cstd-core`** **[implemented]**: strings, `ctype`, `malloc` (the
  program heap), `strtol` and friends, `snprintf`, stack protection. The
  `service-io` feature (default) adds `printf`/`puts`/`abort` writing to
  the service log; applications with std turn it off (`cstd` provides
  stdio).
- **`libs/cstd`** **[verified in QEMU]**: POSIX headers and functions over
  std. It exports the C symbols only on the `*-unknown-magnumopus` target;
  on the host the logic is tested (`cargo test -p cstd`), the symbols would
  clash with the system libc.
- **Functions without a length parameter** (`sprintf`, `vsprintf`,
  `strtok`) are available for ports **with a safeguard where one is
  possible**: the C header maps `sprintf` and `vsprintf` to
  `__sprintf_chk` / `__vsprintf_chk` with `__builtin_object_size` (like
  `_FORTIFY_SOURCE`), so an overflow of a buffer of known size = `abort`
  with a message instead of overwriting memory; `strtok` keeps its state per
  thread. There is no `gets` (no safeguard is possible). `no_std` services
  (`cstd-core`) do not have them; new code should use `snprintf` and
  `strtok_r`.

## A C program

A program is a `#![no_main]` crate that links `cstd` and compiles the C
sources in `build.rs`. `osrt` calls the C `main(argc, argv)` at start-up
and passes its return value to `exit`.

```rust
// src/main.rs
#![no_main]
extern crate cstd;
```

```rust
// build.rs: cstd passes the headers (links = "cstd") in DEP_CSTD_INCLUDE
fn main() {
    let dirs = std::env::var("DEP_CSTD_INCLUDE").unwrap();
    let mut b = cbuild::Build::new("app").trust(cbuild::Trust::Own).file("c/app.c");
    for d in dirs.split(';') {
        b = b.include(d);
    }
    b.compile();
}
```

The program is built like other std applications (`STD_PROGRAMS` in
`xtask/src/stdsys.rs`). For the `*-unknown-magnumopus` targets `cbuild`
compiles hard-float (SSE2 / NEON like the Rust target) with the
`local-exec` TLS model (`_Thread_local` and `errno` share TLS blocks with
Rust).

The `cstd` headers (`stdio.h`, `stdlib.h`, `string.h`) extend the base
`cstd-core` headers via `#include_next`; `errno.h` exists only in `cstd`.

## Functions (part 1)

| Header | Functions | Note |
|---|---|---|
| `errno.h` | `errno` (`__errno_location`), `E*` constants | per thread (TLS); numbers as in Linux/POSIX (`ENOENT` = 2, ...) |
| `stdio.h` | `stdin`, `stdout`, `stderr`, `fopen`, `fclose`, `fflush` (also `NULL`), `fread`, `fwrite`, `fgetc`, `getc`, `getchar`, `ungetc`, `fgets`, `fputc`, `putc`, `putchar`, `fputs`, `puts`, `printf`, `vprintf`, `fprintf`, `vfprintf`, `fseek`, `ftell`, `rewind`, `feof`, `ferror`, `clearerr`, `setvbuf`, `perror`, `remove`, `rename` | `snprintf` and the formatting engine come from `cstd-core` |
| `stdlib.h` | `exit`, `_Exit`, `atexit`, `getenv`, `abort` | `EXIT_SUCCESS`, `EXIT_FAILURE`; `atexit` shares its list with `__cxa_atexit` ([Functions for C++](#functions-for-c-step-7a)) |
| `string.h` | `strerror` | the rest is in `cstd-core` |
| `assert.h` | `assert` | a failure prints file and line to stderr and ends the process |

## Functions (part 2a: fd numbers and files)

| Header | Functions | Note |
|---|---|---|
| `unistd.h` | `close`, `read`, `write`, `pread`, `pwrite`, `lseek`, `dup`, `dup2`, `dup3`, `pipe`, `pipe2`, `isatty`, `fsync`, `fdatasync`, `ftruncate`, `truncate`, `unlink`, `rmdir`, `getcwd`, `chdir`, `access`, `sysconf`, `getpagesize` | `fork`, `vfork`, `exec*` return `ENOSYS` (see [No fork](#processes-locale-wchar)) |
| `fcntl.h` | `open`, `openat` (also relative to a directory, see [Functions for C++](#functions-for-c-step-7a)), `creat`, `fcntl` (`F_DUPFD`, `F_DUPFD_CLOEXEC`, `F_GETFD`, `F_SETFD`, `F_GETFL`, `F_SETFL`) | `O_*` values as Linux, `O_DIRECTORY` the same on both architectures; locks `F_GETLK`, `F_SETLK`, `F_SETLKW` see File locks |
| `sys/stat.h` | `stat`, `lstat` (= `stat`), `fstat`, `mkdir`, `chmod`, `fchmod`, `umask` | `st_mode` = a view of the FS grants ([POSIX mode](file-system.md#posix-mode)); `mkdir`/`open(O_CREAT)` with a mode after `umask` only narrow the grants, `chmod`/`fchmod`/`fchmodat` change them (right `MANAGE`, otherwise `EPERM`) |
| `sys/uio.h` | `readv`, `writev` | `writev` in one write |
| `dirent.h` | `opendir`, `fdopendir`, `readdir`, `closedir`, `rewinddir`, `dirfd` | the first two entries are `.` and `..` |
| `stdio.h` | `fdopen`, `fileno`, `fseeko`, `ftello`, `getline`, `getdelim` | `FILE` sits on an fd number |

## Functions (part 2b: threads)

| Header | Functions | Note |
|---|---|---|
| `pthread.h` | `pthread_create`, `join`, `detach`, `self`, `equal`, `exit`, `cancel`, `testcancel`, `setcancelstate`, `setcanceltype`; attributes (`stacksize`, `detachstate`) | `osrt` threads (TLS like std); default stack 2 MiB, at least 16 KiB |
| `pthread.h` | `pthread_mutex_*` (normal, recursive, error-checking, `timedlock`), `pthread_cond_*` (`CLOCK_REALTIME` and `CLOCK_MONOTONIC`), `pthread_rwlock_*`, `pthread_barrier_*`, `pthread_spin_*`, `pthread_once` | over the futex; types with their own fixed layout, static initialisers = zeros |
| `pthread.h` | `pthread_key_*`, `pthread_getspecific`, `pthread_setspecific`, `pthread_cleanup_push`/`pop` | 1024 keys, destructors at most 4 rounds |
| `sched.h` | `sched_yield` | |

- The target has `panic=abort`, so `pthread_exit` and cancellation end the
  thread with a syscall (`osrt::thread::exit_current`) without unwinding:
  cleanup handlers (`pthread_cleanup_push`) and key destructors run, Rust
  `thread_local!` destructors of that thread do not.
- Cancellation is deferred only (decision 5): cancellation points are
  `pthread_testcancel`, `pthread_join` and waiting on a condition (in
  `CANCEL_SLICE` slices, a hundred milliseconds; cancellation may be that late).
- `pthread_exit` in the main thread ends the process with code 0 (it does
  not wait for the other threads).
- No `PTHREAD_PROCESS_SHARED` (the futex is private to the process).

## Functions (part 2c: time)

| Header | Functions | Note |
|---|---|---|
| `time.h` | `clock_gettime`, `clock_getres` (`REALTIME`, `MONOTONIC`, `BOOTTIME`, `*_COARSE`, `MONOTONIC_RAW`), `nanosleep`, `clock_nanosleep` (also `TIMER_ABSTIME`), `time`, `clock`, `difftime` | the kernel does not provide CPU time (`*_CPUTIME_ID`, `clock`): `EINVAL`, `clock()` = -1 |
| `time.h` | `gmtime_r`, `localtime_r`, `gmtime`, `localtime`, `mktime`, `timegm`, `strftime`, `tzset` | `struct tm` with `tm_gmtoff` and `tm_zone` (like glibc); `strftime` in the C locale incl. ISO week (`%G`, `%V`) |
| `sys/time.h` | `gettimeofday` | |
| `unistd.h` | `sleep`, `usleep` | |
| `stdlib.h` | `setenv`, `unsetenv` | |

- **Local time:** the zone comes from the `TZ` variable - an IANA name
  (`Europe/Prague`) or a fixed offset in ISO style (`UTC+2`), otherwise
  UTC. Zone rules are in `libs/tz` (no tzdata). POSIX strings such as
  `CET-1CEST` are not read. Passing the system zone (time settings) into
  `TZ` when a program starts **[designed]**.
- Sleeping is not interrupted (no signals, no `EINTR`), `rem` is always 0;
  `nanosleep`, `sleep` and `usleep` are thread cancellation points.

## Functions (part 2d: maths and floating point)

| Header | Functions | Note |
|---|---|---|
| `math.h` | trigonometric, hyperbolic, `exp*`, `log*`, `pow`, `sqrt`, `cbrt`, `hypot`, rounding (`ceil`, `floor`, `trunc`, `round`, `rint`, `lround`, ...), `fmod`, `remainder`, `remquo`, `frexp`, `ldexp`, `modf`, `fma`, `erf`, `tgamma`, `lgamma`, Bessel functions; `*f` and `*l` variants; macros `isnan`, `fpclassify`, `M_PI` ... | the `libm` crate (a Rust port of musl libm, MIT/Apache); the functions set neither `errno` nor FPU exceptions (`math_errhandling` = 0) |
| `stdio.h` | `%f %F %e %E %g %G %a %A` in the `printf` family | digits from `core::fmt` (exact rounding of the binary value), output like glibc |
| `stdlib.h` | `strtod`, `strtof`, `strtold`, `atof` | decimal and hexadecimal forms, `inf`, `nan(...)`; `ERANGE` on overflow and underflow |

- **`long double`** has the Linux ABI (x86_64 80 bits, aarch64 128 bits),
  but the `*l` functions and `strtold` compute in `double` precision
  (decision 9). They are written in C (`libs/cstd/c/math_ld.c`, compiled by
  `build.rs` only for the magnumopus target) because Rust has no
  `long double` type.
- `%Lf` (`long double` in `printf`) cannot be read from a `VaList` yet: it
  prints `?` and does not consume the argument.
- `scanf` **[designed]**.

## Functions (part 2e: sockets and waiting)

| Header | Functions | Note |
|---|---|---|
| `sys/socket.h` | `socket` (`AF_INET`, `AF_INET6`; `SOCK_STREAM`, `SOCK_DGRAM`, `SOCK_NONBLOCK`, `SOCK_CLOEXEC`), `bind`, `listen`, `accept`, `accept4`, `connect`, `send`, `recv`, `sendto`, `recvfrom`, `sendmsg`, `recvmsg` (control messages only `SCM_RIGHTS` on `AF_UNIX`), `socketpair`, `shutdown`, `getsockname`, `getpeername`, `setsockopt`, `getsockopt` | `AF_UNIX` only `SOCK_STREAM` (see [AF_UNIX sockets](#af_unix-sockets)); `MSG_PEEK` = `ENOTSUP` |
| `netinet/in.h`, `arpa/inet.h` | `htons` ..., `inet_pton`, `inet_ntop`, `inet_addr`, `inet_ntoa`, `in6addr_any` | |
| `netdb.h` | `getaddrinfo`, `freeaddrinfo`, `gai_strerror`, `getnameinfo` | names via the netd resolver; `getnameinfo` numeric only |
| `poll.h`, `sys/select.h`, `sys/epoll.h` | `poll`, `select`, `epoll_create`, `epoll_create1`, `epoll_ctl`, `epoll_wait` | over `osrt::poll` |

- A socket is an object in the fd table (`osrt::sock`): after `socket()` it
  has no connection at netd yet; `connect`/`listen`/`bind` turn it into a
  TCP connection, a listener or a UDP socket. `read`, `write`, `fdopen` and
  `close` also work on a socket.
- What a program may do (targets, ports, listening) is decided by the
  network session from the manifest: a forbidden target or port = `EACCES`.
  `SO_REUSEADDR`, `SO_KEEPALIVE`, `TCP_NODELAY` and buffer sizes are
  accepted without effect, `SO_RCVTIMEO`/`SO_SNDTIMEO` apply.
- `O_NONBLOCK` (`SOCK_NONBLOCK`, `MSG_DONTWAIT`) = deadline `TRY` at netd,
  `EAGAIN` without data. `connect` completes immediately even in
  non-blocking mode (POSIX allows it), so `SO_ERROR` is always 0.
- **Waiting** (`osrt::poll`): files are always ready, stdin and pipes wait
  for a channel signal (`READABLE`, `PEER_CLOSED`), sockets for the netd
  readiness event (netproto v2). A channel has no "writable" signal, so
  stdout and the write end of a pipe always report `POLLOUT`. Up to 64
  objects `OBJECT_WAIT_MANY`, more via a port.
- **`epoll`** is a set in the fd table; level-triggered mode exactly,
  `EPOLLET` reports only newly set bits, `EPOLLONESHOT` disables the entry
  until `EPOLL_CTL_MOD`. An mio backend directly over readiness
  **[designed]**.

## Functions (part 2f: memory mapping)

| Header | Functions | Note |
|---|---|---|
| `sys/mman.h` | `mmap`, `munmap` (also a part), `mprotect`, `madvise`, `posix_madvise`, `mseal`, `msync`, `mlock`, `munlock` | over `libos::vm` (via `osrt::vm`) |
| `unistd.h` | `sysconf(_SC_PAGESIZE)`, `getpagesize` | page size from the kernel (not a constant) |

- **Anonymous memory** has on-demand pages (first write). `PROT_NONE` =
  an address reservation without memory; `mprotect` with rights fills it in
  (the GC and JIT pattern: reserve a large range, use it in parts).
  `MAP_FIXED` replaces whatever was in the range; `MAP_FIXED_NOREPLACE` only
  free space.
- **Anonymous `MAP_SHARED`** is the same as private within the process (no
  `fork`).
- **File:** the content is read into anonymous memory (`MAP_PRIVATE`, or
  read-only `MAP_SHARED` - later changes to the file are not visible).
  `MAP_SHARED` with writing = `ENOTSUP` until fsd provides a file memory
  object **[designed]**. A pipe or socket = `ENODEV`.
- **W^X:** `PROT_EXEC` on unsealed memory only with the JIT permission from
  the manifest ([memory-management.md](memory-management.md)), otherwise
  `EACCES`.
- `madvise(MADV_DONTNEED | MADV_FREE)` returns the memory and the next read
  returns zeros; other advice is accepted without effect. `mseal` seals the
  range with its current rights (then it cannot be unmapped or have its
  rights changed).

## Functions (part 2g: signals and non-local jumps)

| Header | Functions | Note |
|---|---|---|
| `signal.h` | `signal`, `sigaction` (`SA_SIGINFO`, `SA_RESETHAND`, `SA_NODEFER`), `raise`, `kill`, `sigprocmask`, `pthread_sigmask`, `sigpending`, `sigsuspend`, `sig*set`, `sigaltstack`, `strsignal`, `psignal` | numbers as Linux |
| `unistd.h`, `sys/time.h` | `alarm`, `pause`, `setitimer` (`ITIMER_REAL`), `getpid`, `getppid` | |
| `setjmp.h` | `setjmp`, `longjmp`, `sigsetjmp`, `siglongjmp`, `_setjmp`, `_longjmp` | `jmp_buf` 320 B on both architectures; naked functions in assembly |

Decision 6A - signals without kernel support:

- **Synchronous** (`SIGSEGV`, `SIGBUS`, `SIGFPE`, `SIGILL`, `SIGTRAP`): the
  first `sigaction` with a handler registers the in-thread exception
  handler ([exceptions.md](exceptions.md)); it calls the signal handler in
  the same thread with `si_addr` from the exception. Returning from the
  handler = continue (the instruction is retried - the GC and JIT pattern:
  `mprotect` in the handler), `SIG_DFL` = the next handler and the end of
  the process. Leaving with `siglongjmp` works. `ucontext_t` (the third
  argument, `sys/ucontext.h`) **[verified in QEMU]** x86_64 and aarch64
  (`ctest`, step `signal`): registers from the exception frame in the
  Linux layout (x86_64 `uc_mcontext.gregs[REG_*]`, `fpregs` pointing to
  the FXSAVE image, `REG_CR2` = the fault address; aarch64 `regs`, `sp`,
  `pc`, `pstate`, `fault_address`, the `fpsimd_context` record in
  `__reserved`). Changes of registers and FPU in the handler are applied
  on return (fixing a register and retrying the instruction - Wine, JIT).
  A signal without an exception (`raise`) gets an empty context. On aarch64
  division by zero gives no `SIGFPE` (the CPU does not report it).
- **`raise`, `kill` of the own process:** the handler runs immediately in
  the calling thread; a blocked signal waits and is delivered once
  unblocked.
- **Timers** (`alarm`, `setitimer`): the `SIGALRM` handler runs in a
  dedicated signal thread (like the Windows console Ctrl+C handler); there
  is no `EINTR`, `pause` and `sigsuspend` wake up.
- **Default action:** the process ends with code 128 + signal number
  (`SIGABRT` = `abort`); `SIGCHLD`, `SIGCONT`, `SIGWINCH` and stop signals
  are ignored.
- **Not available:** signals from other processes (terminal, parent; a
  `SIGNAL` event in the parameter block **[designed]**), `SIGPIPE` (writing
  to a closed pipe only returns `EPIPE`).
- **`getpid`:** the kernel does not give programs a process number;
  `getpid` returns a random positive number, stable for the run (temporary
  file names, `kill` of itself) - the same as `std::process::id`
  (`osrt::process::self_id`), so other processes see it the same (`l_pid`,
  `SO_PEERCRED`). `getppid` = 1.

### File locks

`fcntl(F_GETLK/F_SETLK/F_SETLKW)`, `flock` (`sys/file.h`) and `lockf`
**[verified in QEMU]** (suite `cstd`, test `ctest lock`: two processes over
one file, conflict, `F_GETLK`, deadlock, release at process exit). fsd holds
the locks (message `POSIX_LOCK`, [file-system.md](file-system.md)): the
`fcntl` owner is the process (FS session), the `flock` owner is the open
file. They are released by closing the process's last fd on the file
(`flock`: closing the fd) and by the end of the process. `F_SETLKW` and
`flock` without `LOCK_NB` wait by retrying; fsd watches for a wait cycle
(`EDEADLK`). `l_pid` from `F_GETLK` = the holder's `getpid` (the client
sends it with every lock, fsd does not verify it; unknown = -1)
**[verified in QEMU]** (test `ctest lock`).

## Functions for C++ (step 7a)

**[verified in QEMU]** x86_64 and aarch64 (suite `cpp`, [C++ runtime](cpp.md)); host
tests `cxxsup_tests`, `fsat_tests`.

| Header | Functions | Notes |
|---|---|---|
| `stdlib.h` | `lldiv`, `realpath` | aligned memory (`aligned_alloc`, `posix_memalign`, `memalign`) is in cstd-core: plain `free` releases the block |
| `unistd.h`, `sys/random.h` | `getentropy` (at most 256 B), `unlinkat`, `pathconf`, `fpathconf` | `getentropy` waits until the kernel generator is seeded (like Linux) |
| `time.h` | `timespec_get` (`TIME_UTC`, `TIME_MONOTONIC`) | |
| `fcntl.h`, `sys/stat.h` | `openat`, `mkdirat`, `fstatat`, `fchmodat` relative to a directory; `AT_REMOVEDIR`, `AT_SYMLINK_NOFOLLOW` | the directory path remembered at open + the name (the FS works with paths: no protection against the directory being renamed between open and the call) |
| `sys/statvfs.h` | `statvfs`, `fstatvfs` | size and free blocks of the fsd session volume; inode counts 0 |
| `sys/sendfile.h` | `sendfile` | copy through a 64 KiB buffer in the process |
| `stdio.h` | `setbuf` | like `setvbuf` (`NULL` = unbuffered); `std::filebuf` |
| `strings.h`, `string.h` | `ffs`, `ffsl`, `ffsll` | index of the lowest set bit from 1 |
| `signal.h` | `si_code` values (`SEGV_*`, `BUS_*`, `FPE_*`, `ILL_*`, `TRAP_*`) | values as Linux |
| `magnumopus/thread.h` (extension, not POSIX) | `mos_thread_set_second_pointer`, `mos_thread_get_second_pointer` | the second thread pointer (x86_64 GS base, the TEB of the [Windows layer](windows-layer.md)); aarch64 -1 and `ENOTSUP` |
| `limits.h` | `PATH_MAX` (4096), `NAME_MAX` (255) | |
| `link.h`, `elf.h` | `dl_iterate_phdr`, `struct dl_phdr_info`, ELF64 types | static program: one module from linker script symbols (`PT_LOAD`, `PT_GNU_EH_FRAME`) |
| (C++ ABI) | `__cxa_atexit`, `__cxa_finalize`, `__cxa_thread_atexit_impl`, `__dso_handle` | `atexit` and `__cxa_atexit` share one list (one reverse order) |

The C sysroot compiles with `-funwind-tables` (C++ exceptions pass through
C frames) and links with `--eh-frame-hdr`; the dynamic part has
`lib/crtdso.o` with a hidden `__dso_handle` for every shared library.

## C sysroot and ports (part 2i)

**[verified in QEMU]** x86_64 and aarch64 (suite `cstd porty`).

<figure class="diagram">
<svg viewBox="0 0 780 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Building a C port against the sysroot">
<defs><marker id="cs3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="220" height="60" rx="6"/><text class="t b" x="120" y="34" text-anchor="middle">port sources</text><text class="m" x="120" y="52" text-anchor="middle">HTTPS + SHA-256, tar</text>
<rect class="acc" x="280" y="10" width="220" height="60" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">clang + C sysroot</text><text class="m" x="390" y="52" text-anchor="middle">include, clang.cfg</text>
<rect class="acc" x="550" y="10" width="220" height="60" rx="6"/><text class="t b" x="660" y="34" text-anchor="middle">ld.lld</text><text class="m" x="660" y="52" text-anchor="middle">libc.a, magnumopus.ld</text>
<path class="ln" d="M230 40 L276 40" marker-end="url(#cs3)"/>
<path class="ln" d="M500 40 L546 40" marker-end="url(#cs3)"/>
<path class="ln" d="M660 70 L660 98" marker-end="url(#cs3)"/>
<rect class="ok" x="400" y="100" width="370" height="60" rx="6"/><text class="t b" x="585" y="124" text-anchor="middle">static ET_EXEC program in initfs</text><text class="m" x="585" y="142" text-anchor="middle">zlib-example, minigzip, sqlite3, sqltest</text>
<rect class="box" x="10" y="100" width="370" height="60" rx="6"/><text class="t b" x="195" y="124" text-anchor="middle">libc.a = cstd + std + osrt</text><text class="m" x="195" y="142" text-anchor="middle">crate libs/cstd-libc (staticlib), _start from osrt</text>
</svg>
<figcaption>A C port is compiled by clang against the sysroot and linked with libc.a; _start from osrt calls its main.</figcaption>
</figure>

- **`libc.a`** (crate `libs/cstd-libc`, `staticlib`): `cstd`, `std` and
  `osrt` including `_start`, which calls the C program's `main`. Built with
  the std applications (`stdsys::STD_LIBS`).
- **C sysroot** `target/c-sysroot/<arch>/` (`xtask/src/csysroot.rs`):
  `include/` (cstd) and `include-core/` (cstd-core), `lib/libc.a`,
  `lib/magnumopus.ld` (the linker script of std programs) and `clang.cfg`
  with compile and link flags for `clang --config=...` - foreign build
  systems (`configure`, CMake, Meson) use it as a cross build
  **[implemented]** (the ports below use the same flags).
- **Ports** (`xtask/src/cports/`, the mechanism is described in [C ports](c-ports.md)): sources from the official places over
  HTTPS (`curl`), verified SHA-256 (zlib matches the release hash; sqlite
  pinned from the first download from sqlite.org), unpacked with `tar`,
  compiled by clang with `-w` and linked with `libc.a`. Objects are
  recompiled only on change.
  - **zlib 1.3.1** (zlib licence): `zlib-example` (the package's own test)
    and `minigzip`; test: every `example` step, compressing a file and back
    with identical content.
  - **sqlite 3.46.1** (public domain): the `sqlite3` shell and `sqltest`
    (`user/ctest/c/sqltest.c`); the default unix VFS with `fcntl` locks in
    fsd. No WAL and no mmap I/O (`SQLITE_OMIT_WAL`,
    `SQLITE_MAX_MMAP_SIZE=0`: shared file memory does not exist yet), no
    `load_extension` (static program). Test: the shell over a file and
    persistence, two processes over one database - a write during a
    foreign exclusive transaction = `SQLITE_BUSY`, with `busy_timeout` it
    succeeds after the other process's `COMMIT`.
- Functions added for the ports: `strspn`, `strcspn`, `strpbrk`,
  `strtok_r`, `strdup`, `strndup`, `strcasecmp`, `strerror_r`, `qsort`,
  `bsearch`, `rand`, `div`, `atol`, the `scanf` family, `getpwuid` (user
  from `USER` and `HOME`), `getuid` and friends (the FS session identity, see [AF_UNIX sockets](#af_unix-sockets)), `fchown` and `utimes`
  without effect, `readlink`/`symlink`/`link` (no FS symlinks yet),
  `ioctl` (`ENOTTY`), `setlocale`/`localeconv` (see [Wide characters and locale](#wide-characters-and-locale)), `uname`,
  `getrusage`/`getrlimit`, `system` (`ENOSYS`; starting processes via
  `posix_spawn`, see [POSIX additions](#processes-posix_spawn-and-waitpid)).
- **`libc.so`**: see the [libc.so](#libcso) chapter below.

## libc.so

**[verified in QEMU]** x86_64 and aarch64 (suite `cstd dyn`).

- **A second std target** `*-unknown-magnumopus-dyn`
  (`third_party/rust-std/targets/*-dyn.json`): `relocation-model: pic`,
  `tls-model: initial-exec`, `dynamic-linking`, `env: dyn`. Its own sysroot
  next to the static one (`stdsys::build_sysroot_dyn_locked`).
- **`osrt` under the loader** (`cfg(target_env = "dyn")`): no `_start` and
  no own TLS block (`ld.so` builds the TLS blocks of all modules including
  `libc.so`); the start is `__libc_start(main, bootstrap, sp, magic)`, called
  by the program's `_start` from `crt1.o`; after the `atexit` functions
  `exit` runs the module finalisers (`run_fini`).
- **`libc.so`** (crate `libs/cstd-so`, `cdylib`, `SONAME libc.so`): cstd +
  std + osrt, `BIND_NOW`, full RELRO, no TEXTREL, 64 KiB segments;
  initial-exec TLS (the library is in the static TLS from start-up). No
  undefined symbols; `dlopen` and friends go through the loader table
  (`dlfcn.h`).
- **C sysroot for dynamic linking:** `lib/libc.so`, `lib/crt1.o`
  (`libs/cstd/crt/crt1.c`) and `clang-dyn.cfg` (`-fPIE`, `-pie`,
  `PT_INTERP /system/lib/ld.so`, `-Wl,-Bdynamic -l:libc.so` - the clang
  baremetal driver would otherwise link statically). The static
  `clang.cfg` stays.
- **Test** (suite `cstd dyn`): `cdyn` (PIE via cbuild) - start via `crt1.o`,
  stdio, maths, 4 threads with their own TLS and `errno`,
  `dlopen("libcplug.so")` with per-thread plugin TLS (dynamic TLS via the
  DTV), `malloc` in the plugin and `free` in the program (one heap), files,
  `atexit`; `cdyn-hello` built only via `clang --config=clang-dyn.cfg` (the
  path for foreign build systems).
- Rust programs stay static; C processes are either static (`libc.a`) or
  dynamic (`libc.so`), always with one runtime.

std build time (clean build, x86_64, host with 32 threads; the second
sysroot is built only when std or osrt changes):

| What | Time | Revision |
|---|---|---|
| static std, state before `libc.so` | 20 s | `eade6bf` |
| static std | 19 s | `fcbc153` |
| std as PIC (second target) | 17 s | `fcbc153` |
| `libc.so` on top of the PIC sysroot | 3 s | `fcbc153` |

### fd numbers

- The table is in `osrt` (`osrt::fd`), one per process; `cstd` sees it via
  `std::os::magnumopus::runtime` (a re-export of the `osrt` crate in std,
  unstable feature `magnumopus_runtime`, only for libraries over std).
- A number points to an open description: an FS session file with a
  position, the process stdio (0, 1, 2 from start-up) or a pipe end. `dup`
  shares the description and the position.
- At most 4096 numbers (`EMFILE`). `FD_CLOEXEC` is stored (for
  `posix_spawn`: the child inherits numbers without it).
- A pipe is a channel pair; the read end cannot be written (`EBADF`), a
  pipe has no position (`ESPIPE`).
- On the host (`cargo test -p cstd`) the same rules over `std::fs`
  (`sys/host.rs`), so the `FILE` and fd logic can be tested without QEMU.
- `std` does not see fd numbers yet (`AsRawFd`/`FromRawFd` for Rust `File`
  **[designed]**).

### Streams and buffers

- `stdout` line-buffered, `stderr` unbuffered, files 4 KiB (`BUFSIZ`).
  `setvbuf` changes the mode; a caller-supplied buffer is not used.
- Reading `stdin` flushes `stdout` first (a prompt without a newline).
- Every `FILE` has its own lock; threads may share a stream.
- At process exit (`exit`, return from `main`) all streams are flushed:
  `cstd` registers the flush via `std::os::magnumopus::rt::at_exit` (an
  `osrt` function); `atexit` uses the same list. Functions run in reverse
  registration order; the flush is registered first, so it runs after all
  `atexit` functions (they may still write).
- `_Exit` ends without `atexit` and without flushing (`rt::exit_now`).

### Errors

A system error (`abi::Error`, [errors.md](errors.md)) is mapped to `errno`
(`errno::from_os`: `NOT_FOUND` -> `ENOENT`, `ACCESS_DENIED` -> `EACCES`,
`BAD_HANDLE` -> `EBADF`, `PEER_CLOSED` -> `EPIPE`, ...; unknown `EIO`), a
`std::io::Error` by its kind (`errno::from_io`). An invalid `fopen` mode =
`EINVAL`.

## Limitations of part 1

- Parts 2a-2d above added fd numbers, threads, time, `math.h` and
  floating point; sockets and more come with the continuation below.

## Continuation: a complete libc and POSIX (parts 2-8)

**[designed]**, approved 2026-11-14 (step 2 in the order of adopting
foreign software, [architecture.md](architecture.md#security)); part 2a is
done (above). Goal: build zlib, sqlite, then the SDL and FFmpeg
dependencies (LGPL, separate process) without source changes. The
decisions are in [Decisions](#decisions).

### Two kinds of process, one runtime

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Static and dynamic process with the C library">
<defs><marker id="cs2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="t b" x="195" y="20" text-anchor="middle">static (ET_EXEC)</text>
<text class="t b" x="585" y="20" text-anchor="middle">dynamic (PIE + ld.so)</text>
<rect class="ok" x="10" y="30" width="370" height="44" rx="6"/><text class="t b" x="195" y="50" text-anchor="middle">Rust program + static C</text><text class="m" x="195" y="66" text-anchor="middle">player with a decoder, ctest, zlib in Rust</text>
<rect class="ok" x="400" y="30" width="370" height="44" rx="6"/><text class="t b" x="585" y="50" text-anchor="middle">C/C++ program + .so libraries</text><text class="m" x="585" y="66" text-anchor="middle">FFmpeg (process), sqlite3, ports from configure/cmake</text>
<path class="ln" d="M195 74 L195 98" marker-end="url(#cs2)"/>
<path class="ln" d="M585 74 L585 98" marker-end="url(#cs2)"/>
<rect class="acc" x="10" y="100" width="370" height="60" rx="6"/><text class="t b" x="195" y="122" text-anchor="middle">libc.a = cstd + std + osrt</text><text class="m" x="195" y="140" text-anchor="middle">linked into the program</text>
<rect class="acc" x="400" y="100" width="370" height="60" rx="6"/><text class="t b" x="585" y="122" text-anchor="middle">libc.so = cstd + std + osrt</text><text class="m" x="585" y="140" text-anchor="middle">one copy per process, code shared via libd</text>
<path class="ln" d="M195 160 L195 188" marker-end="url(#cs2)"/>
<path class="ln" d="M585 160 L585 188" marker-end="url(#cs2)"/>
<rect class="box" x="10" y="190" width="760" height="50" rx="6"/><text class="t b" x="390" y="212" text-anchor="middle">libos + services (fsd, netd, launchd, libd)</text><text class="m" x="390" y="230" text-anchor="middle">fd table, threads, heap, TLS and at_exit live in osrt: C and Rust see the same</text>
</svg>
<figcaption>Every process has exactly one runtime (osrt): statically in the program, or as libc.so under the loader.</figcaption>
</figure>

- **Static process** (today): a Rust program with std, C linked statically
  (`libc.a`). Rust stays static ([dynamic-linking.md](dynamic-linking.md)).
- **Dynamic process**: a C/C++ program as PIE, `libc.so` from `/system/lib`.
  `libc.so` contains `cstd`, `std` and `osrt` (one `cdylib`), so there too
  there is one heap, one fd table and one TLS.
- ⚠️ **Limitation:** a process must not have two runtimes. A Rust plugin
  with std (`cdylib`) loaded via `dlopen` into a C process would bring a
  second std (second heap, second `at_exit`). Rust plugins for C processes
  will be `no_std` over the C ABI, or std is solved later with Rust std as a
  `.so`.

### Own code versus adopted code

The system layer is **our own, in Rust** (the binding to libos and the
services cannot be taken from musl): stdio, fd, threads, time, sockets,
`mmap`, signals, `posix_spawn`. Purely algorithmic parts are **adopted**
(MIT/BSD, no GPL/LGPL): mathematics, number conversions, `qsort`, regular
expressions, `getopt`.

| Area | Solution | Estimate |
|---|---|---|
| stdio, `unistd`, `fcntl`, `stat`, `dirent`, `pipe`, `dup` | own (osrt + cstd) | ~3,500 lines of Rust |
| `pthread` (threads, mutex, cond, rwlock, barrier, spin, once, keys) | own over `osrt::thread` and the futex | ~1,500 |
| time (`clock_gettime`, `nanosleep`, `localtime_r`, `strftime`) | own over `libos::time` and `libs/tz` | ~900 |
| BSD sockets, `getaddrinfo`, `poll`, `select`, `epoll` | own over netproto v2 and readiness | ~2,000 |
| `mmap`, `mprotect`, `munmap`, `madvise` | own over `libos::vm` | ~400 |
| signals (`sigaction`, `raise`, `kill`, `sigprocmask`) | own over `libos::exception` | ~800 |
| `math.h` | adopted (question 3) | ~10,000 adopted |
| `printf` with floating point, `strtod`, `scanf` | own over `core` (Rust's exact conversions) | ~800 |
| `qsort`, `bsearch`, `getopt`, `regex.h` (TRE), `fnmatch`, `glob` | adopted from musl (MIT) | ~4,000 adopted |
| `wchar.h`, `wctype.h`, `locale.h`, `iconv` (UTF-8/16/32, Latin-1 only) | own | ~900 |

About 11,000 lines of own Rust, about 14,000 adopted lines. All of musl is
~90,000 lines; adopting all of it would mean rewriting its system layer
(Linux syscalls) and having two runtimes next to Rust.

### Parts

1. **pthread.** Threads via `osrt::thread` (under the loader `ld.so` makes
   the TLS blocks, [DL-3](dynamic-linking.md)), so `_Thread_local`, `errno`
   and Rust `thread_local!` work in threads created from C. Mutex (normal,
   recursive, error-checking), `cond` (also `CLOCK_MONOTONIC` via
   `pthread_condattr_setclock`), rwlock, barrier, spin, once and keys
   (1024, destructors at thread exit) over the futex. Stack size from the
   attribute, guard area below the stack (`map_stack`).
   `PTHREAD_PROCESS_SHARED` no (the futex is private to the process).
   Thread cancellation: question 5.
2. **Files and fd.** fd table in `osrt` (`osrt::fd`): number -> open
   description (fsproto file with position, stdio stream, pipe end, socket,
   directory). `dup` shares the description and position as in POSIX.
   `std` gets an fd only on request (`AsRawFd`, `FromRawFd`, `IntoRawFd` in
   `std::os::magnumopus::io`); an ordinary Rust `File` takes no number.
   `open` (`O_CREAT`, `O_EXCL`, `O_TRUNC`, `O_APPEND`, `O_NONBLOCK`,
   `O_CLOEXEC`, `O_DIRECTORY`), `read`/`write`/`pread`/`pwrite`/`readv`/
   `writev`, `lseek`, `ftruncate`, `fsync`, `stat`/`fstat`/`lstat`
   (`st_mode` as a view of the grants, `st_uid` = the owner), `opendir`/`readdir`, `mkdir`, `unlink`, `rmdir`, `rename`,
   `access`, `getcwd`/`chdir`, `pipe` (a channel pair), `isatty`.
   `chmod` changes the grants (since T1, [POSIX mode](file-system.md#posix-mode)), `link`/
   `symlink` = `ENOTSUP` until FS v3. `fcntl`/`flock` locks are held by
   fsd (decision 8).
3. **`math.h`** (question 3) and floating point in `printf`/`scanf`/
   `strtod`. Hard-float everywhere (x86-64-v2, NEON). `long double`:
   question 9.
4. **Time and signals.** `clock_gettime` (`REALTIME`, `MONOTONIC`,
   `BOOTTIME`; `PROCESS_CPUTIME_ID`/`THREAD_CPUTIME_ID` only if the kernel
   provides CPU time, otherwise `EINVAL`), `nanosleep`, `clock_nanosleep`,
   `time`, `gettimeofday`, `localtime_r`/`mktime`/`strftime` with the
   system zone (`libs/tz`). Synchronous signals (`SIGSEGV`, `SIGBUS`,
   `SIGFPE`, `SIGILL`, `SIGTRAP`) over the in-thread exception handler
   ([exceptions.md](exceptions.md)): `sigaction` with `SA_SIGINFO`,
   `ucontext_t` from `Frame`, `sigaltstack`. Asynchronous signals
   (`SIGINT`, `SIGTERM`, `SIGCHLD`, `SIGALRM`, `SIGPIPE`): question 6. On
   aarch64 division by zero gives no `SIGFPE` (the CPU does not report it).
5. **Sockets and waiting.** `socket` (`AF_INET`, `AF_INET6`;
   `SOCK_STREAM`, `SOCK_DGRAM`), `bind`, `listen`, `accept`, `connect`,
   `send`/`recv`/`sendto`/`recvfrom`/`sendmsg`/`recvmsg` (no control
   messages), `shutdown`, `getsockname`/`getpeername`, `setsockopt`
   (subset: `SO_REUSEADDR`, `TCP_NODELAY`, `SO_RCVTIMEO`, `SO_SNDTIMEO`,
   `SO_KEEPALIVE`), `getaddrinfo`/`getnameinfo` via the netd resolver,
   `O_NONBLOCK` (deadline `TRY`). `poll` and `select` via
   `OBJECT_WAIT_MANY` (up to 64 objects), beyond that via a port
   (`libos::port`): files are always ready, channels have `READABLE`/
   `PEER_CLOSED`, sockets readiness events (netproto v2). `AF_UNIX` later
   (step T1, see [AF_UNIX sockets](#af_unix-sockets)). `epoll`: question 7.
6. **Memory.** Anonymous `mmap` (`MAP_PRIVATE`/`MAP_SHARED` within the
   process, `MAP_FIXED`, `MAP_NORESERVE` = reservation with on-demand
   pages), `munmap` including parts, `mprotect`, `madvise(DONTNEED)` =
   decommit, `mseal` = seal. `PROT_EXEC` on unsealed memory only with the
   JIT permission from the manifest (W^X,
   [memory-management.md](memory-management.md)), otherwise `EACCES`.
   File `mmap`: read-only `MAP_PRIVATE` by reading into anonymous memory;
   file `MAP_SHARED` = `ENOTSUP` until fsd provides a file VMO.
7. **libc.a and libc.so** (questions 1 and 2): `libc.a` (Rust
   `staticlib`) and `libc.so` (`cdylib`, `SONAME` `libc.so`),
   `crt1.o`/`crti.o`/`crtn.o`, headers. xtask assembles a **C sysroot**
   from them (`target/c-sysroot/<arch>`), so ports build with their own
   build systems (`configure`, CMake, Meson) as a cross build with
   `--sysroot`.
8. **Ports as tests:** zlib (the package's own tests in QEMU), sqlite
   (amalgamation, test shell with `:memory:` and files), then the SDL and
   FFmpeg dependencies (question 10).

### Processes, locale, wchar

- **No `fork`** (the user's decision, not even a limited `fork`):
  processes only via `spawn` ([rust-std.md](rust-std.md)). `fork()`
  returns -1 with `errno` = `ENOSYS`. `posix_spawn` and `posix_spawnp`
  with `posix_spawn_file_actions_*` and `posix_spawnattr_*`: applications
  with a manifest via `launchd`, helper programs via `spawn` (like
  `std::process`), file actions for fd 0-2 (question 4; since T4 all fds without `FD_CLOEXEC`), `waitpid` over the
  process handle (done, see [POSIX additions](#processes-posix_spawn-and-waitpid)),
  `system`/`popen` via the system shell **[designed]**. Starting a PIE
  program (dynamic process) needs a libd session in the spawner
  (`spawn::Options::libd`, DL-4b); `cstd` passes it on when the process has
  one itself.
- **`vfork`** (question 11): proposal A **(recommended)** also -1 with
  `ENOSYS`. An alias to a spawn helper is impossible: `vfork` returns twice
  and the child runs in the parent's memory until `exec*`, which we cannot
  provide without `fork` in the kernel; a silent alias would cause wrong
  behaviour instead of a clear error. `exec*` (`execv`, `execvp`, ...)
  without a prior `fork` = replacing the process, which we do not have
  either: `ENOSYS`.
- **The headers warn at compile time:** `fork`, `vfork` and `exec*` are
  declared in `unistd.h` with `__attribute__((deprecated("MagnumOpus: use
  posix_spawn")))`. The symbols exist (linking succeeds, cross-build
  `configure` checks find them), calls return `ENOSYS` at run time.

#### How to adapt a ported program

| In the program | Change |
|---|---|
| `fork()` + `exec*()` in the child (running a command) | `posix_spawn`/`posix_spawnp`; redirection (`dup2` in the child) -> `posix_spawn_file_actions_adddup2`, close -> `..._addclose`, `chdir` -> `..._addchdir` |
| `fork()` + `exec` via the shell, pipe to the output | `popen` / `system` |
| `fork()` for parallel work (workers) | threads (`pthread`), or spawning itself with a role argument via `posix_spawn` and passing data over a pipe |
| `fork()` for a daemon (detaching from the terminal) | drop it: init/launchd start and supervise services |
| `vfork()` + `exec*()` | same as `fork` + `exec` |
| `fork()` for isolation (sandboxing the child) | `posix_spawn` of another program with its own manifest (isolation comes from the manifest, not from a copy of memory) |

Changes are stored as port patches (`third_party/`); programs with
configuration often only need an option (`--disable-fork`, `HAVE_FORK=0`);
FFmpeg, sqlite, zlib and SDL do not need `fork`.
- **Locale and wchar:** done, see [Wide characters and locale](#wide-characters-and-locale)
  (`C` single-byte, `C.UTF-8` and other names UTF-8, categories not
  localised yet; 32-bit `wchar_t`, complete `wchar.h` and `wctype.h`
  including wide streams).

### Decisions

Approved 2026-11-14: questions 1-7 and 9-11 as recommended (A), no `fork`,
not even a limited one. **Question 8 = B:** real locks in fsd already in
this work - a new fsproto message for byte-range locks (`fcntl`
`F_SETLK`, `F_SETLKW`, `F_GETLK`) and `flock`; owner = process (session),
released at process exit or when the last fd is closed, waiting with a
deadline, deadlock (at least a direct cycle) = `EDEADLK`. sqlite runs with
the default unix VFS. The fsd and fsproto change is a separate commit with
`libs/fs` host tests and a test of two processes over one database in
QEMU.

The original questions with their options:

1. **Contents of libc.so.** A **(recommended)** one `cdylib` `cstd + std +
   osrt` (one runtime, no new code; ~1.5 MB, code shared via libd). B
   rewrite cstd over `osrt` without `std` (smaller `libc.so`, but a second
   implementation of stdio and files next to std). C only a static
   `libc.a`, dynamic later with FFmpeg.
2. **Loader and libc.** The original DL design ([dynamic-linking.md](dynamic-linking.md))
   planned `ld.so` as "also `libc.so` in the musl style"; with libc over std
   I reconsider it. A **(recommended)** `ld.so` stays separate and
   small (Rust, as today, `dl*` built in), `libc.so` is an ordinary library
   from `DT_NEEDED`. B merge `ld.so` and `libc.so` into one file like musl
   (faster start-up, but std inside the loader and a bigger loader).
3. **`math.h`.** A **(recommended)** the `libm` crate (Rust, MIT/Apache,
   a port of musl libm, same precision, no C to compile; `compiler-builtins`
   uses it too). B musl libm in C via `cbuild`. C openlibm (MIT/BSD/ISC,
   more `long double` functions, bigger).
4. **`posix_spawn` and fd.** A **(recommended)** the child gets only fd 0,
   1, 2 (the parameter block supports them), other file actions `ENOTSUP`.
   B extend the parameter block with arbitrary fds (new tag, change to
   `abi::params`).
5. **Thread cancellation.** A **(recommended)** deferred cancellation only
   (`pthread_cancel` + cancellation points in our blocking functions), not
   asynchronous. B `pthread_cancel` = `ENOTSUP`. C asynchronous too, via
   `THREAD_SUSPEND` and PC redirection.
6. **Asynchronous signals.** A **(recommended for now)** the handler runs
   in a dedicated signal thread (like the Windows console Ctrl+C handler);
   delivery via an event in the parameter block (`SIGNAL`) held by the
   parent, termd or launchd; `kill` only to children, no `EINTR`. B
   emulation in cstd: the signal thread suspends the target thread
   (`THREAD_SUSPEND`) and redirects it into the handler; cstd blocking
   functions also wait for the signal event and return `EINTR`. C signals
   in the kernel (new syscall, interruptible syscalls with `EINTR`).
7. **`epoll`.** A **(recommended)** `poll` and `select` now, `epoll_*`
   (Linux API, over a port with one-shot registration) right in part 5,
   because libuv and mio want it. B only `poll` and `select`.
8. **File locks** (sqlite, `fcntl(F_SETLK)`, `flock`). A **(recommended)**
   in-process locks (sqlite works within one process, unprotected across
   processes, stated in the docs) and real locks later with fsd. B locks in
   fsd now (new fsproto message, outside this branch). C `ENOLCK` and
   sqlite without locks (`unix-none`).
9. **`long double`.** A **(recommended)** ABI as Linux: x86_64 80 bits,
   aarch64 128 bits; the `*l` functions compute in `double` for now
   (`double` precision, stated in the docs). B `long double` = `double` on
   both (`-mlong-double-64`, an ABI different from Linux; adopted code with
   `long double` tolerates it). C full precision (openlibm, software
   128-bit arithmetic on aarch64).
10. **Scope of ports in this branch.** A **(recommended)** zlib and sqlite
    with tests in QEMU; SDL and FFmpeg in separate branches. B also a
    minimal FFmpeg (`--disable-everything`, a few decoders) as a test of
    `libc.so`.
11. **`vfork`.** A **(recommended)** -1 with `ENOSYS` like `fork`. B
    `vfork` returns an error, but the header turns `vfork` + `exec*` in one
    macro into `posix_spawn` (only a simple pattern with no code in
    between, fragile).

### Kernel: AVX and XSAVE

**[verified in QEMU]** The kernel saves the FPU/SIMD state with XSAVE (x87,
SSE, AVX and AVX-512 per CPUID, FXSAVE without XSAVE), see
[Kernel](kernel.md). C code is compiled for the CPU baseline (x86-64-v2,
NEON, [hardware-requirements.md](hardware-requirements.md)); libraries
(FFmpeg, codecs) pick AVX/AVX2 and higher at run time via CPUID.

## SDL (step 6)

**[verified in QEMU]** x86_64 and aarch64 (suite `sdl`, [SDL](sdl.md)).
What the SDL port needed and was added:

| What | How |
|---|---|
| `environ` | a NULL-terminated array of `NAME=VALUE` in `osrt` (`env.rs`); rebuilt at startup and on every `setenv`/`unsetenv` (also from Rust `std::env::set_var`). Old arrays are not freed (other code may hold pointers into them, as in glibc) |
| `nl_langinfo` (`langinfo.h`) | C locale texts (date and time formats, days, months); `CODESET` is `UTF-8` in `C.UTF-8`, otherwise `ANSI_X3.4-1968` |
| `sched.h`, `pthread_getschedparam`/`setschedparam` | only `SCHED_OTHER` with priority 0; `SCHED_FIFO`/`SCHED_RR` = `EPERM` (like Linux without privileges), nonsense = `EINVAL` |
| `fenv.h` | exceptions, rounding, `fegetenv`/`feholdexcept`/`fesetenv`/`feupdateenv`, `FE_DFL_ENV`; x86_64 over x87 and MXCSR, aarch64 over FPCR and FPSR (`src/fenv.rs`), values and `fenv_t` as in glibc. Exception traps are never enabled. Test: step `math` (`ctest`) on both architectures, host test on x86_64 (SDL converts audio with NEON under `feholdexcept`) |
| `SIGVTALRM`, `SIGPROF` | numbers as in Linux (26, 27) for signal masks; `setitimer` timers do not exist yet |
| `wchar.h` | complete, see [Wide characters and locale](#wide-characters-and-locale) |
| `inttypes.h` | `PRI*`/`SCN*` macros (LP64), `imaxabs`, `imaxdiv`, `strtoimax`, `strtoumax` |
| `libm.a`, `libpthread.a`, `librt.a` | empty archives in the sysroot (everything is in `libc.a`, as in musl); build systems link `-lm -lpthread` |

`dlfcn.h` comes with `libc.so` (a separate branch); SDL does not need it
yet (our patch switches off GTK and `dlopen` for MagnumOpus).

## Wide characters and locale

**[verified in QEMU]** x86_64 and aarch64 (step `wchar` of the `ctest`
test, suite `cstd`; host tests `cargo test -p cstd`, `-p cstd-core`,
`-p uniprop`). Groundwork for libc++ (step 7a-2, the C++ runtime): complete
`wchar.h` and `wctype.h`, `locale.h` with `locale_t` objects and the `*_l`
variants.

### Encoding

| Locale | Encoding | `MB_CUR_MAX` | `CODESET` |
|---|---|---|---|
| `C`, `POSIX` (the default at start-up) | single-byte: 0x00-0x7F ASCII, bytes 0x80-0xFF map reversibly to U+DF80-U+DFFF as wide characters (like musl) | 1 | `ANSI_X3.4-1968` |
| `C.UTF-8`, `""` and any other name without a foreign codeset (`cs_CZ.UTF-8`, `en_US`, `de_DE@euro`) | UTF-8 (valid only: no overlong forms, no surrogates) | 4 | `UTF-8` |
| a name with another codeset (`cs_CZ.ISO-8859-2`) | rejected (`setlocale` returns `NULL`, `newlocale` `ENOENT`) | - | - |

- Why a single-byte `C`: a program in the C locale (the default) can pass
  arbitrary data through wide characters without loss and without errors;
  POSIX requires it and musl does the same. Programs that want UTF-8 call
  `setlocale(LC_ALL, "")` as everywhere else.
- `""` reads `LC_ALL`, `LC_<category>`, `LANG`; nothing set = `C.UTF-8`
  (the system is UTF-8).
- Categories are **not localised yet**: numbers, dates and collation are
  as in the C locale everywhere (decimal point, English month names,
  `strcoll` = `strcmp`). Only `LC_CTYPE` decides the encoding; the other
  categories just remember the name (`setlocale(LC_ALL, NULL)` returns the
  composite name `C.UTF-8;C;C;C;C;C`, which can be passed back). Czech
  collation comes later.
- Thread locale: `uselocale` (otherwise the global one from `setlocale`
  applies), `newlocale`/`duplocale`/`freelocale`. Uniform locales (`C`,
  `C.UTF-8`) are shared without allocation. `LC_GLOBAL_LOCALE` =
  `(locale_t)-1`.

### Functions

| Header | Functions | Note |
|---|---|---|
| `wchar.h` conversions | `mbrtowc`, `mbrlen`, `mbsinit`, `wcrtomb`, `btowc`, `wctob`, `mbsrtowcs`, `mbsnrtowcs`, `wcsrtombs`, `wcsnrtombs` | `mbstate_t` state (8 B, zeros = initial); a null state pointer = per-thread state |
| `stdlib.h` | `mblen`, `mbtowc`, `wctomb`, `mbstowcs`, `wcstombs`, `MB_CUR_MAX` (`__ctype_get_mb_cur_max`) | `MB_LEN_MAX` 4 (`limits.h`) |
| `wchar.h` strings | `wcslen`, `wcsnlen`, `wcscpy`, `wcpcpy`, `wcsncpy`, `wcpncpy`, `wcscat`, `wcsncat`, `wcslcpy`, `wcslcat`, `wcscmp`, `wcsncmp`, `wcscasecmp`, `wcsncasecmp`, `wcscoll`, `wcsxfrm`, `wcschr`, `wcsrchr`, `wcsspn`, `wcscspn`, `wcspbrk`, `wcsstr`, `wcstok`, `wcsdup`, `wmem*`, `wcwidth`, `wcswidth` | comparison by `wchar_t` value; `wcscasecmp` via simple Unicode case mapping |
| `wchar.h` numbers and time | `wcstol`, `wcstoll`, `wcstoul`, `wcstoull`, `wcstoimax`, `wcstoumax`, `wcstod`, `wcstof`, `wcstold`, `wcsftime` | leading white space by `iswspace`, ASCII digits only; `ERANGE` |
| `wchar.h` streams | `fgetwc`, `getwc`, `getwchar`, `fputwc`, `putwc`, `putwchar`, `ungetwc`, `fgetws`, `fputws`, `fwide` | the stream holds the thread locale's encoding; `FILE` remembers a partial character |
| `wchar.h` formatting | `wprintf`, `fwprintf`, `swprintf`, `vwprintf`, `vfwprintf`, `vswprintf`, `wscanf`, `fwscanf`, `swscanf` and the `v` variants | the same engine as `printf`/`scanf`; `swprintf` returns -1 when the output does not fit |
| `wctype.h` | `isw*`, `towupper`, `towlower`, `wctype`, `iswctype`, `wctrans`, `towctrans` | by Unicode, locale-independent; `iswdigit`/`iswxdigit` ASCII only |
| `locale.h` | `setlocale`, `localeconv`, `newlocale`, `uselocale`, `duplocale`, `freelocale`, `LC_*_MASK` | see above |
| `*_l` | `is*_l`, `toupper_l`, `tolower_l`, `isw*_l`, `towupper_l`, `towlower_l`, `iswctype_l`, `wctype_l`, `wctrans_l`, `towctrans_l`, `strcoll_l`, `strxfrm_l`, `wcscoll_l`, `wcsxfrm_l`, `strftime_l`, `wcsftime_l`, `strto{d,f,ld}_l`, `strto{l,ll,ul,ull}_l`, `nl_langinfo_l` | libc++ calls these instead of `uselocale` |
| `printf` | `%lc`, `%ls`, `%C`, `%S`; `asprintf`, `vasprintf` (buffer from `malloc`) | converted to the locale encoding; a character outside it = -1 and `EILSEQ`; precision in bytes, never half a character |
| `scanf` | `%lc`, `%ls`, `%l[` | in `scanf` converted from the locale encoding, in `wscanf` the other way round for `%c %s %[` |

### Unicode tables

Character classes (`iswalpha` = the Alphabetic property, `iswupper`/
`iswlower` = Uppercase/Lowercase, `iswspace` = White_Space without the
no-break spaces U+00A0, U+2007, U+202F), simple case mappings (fields 12
and 13 of `UnicodeData.txt`) and the terminal width of a character
(`wcwidth`: 2 for East_Asian_Width W/F, 0 for combining marks and format
characters, -1 for non-printable) live in the `libs/uniprop` crate
(`no_std`, the terminal will use it too). The tables (`src/tables.rs`,
sorted ranges, binary search, an ASCII fast path) are generated by
`cargo xtask unicode-tables` from the UCD in the compiler's
`core::char::UNICODE_VERSION` (18.0.0 today); the files are downloaded over
HTTPS with a SHA-256 check. A host test checks that the version matches and
that `iswalpha`, `iswupper`, `iswlower` and the case mappings agree with
Rust's `char` for all 1.1 million code points. The UCD data is licensed
under Unicode-3.0 (file `THIRD-PARTY.md` in the repository root).

### Internals

- `cstd-core/src/mb.rs`: the UTF-8 automaton and the single-byte encoding
  (`Enc`), `mbstate_t`. The formatting engine (`format.rs`) is shared by
  bytes and wide characters (`Out<u8>`/`Out<u32>`); it learns the locale
  encoding through a hook that `cstd` registers on the first locale change
  (until then C applies, also in `no_std` services).
- `cstd/src/locale/`: locale state (a global mask plus a per-thread locale
  in TLS), `nl_langinfo`, the `*_l` variants (`xl.rs`).
- `cstd/src/wchar/`: `conv.rs`, `wcs.rs`, `num.rs`, `wctype.rs`, `wio.rs`,
  `wprintf.rs`, `wscanf.rs`. The `scanf` engine works on `u32` characters
  in byte and wide mode.
- `long double` (`wcstold`, `strtold_l`) is in C (`c/math_ld.c`) with
  `double` precision like `strtold`.

## POSIX additions for ports and the Windows layer

**[verified in QEMU]** x86_64 and aarch64 (steps `posix2` and `inherit` (fd inheritance) of the `ctest`
test, suite `cstd`; host tests `posix_tests.rs` and `spawn_tests.rs` in
`cargo test -p cstd`). Functions that ports and Wine need on top of what
the library already had (Windows layer = step 8), and `termios.h` (see
[termios](#termios)).

### Processes: `posix_spawn` and `waitpid`

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="posix_spawn without fork">
<defs><marker id="ce9" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="230" height="90" rx="6"/><text class="t b" x="125" y="34" text-anchor="middle">parent: fd table</text><text class="m" x="125" y="54" text-anchor="middle">0 stdin, 1 stdout, 2 stderr</text><text class="m" x="125" y="72" text-anchor="middle">5, 6 pipe, 7 file ...</text><text class="m" x="125" y="90" text-anchor="middle">file actions (adddup2 ...)</text>
<path class="ln" d="M240 55 L288 55" marker-end="url(#ce9)"/>
<rect class="acc" x="290" y="10" width="200" height="90" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">plan (cstd)</text><text class="m" x="390" y="54" text-anchor="middle">fds without FD_CLOEXEC:</text><text class="m" x="390" y="72" text-anchor="middle">0-2 streams, above 2</text><text class="m" x="390" y="90" text-anchor="middle">inherited fds (handles)</text>
<path class="ln" d="M490 55 L538 55" marker-end="url(#ce9)"/>
<rect class="box" x="540" y="10" width="230" height="90" rx="6"/><text class="t b" x="655" y="34" text-anchor="middle">launchd</text><text class="m" x="655" y="54" text-anchor="middle">program, manifest, job,</text><text class="m" x="655" y="72" text-anchor="middle">FS and network sessions</text><text class="m" x="655" y="90" text-anchor="middle">(like std::process)</text>
<path class="ln" d="M655 100 L655 148" marker-end="url(#ce9)"/>
<rect class="ok" x="540" y="150" width="230" height="90" rx="6"/><text class="t b" x="655" y="174" text-anchor="middle">child</text><text class="m" x="655" y="194" text-anchor="middle">new process in a child job,</text><text class="m" x="655" y="212" text-anchor="middle">fds from the parameter block,</text><text class="m" x="655" y="230" text-anchor="middle">default signals</text>
<path class="ln" d="M538 195 L242 195" marker-end="url(#ce9)"/>
<rect class="box" x="10" y="150" width="230" height="90" rx="6"/><text class="t b" x="125" y="174" text-anchor="middle">parent: child table</text><text class="m" x="125" y="194" text-anchor="middle">pid_t from cstd -> process handle</text><text class="m" x="125" y="212" text-anchor="middle">waitpid, wait4, waitid,</text><text class="m" x="125" y="230" text-anchor="middle">kill (termination)</text>
</svg>
<figcaption>posix_spawn without fork: file actions are evaluated in the parent, launchd starts the child.</figcaption>
</figure>

There is no `fork` in the system (the user's decision, see [Processes,
locale, wchar](#processes-locale-wchar)). `posix_spawn` and `posix_spawnp`
start the child through launchd exactly like `std::process::Command`
([Rust std](rust-std.md)): a child job, a new FS session with the same
identity, a network session and three standard streams. The child also
**inherits the parent's fds without `FD_CLOEXEC`** at the same numbers
(decision T4A, as in POSIX; formerly 4A only 0-2). File actions are
evaluated in the parent over a copy of its fd table and numbers with
`FD_CLOEXEC` are dropped at the end:

| Action | Result |
|---|---|
| `addclose(n)`, `addclosefrom_np(n)` | the child does not get the number (numbers from `n`); a closed stream 0-2 = empty input / discarded output |
| `adddup2(x, y)` | the child's `y` = the description behind `x` (in the parent or after previous actions), without `FD_CLOEXEC`; `x == y` only clears `FD_CLOEXEC`; a closed `x` = `EBADF` |
| `addopen(0-2, "/dev/null")` | empty input / discarded output (above 2 `ENOTSUP`) |
| `addopen(y, file)` | the parent opens the file (path relative to the child's directory at that step, `open` flags; with `O_CLOEXEC` it is not passed) |
| `addchdir(_np)`, `addfchdir(_np)` | the child's working directory (relative to the previous one); a missing one makes `posix_spawn` fail |

**Fds 0-2 as streams** (fsd message `OPEN_STREAM`, [file system](file-system.md#the-fsproto-protocol)):
the child's standard streams are byte-stream channels, so fsd "plays" a
file on 0-2 into the channel (stdin: from the current position to the
end, then end of stream) or writes from the channel (stdout/stderr: from
the current position, with `O_APPEND` at the end). fsd opens its own handle
of the same file, so the stream survives closing the fd and the end of the
parent. The child's writes are in the file once fsd has processed them
(shortly after the child ends, not necessarily at once). An fsd restart
ends the stream.

**Fds above 2** go in the parameter block (`abi::params`, version 3: `Fd`
entries + handles with the tag `tag::FD`, launchd passes them unchanged)
and the child's runtime (`osrt::inherit`) puts them into the fd table
before `main`:

| Kind | How it is passed |
|---|---|
| pipe, the parent's stdio under another number | a duplicate of the stream channel (both processes write/read the same channel) |
| memory fd (`memfd_create`) | a duplicate of the memory object, seals, a copy of the position |
| file, directory | the durable identity of the object (dataset, oid, generation); the child opens it in its own FS session with `REOPEN` - fsd checks the rights and the session's mounted paths again |
| socket, `epoll` | **not inherited** (the connection to netd belongs to the parent's session); an inherited number is skipped, `adddup2` of a socket = `ENOTSUP` |

Differences from POSIX (explicitly): the file position is **not shared**
between the parent and the child - the child starts at the parent's
position and then has its own (a shared open file description between
processes does not exist, also for 0-2); an inherited file that has
disappeared in the meantime (or that the child's session cannot see) is
missing in the child (`EBADF`). There are at most `abi::params::MAX_FDS` =
48 inherited fds above 2 (more = `EMFILE`), because the handles go in one
start-up message together with stdio and sessions (channel message limit
64 handles). `AF_UNIX` sockets are inherited (a duplicate of the
connection or listener channel, [AF_UNIX sockets](#af_unix-sockets)).


- **The child's network:** inherited as in POSIX when the parent has one
  (unlike `std::process::Command`, where only with `network(true)`). An
  application with its own manifest, however, gets the network only from
  its manifest: launchd decides, not the parent (the intersection of the
  parent's session and the manifest's `[network]`; a manifest without
  `[network]` = no network). A helper program without a manifest inherits
  the parent's network session unchanged.
- **Program:** a path with `/` (relative to the child's directory). With
  `posix_spawn` a bare name = a file in the child's directory if it exists;
  `posix_spawnp` searches `PATH` (without the variable `/system/bin`). A
  bare name that is not found goes to launchd as a system program (command
  registry, `/system/bin`, initfs). A launchd error comes back as an error
  code (`ENOENT`, `EACCES`, without a launchd channel `ENOSYS`).
- **Attributes:** `SETSID`, `SETPGROUP` with group 0, `SETSIGDEF`,
  `SETSIGMASK`, `RESETIDS` are accepted (the child is always a new process
  in a new job with default handlers and an empty signal mask; the stored
  mask is not passed on). A group other than 0 = `ENOTSUP`, scheduling
  other than `SCHED_OTHER` or a priority other than 0 = `EPERM`.
- **The child's `pid_t`** is allocated by `cstd` (a table in the process,
  numbers from `getpid() + 1`); other processes do not know them.
- **`waitpid`, `wait`, `wait3`, `wait4`, `waitid`:** `WNOHANG`, `WNOWAIT`;
  there are no process groups, `waitpid(0)` and `waitpid(-pgid)` wait like
  `-1` for any child. Status like Linux: `WEXITSTATUS` = the low 8 bits of
  the exit code; an end caused by the system is reported as a signal
  (`PROCESS_KILL` = `SIGKILL`, or the signal from `kill`, a CPU exception =
  `SIGSEGV`/`SIGILL`/`SIGFPE`/`SIGTRAP`/`SIGBUS`, a panic = `SIGABRT`).
  `struct rusage` is zeroed (the kernel does not report CPU time yet).
  Waiting for several children uses `OBJECT_WAIT_MANY` (at most 64 at a
  time) in rounds of 50 ms, so a child started by another thread also gets
  into the wait.
- **`kill` on a child:** `SIGKILL`, `SIGTERM`, `SIGINT`, `SIGHUP`,
  `SIGQUIT` end the child at once (`PROCESS_KILL`, its signal handler does
  not run), signal 0 only checks it exists. Delivering other signals to the
  child's handler is not possible yet (`ENOTSUP`); the parent gets no
  `SIGCHLD`.

### AF_UNIX sockets

**[verified in QEMU]** x86_64 and aarch64 (step `unix` of the `ctest`
test, suite `cstd`, modelled on wineserver: `socketpair`,
`bind`/`listen`/`accept` on a relative path, a non-blocking listener in
`poll`, `connect` from another process, `SCM_RIGHTS` in both directions,
passing one end of a `socketpair` and inheriting the other through
`posix_spawn`; host tests `unix_tests.rs`). Decision T1A
(Windows layer, step 8): general `AF_UNIX`, so that the code of
Wine and other ports (X11, D-Bus, Wayland) stays unchanged.

<figure class="diagram">
<svg viewBox="0 0 780 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="AF_UNIX over channels and fsd">
<defs><marker id="en11" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="220" height="80" rx="6"/><text class="t b" x="120" y="34" text-anchor="middle">server</text><text class="m" x="120" y="54" text-anchor="middle">bind("socket"), listen</text><text class="m" x="120" y="72" text-anchor="middle">listener channel</text>
<rect class="acc" x="280" y="10" width="220" height="80" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">fsd</text><text class="m" x="390" y="54" text-anchor="middle">socket node (kind 5)</text><text class="m" x="390" y="72" text-anchor="middle">node -> listener (memory)</text>
<rect class="box" x="550" y="10" width="220" height="80" rx="6"/><text class="t b" x="660" y="34" text-anchor="middle">client</text><text class="m" x="660" y="54" text-anchor="middle">connect("…/socket")</text><text class="m" x="660" y="72" text-anchor="middle">new channel pair</text>
<path class="ln" d="M230 50 L278 50" marker-end="url(#en11)"/>
<path class="ln" d="M548 50 L502 50" marker-end="url(#en11)"/>
<rect class="ok" x="10" y="120" width="760" height="70" rx="6"/><text class="t b" x="390" y="144" text-anchor="middle">connection = a channel pair between processes (fsd is not in it)</text><text class="m" x="390" y="166" text-anchor="middle">message = flags, fds (SCM_RIGHTS: pipe, memfd, file, directory, AF_UNIX) + data</text>
</svg>
<figcaption>AF_UNIX: fsd only arranges the first meeting through the socket node, data go directly over the channel.</figcaption>
</figure>

- **Types:** only `SOCK_STREAM` (`SOCK_SEQPACKET`, `SOCK_DGRAM` =
  `EPROTONOSUPPORT`); `SOCK_NONBLOCK`, `SOCK_CLOEXEC`, `O_NONBLOCK` through
  `fcntl` (empty = `EAGAIN`, `accept` without a connection = `EAGAIN`).
- **Name** = a path (also relative to the working directory) as a socket
  node in fsd (`SOCKET_BIND`, [file system](file-system.md#the-fsproto-protocol));
  `lstat` reports `S_IFSOCK` and the owner. `connect` needs the write
  right on the node. A node without a listener (the process ended, an fsd
  restart) = `ECONNREFUSED`, a missing one = `ENOENT`, `bind` to an
  existing one = `EADDRINUSE` (as on Unix: `unlink` before `bind`). There
  is no abstract name (`sun_path[0] == 0`, `ENOTSUP`). Connections that
  arrive after `bind` and before `listen` wait in the queue (Linux would
  refuse them).
- **Data:** every `send`/`sendmsg` = one channel message (at most 64 KiB,
  longer ones are split); `recv` reads at most the rest of one message (a
  short read is allowed for `SOCK_STREAM`). `shutdown(SHUT_WR)` sends an
  end mark: the other side reads the data and gets 0. The other side
  closing = `POLLHUP`/`POLLIN` and `recv` 0, `send` = `EPIPE` (no
  `SIGPIPE` is sent).
- **`SCM_RIGHTS`:** at most 32 fds in a message. Fds are passed as with
  inheritance ([posix_spawn](#processes-posix_spawn-and-waitpid)): a pipe
  and `AF_UNIX` = a duplicate of the channel, a memory fd = a duplicate of
  the memory object, a file and a directory = the durable identity +
  `REOPEN` in the receiver's session (the receiver must see the path; the
  position is not shared). TCP/UDP sockets and `epoll` cannot be passed
  (`ENOTSUP`); an fd the receiver cannot open is left out. The receiver
  gets the fds with the first `recvmsg` that starts reading the message;
  `recv`/`read` drop them. `MSG_CMSG_CLOEXEC` sets `FD_CLOEXEC`, too little
  room in `msg_control` = `MSG_CTRUNC` (extra fds are closed).
- **`SO_PEERCRED`:** the side that accepted the connection gets the
  client's user and group verified by fsd from the identity of its
  session, and the process number (the client's `getpid`, not verified);
  the client knows only zeros about the listener (fsd does not pass the
  listener's credentials). `socketpair` = own values. `getuid`/`getgid` =
  the FS session identity (`WHOAMI`; system = 0), equal to the `st_uid` of
  objects the user created.
- **`getsockname`** = the name from `bind` (for an accepted connection the
  listener's name), **`getpeername`** = the path from `connect` (otherwise
  no name); `SO_TYPE`, `SO_DOMAIN`, `SO_ACCEPTCONN`, `SO_PEERCRED`;
  `SO_PASSCRED` and buffer sizes are accepted without effect.

### Memory fd

**[verified in QEMU]** x86_64 and aarch64 (step `memfd` of the `ctest`
test, test `ctest memfd` of two processes in the suite `cstd`).
`memfd_create` (`sys/mman.h`, `MFD_CLOEXEC`, `MFD_ALLOW_SEALING`;
`MFD_HUGETLB` without effect) creates an fd over a resizable kernel memory
object ([memory management](memory-management.md#resizable-memory-object))
with a capacity of 64 MiB (`mo_memfd_create` for another one):

- `read`, `write` (past the end grows the object), `pread`/`pwrite`,
  `lseek`, `fstat` (a regular file, size from the kernel - other processes
  see it too), `ftruncate` (above the capacity `EFBIG`).
- `mmap(MAP_SHARED)`, also with writing, maps the object itself (the same
  frames in all processes); a mapping may cover the whole capacity, an
  access past the size = `SIGBUS`; shrinking unmaps the pages past the end
  in other processes too. `MAP_PRIVATE` = a copy of the content (like a
  file).
- Seals `fcntl(F_ADD_SEALS / F_GET_SEALS)`: `F_SEAL_SHRINK`,
  `F_SEAL_GROW`, `F_SEAL_WRITE` (seals the object in the kernel; writable
  mappings must be gone, otherwise `EBUSY`), `F_SEAL_SEAL`. Without
  `MFD_ALLOW_SEALING` = `EPERM`.
- **Passing to another process** (`<magnumopus/memfd.h>`, there is no
  `SCM_RIGHTS`): `mo_memfd_export(fd, readonly, &handle)` gives a new
  handle of the object, the program sends it over its own channel, the
  recipient makes an fd of it with `mo_memfd_import(handle, O_RDWR |
  O_RDONLY)`. Read-only = a handle without `WRITE` and `RESIZE` (the kernel
  refuses writing, writable mappings and `ftruncate`). `mo_start_handle(tag)`
  takes a handle the parent passed in the start message.

### Semaphores

`semaphore.h`: unnamed semaphores (`sem_init`, `sem_wait`, `sem_trywait`,
`sem_timedwait`, `sem_clockwait`, `sem_post`, `sem_getvalue`,
`sem_destroy`) over the futex, `sem_t` 32 B like Linux. Waiting is a thread
cancellation point. `pshared` = 1 is accepted, but the semaphore only works
between the threads of a process: the system has no memory shared between
processes (`MAP_SHARED` with writing is `ENOTSUP`) and the kernel futex is
in the process address space. Named semaphores (`sem_open`, `sem_close`,
`sem_unlink`) therefore return `SEM_FAILED` / -1 with `ENOSYS`; real ones
will come with shared memory between processes (a separate task:
`shm_open` and a futex between processes).

### Threads, files, time

- **`pthread_attr_setstack`:** the thread really runs in the caller's
  memory (`libos::thread::spawn_on_stack`); the memory must live until the
  thread ends and the caller provides the guard area below it (Wine
  allocates thread stacks itself). Also `getstack`, `set/getguardsize`
  (without an own stack the thread always has the `libos` guard area),
  `set/getscope` (only `PTHREAD_SCOPE_SYSTEM`, process scope `ENOTSUP`),
  `set/getinheritsched`, `set/getschedpolicy` and `set/getschedparam`
  (only `SCHED_OTHER`, priority 0).
- **`fchdir`:** the directory opened as an fd; the path is from the time of
  opening (`osrt::fs::File::path`) and goes stale if the directory is
  renamed.
- **`statfs`, `fstatfs`** (`sys/statfs.h`, `sys/vfs.h`, layout like
  64-bit Linux; `statvfs` comes with the C++ branch 7a from the same source): block size,
  total and free blocks of the FS session's volume (the fsd `INFO` message;
  blocks waiting to be freed are not counted); all paths of a process are
  on one volume, the path or fd is only validated. `f_type` =
  `MOFS_SUPER_MAGIC` (0x4D4F4653), file counts 0 (the FS has no fixed
  number), `ST_RDONLY` is not set (rights come from the file's ACL).
- **`times`** (`sys/times.h`): elapsed time in ticks of
  `sysconf(_SC_CLK_TCK)` = 100; the kernel does not report CPU time yet, so
  the `tms` fields are 0 (like `clock()` = -1).
- **`asctime`, `asctime_r`, `ctime`, `ctime_r`:** the C standard format
  (`"Sun Sep 16 01:03:52 1973\n"`), a year above 9999 = `EOVERFLOW`; the
  `asctime`/`ctime` buffer is per thread.
- **`ffs`, `ffsl`, `ffsll`** (`strings.h`).
- **No sessions or process groups:** a process leads its own session and
  group (number = `getpid()`); `setsid`, `getsid`, `getpgid`, `getpgrp`
  return it, `setpgid` only "stay in the own group", otherwise `EPERM`.
- **`putenv`** (a copy of the string: a later change does not reach the
  environment, unlike glibc; without `=` it removes the variable),
  **`gethostname`** (= `uname().nodename`, `sethostname` = `EPERM`), the BSD
  types `caddr_t`, `u_char` ... (`sys/types.h`).
- **`_exit`** (`unistd.h`, = `_Exit`), `select` also via `sys/time.h` (like glibc), the signal
  numbers `SIGIO`, `SIGXCPU`, `SIGXFSZ`, `SIGPWR`, `SIGSYS`, `SIGSTKFLT`
  (cstd does not raise them itself) and the bits `S_ISUID`, `S_ISGID`,
  `S_ISVTX`.
- **`errno`:** the full Linux table (`asm-generic`, numbers 1-133 without
  the holes 41 and 58, the same on both architectures) with `strerror`
  texts, aliases `EWOULDBLOCK`, `EDEADLOCK`, `EOPNOTSUPP`.

### Network

- **Constants** like Linux: `IPPROTO_*` (`netinet/in.h`), `IP_*` and
  `IPV6_*` options, `struct ip_mreq`, `ip_mreqn`, `ipv6_mreq`,
  `in_pktinfo`, `in6_pktinfo`, the `IN6_IS_ADDR_*` macros, `INADDR_*`;
  `SO_*`, `MSG_*` (`sys/socket.h`), `TCP_*` (`netinet/tcp.h`). A constant
  does not mean support: `setsockopt` accepts only timeouts and options
  without effect (`SO_REUSEADDR`, `SO_KEEPALIVE`, buffer sizes,
  `SO_LINGER`, `TCP_NODELAY`), the others are `ENOPROTOOPT`. `getsockopt`
  also gives `SO_DOMAIN`, `SO_PROTOCOL` and `IP_TTL` (64).
- **`SOCK_RAW` and `AF_PACKET` = `EPERM`** (like Linux without
  `CAP_NET_RAW`): only netd has raw packets and frames; `AF_NETLINK` =
  `EAFNOSUPPORT`.
- **Control messages:** the macros `CMSG_FIRSTHDR`, `CMSG_NXTHDR`,
  `CMSG_DATA`, `CMSG_SPACE`, `CMSG_LEN`, `CMSG_ALIGN` and `struct cmsghdr`
  like Linux. `sendmsg` with a control message returns `EOPNOTSUPP`:
  `SCM_RIGHTS` (passing an fd) works differently in the system (handles in
  channels, Wine has its own transport), the others (`IP_PKTINFO` ...) are
  not carried by netd. `recvmsg` returns none (`msg_controllen` = 0).
- **`getifaddrs`, `freeifaddrs`** (`ifaddrs.h`) and **`if_nameindex`,
  `if_freenameindex`, `if_nametoindex`, `if_indextoname`** (`net/if.h`):
  the loopback `lo` (index 1; 127.0.0.1/8 and ::1/128) and the network
  interface `eth0` (index 2) from the netd state (`INFO`: address, mask from
  the prefix, IPv4 broadcast, `IFF_UP`/`IFF_RUNNING` from the link) - only
  with a network session that has the `INFO` right. An interface without an
  address has `ifa_addr` = NULL; there are no `AF_PACKET` (MAC) entries.
- **`ioctl` `SIOCGIF*` on a socket** (`struct ifreq`, `struct ifconf`,
  `net/if.h`): `SIOCGIFFLAGS`, `SIOCGIFMTU` (loopback 65536),
  `SIOCGIFINDEX`, `SIOCGIFNAME`, `SIOCGIFADDR`, `SIOCGIFNETMASK`,
  `SIOCGIFBRDADDR` (IPv4 only), `SIOCGIFHWADDR` (MAC, loopback
  `ARPHRD_LOOPBACK`), `SIOCGIFCONF` over the same interfaces; setting
  (`SIOCSIF*`) = `ENOTTY`.
- **Headers for building ports** (no new functionality): `sys/un.h`
  (`struct sockaddr_un`, sockets see [AF_UNIX sockets](#af_unix-sockets)),
  `struct ucred` (`SO_PEERCRED`), `struct tcp_info` and the `TCP_*` states
  (`getsockopt(TCP_INFO)` = `ENOPROTOOPT`), `net/if_arp.h` (`ARPHRD_*`,
  `struct arphdr`), `net/ethernet.h` (`ETH_ALEN`, `struct ether_header`), `netinet/if_ether.h` (`struct ether_arp`);
  `ioctl(SIOCATMARK)` on a socket returns 0 (netd does not carry
  out-of-band data).
- **`gethostbyname`, `gethostbyaddr`** (`struct hostent`, `h_errno`,
  `hstrerror`): the old interface returns NULL with `h_errno` =
  `NO_RECOVERY`; programs should use `getaddrinfo`.

### termios

**[verified in QEMU]** the program side (step `termios` of the `ctest`
test against a test terminal in init, host tests `termios_tests.rs` and
`cargo test -p ttyproto`); the terminal window side (`shsess` starting
external programs) **[designed]**.

A process started in a terminal gets, next to the stdin/stdout/stderr
streams, a **terminal control channel** in its start message (tag
`abi::params::tag::TTY`, protocol `libs/ttyproto`). Data still flows over
the byte streams; the channel only carries control:

| op | Request | Reply |
|---|---|---|
| `GET_MODE` 1 | - | mode (the `struct termios` fields) |
| `SET_MODE` 2 | when (`TCSANOW`/`TCSADRAIN`/`TCSAFLUSH`), mode | the mode the terminal actually set |
| `GET_SIZE` 3 | - | window size (`struct winsize`) |
| `SET_SIZE` 4 | size | - (a terminal window does not let the client change it: `ACCESS_DENIED`) |
| `FLUSH` 5 | queue | - |
| `DRAIN` 6 | - | - |
| `WATCH_SIZE` 7 | - | handle of an event the terminal signals when the size changes |

- **The line discipline lives on the terminal side** (decision A): line
  editor, echo and Ctrl+C (`SIGINT`) stay in the terminal, which knows the
  native window protocol; VT sequences are only an adapter for adopted
  programs. In raw mode (`cfmakeraw`) the terminal sends key bytes (arrows
  as VT sequences). The terminal clears unsupported flags in the `SET_MODE`
  reply (a following `tcgetattr` shows them); the line is virtual (always 8
  bits without parity, the speed is only stored).
- **cstd:** `tcgetattr`, `tcsetattr`, `cfmakeraw`, `cfget/setispeed`,
  `cfget/setospeed`, `cfsetspeed`, `tcflush`, `tcdrain`, `tcflow` and
  `tcsendbreak` (only check the terminal), `tcgetpgrp`/`tcsetpgrp`/
  `tcgetsid` (the process's own group), `ioctl` `TCGETS`, `TCSETS(W/F)`,
  `TCFLSH`, `TIOCGWINSZ`, `TIOCSWINSZ`, `TIOCGPGRP`. Values and
  `struct termios` (60 B) like glibc.
- **`isatty`** = fd 0-2 with stdio **and** the process has a terminal
  control channel; a stream channel from init or a pipe is not a terminal
  (stdout is then fully buffered as on Linux). Another fd = `ENOTTY`.
- **`SIGWINCH`:** the first `tcgetattr` or `TIOCGWINSZ` asks for the event
  (`WATCH_SIZE`) and starts a thread that calls `raise(SIGWINCH)` when it
  is signalled; the handler runs in that thread like the timer's
  (decision 6A).

### Internals (additions)

- `cstd/src/spawn/mod.rs` file actions (`plan`), attributes, program
  lookup; `spawn/wait.rs` the child table, `waitpid` and friends, `kill` on
  a child; the system part `sys/os_proc.rs` over `osrt::process`
  (`wait_any`). Host tests have stand-in children (`exit:<code>`,
  `sleep:<ms>:<code>`) in `sys/host.rs`.
- `cstd/src/termios.rs` (the `ttyproto` client, terminal `ioctl`, the
  `SIGWINCH` thread), `osrt::tty` (the control channel from the start
  message); host tests have an in-memory terminal (`sys/host.rs`,
  `ttyproto::Tty`).
- `cstd/src/semaphore.rs`, `pthread/attr.rs`, `statfs.rs`, `ifaddrs.rs`, `ifreq.rs`;
  `time.rs` (`asctime`, `times`), `unistd.rs` (`setsid`), `misc.rs` (`ffs`), `socket.rs`
  (`SOCK_RAW`, control messages).
- `osrt`: `fs::volume_space`, `fs::File::path`, `net::interface`,
  `process::wait_any`, `thread::spawn_on`; `libos::thread::spawn_on_stack`.

## FFmpeg and licences

FFmpeg will run as a **separate isolated process** (the playback and
decoding service, see [Architecture](architecture.md)); applications do not
link it, they talk to the service. Licensing:

- It is built only under **LGPL-2.1-or-later** - never `--enable-gpl` or
  `--enable-nonfree` (GPL and non-free parts must not enter the system).
- The LGPL obligation (the ability to relink a modified library) applies
  only to the FFmpeg process binary, not to the rest of the system: we ship
  the sources of the FFmpeg version used, our changes and the build script
  with which the user can rebuild the process with their own version of the
  library.
- Later dynamic linking (a shared FFmpeg library), once there is a shared
  library loader; then replacing the library is enough.
- The `THIRD-PARTY.md` entry is created with the first vendored code (the
  FFmpeg section with the rules is already there).
