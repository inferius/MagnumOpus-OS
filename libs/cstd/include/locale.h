/* locale.h - locale procesu a vlaken (cstd, docs/cs/cstd.md, kapitola
 * Siroke znaky a locale). "C"/"POSIX" je jednobajtove, "C.UTF-8" a jina
 * jmena bez ciziho kodovani UTF-8; kategorie se zatim nelokalizuji.
 * Cisla LC_* a masky musi sedet s src/locale/mod.rs (test
 * locale_header_matches). */
#ifndef _CSTD_LOCALE_H
#define _CSTD_LOCALE_H

#include <_cstd.h>
#define __need_NULL
#include <stddef.h>

_CSTD_BEGIN

#define LC_CTYPE 0
#define LC_NUMERIC 1
#define LC_TIME 2
#define LC_COLLATE 3
#define LC_MONETARY 4
#define LC_MESSAGES 5
#define LC_ALL 6

#define LC_CTYPE_MASK (1 << LC_CTYPE)
#define LC_NUMERIC_MASK (1 << LC_NUMERIC)
#define LC_TIME_MASK (1 << LC_TIME)
#define LC_COLLATE_MASK (1 << LC_COLLATE)
#define LC_MONETARY_MASK (1 << LC_MONETARY)
#define LC_MESSAGES_MASK (1 << LC_MESSAGES)
#define LC_ALL_MASK 0x3f

struct lconv {
    char *decimal_point;
    char *thousands_sep;
    char *grouping;
    char *int_curr_symbol;
    char *currency_symbol;
    char *mon_decimal_point;
    char *mon_thousands_sep;
    char *mon_grouping;
    char *positive_sign;
    char *negative_sign;
    char int_frac_digits;
    char frac_digits;
    char p_cs_precedes;
    char p_sep_by_space;
    char n_cs_precedes;
    char n_sep_by_space;
    char p_sign_posn;
    char n_sign_posn;
    char int_p_cs_precedes;
    char int_p_sep_by_space;
    char int_n_cs_precedes;
    char int_n_sep_by_space;
    char int_p_sign_posn;
    char int_n_sign_posn;
};

#ifndef _CSTD_LOCALE_T
#define _CSTD_LOCALE_T
typedef struct __locale_struct *locale_t;
#endif

#define LC_GLOBAL_LOCALE ((locale_t)-1)

char *setlocale(int category, const char *name);
struct lconv *localeconv(void);
locale_t newlocale(int mask, const char *name, locale_t base);
locale_t duplocale(locale_t l);
void freelocale(locale_t l);
locale_t uselocale(locale_t l);

_CSTD_END

#endif
