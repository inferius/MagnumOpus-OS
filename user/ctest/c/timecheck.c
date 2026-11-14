/*
 * Test casti 2c knihovny cstd: hodiny, spanek, kalendar, strftime,
 * mistni cas z TZ (init predava TZ=Europe/Prague) a setenv.
 * Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>
#include <time.h>

static long long ns(const struct timespec *t) { return t->tv_sec * 1000000000LL + t->tv_nsec; }

int time_test(void) {
    struct timespec a, b;
    if (clock_gettime(CLOCK_MONOTONIC, &a) != 0) return 1;
    struct timespec d = {0, 20 * 1000 * 1000};
    if (nanosleep(&d, NULL) != 0) return 2;
    if (clock_gettime(CLOCK_MONOTONIC, &b) != 0) return 3;
    if (ns(&b) - ns(&a) < 20 * 1000 * 1000) return 4;
    if (clock_gettime(99, &a) != -1 || errno != EINVAL) return 5;

    /* Realny cas: po roce 2024 (RTC v QEMU). */
    time_t now = time(NULL);
    struct timeval tv;
    if (now < 1704067200 || gettimeofday(&tv, NULL) != 0 || tv.tv_sec < now) return 6;

    /* Kalendar v UTC a strftime. */
    time_t t = 1794659696; /* 2026-11-14 12:34:56 UTC */
    struct tm g;
    char buf[64];
    if (gmtime_r(&t, &g) == NULL || g.tm_wday != 6 || g.tm_yday != 317) return 10;
    if (strftime(buf, sizeof buf, "%F %T %Z", &g) == 0 || strcmp(buf, "2026-11-14 12:34:56 UTC") != 0) return 11;
    if (strftime(buf, 5, "%F", &g) != 0) return 12;
    if (timegm(&g) != t) return 13;

    /* Mistni cas: Praha, v listopadu +1 h, v cervenci +2 h (CEST). */
    struct tm l;
    if (localtime_r(&t, &l) == NULL || l.tm_hour != 13 || l.tm_gmtoff != 3600 || l.tm_isdst != 0) return 20;
    time_t summer = 1783000000;
    if (localtime_r(&summer, &l) == NULL || l.tm_gmtoff != 7200 || l.tm_isdst != 1) return 21;
    if (strftime(buf, sizeof buf, "%z %Z", &l) == 0 || strcmp(buf, "+0200 CEST") != 0) return 22;
    if (mktime(&l) != summer) return 23;

    /* mktime normalizuje (31. listopadu = 1. prosince). */
    struct tm n = {0};
    n.tm_year = 126;
    n.tm_mon = 10;
    n.tm_mday = 31;
    n.tm_hour = 12;
    n.tm_isdst = -1;
    if (mktime(&n) == -1 || n.tm_mon != 11 || n.tm_mday != 1) return 24;

    /* setenv + tzset: jine pasmo. */
    if (setenv("TZ", "UTC", 1) != 0 || strcmp(getenv("TZ"), "UTC") != 0) return 30;
    tzset();
    if (localtime_r(&t, &l) == NULL || l.tm_hour != 12) return 31;
    if (setenv("TZ", "x", 0) != 0 || strcmp(getenv("TZ"), "UTC") != 0) return 32;
    if (unsetenv("TZ") != 0 || getenv("TZ") != NULL) return 33;
    if (setenv("A=B", "x", 1) != -1 || errno != EINVAL) return 34;
    return 0;
}
