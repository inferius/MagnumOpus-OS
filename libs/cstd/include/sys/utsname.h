/* sys/utsname.h - jmeno systemu (cstd). */
#ifndef _CSTD_SYS_UTSNAME_H
#define _CSTD_SYS_UTSNAME_H

#include <_cstd.h>

_CSTD_BEGIN

struct utsname {
    char sysname[65];
    char nodename[65];
    char release[65];
    char version[65];
    char machine[65];
};

int uname(struct utsname *u);

_CSTD_END

#endif
