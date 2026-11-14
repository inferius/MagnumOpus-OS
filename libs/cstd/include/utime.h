/* utime.h - casy souboru (cstd): bez ucinku, uspech kdyz soubor existuje. */
#ifndef _CSTD_UTIME_H
#define _CSTD_UTIME_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

struct utimbuf {
    time_t actime;
    time_t modtime;
};

int utime(const char *path, const struct utimbuf *t);

_CSTD_END

#endif
