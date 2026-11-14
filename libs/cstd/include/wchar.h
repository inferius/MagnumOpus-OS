/* wchar.h - siroke znaky (cstd, docs/cs/cstd.md, kapitola Siroke znaky
 * a locale). wchar_t ma 32 bitu (UTF-32) na obou architekturach (x86_64 se
 * znamenkem, aarch64 bez, jako Linux). Prevody vicebajtovych znaku podle
 * LC_CTYPE: locale C je jednobajtove (bajty 0x80-0xFF <-> U+DF80-U+DFFF),
 * C.UTF-8 a ostatni jmena UTF-8. Implementace: libs/cstd/src/wchar/. */
#ifndef _CSTD_WCHAR_H
#define _CSTD_WCHAR_H

#include <_cstd.h>
#define __need_size_t
#define __need_wchar_t
#define __need_NULL
#include <stddef.h>
#include <stdarg.h>
#include <stdint.h>

_CSTD_BEGIN

#ifndef _CSTD_WINT_T
#define _CSTD_WINT_T
typedef unsigned int wint_t;
#endif

/* Stav prevodu vicebajtovych znaku (UTF-8: rozpracovany znak). Nuly =
 * pocatecni stav. */
#ifndef _CSTD_MBSTATE_T
#define _CSTD_MBSTATE_T
typedef struct {
    unsigned int __count;
    unsigned int __value;
} mbstate_t;
#endif

#ifndef _CSTD_LOCALE_T
#define _CSTD_LOCALE_T
typedef struct __locale_struct *locale_t;
#endif

#define WEOF ((wint_t)0xFFFFFFFFu)

#ifndef WCHAR_MIN
#define WCHAR_MIN __WCHAR_MIN__
#endif
#ifndef WCHAR_MAX
#define WCHAR_MAX __WCHAR_MAX__
#endif

typedef struct FILE FILE;
struct tm;

/* Prevody (stavove a bezstavove). */
wint_t btowc(int c);
int wctob(wint_t c);
int mbsinit(const mbstate_t *ps);
size_t mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps);
size_t mbrlen(const char *s, size_t n, mbstate_t *ps);
size_t wcrtomb(char *s, wchar_t wc, mbstate_t *ps);
size_t mbsrtowcs(wchar_t *dst, const char **src, size_t len, mbstate_t *ps);
size_t mbsnrtowcs(wchar_t *dst, const char **src, size_t nms, size_t len, mbstate_t *ps);
size_t wcsrtombs(char *dst, const wchar_t **src, size_t len, mbstate_t *ps);
size_t wcsnrtombs(char *dst, const wchar_t **src, size_t nwc, size_t len, mbstate_t *ps);

/* Retezce. */
size_t wcslen(const wchar_t *s);
size_t wcsnlen(const wchar_t *s, size_t n);
wchar_t *wcscpy(wchar_t *d, const wchar_t *s);
wchar_t *wcpcpy(wchar_t *d, const wchar_t *s);
wchar_t *wcsncpy(wchar_t *d, const wchar_t *s, size_t n);
wchar_t *wcpncpy(wchar_t *d, const wchar_t *s, size_t n);
wchar_t *wcscat(wchar_t *d, const wchar_t *s);
wchar_t *wcsncat(wchar_t *d, const wchar_t *s, size_t n);
size_t wcslcpy(wchar_t *d, const wchar_t *s, size_t n);
size_t wcslcat(wchar_t *d, const wchar_t *s, size_t n);
int wcscmp(const wchar_t *a, const wchar_t *b);
int wcsncmp(const wchar_t *a, const wchar_t *b, size_t n);
int wcscasecmp(const wchar_t *a, const wchar_t *b);
int wcsncasecmp(const wchar_t *a, const wchar_t *b, size_t n);
int wcscoll(const wchar_t *a, const wchar_t *b);
int wcscoll_l(const wchar_t *a, const wchar_t *b, locale_t l);
size_t wcsxfrm(wchar_t *d, const wchar_t *s, size_t n);
size_t wcsxfrm_l(wchar_t *d, const wchar_t *s, size_t n, locale_t l);
wchar_t *wcschr(const wchar_t *s, wchar_t c);
wchar_t *wcsrchr(const wchar_t *s, wchar_t c);
size_t wcsspn(const wchar_t *s, const wchar_t *set);
size_t wcscspn(const wchar_t *s, const wchar_t *set);
wchar_t *wcspbrk(const wchar_t *s, const wchar_t *set);
wchar_t *wcsstr(const wchar_t *h, const wchar_t *n);
wchar_t *wcstok(wchar_t *s, const wchar_t *delim, wchar_t **save);
wchar_t *wcsdup(const wchar_t *s);
wchar_t *wmemchr(const wchar_t *s, wchar_t c, size_t n);
int wmemcmp(const wchar_t *a, const wchar_t *b, size_t n);
wchar_t *wmemcpy(wchar_t *d, const wchar_t *s, size_t n);
wchar_t *wmemmove(wchar_t *d, const wchar_t *s, size_t n);
wchar_t *wmemset(wchar_t *d, wchar_t c, size_t n);
int wcwidth(wchar_t c);
int wcswidth(const wchar_t *s, size_t n);

/* Cisla a cas. */
long wcstol(const wchar_t *s, wchar_t **end, int base);
long long wcstoll(const wchar_t *s, wchar_t **end, int base);
unsigned long wcstoul(const wchar_t *s, wchar_t **end, int base);
unsigned long long wcstoull(const wchar_t *s, wchar_t **end, int base);
double wcstod(const wchar_t *s, wchar_t **end);
float wcstof(const wchar_t *s, wchar_t **end);
long double wcstold(const wchar_t *s, wchar_t **end);
size_t wcsftime(wchar_t *buf, size_t max, const wchar_t *fmt, const struct tm *t);

/* Vstup a vystup. */
wint_t fgetwc(FILE *f);
wint_t getwc(FILE *f);
wint_t getwchar(void);
wint_t fputwc(wchar_t c, FILE *f);
wint_t putwc(wchar_t c, FILE *f);
wint_t putwchar(wchar_t c);
wint_t ungetwc(wint_t c, FILE *f);
wchar_t *fgetws(wchar_t *buf, int n, FILE *f);
int fputws(const wchar_t *s, FILE *f);
int fwide(FILE *f, int mode);
int wprintf(const wchar_t *fmt, ...);
int fwprintf(FILE *f, const wchar_t *fmt, ...);
int swprintf(wchar_t *buf, size_t n, const wchar_t *fmt, ...);
int vwprintf(const wchar_t *fmt, va_list ap);
int vfwprintf(FILE *f, const wchar_t *fmt, va_list ap);
int vswprintf(wchar_t *buf, size_t n, const wchar_t *fmt, va_list ap);
int wscanf(const wchar_t *fmt, ...);
int fwscanf(FILE *f, const wchar_t *fmt, ...);
int swscanf(const wchar_t *s, const wchar_t *fmt, ...);
int vwscanf(const wchar_t *fmt, va_list ap);
int vfwscanf(FILE *f, const wchar_t *fmt, va_list ap);
int vswscanf(const wchar_t *s, const wchar_t *fmt, va_list ap);

_CSTD_END

#endif
