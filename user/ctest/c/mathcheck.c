/*
 * Test casti 2d knihovny cstd: math.h (libm), long double (presnost
 * double), plovouci carka v printf a strtod; fenv.h (zaokrouhleni,
 * priznaky vyjimek, ulozeni prostredi).
 * Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <fenv.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int near(double a, double b) { return fabs(a - b) <= 1e-12 * (fabs(b) > 1 ? fabs(b) : 1); }

static int same(const char *got, const char *want) { return strcmp(got, want) == 0; }

int math_test(void) {
    /* volatile: prekladac nesmi vysledek spocitat sam (builtin). */
    volatile double x = 2.0, y = 0.5;
    if (!near(sqrt(x), M_SQRT2) || !near(sin(M_PI / 6), 0.5) || !near(cos(0.0), 1.0)) return 1;
    if (!near(pow(x, 10), 1024) || !near(exp(log(x * 21)), 42) || !near(atan2(1, 1), M_PI_4)) return 2;
    if (floor(-y) != -1.0 || ceil(y) != 1.0 || round(2.5) != 3.0 || trunc(-2.7) != -2.0) return 3;
    if (!near(fmod(7.5, 2), 1.5) || !near(hypot(3, 4), 5) || !near(cbrt(27.0), 3)) return 4;
    int e;
    if (frexp(48.0, &e) != 0.75 || e != 6 || ldexp(0.75, 6) != 48.0) return 5;
    double ip;
    if (modf(-3.25, &ip) != -0.25 || ip != -3.0) return 6;
    if (fabsf(sinf((float)M_PI_2) - 1.0f) > 1e-6f || sqrtf(16.0f) != 4.0f || powf(2.0f, 3.0f) != 8.0f) return 7;
    if (!isnan(sqrt(-x)) || !isinf(log(0.0)) || !signbit(-0.0) || fpclassify(1e-310) != FP_SUBNORMAL) return 8;
    if (lround(2.5) != 3 || lrint(2.5) != 2 || ilogb(1024.0) != 10) return 9;

    /* long double: ABI jako Linux, vypocet v double. */
    volatile long double lx = 2.0L;
    if (fabsl(sqrtl(lx) - 1.4142135623730951L) > 1e-15L || powl(lx, 3.0L) != 8.0L) return 10;
    char *end;
    if (strtold("1.5e2x", &end) != 150.0L || *end != 'x') return 11;

    /* printf s plovouci carkou (vystup jako glibc; 0.000125 je binarne o kousek vic, proto 1.3e-04). */
    char b[128];
    snprintf(b, sizeof b, "%.3f|%e|%g|%g|%a", 3.14159, 1234.5, 0.0001, 1e20, 1.0);
    if (!same(b, "3.142|1.234500e+03|0.0001|1e+20|0x1p+0")) return 20;
    snprintf(b, sizeof b, "%8.2f|%-8.1e|%+.0f|%G|%f", -2.5, 0.000125, 2.5, 1e-10, (double)1.5f);
    if (!same(b, "   -2.50|1.3e-04 |+2|1E-10|1.500000")) return 21;
    snprintf(b, sizeof b, "%f|%f|%d|%.2f", INFINITY, -NAN, 7, 0.125);
    if (!same(b, "inf|-nan|7|0.12")) return 22;

    /* strtod: konec cisla, sestnactkovy tvar, ERANGE. */
    if (strtod("  -12.5e-1kg", &end) != -1.25 || strcmp(end, "kg") != 0) return 30;
    if (strtod("0x1.8p1", NULL) != 3.0 || atof("42") != 42.0) return 31;
    errno = 0;
    if (!isinf(strtod("1e999", NULL)) || errno != ERANGE) return 32;
    errno = 0;
    if (strtod("nic", &end) != 0.0 || strcmp(end, "nic") != 0 || errno != 0) return 33;
    if (strtof("0.1", NULL) != 0.1f) return 34;
    /* fenv.h: zaokrouhleni meni vysledek deleni, priznaky se nastavi
       a smazou, feholdexcept/feupdateenv je zachova. */
    fenv_t saved;
    if (fegetenv(&saved) != 0 || fegetround() != FE_TONEAREST) return 40;
    volatile double one = 1.0, three = 3.0, zero = 0.0;
    if (fesetround(FE_UPWARD) != 0 || fegetround() != FE_UPWARD) return 41;
    double up = one / three;
    fesetround(FE_DOWNWARD);
    double down = one / three;
    if (!(up > down) || fesetround(12345) == 0) return 42;
    fesetround(FE_TONEAREST);
    feclearexcept(FE_ALL_EXCEPT);
    volatile double inf = one / zero;
    if (!isinf(inf) || !fetestexcept(FE_DIVBYZERO)) return 43;
    feclearexcept(FE_DIVBYZERO);
    if (fetestexcept(FE_DIVBYZERO)) return 44;
    fenv_t held;
    feraiseexcept(FE_OVERFLOW);
    if (feholdexcept(&held) != 0 || fetestexcept(FE_ALL_EXCEPT)) return 45;
    feraiseexcept(FE_INEXACT);
    feupdateenv(&held);
    if (!fetestexcept(FE_OVERFLOW) || !fetestexcept(FE_INEXACT)) return 46;
    fesetenv(FE_DFL_ENV);
    if (fegetround() != FE_TONEAREST || fetestexcept(FE_ALL_EXCEPT)) return 47;
    fesetenv(&saved);
    return 0;
}
