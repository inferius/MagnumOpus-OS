/* net/if_arp.h - druhy hardwarovych adres rozhrani a hlavicka ARP (cstd,
 * hodnoty jako Linux). cstd hlasi ARPHRD_ETHER (eth0) a ARPHRD_LOOPBACK
 * (lo) v SIOCGIFHWADDR; samotne pakety ARP aplikace nedostanou. */
#ifndef _CSTD_NET_IF_ARP_H
#define _CSTD_NET_IF_ARP_H

#include <_cstd.h>
#include <stdint.h>

_CSTD_BEGIN

#define ARPHRD_NETROM 0
#define ARPHRD_ETHER 1
#define ARPHRD_EETHER 2
#define ARPHRD_AX25 3
#define ARPHRD_PRONET 4
#define ARPHRD_CHAOS 5
#define ARPHRD_IEEE802 6
#define ARPHRD_ARCNET 7
#define ARPHRD_DLCI 15
#define ARPHRD_ATM 19
#define ARPHRD_METRICOM 23
#define ARPHRD_IEEE1394 24
#define ARPHRD_EUI64 27
#define ARPHRD_INFINIBAND 32
#define ARPHRD_SLIP 256
#define ARPHRD_CSLIP 257
#define ARPHRD_PPP 512
#define ARPHRD_HDLC 513
#define ARPHRD_TUNNEL 768
#define ARPHRD_TUNNEL6 769
#define ARPHRD_LOOPBACK 772
#define ARPHRD_SIT 776
#define ARPHRD_IPGRE 778
#define ARPHRD_IEEE80211 801
#define ARPHRD_IEEE80211_RADIOTAP 803
#define ARPHRD_IP6GRE 823
#define ARPHRD_NONE 0xFFFE
#define ARPHRD_VOID 0xFFFF

#define ARPOP_REQUEST 1
#define ARPOP_REPLY 2
#define ARPOP_RREQUEST 3
#define ARPOP_RREPLY 4

struct arphdr {
    uint16_t ar_hrd;
    uint16_t ar_pro;
    uint8_t ar_hln;
    uint8_t ar_pln;
    uint16_t ar_op;
};

_CSTD_END

#endif
