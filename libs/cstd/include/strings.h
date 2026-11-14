/* strings.h - porovnani bez ohledu na velikost pismen (cstd). */
#ifndef _CSTD_STRINGS_H
#define _CSTD_STRINGS_H

#include <_cstd.h>
#include <stddef.h>

_CSTD_BEGIN

int strcasecmp(const char *a, const char *b);
int strncasecmp(const char *a, const char *b, size_t n);

_CSTD_END

#endif
