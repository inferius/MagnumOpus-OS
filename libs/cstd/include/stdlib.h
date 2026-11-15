/* stdlib.h - konec procesu a prostredi (cstd nad std); zaklad v cstd-core. */
#ifndef _CSTD_POSIX_STDLIB_H
#define _CSTD_POSIX_STDLIB_H

#include_next <stdlib.h>

_CSTD_BEGIN

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1

_CSTD_NORETURN void exit(int code);
_CSTD_NORETURN void _Exit(int code);
int atexit(void (*f)(void));
char *getenv(const char *name);
int setenv(const char *name, const char *value, int overwrite);
int unsetenv(const char *name);
double strtod(const char *s, char **end);
float strtof(const char *s, char **end);
long double strtold(const char *s, char **end);
double atof(const char *s);

#define RAND_MAX 2147483647

typedef struct {
    int quot;
    int rem;
} div_t;
typedef struct {
    long quot;
    long rem;
} ldiv_t;
typedef struct {
    long long quot;
    long long rem;
} lldiv_t;

void qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *));
void *bsearch(const void *key, const void *base, size_t n, size_t size, int (*cmp)(const void *, const void *));
long labs(long v);
long long llabs(long long v);
div_t div(int a, int b);
ldiv_t ldiv(long a, long b);
lldiv_t lldiv(long long a, long long b);
long atol(const char *s);
long long atoll(const char *s);
int rand(void);
void srand(unsigned seed);
long random(void);
void srandom(unsigned seed);
int system(const char *cmd);

/* Absolutni cesta bez . a .. (resolved NULL = malloc). */
char *realpath(const char *path, char *resolved);
/* Vicebajtove znaky podle LC_CTYPE (wchar.h ma stavove varianty). */
size_t __ctype_get_mb_cur_max(void);
#define MB_CUR_MAX (__ctype_get_mb_cur_max())
int mblen(const char *s, size_t n);
int mbtowc(wchar_t *pwc, const char *s, size_t n);
int wctomb(char *s, wchar_t wc);
size_t mbstowcs(wchar_t *dst, const char *src, size_t n);
size_t wcstombs(char *dst, const wchar_t *src, size_t n);

#ifndef _CSTD_LOCALE_T
#define _CSTD_LOCALE_T
typedef struct __locale_struct *locale_t;
#endif
double strtod_l(const char *s, char **end, locale_t l);
float strtof_l(const char *s, char **end, locale_t l);
long double strtold_l(const char *s, char **end, locale_t l);
long strtol_l(const char *s, char **end, int base, locale_t l);
long long strtoll_l(const char *s, char **end, int base, locale_t l);
unsigned long strtoul_l(const char *s, char **end, int base, locale_t l);
unsigned long long strtoull_l(const char *s, char **end, int base, locale_t l);

_CSTD_END

#endif
