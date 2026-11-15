/* sys/types.h - zakladni typy POSIX (cstd). Stejne na x86_64 i aarch64. */
#ifndef _CSTD_SYS_TYPES_H
#define _CSTD_SYS_TYPES_H

#include <stddef.h>
#include <stdint.h>

typedef long ssize_t;
typedef int64_t off_t;
typedef int64_t off64_t;
typedef uint32_t mode_t;
typedef uint64_t dev_t;
typedef uint64_t ino_t;
typedef uint32_t nlink_t;
typedef uint32_t uid_t;
typedef uint32_t gid_t;
typedef int32_t pid_t;
typedef int64_t blksize_t;
typedef int64_t blkcnt_t;
#ifndef _CSTD_FSBLKCNT_T
#define _CSTD_FSBLKCNT_T
typedef unsigned long fsblkcnt_t;
typedef unsigned long fsfilcnt_t;
#endif
typedef int64_t time_t;
typedef int64_t suseconds_t;
typedef int clockid_t;

#ifndef _CSTD_TIMESPEC
#define _CSTD_TIMESPEC
struct timespec {
    time_t tv_sec;
    long tv_nsec;
};
#endif

#endif
