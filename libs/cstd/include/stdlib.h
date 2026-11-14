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

_CSTD_END

#endif
