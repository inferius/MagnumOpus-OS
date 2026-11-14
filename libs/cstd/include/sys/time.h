/* sys/time.h - gettimeofday (cstd). */
#ifndef _CSTD_SYS_TIME_H
#define _CSTD_SYS_TIME_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

struct timeval {
    time_t tv_sec;
    suseconds_t tv_usec;
};

int gettimeofday(struct timeval *tv, void *tz);

_CSTD_END

#endif
