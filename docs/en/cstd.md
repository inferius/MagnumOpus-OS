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
> `threads`): `pthread`. Further parts (time, sockets, `math.h`, `mmap`,
> signals, file locks, `libc.so`, ports) **[designed]**, see [Continuation](#continuation-a-complete-libc-and-posix-parts-2-8).

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
| `stdlib.h` | `exit`, `_Exit`, `atexit`, `getenv`, `abort` | `EXIT_SUCCESS`, `EXIT_FAILURE` |
| `string.h` | `strerror` | the rest is in `cstd-core` |
| `assert.h` | `assert` | a failure prints file and line to stderr and ends the process |

## Functions (part 2a: fd numbers and files)

| Header | Functions | Note |
|---|---|---|
| `unistd.h` | `close`, `read`, `write`, `pread`, `pwrite`, `lseek`, `dup`, `dup2`, `dup3`, `pipe`, `pipe2`, `isatty`, `fsync`, `fdatasync`, `ftruncate`, `truncate`, `unlink`, `rmdir`, `getcwd`, `chdir`, `access`, `sysconf`, `getpagesize` | `fork`, `vfork`, `exec*` return `ENOSYS` (see [No fork](#processes-locale-wchar)) |
| `fcntl.h` | `open`, `openat` (only `AT_FDCWD` or an absolute path), `creat`, `fcntl` (`F_DUPFD`, `F_DUPFD_CLOEXEC`, `F_GETFD`, `F_SETFD`, `F_GETFL`, `F_SETFL`) | `O_*` values as Linux, `O_DIRECTORY` the same on both architectures; `F_*LK` locks `ENOLCK` for now |
| `sys/stat.h` | `stat`, `lstat` (= `stat`), `fstat`, `mkdir`, `chmod`, `fchmod`, `umask` | `st_mode` from the kind and the session's effective rights; `chmod` has no effect (rights are FS grants) |
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
  100 ms slices, so cancellation is at most 100 ms late).
- `pthread_exit` in the main thread ends the process with code 0 (it does
  not wait for the other threads).
- No `PTHREAD_PROCESS_SHARED` (the futex is private to the process).

### fd numbers

- The table is in `osrt` (`osrt::fd`), one per process; `cstd` sees it via
  `std::os::magnumopus::runtime` (a re-export of the `osrt` crate in std,
  unstable feature `magnumopus_runtime`, only for libraries over std).
- A number points to an open description: an FS session file with a
  position, the process stdio (0, 1, 2 from start-up) or a pipe end. `dup`
  shares the description and the position.
- At most 4096 numbers (`EMFILE`). `FD_CLOEXEC` is stored (for
  `posix_spawn`, the child gets only 0-2).
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

- **Floating point in `printf`** is missing (`%f`, `%g`, `%e` print `?`).
- Only `FILE` streams; fd numbers, threads, time, sockets and `math.h`
  come with the continuation below.

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
   (`st_mode` from the kind and effective rights, `st_uid` from the session
   identity), `opendir`/`readdir`, `mkdir`, `unlink`, `rmdir`, `rename`,
   `access`, `getcwd`/`chdir`, `pipe` (a channel pair), `isatty`.
   `chmod` without effect (rights are FS grants, not bits), `link`/
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
   `PEER_CLOSED`, sockets readiness events (netproto v2). No `AF_UNIX`
   (local IPC uses channels). `epoll`: question 7.
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
  `std::process`), file actions for fd 0-2 (question 4), `waitpid` over the
  process handle, `system`/`popen` via the system shell. Starting a PIE
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
- **Locale:** only `C` (the default at start-up, as POSIX requires) and
  `C.UTF-8`. `setlocale(LC_ALL, "")` selects `C.UTF-8`; any `*.UTF-8` name
  is accepted as `C.UTF-8`. The decimal separator is always `.`, texts and
  months in English. Localising applications is not the job of libc.
- **wchar:** `wchar_t` is 32 bits (UTF-32) on both architectures,
  `MB_CUR_MAX` 1 in `C` and 4 in `C.UTF-8`. Conversions (`mbrtowc`,
  `wcrtomb`, `mbstowcs`, ...), `wcs*` strings, `wctype.h` (Unicode classes
  from tables), `swprintf`. Wide streams (`fwprintf`, `fgetwc`) only when a
  port needs them.

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
