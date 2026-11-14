/*
 * math.h - matematicke funkce (cstd nad crate libm, port musl libm).
 * Funkce *l (long double) pocitaji v presnosti double (docs/cs/cstd.md).
 * Vyjimky FPU a errno funkce math.h nenastavuji (math_errhandling = 0).
 */
#ifndef _CSTD_MATH_H
#define _CSTD_MATH_H

#include <_cstd.h>

_CSTD_BEGIN

typedef double double_t;
typedef float float_t;

#define HUGE_VAL __builtin_huge_val()
#define HUGE_VALF __builtin_huge_valf()
#define HUGE_VALL __builtin_huge_vall()
#define INFINITY __builtin_inff()
#define NAN __builtin_nanf("")
#define math_errhandling 0
#define MATH_ERRNO 1
#define MATH_ERREXCEPT 2

#define FP_NAN 0
#define FP_INFINITE 1
#define FP_ZERO 2
#define FP_SUBNORMAL 3
#define FP_NORMAL 4
#define FP_ILOGB0 (-2147483647 - 1)
#define FP_ILOGBNAN (-2147483647 - 1)

#define fpclassify(x) __builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, x)
#define isnan(x) __builtin_isnan(x)
#define isinf(x) __builtin_isinf_sign(x)
#define isfinite(x) __builtin_isfinite(x)
#define isnormal(x) __builtin_isnormal(x)
#define signbit(x) __builtin_signbit(x)
#define isgreater(x, y) __builtin_isgreater(x, y)
#define isgreaterequal(x, y) __builtin_isgreaterequal(x, y)
#define isless(x, y) __builtin_isless(x, y)
#define islessequal(x, y) __builtin_islessequal(x, y)
#define islessgreater(x, y) __builtin_islessgreater(x, y)
#define isunordered(x, y) __builtin_isunordered(x, y)

#define M_E 2.7182818284590452354
#define M_LOG2E 1.4426950408889634074
#define M_LOG10E 0.43429448190325182765
#define M_LN2 0.69314718055994530942
#define M_LN10 2.30258509299404568402
#define M_PI 3.14159265358979323846
#define M_PI_2 1.57079632679489661923
#define M_PI_4 0.78539816339744830962
#define M_1_PI 0.31830988618379067154
#define M_2_PI 0.63661977236758134308
#define M_2_SQRTPI 1.12837916709551257390
#define M_SQRT2 1.41421356237309504880
#define M_SQRT1_2 0.70710678118654752440

#define _CSTD_M1(n) double n(double); float n##f(float); long double n##l(long double);
#define _CSTD_M2(n) double n(double, double); float n##f(float, float); long double n##l(long double, long double);

_CSTD_M1(acos) _CSTD_M1(asin) _CSTD_M1(atan) _CSTD_M1(cos) _CSTD_M1(sin) _CSTD_M1(tan)
_CSTD_M1(acosh) _CSTD_M1(asinh) _CSTD_M1(atanh) _CSTD_M1(cosh) _CSTD_M1(sinh) _CSTD_M1(tanh)
_CSTD_M1(exp) _CSTD_M1(exp2) _CSTD_M1(expm1) _CSTD_M1(log) _CSTD_M1(log10) _CSTD_M1(log1p)
_CSTD_M1(log2) _CSTD_M1(cbrt) _CSTD_M1(sqrt) _CSTD_M1(ceil) _CSTD_M1(floor) _CSTD_M1(trunc)
_CSTD_M1(round) _CSTD_M1(rint) _CSTD_M1(nearbyint) _CSTD_M1(fabs) _CSTD_M1(erf) _CSTD_M1(erfc)
_CSTD_M1(tgamma) _CSTD_M1(lgamma)
_CSTD_M2(atan2) _CSTD_M2(pow) _CSTD_M2(fmod) _CSTD_M2(hypot) _CSTD_M2(fmin) _CSTD_M2(fmax)
_CSTD_M2(fdim) _CSTD_M2(copysign) _CSTD_M2(remainder) _CSTD_M2(nextafter)

double exp10(double x);
float exp10f(float x);
double logb(double x);
long double logbl(long double x);
double fma(double x, double y, double z);
float fmaf(float x, float y, float z);
long double fmal(long double x, long double y, long double z);
double frexp(double x, int *e);
float frexpf(float x, int *e);
long double frexpl(long double x, int *e);
double ldexp(double x, int e);
float ldexpf(float x, int e);
long double ldexpl(long double x, int e);
double scalbn(double x, int e);
float scalbnf(float x, int e);
long double scalbnl(long double x, int e);
double modf(double x, double *ip);
float modff(float x, float *ip);
long double modfl(long double x, long double *ip);
double remquo(double x, double y, int *q);
long double remquol(long double x, long double y, int *q);
int ilogb(double x);
int ilogbl(long double x);
long lround(double x);
long lroundf(float x);
long lroundl(long double x);
long long llround(double x);
long long llroundl(long double x);
long lrint(double x);
long lrintf(float x);
long lrintl(long double x);
long long llrint(double x);
long long llrintl(long double x);
double nan(const char *tag);
float nanf(const char *tag);
long double nanl(const char *tag);
double j0(double x);
double j1(double x);
double jn(int n, double x);
double y0(double x);
double y1(double x);
double yn(int n, double x);
float j0f(float x);
float j1f(float x);
float y0f(float x);
float y1f(float x);
void sincos(double x, double *s, double *c);

#undef _CSTD_M1
#undef _CSTD_M2

_CSTD_END

#endif
