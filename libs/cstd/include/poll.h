/* poll.h - cekani na vice fd (cstd). */
#ifndef _CSTD_POLL_H
#define _CSTD_POLL_H

#include <_cstd.h>

_CSTD_BEGIN

typedef unsigned long nfds_t;

#define POLLIN 0x001
#define POLLPRI 0x002
#define POLLOUT 0x004
#define POLLERR 0x008
#define POLLHUP 0x010
#define POLLNVAL 0x020
#define POLLRDNORM POLLIN
#define POLLWRNORM POLLOUT

struct pollfd {
    int fd;
    short events;
    short revents;
};

int poll(struct pollfd *fds, nfds_t n, int timeout);

_CSTD_END

#endif
