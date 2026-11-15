/* ifaddrs.h - getifaddrs, freeifaddrs (cstd, src/ifaddrs.rs). Smycka lo
 * a rozhrani site eth0 (jen s relaci site s pravem INFO); rozhrani bez
 * adresy ma ifa_addr NULL. Polozky AF_PACKET nejsou. */
#ifndef _CSTD_IFADDRS_H
#define _CSTD_IFADDRS_H

#include <_cstd.h>
#include <sys/socket.h>

_CSTD_BEGIN

struct ifaddrs {
    struct ifaddrs *ifa_next;
    char *ifa_name;
    unsigned int ifa_flags;
    struct sockaddr *ifa_addr;
    struct sockaddr *ifa_netmask;
    union {
        struct sockaddr *ifu_broadaddr;
        struct sockaddr *ifu_dstaddr;
    } ifa_ifu;
    void *ifa_data;
};
#define ifa_broadaddr ifa_ifu.ifu_broadaddr
#define ifa_dstaddr ifa_ifu.ifu_dstaddr

int getifaddrs(struct ifaddrs **out);
void freeifaddrs(struct ifaddrs *p);

_CSTD_END

#endif
