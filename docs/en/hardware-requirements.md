# Hardware requirements

What a computer must support to run MagnumOpus. The page covers the
processor for now; the rest (memory, firmware, devices) comes with
measurements.

Contents:

1. [Processor](#processor)
2. [What happens on an older processor](#what-happens-on-an-older-processor)
3. [Why this level](#why-this-level)

## Processor

All programs (services, drivers, applications) are built for the targets
`x86_64-unknown-magnumopus` and `aarch64-unknown-magnumopus` with FPU and
SIMD (hard-float, [architecture.md](architecture.md)). The kernel itself
does not use the FPU, but it saves the FPU state of every thread
([kernel.md](kernel.md)).

| Architecture | Baseline | Instructions beyond the oldest CPU of the architecture | Roughly since |
|---|---|---|---|
| x86_64 | **x86-64-v2** | SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B, LAHF/SAHF in 64-bit mode | Intel Nehalem (2008), AMD Bulldozer (2011) |
| aarch64 | ARMv8-A with FP and AdvSIMD (NEON) | - (FP and NEON are always present on common cores) | every common ARMv8 |

Programs do not assume higher levels (AVX, AVX2, AVX-512, AES-NI);
libraries that can use them detect them at run time (CPUID and XGETBV:
the kernel enables AVX and AVX-512 in XCR0 when the CPU has them and
saves their state with XSAVE, [kernel.md](kernel.md)). AMX is not enabled
yet.

## What happens on an older processor

**[implemented]** Right after start (before CPU initialisation,
`check_cpu_baseline` in `kernel/src/main.rs`, `arch::missing_baseline`)
the kernel checks the baseline. When something is missing it writes to the
screen and the log

```
procesor nesplnuje pozadavky systemu (x86-64-v2): chybi SSE4.1, SSE4.2, POPCNT
```

("the processor does not meet the system requirements: missing ...") and
stops **without a restart** (the same machine would only loop). In a test
run QEMU exits with a failure. Without the check some program would crash
on an invalid instruction (`#UD`) at a random moment later.

## Why this level

- **x86-64-v2:** the Windows layer needs CMPXCHG16B (16-byte atomic
  operations), SSE4.2 and POPCNT speed up text and bitmap work. It drops
  only processors from before 2008 (Intel) and 2011 (AMD). A higher level
  (v3 = AVX2) would drop machines still in common use (e.g. Intel Pentium
  and Celeron until 2020), so AVX2 is detected at run time.
- **aarch64:** FP and NEON are on all common cores; SVE is not used yet
  (common hardware rarely has it).
- QEMU tests run with `-cpu max` (x86_64) and `-cpu cortex-a76` (aarch64), so they meet the baseline.
