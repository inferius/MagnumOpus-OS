/* dirent.h - vypis adresare (cstd). Prvni dve polozky jsou "." a "..". */
#ifndef _CSTD_DIRENT_H
#define _CSTD_DIRENT_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

#define DT_UNKNOWN 0
#define DT_FIFO 1
#define DT_CHR 2
#define DT_DIR 4
#define DT_REG 8
#define DT_LNK 10

struct dirent {
    ino_t d_ino;
    off_t d_off;
    unsigned short d_reclen;
    unsigned char d_type;
    char d_name[256];
};

typedef struct DIR DIR;

DIR *opendir(const char *path);
DIR *fdopendir(int fd);
struct dirent *readdir(DIR *d);
int closedir(DIR *d);
void rewinddir(DIR *d);
int dirfd(DIR *d);

_CSTD_END

#endif
