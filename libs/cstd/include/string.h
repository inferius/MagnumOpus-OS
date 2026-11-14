/* string.h - doplnek cstd nad std (strerror); zaklad v cstd-core. */
#ifndef _CSTD_POSIX_STRING_H
#define _CSTD_POSIX_STRING_H

#include_next <string.h>

_CSTD_BEGIN

char *strerror(int e);

_CSTD_END

#endif
