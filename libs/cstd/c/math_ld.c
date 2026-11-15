/*
 * Funkce math.h pro long double a strtold (cstd, rozhodnuti 9A).
 *
 * ABI long double je jako Linux (x86_64: 80 bitu v x87, aarch64: 128 bitu),
 * ale vypocet probiha v double: vysledek ma presnost double. Psano v C,
 * protoze Rust typ long double nezna (prevod a predani resi prekladac).
 */
#include <math.h>
#include <stdlib.h>

#define LD1(name) \
    long double name##l(long double x) { return (long double)name((double)x); }
#define LD2(name) \
    long double name##l(long double x, long double y) { return (long double)name((double)x, (double)y); }

LD1(acos) LD1(asin) LD1(atan) LD1(cos) LD1(sin) LD1(tan)
LD1(acosh) LD1(asinh) LD1(atanh) LD1(cosh) LD1(sinh) LD1(tanh)
LD1(exp) LD1(exp2) LD1(expm1) LD1(log) LD1(log10) LD1(log1p) LD1(log2)
LD1(cbrt) LD1(sqrt) LD1(ceil) LD1(floor) LD1(trunc) LD1(round) LD1(rint)
LD1(nearbyint) LD1(fabs) LD1(erf) LD1(erfc) LD1(tgamma) LD1(lgamma) LD1(logb)
LD2(atan2) LD2(pow) LD2(fmod) LD2(hypot) LD2(fmin) LD2(fmax) LD2(fdim)
LD2(copysign) LD2(remainder) LD2(nextafter)

long double fmal(long double x, long double y, long double z) {
    return (long double)fma((double)x, (double)y, (double)z);
}
long double frexpl(long double x, int *e) { return (long double)frexp((double)x, e); }
long double ldexpl(long double x, int e) { return (long double)ldexp((double)x, e); }
long double scalbnl(long double x, int e) { return (long double)scalbn((double)x, e); }
int ilogbl(long double x) { return ilogb((double)x); }
long lroundl(long double x) { return lround((double)x); }
long lrintl(long double x) { return lrint((double)x); }
long long llroundl(long double x) { return llround((double)x); }
long long llrintl(long double x) { return llrint((double)x); }
long double nanl(const char *tag) { return (long double)nan(tag); }
long double modfl(long double x, long double *ip) {
    double i;
    double f = modf((double)x, &i);
    if (ip) *ip = (long double)i;
    return (long double)f;
}
long double remquol(long double x, long double y, int *q) {
    return (long double)remquo((double)x, (double)y, q);
}
long double strtold(const char *s, char **end) { return (long double)strtod(s, end); }

/* wchar.h a locale.h s long double (stejny objekt jako strtold: libc.so
 * vtahuje z archivu jen objekty, na ktere neco odkazuje). */
#include <locale.h>
#include <wchar.h>

long double wcstold(const wchar_t *s, wchar_t **end) { return (long double)wcstod(s, end); }
long double strtold_l(const char *s, char **end, locale_t l) {
    (void)l;
    return (long double)strtod(s, end);
}
