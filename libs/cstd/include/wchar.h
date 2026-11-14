/* wchar.h - zatim jen typy a konstanty (cstd, docs/cs/cstd.md, kapitola
 * Procesy, locale, wchar). wchar_t je 32bitovy (UTF-32) na obou
 * architekturach. Funkce wcs* a prevody (mbrtowc, ...) prijdou s plnou
 * podporou wchar; porty, ktere je hledaji (CMake check_symbol_exists,
 * configure), je zatim nenajdou a pouziji vlastni nahrady (SDL). */
#ifndef _CSTD_WCHAR_H
#define _CSTD_WCHAR_H

#include <_cstd.h>
#include <stddef.h>
#include <stdint.h>

_CSTD_BEGIN

#ifndef __WINT_TYPE__
typedef unsigned int wint_t;
#else
typedef __WINT_TYPE__ wint_t;
#endif

/* Stav prevodu vicebajtovych znaku (UTF-8: rozpracovany znak). */
typedef struct {
    unsigned int __count;
    unsigned int __value;
} mbstate_t;

#define WEOF ((wint_t)0xFFFFFFFFu)

#ifndef WCHAR_MIN
#define WCHAR_MIN __WCHAR_MIN__
#endif
#ifndef WCHAR_MAX
#define WCHAR_MAX __WCHAR_MAX__
#endif

_CSTD_END

#endif
