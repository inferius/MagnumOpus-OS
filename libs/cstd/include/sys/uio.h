/* sys/uio.h - cteni a zapis po kusech (cstd). */
#ifndef _CSTD_SYS_UIO_H
#define _CSTD_SYS_UIO_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

struct iovec {
    void *iov_base;
    size_t iov_len;
};

ssize_t readv(int fd, const struct iovec *iov, int cnt);
ssize_t writev(int fd, const struct iovec *iov, int cnt);

_CSTD_END

#endif
