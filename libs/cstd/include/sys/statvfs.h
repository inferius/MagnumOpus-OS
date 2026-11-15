/*
 * sys/statvfs.h - velikost svazku (cstd, docs/cs/cstd.md).
 * Hodnoty svazku relace fsd (fsproto INFO); pocty inodu nejsou (0).
 */
#ifndef _CSTD_SYS_STATVFS_H
#define _CSTD_SYS_STATVFS_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

#ifndef _CSTD_FSBLKCNT_T
#define _CSTD_FSBLKCNT_T
typedef unsigned long fsblkcnt_t;
typedef unsigned long fsfilcnt_t;
#endif

struct statvfs {
    unsigned long f_bsize;
    unsigned long f_frsize;
    fsblkcnt_t f_blocks;
    fsblkcnt_t f_bfree;
    fsblkcnt_t f_bavail;
    fsfilcnt_t f_files;
    fsfilcnt_t f_ffree;
    fsfilcnt_t f_favail;
    unsigned long f_fsid;
    unsigned long f_flag;
    unsigned long f_namemax;
};

#define ST_RDONLY 1
#define ST_NOSUID 2

int statvfs(const char *path, struct statvfs *out);
int fstatvfs(int fd, struct statvfs *out);

_CSTD_END

#endif
