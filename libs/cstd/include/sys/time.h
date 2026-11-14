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

struct itimerval {
    struct timeval it_interval;
    struct timeval it_value;
};
#define ITIMER_REAL 0
int setitimer(int which, const struct itimerval *nv, struct itimerval *ov);
int utimes(const char *path, const struct timeval tv[2]);

_CSTD_END

#endif
