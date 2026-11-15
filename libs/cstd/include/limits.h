/*
 * limits.h - meze typu (clang) a meze systemu (cstd, docs/cs/cstd.md).
 */
#ifndef _CSTD_LIMITS_H
#define _CSTD_LIMITS_H

#include_next <limits.h>

/* Nejdelsi cesta vcetne nuly a nejdelsi jmeno polozky (pathconf). */
#ifndef PATH_MAX
#define PATH_MAX 4096
#endif
#ifndef NAME_MAX
#define NAME_MAX 255
#endif

#endif
