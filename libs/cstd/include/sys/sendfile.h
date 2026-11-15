/*
 * sys/sendfile.h - kopie mezi deskriptory (cstd, docs/cs/cstd.md).
 * Pres buffer v procesu (fsd kopii na strane sluzby nema).
 */
#ifndef _CSTD_SYS_SENDFILE_H
#define _CSTD_SYS_SENDFILE_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

ssize_t sendfile(int out_fd, int in_fd, off_t *offset, size_t count);

_CSTD_END

#endif
