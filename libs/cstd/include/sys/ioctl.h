/* sys/ioctl.h - ioctl (cstd): terminal zatim nema ovladani, vraci ENOTTY. */
#ifndef _CSTD_SYS_IOCTL_H
#define _CSTD_SYS_IOCTL_H

#include <_cstd.h>

_CSTD_BEGIN

#define TIOCGWINSZ 0x5413
#define FIONREAD 0x541B
#define FIONBIO 0x5421

struct winsize {
    unsigned short ws_row;
    unsigned short ws_col;
    unsigned short ws_xpixel;
    unsigned short ws_ypixel;
};

int ioctl(int fd, unsigned long req, ...);

_CSTD_END

#endif
