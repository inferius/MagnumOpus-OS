/* fenv.h - prostredi plovouci carky (cstd, docs/cs/cstd.md, src/fenv.rs).
 * Hodnoty a rozlozeni fenv_t jako glibc / musl na dane architekture
 * (test fenv_header_matches). Pasti vyjimek se nezapinaji. */
#ifndef _CSTD_FENV_H
#define _CSTD_FENV_H

#include <_cstd.h>

_CSTD_BEGIN

#if defined(__x86_64__)

#define FE_INVALID 1
#define __FE_DENORM 2
#define FE_DIVBYZERO 4
#define FE_OVERFLOW 8
#define FE_UNDERFLOW 16
#define FE_INEXACT 32
#define FE_ALL_EXCEPT 63

#define FE_TONEAREST 0
#define FE_DOWNWARD 0x400
#define FE_UPWARD 0x800
#define FE_TOWARDZERO 0xc00

typedef unsigned short fexcept_t;

/* Prostredi x87 (fnstenv, 28 B) a MXCSR. */
typedef struct {
    unsigned int __x87[7];
    unsigned int __mxcsr;
} fenv_t;

#elif defined(__aarch64__)

#define FE_INVALID 1
#define FE_DIVBYZERO 2
#define FE_OVERFLOW 4
#define FE_UNDERFLOW 8
#define FE_INEXACT 16
#define FE_ALL_EXCEPT 31

#define FE_TONEAREST 0
#define FE_UPWARD 0x400000
#define FE_DOWNWARD 0x800000
#define FE_TOWARDZERO 0xc00000

typedef unsigned int fexcept_t;

typedef struct {
    unsigned int __fpcr;
    unsigned int __fpsr;
} fenv_t;

#else
#error "fenv.h: nepodporovana architektura"
#endif

#define FE_DFL_ENV ((const fenv_t *)-1)

int feclearexcept(int excepts);
int fegetexceptflag(fexcept_t *flagp, int excepts);
int feraiseexcept(int excepts);
int fesetexceptflag(const fexcept_t *flagp, int excepts);
int fetestexcept(int excepts);
int fegetround(void);
int fesetround(int round);
int fegetenv(fenv_t *envp);
int feholdexcept(fenv_t *envp);
int fesetenv(const fenv_t *envp);
int feupdateenv(const fenv_t *envp);

_CSTD_END

#endif
