/* wctype.h - tridy sirokych znaku a prevody velikosti pismen podle Unicode
 * (tabulky z UCD, stejna verze jako core::char v Rustu; libs/uniprop).
 * Nezavisi na locale; iswdigit a iswxdigit jsou jen ASCII. */
#ifndef _CSTD_WCTYPE_H
#define _CSTD_WCTYPE_H

#include <_cstd.h>
#define __need_wchar_t
#include <stddef.h>

_CSTD_BEGIN

#ifndef _CSTD_WINT_T
#define _CSTD_WINT_T
typedef unsigned int wint_t;
#endif

#ifndef _CSTD_LOCALE_T
#define _CSTD_LOCALE_T
typedef struct __locale_struct *locale_t;
#endif

typedef unsigned long wctype_t;
typedef const int *wctrans_t;

#ifndef WEOF
#define WEOF ((wint_t)0xFFFFFFFFu)
#endif

int iswalnum(wint_t c);
int iswalpha(wint_t c);
int iswblank(wint_t c);
int iswcntrl(wint_t c);
int iswdigit(wint_t c);
int iswgraph(wint_t c);
int iswlower(wint_t c);
int iswprint(wint_t c);
int iswpunct(wint_t c);
int iswspace(wint_t c);
int iswupper(wint_t c);
int iswxdigit(wint_t c);
wint_t towlower(wint_t c);
wint_t towupper(wint_t c);
wctype_t wctype(const char *name);
int iswctype(wint_t c, wctype_t t);
wctrans_t wctrans(const char *name);
wint_t towctrans(wint_t c, wctrans_t t);

int iswalnum_l(wint_t c, locale_t l);
int iswalpha_l(wint_t c, locale_t l);
int iswblank_l(wint_t c, locale_t l);
int iswcntrl_l(wint_t c, locale_t l);
int iswdigit_l(wint_t c, locale_t l);
int iswgraph_l(wint_t c, locale_t l);
int iswlower_l(wint_t c, locale_t l);
int iswprint_l(wint_t c, locale_t l);
int iswpunct_l(wint_t c, locale_t l);
int iswspace_l(wint_t c, locale_t l);
int iswupper_l(wint_t c, locale_t l);
int iswxdigit_l(wint_t c, locale_t l);
wint_t towlower_l(wint_t c, locale_t l);
wint_t towupper_l(wint_t c, locale_t l);
wctype_t wctype_l(const char *name, locale_t l);
int iswctype_l(wint_t c, wctype_t t, locale_t l);
wctrans_t wctrans_l(const char *name, locale_t l);
wint_t towctrans_l(wint_t c, wctrans_t t, locale_t l);

_CSTD_END

#endif
