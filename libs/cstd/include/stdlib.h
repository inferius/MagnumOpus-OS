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

void qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *));
void *bsearch(const void *key, const void *base, size_t n, size_t size, int (*cmp)(const void *, const void *));
long labs(long v);
long long llabs(long long v);
div_t div(int a, int b);
ldiv_t ldiv(long a, long b);
long atol(const char *s);
long long atoll(const char *s);
int rand(void);
void srand(unsigned seed);
long random(void);
void srandom(unsigned seed);
int system(const char *cmd);

_CSTD_END

#endif
