/* limits.h - vestavena hlavicka clangu a meze libc (cstd). Clang bez
 * hostitelske libc nastavi MB_LEN_MAX na 1; UTF-8 potrebuje 4. PATH_MAX
 * a NAME_MAX jako Linux (std::filesystem v libc++). */
#ifndef _CSTD_LIMITS_H
#define _CSTD_LIMITS_H

#undef MB_LEN_MAX
#define MB_LEN_MAX 4
#include_next <limits.h>

#ifndef PATH_MAX
#define PATH_MAX 4096
#endif
#ifndef NAME_MAX
#define NAME_MAX 255
#endif

#endif
