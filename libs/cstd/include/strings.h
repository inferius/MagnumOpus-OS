/* strings.h - porovnani bez ohledu na velikost pismen (cstd). */
#ifndef _CSTD_STRINGS_H
#define _CSTD_STRINGS_H

#include <_cstd.h>
#include <stddef.h>

_CSTD_BEGIN

int strcasecmp(const char *a, const char *b);
/* Index nejnizsiho nastaveneho bitu od 1 (0 = zadny). */
int ffs(int v);
int ffsl(long v);
int ffsll(long long v);
int strncasecmp(const char *a, const char *b, size_t n);

_CSTD_END

#endif
