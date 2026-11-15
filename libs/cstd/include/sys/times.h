/* sys/times.h - times() (cstd). Cas CPU jadro zatim nedava: pole tms
 * jsou 0, navrat = uplynuly cas v tiku sysconf(_SC_CLK_TCK) = 100. */
#ifndef _CSTD_SYS_TIMES_H
#define _CSTD_SYS_TIMES_H

#include <_cstd.h>
#include <time.h>

_CSTD_BEGIN

struct tms {
    clock_t tms_utime;
    clock_t tms_stime;
    clock_t tms_cutime;
    clock_t tms_cstime;
};

clock_t times(struct tms *out);

_CSTD_END

#endif
