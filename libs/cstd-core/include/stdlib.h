/* stdlib.h - pamet, prevody cisel, abort (cstd). */
#ifndef _STDLIB_H
#define _STDLIB_H

#include <stddef.h>
#include "_cstd.h"

_CSTD_BEGIN

void *malloc(size_t size);
void *calloc(size_t count, size_t size);
void *realloc(void *p, size_t size);
void free(void *p);
/* Zarovnane bloky (align mocnina 2; posix_memalign navic nasobek sizeof(void *)). */
int posix_memalign(void **out, size_t align, size_t size);
void *aligned_alloc(size_t align, size_t size);
void *memalign(size_t align, size_t size);

unsigned long strtoul(const char *s, char **end, int base);
unsigned long long strtoull(const char *s, char **end, int base);
long strtol(const char *s, char **end, int base);
long long strtoll(const char *s, char **end, int base);
int atoi(const char *s);
int abs(int v);

_CSTD_NORETURN void abort(void);

_CSTD_END

#endif
