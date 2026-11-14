/* ctype.h - tridy znaku ASCII, locale "C" (cstd). */
#ifndef _CTYPE_H
#define _CTYPE_H

#include "_cstd.h"

_CSTD_BEGIN

int isdigit(int c);
int isxdigit(int c);
int isalpha(int c);
int isalnum(int c);
int isupper(int c);
int islower(int c);
int isspace(int c);
int isprint(int c);
int isgraph(int c);
int iscntrl(int c);
int ispunct(int c);
int toupper(int c);
int tolower(int c);

_CSTD_END

#endif
