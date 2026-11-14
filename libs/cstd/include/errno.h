/* errno.h - cislo chyby posledniho volani, per vlakno (cstd). Hodnoty POSIX. */
#ifndef _ERRNO_H
#define _ERRNO_H

#include "_cstd.h"

_CSTD_BEGIN
int *__errno_location(void);
_CSTD_END

#define errno (*__errno_location())

#define EPERM 1
#define ENOENT 2
#define EINTR 4
#define EIO 5
#define EBADF 9
#define ECHILD 10
#define EAGAIN 11
#define EWOULDBLOCK EAGAIN
#define ENOMEM 12
#define EACCES 13
#define EFAULT 14
#define EBUSY 16
#define EEXIST 17
#define EXDEV 18
#define ENOTDIR 20
#define EISDIR 21
#define EINVAL 22
#define EMFILE 24
#define EFBIG 27
#define ENOSPC 28
#define ESPIPE 29
#define EROFS 30
#define EPIPE 32
#define EDOM 33
#define ERANGE 34
#define ENOSYS 38
#define ENOTEMPTY 39
#define ENOTSUP 95
#define EOPNOTSUPP ENOTSUP
#define EADDRINUSE 98
#define EADDRNOTAVAIL 99
#define ENETUNREACH 101
#define ECONNRESET 104
#define ENOTCONN 107
#define ETIMEDOUT 110
#define ECONNREFUSED 111
#define EHOSTUNREACH 113
#define EALREADY 114
#define EINPROGRESS 115
#define ECANCELED 125

#endif
