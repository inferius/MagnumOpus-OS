/* sys/select.h - select (cstd, nad poll); FD_SETSIZE 1024. */
#ifndef _CSTD_SYS_SELECT_H
#define _CSTD_SYS_SELECT_H

#include <_cstd.h>
#include <sys/time.h>

_CSTD_BEGIN

#define FD_SETSIZE 1024

typedef struct {
    unsigned long fds_bits[FD_SETSIZE / 64];
} fd_set;

#define FD_ZERO(s) __builtin_memset((s), 0, sizeof(fd_set))
#define FD_SET(fd, s) ((s)->fds_bits[(fd) / 64] |= (1UL << ((fd) % 64)))
#define FD_CLR(fd, s) ((s)->fds_bits[(fd) / 64] &= ~(1UL << ((fd) % 64)))
#define FD_ISSET(fd, s) (((s)->fds_bits[(fd) / 64] & (1UL << ((fd) % 64))) != 0)

int select(int nfds, fd_set *r, fd_set *w, fd_set *e, struct timeval *timeout);

_CSTD_END

#endif
