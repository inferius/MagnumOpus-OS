/*
 * sys/ucontext.h - kontext obsluhy signalu (cstd, docs/cs/cstd.md).
 * Rozlozeni jako Linux (glibc), aby prevzaty kod (Wine, JIT, GC) pouzival
 * pole beze zmen. Obsluha se SA_SIGINFO dostane ve tretim argumentu
 * ucontext_t z okamziku vyjimky; zmeny registru se po navratu pouziji.
 * Hodnoty a rozlozeni hlida test ucontext_header_matches.
 */
#ifndef _CSTD_SYS_UCONTEXT_H
#define _CSTD_SYS_UCONTEXT_H

#include <_cstd.h>
#include <signal.h>

_CSTD_BEGIN

#if defined(__x86_64__)

#define NGREG 23
typedef long long greg_t;
typedef greg_t gregset_t[NGREG];

enum {
    REG_R8 = 0,
    REG_R9 = 1,
    REG_R10 = 2,
    REG_R11 = 3,
    REG_R12 = 4,
    REG_R13 = 5,
    REG_R14 = 6,
    REG_R15 = 7,
    REG_RDI = 8,
    REG_RSI = 9,
    REG_RBP = 10,
    REG_RBX = 11,
    REG_RDX = 12,
    REG_RAX = 13,
    REG_RCX = 14,
    REG_RSP = 15,
    REG_RIP = 16,
    REG_EFL = 17,
    REG_CSGSFS = 18,
    REG_ERR = 19,
    REG_TRAPNO = 20,
    REG_OLDMASK = 21,
    REG_CR2 = 22
};

/* Obraz FXSAVE (512 B). */
struct _libc_fpxreg {
    unsigned short significand[4];
    unsigned short exponent;
    unsigned short __glibc_reserved1[3];
};
struct _libc_xmmreg {
    unsigned int element[4];
};
struct _libc_fpstate {
    unsigned short cwd;
    unsigned short swd;
    unsigned short ftw;
    unsigned short fop;
    unsigned long rip;
    unsigned long rdp;
    unsigned int mxcsr;
    unsigned int mxcr_mask;
    struct _libc_fpxreg _st[8];
    struct _libc_xmmreg _xmm[16];
    unsigned int __glibc_reserved1[24];
};
typedef struct _libc_fpstate *fpregset_t;

typedef struct {
    gregset_t gregs;
    fpregset_t fpregs;
    unsigned long long __reserved1[8];
} mcontext_t;

typedef struct ucontext_t {
    unsigned long uc_flags;
    struct ucontext_t *uc_link;
    stack_t uc_stack;
    mcontext_t uc_mcontext;
    sigset_t uc_sigmask;
    struct _libc_fpstate __fpregs_mem;
    unsigned long long __ssp[4];
} ucontext_t;

#elif defined(__aarch64__)

/* Zaznam FP v __reserved (Linux fpsimd_context). */
#define FPSIMD_MAGIC 0x46508001
struct _aarch64_ctx {
    unsigned int magic;
    unsigned int size;
};
struct fpsimd_context {
    struct _aarch64_ctx head;
    unsigned int fpsr;
    unsigned int fpcr;
    __uint128_t vregs[32];
};

typedef struct {
    unsigned long long fault_address;
    unsigned long long regs[31];
    unsigned long long sp;
    unsigned long long pc;
    unsigned long long pstate;
    unsigned char __reserved[4096] __attribute__((__aligned__(16)));
} mcontext_t;

typedef struct ucontext_t {
    unsigned long uc_flags;
    struct ucontext_t *uc_link;
    stack_t uc_stack;
    sigset_t uc_sigmask;
    mcontext_t uc_mcontext;
} ucontext_t;

#endif

_CSTD_END

#endif
