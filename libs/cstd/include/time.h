/*
 * time.h - hodiny, spanek a kalendar (cstd, docs/cs/cstd.md).
 * Mistni cas podle promenne TZ (jmeno IANA nebo UTC+N), jinak UTC.
 */
#ifndef _CSTD_TIME_H
#define _CSTD_TIME_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

typedef long clock_t;

#define CLOCKS_PER_SEC 1000000L
#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#define CLOCK_PROCESS_CPUTIME_ID 2
#define CLOCK_THREAD_CPUTIME_ID 3
#define CLOCK_MONOTONIC_RAW 4
#define CLOCK_REALTIME_COARSE 5
#define CLOCK_MONOTONIC_COARSE 6
#define CLOCK_BOOTTIME 7
#define TIMER_ABSTIME 1

struct tm {
    int tm_sec;
    int tm_min;
    int tm_hour;
    int tm_mday;
    int tm_mon;
    int tm_year;
    int tm_wday;
    int tm_yday;
    int tm_isdst;
    long tm_gmtoff;
    const char *tm_zone;
};

int clock_gettime(clockid_t clock, struct timespec *out);
int clock_getres(clockid_t clock, struct timespec *out);
int clock_nanosleep(clockid_t clock, int flags, const struct timespec *req, struct timespec *rem);
int nanosleep(const struct timespec *req, struct timespec *rem);
time_t time(time_t *out);
clock_t clock(void);
double difftime(time_t a, time_t b);

struct tm *gmtime_r(const time_t *t, struct tm *out);
struct tm *localtime_r(const time_t *t, struct tm *out);
struct tm *gmtime(const time_t *t);
struct tm *localtime(const time_t *t);
time_t mktime(struct tm *t);
time_t timegm(struct tm *t);
char *asctime(const struct tm *t);
char *asctime_r(const struct tm *t, char *buf);
char *ctime(const time_t *t);
char *ctime_r(const time_t *t, char *buf);
size_t strftime(char *buf, size_t max, const char *fmt, const struct tm *t);
#ifndef _CSTD_LOCALE_T
#define _CSTD_LOCALE_T
typedef struct __locale_struct *locale_t;
#endif
size_t strftime_l(char *buf, size_t max, const char *fmt, const struct tm *t, locale_t l);
void tzset(void);

/* C11 / C23 */
#define TIME_UTC 1
#define TIME_MONOTONIC 2
int timespec_get(struct timespec *ts, int base);

_CSTD_END

#endif
