/* arpa/inet.h - prevody adres (cstd). */
#ifndef _CSTD_ARPA_INET_H
#define _CSTD_ARPA_INET_H

#include <_cstd.h>
#include <netinet/in.h>

_CSTD_BEGIN

int inet_pton(int family, const char *src, void *dst);
const char *inet_ntop(int family, const void *src, char *dst, socklen_t size);
in_addr_t inet_addr(const char *s);
char *inet_ntoa(struct in_addr a);

_CSTD_END

#endif
