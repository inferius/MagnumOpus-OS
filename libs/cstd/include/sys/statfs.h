/* sys/statfs.h - statfs, fstatfs (cstd, src/statfs.rs). Rozlozeni jako
 * Linux na 64 bitech. Svazek relace FS: f_type = MOFS_SUPER_MAGIC, pocty
 * souboru 0 (FS je nema pevne dane). */
#ifndef _CSTD_SYS_STATFS_H
#define _CSTD_SYS_STATFS_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

#ifndef _CSTD_FSID_T
#define _CSTD_FSID_T
typedef struct { int __val[2]; } fsid_t;
#endif

#define MOFS_SUPER_MAGIC 0x4D4F4653

struct statfs {
    long f_type;
    long f_bsize;
    fsblkcnt_t f_blocks;
    fsblkcnt_t f_bfree;
    fsblkcnt_t f_bavail;
    fsfilcnt_t f_files;
    fsfilcnt_t f_ffree;
    fsid_t f_fsid;
    long f_namelen;
    long f_frsize;
    long f_flags;
    long f_spare[4];
};

int statfs(const char *path, struct statfs *out);
int fstatfs(int fd, struct statfs *out);

_CSTD_END

#endif
