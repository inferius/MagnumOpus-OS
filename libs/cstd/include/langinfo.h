/* langinfo.h - nl_langinfo pro locale C a C.UTF-8 (cstd, docs/cs/cstd.md).
 * Cisla polozek musi sedet s src/misc.rs (modul nl, test
 * langinfo_header_matches). */
#ifndef _CSTD_LANGINFO_H
#define _CSTD_LANGINFO_H

#include <_cstd.h>

_CSTD_BEGIN

typedef int nl_item;

#define CODESET 0
#define D_T_FMT 1
#define D_FMT 2
#define T_FMT 3
#define T_FMT_AMPM 4
#define AM_STR 5
#define PM_STR 6
#define RADIXCHAR 7
#define THOUSEP 8
#define YESEXPR 9
#define NOEXPR 10
#define DAY_1 11
#define DAY_2 12
#define DAY_3 13
#define DAY_4 14
#define DAY_5 15
#define DAY_6 16
#define DAY_7 17
#define ABDAY_1 18
#define ABDAY_2 19
#define ABDAY_3 20
#define ABDAY_4 21
#define ABDAY_5 22
#define ABDAY_6 23
#define ABDAY_7 24
#define MON_1 25
#define MON_2 26
#define MON_3 27
#define MON_4 28
#define MON_5 29
#define MON_6 30
#define MON_7 31
#define MON_8 32
#define MON_9 33
#define MON_10 34
#define MON_11 35
#define MON_12 36
#define ABMON_1 37
#define ABMON_2 38
#define ABMON_3 39
#define ABMON_4 40
#define ABMON_5 41
#define ABMON_6 42
#define ABMON_7 43
#define ABMON_8 44
#define ABMON_9 45
#define ABMON_10 46
#define ABMON_11 47
#define ABMON_12 48
#define CRNCYSTR 49

#ifndef _CSTD_LOCALE_T
#define _CSTD_LOCALE_T
typedef struct __locale_struct *locale_t;
#endif
char *nl_langinfo(nl_item item);
char *nl_langinfo_l(nl_item item, locale_t l);

_CSTD_END

#endif
