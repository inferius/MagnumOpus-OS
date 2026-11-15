/* netinet/tcp.h - volby TCP (cstd, cisla jako Linux). TCP_NODELAY se
 * prijme bez ucinku (netd posila hned); ostatni setsockopt = ENOPROTOOPT. */
#ifndef _CSTD_NETINET_TCP_H
#define _CSTD_NETINET_TCP_H

#define TCP_NODELAY 1
#define TCP_MAXSEG 2
#define TCP_CORK 3
#define TCP_KEEPIDLE 4
#define TCP_KEEPINTVL 5
#define TCP_KEEPCNT 6
#define TCP_SYNCNT 7
#define TCP_LINGER2 8
#define TCP_DEFER_ACCEPT 9
#define TCP_WINDOW_CLAMP 10
#define TCP_INFO 11
#define TCP_QUICKACK 12
#define TCP_CONGESTION 13
#define TCP_USER_TIMEOUT 18
#define TCP_FASTOPEN 23

#include <stdint.h>

/* Stavy spojeni (tcpi_state). */
enum {
    TCP_ESTABLISHED = 1,
    TCP_SYN_SENT,
    TCP_SYN_RECV,
    TCP_FIN_WAIT1,
    TCP_FIN_WAIT2,
    TCP_TIME_WAIT,
    TCP_CLOSE,
    TCP_CLOSE_WAIT,
    TCP_LAST_ACK,
    TCP_LISTEN,
    TCP_CLOSING
};

/* getsockopt(TCP_INFO): rozlozeni jako Linux; netd udaje zatim nedava
 * (ENOPROTOOPT), struktura je pro preklad portu. */
struct tcp_info {
    uint8_t tcpi_state;
    uint8_t tcpi_ca_state;
    uint8_t tcpi_retransmits;
    uint8_t tcpi_probes;
    uint8_t tcpi_backoff;
    uint8_t tcpi_options;
    uint8_t tcpi_snd_wscale : 4, tcpi_rcv_wscale : 4;
    uint8_t tcpi_delivery_rate_app_limited : 1, tcpi_fastopen_client_fail : 2;
    uint32_t tcpi_rto;
    uint32_t tcpi_ato;
    uint32_t tcpi_snd_mss;
    uint32_t tcpi_rcv_mss;
    uint32_t tcpi_unacked;
    uint32_t tcpi_sacked;
    uint32_t tcpi_lost;
    uint32_t tcpi_retrans;
    uint32_t tcpi_fackets;
    uint32_t tcpi_last_data_sent;
    uint32_t tcpi_last_ack_sent;
    uint32_t tcpi_last_data_recv;
    uint32_t tcpi_last_ack_recv;
    uint32_t tcpi_pmtu;
    uint32_t tcpi_rcv_ssthresh;
    uint32_t tcpi_rtt;
    uint32_t tcpi_rttvar;
    uint32_t tcpi_snd_ssthresh;
    uint32_t tcpi_snd_cwnd;
    uint32_t tcpi_advmss;
    uint32_t tcpi_reordering;
    uint32_t tcpi_rcv_rtt;
    uint32_t tcpi_rcv_space;
    uint32_t tcpi_total_retrans;
    uint64_t tcpi_pacing_rate;
    uint64_t tcpi_max_pacing_rate;
    uint64_t tcpi_bytes_acked;
    uint64_t tcpi_bytes_received;
    uint32_t tcpi_segs_out;
    uint32_t tcpi_segs_in;
    uint32_t tcpi_notsent_bytes;
    uint32_t tcpi_min_rtt;
    uint32_t tcpi_data_segs_in;
    uint32_t tcpi_data_segs_out;
    uint64_t tcpi_delivery_rate;
    uint64_t tcpi_busy_time;
    uint64_t tcpi_rwnd_limited;
    uint64_t tcpi_sndbuf_limited;
    uint32_t tcpi_delivered;
    uint32_t tcpi_delivered_ce;
    uint64_t tcpi_bytes_sent;
    uint64_t tcpi_bytes_retrans;
    uint32_t tcpi_dsack_dups;
    uint32_t tcpi_reord_seen;
    uint32_t tcpi_rcv_ooopack;
    uint32_t tcpi_snd_wnd;
};

#endif
