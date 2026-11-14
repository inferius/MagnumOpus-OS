/*
 * Funkce wchar a locale s long double (cstd): Rust typ long double nezna,
 * vypocet je v double jako strtold (c/math_ld.c).
 */
#include <locale.h>
#include <stdlib.h>
#include <wchar.h>

long double wcstold(const wchar_t *s, wchar_t **end) { return (long double)wcstod(s, end); }
long double strtold_l(const char *s, char **end, locale_t l) {
    (void)l;
    return (long double)strtod(s, end);
}
