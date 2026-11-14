/* sys/resource.h - spotreba a limity (cstd): jadro je zatim nehlasi. */
#ifndef _CSTD_SYS_RESOURCE_H
#define _CSTD_SYS_RESOURCE_H

#include <_cstd.h>
#include <sys/time.h>

_CSTD_BEGIN

#define RUSAGE_SELF 0
#define RUSAGE_CHILDREN (-1)
#define RLIMIT_NOFILE 7
#define RLIMIT_STACK 3
#define RLIM_INFINITY (~0UL)

typedef unsigned long rlim_t;

struct rusage {
    struct timeval ru_utime;
    struct timeval ru_stime;
    long __rest[14];
};

struct rlimit {
    rlim_t rlim_cur;
    rlim_t rlim_max;
};

int getrusage(int who, struct rusage *u);
int getrlimit(int res, struct rlimit *r);
int setrlimit(int res, const struct rlimit *r);

_CSTD_END

#endif
