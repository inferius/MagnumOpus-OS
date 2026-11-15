/* netinet/if_ether.h - Ethernet a ARP (cstd, jako glibc: net/ethernet.h,
 * net/if_arp.h a struct ether_arp). Jen pro preklad portu: ramce ani
 * pakety ARP aplikace nedostanou. */
#ifndef _CSTD_NETINET_IF_ETHER_H
#define _CSTD_NETINET_IF_ETHER_H

#include <_cstd.h>
#include <net/ethernet.h>
#include <net/if_arp.h>

_CSTD_BEGIN

#define ETH_P_LOOP 0x0060
#define ETH_P_IP 0x0800
#define ETH_P_ARP 0x0806
#define ETH_P_IPV6 0x86DD
#define ETH_P_ALL 0x0003

struct ether_arp {
    struct arphdr ea_hdr;
    uint8_t arp_sha[ETH_ALEN];
    uint8_t arp_spa[4];
    uint8_t arp_tha[ETH_ALEN];
    uint8_t arp_tpa[4];
};
#define arp_hrd ea_hdr.ar_hrd
#define arp_pro ea_hdr.ar_pro
#define arp_hln ea_hdr.ar_hln
#define arp_pln ea_hdr.ar_pln
#define arp_op ea_hdr.ar_op

_CSTD_END

#endif
