/* sys/un.h - adresa socketu AF_UNIX (cstd). Sockety AF_UNIX zatim nejsou
 * (socket(AF_UNIX) = EAFNOSUPPORT); struktura je pro preklad portu.
 * Rozlozeni jako Linux. */
#ifndef _CSTD_SYS_UN_H
#define _CSTD_SYS_UN_H

#include <_cstd.h>
#include <sys/socket.h>

_CSTD_BEGIN

struct sockaddr_un {
    sa_family_t sun_family;
    char sun_path[108];
};

#define SUN_LEN(p) ((size_t)(((struct sockaddr_un *)0)->sun_path) + __builtin_strlen((p)->sun_path))

_CSTD_END

#endif
