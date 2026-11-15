/* sys/ioctl.h - ioctl (cstd): terminal (TCGETS ... TIOCSWINSZ, src/termios.rs)
 * a dotazy na rozhrani na socketu (SIOCGIF* v net/if.h); jinak ENOTTY. */
#ifndef _CSTD_SYS_IOCTL_H
#define _CSTD_SYS_IOCTL_H

#include <_cstd.h>

_CSTD_BEGIN

#define TCGETS 0x5401
#define TCSETS 0x5402
#define TCSETSW 0x5403
#define TCSETSF 0x5404
#define TCFLSH 0x540B
#define TIOCGPGRP 0x540F
#define TIOCSPGRP 0x5410
#define TIOCGWINSZ 0x5413
#define TIOCSWINSZ 0x5414
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
