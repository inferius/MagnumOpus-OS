/* sys/file.h - flock (cstd): zamky celeho souboru, drzi je fsd. */
#ifndef _CSTD_SYS_FILE_H
#define _CSTD_SYS_FILE_H

#include <_cstd.h>

_CSTD_BEGIN

#define LOCK_SH 1
#define LOCK_EX 2
#define LOCK_NB 4
#define LOCK_UN 8

int flock(int fd, int op);

_CSTD_END

#endif
