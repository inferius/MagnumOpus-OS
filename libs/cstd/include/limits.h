/* limits.h - vestavena hlavicka clangu a MB_LEN_MAX pro UTF-8 (cstd).
 * Clang bez hostitelske libc nastavi MB_LEN_MAX na 1. */
#ifndef _CSTD_LIMITS_H
#define _CSTD_LIMITS_H

#undef MB_LEN_MAX
#define MB_LEN_MAX 4
#include_next <limits.h>

#endif
