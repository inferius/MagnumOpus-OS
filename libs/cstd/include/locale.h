/* locale.h - jen locale C a C.UTF-8 (cstd, docs/cs/cstd.md). */
#ifndef _CSTD_LOCALE_H
#define _CSTD_LOCALE_H

#include <_cstd.h>

_CSTD_BEGIN

#define LC_CTYPE 0
#define LC_NUMERIC 1
#define LC_TIME 2
#define LC_COLLATE 3
#define LC_MONETARY 4
#define LC_MESSAGES 5
#define LC_ALL 6

struct lconv {
    char *decimal_point;
    char *thousands_sep;
    char *grouping;
    char *__rest[7];
    char __chars[14];
};

char *setlocale(int category, const char *name);
struct lconv *localeconv(void);

_CSTD_END

#endif
