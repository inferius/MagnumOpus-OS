/* sched.h - sched_yield a priority planovani (cstd, docs/cs/cstd.md).
 * Planovac MagnumOpus zatim nedava aplikacim realny cas: jen SCHED_OTHER
 * s prioritou 0; zadost o SCHED_FIFO / SCHED_RR = EPERM (jako Linux bez
 * opravneni). */
#ifndef _CSTD_SCHED_H
#define _CSTD_SCHED_H

#include <_cstd.h>

_CSTD_BEGIN

#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2

struct sched_param {
    int sched_priority;
};

int sched_yield(void);
int sched_get_priority_min(int policy);
int sched_get_priority_max(int policy);

_CSTD_END

#endif
