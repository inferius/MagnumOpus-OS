/* ctype.h - tridy znaku (zaklad v cstd-core: ASCII) a varianty *_l
 * s locale (cstd). Tridy bajtu nezavisi na locale (bajty 0x80-0xFF nepatri
 * do zadne tridy ani v UTF-8). */
#ifndef _CSTD_POSIX_CTYPE_H
#define _CSTD_POSIX_CTYPE_H

#include_next <ctype.h>

_CSTD_BEGIN

#ifndef _CSTD_LOCALE_T
#define _CSTD_LOCALE_T
typedef struct __locale_struct *locale_t;
#endif

int isalnum_l(int c, locale_t l);
int isalpha_l(int c, locale_t l);
int isblank_l(int c, locale_t l);
int iscntrl_l(int c, locale_t l);
int isdigit_l(int c, locale_t l);
int isgraph_l(int c, locale_t l);
int islower_l(int c, locale_t l);
int isprint_l(int c, locale_t l);
int ispunct_l(int c, locale_t l);
int isspace_l(int c, locale_t l);
int isupper_l(int c, locale_t l);
int isxdigit_l(int c, locale_t l);
int tolower_l(int c, locale_t l);
int toupper_l(int c, locale_t l);

_CSTD_END

#endif
