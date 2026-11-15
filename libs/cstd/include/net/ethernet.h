/* net/ethernet.h - konstanty a hlavicka ramce Ethernetu (cstd, hodnoty
 * jako Linux). Ramce aplikace nedostanou (AF_PACKET = EPERM). */
#ifndef _CSTD_NET_ETHERNET_H
#define _CSTD_NET_ETHERNET_H

#include <_cstd.h>
#include <stdint.h>

_CSTD_BEGIN

#define ETH_ALEN 6
#define ETH_TLEN 2
#define ETH_HLEN 14
#define ETH_ZLEN 60
#define ETH_DATA_LEN 1500
#define ETH_FRAME_LEN 1514
#define ETHER_ADDR_LEN ETH_ALEN
#define ETHER_TYPE_LEN ETH_TLEN
#define ETHER_HDR_LEN ETH_HLEN
#define ETHERMTU ETH_DATA_LEN

#define ETHERTYPE_IP 0x0800
#define ETHERTYPE_ARP 0x0806
#define ETHERTYPE_REVARP 0x8035
#define ETHERTYPE_VLAN 0x8100
#define ETHERTYPE_IPV6 0x86dd
#define ETHERTYPE_LOOPBACK 0x9000

struct ether_addr {
    uint8_t ether_addr_octet[ETH_ALEN];
} __attribute__((packed));

struct ether_header {
    uint8_t ether_dhost[ETH_ALEN];
    uint8_t ether_shost[ETH_ALEN];
    uint16_t ether_type;
} __attribute__((packed));

_CSTD_END

#endif
