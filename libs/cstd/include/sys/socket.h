/*
 * sys/socket.h - sockety BSD (cstd nad relaci site, netproto v2).
 * AF_INET a AF_INET6, SOCK_STREAM a SOCK_DGRAM; AF_UNIX neni, SOCK_RAW
 * a AF_PACKET = EPERM. Ridici zpravy (msg_control, SCM_RIGHTS) sendmsg
 * odmitne s EOPNOTSUPP (handly jdou v systemu kanaly), recvmsg zadne
 * nevraci.
 */
#ifndef _CSTD_SYS_SOCKET_H
#define _CSTD_SYS_SOCKET_H

#include <_cstd.h>
#include <sys/types.h>
#include <sys/uio.h>

_CSTD_BEGIN

typedef uint32_t socklen_t;
typedef uint16_t sa_family_t;

#define AF_UNSPEC 0
#define AF_UNIX 1
#define AF_LOCAL AF_UNIX
#define AF_INET 2
#define AF_INET6 10
#define AF_NETLINK 16
#define AF_PACKET 17
#define PF_UNSPEC AF_UNSPEC
#define PF_INET AF_INET
#define PF_INET6 AF_INET6
#define PF_UNIX AF_UNIX
#define PF_LOCAL AF_LOCAL
#define PF_NETLINK AF_NETLINK
#define PF_PACKET AF_PACKET

#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define SOCK_RAW 3
#define SOCK_RDM 4
#define SOCK_SEQPACKET 5
#define SOCK_NONBLOCK 04000
#define SOCK_CLOEXEC 02000000

#define SOL_SOCKET 1
#define SO_DEBUG 1
#define SO_REUSEADDR 2
#define SO_TYPE 3
#define SO_ERROR 4
#define SO_DONTROUTE 5
#define SO_BROADCAST 6
#define SO_SNDBUF 7
#define SO_RCVBUF 8
#define SO_KEEPALIVE 9
#define SO_OOBINLINE 10
#define SO_NO_CHECK 11
#define SO_PRIORITY 12
#define SO_LINGER 13
#define SO_BSDCOMPAT 14
#define SO_REUSEPORT 15
#define SO_PASSCRED 16
#define SO_PEERCRED 17
#define SO_RCVLOWAT 18
#define SO_SNDLOWAT 19
#define SO_RCVTIMEO 20
#define SO_SNDTIMEO 21
#define SO_TIMESTAMP 29
#define SO_ACCEPTCONN 30
#define SO_PROTOCOL 38
#define SO_DOMAIN 39

#define SCM_RIGHTS 1
#define SCM_CREDENTIALS 2
#define SCM_TIMESTAMP SO_TIMESTAMP

#define MSG_OOB 1
#define MSG_PEEK 2
#define MSG_DONTROUTE 4
#define MSG_CTRUNC 8
#define MSG_TRUNC 0x20
#define MSG_DONTWAIT 0x40
#define MSG_EOR 0x80
#define MSG_WAITALL 0x100
#define MSG_NOSIGNAL 0x4000
#define MSG_MORE 0x8000
#define MSG_CMSG_CLOEXEC 0x40000000

#define SHUT_RD 0
#define SHUT_WR 1
#define SHUT_RDWR 2

#define SOMAXCONN 4096

struct sockaddr {
    sa_family_t sa_family;
    char sa_data[14];
};

struct sockaddr_storage {
    sa_family_t ss_family;
    char __ss_data[126];
} __attribute__((aligned(8)));

struct msghdr {
    void *msg_name;
    socklen_t msg_namelen;
    struct iovec *msg_iov;
    size_t msg_iovlen;
    void *msg_control;
    size_t msg_controllen;
    int msg_flags;
};

struct cmsghdr {
    size_t cmsg_len;
    int cmsg_level;
    int cmsg_type;
};

/* Prihlasovaci udaje druhe strany (SO_PEERCRED; az s AF_UNIX). */
struct ucred {
    pid_t pid;
    uid_t uid;
    gid_t gid;
};

struct linger {
    int l_onoff;
    int l_linger;
};

#define CMSG_ALIGN(len) (((len) + sizeof(size_t) - 1) & ~(sizeof(size_t) - 1))
#define CMSG_DATA(c) ((unsigned char *)((struct cmsghdr *)(c) + 1))
#define CMSG_SPACE(len) (CMSG_ALIGN(len) + CMSG_ALIGN(sizeof(struct cmsghdr)))
#define CMSG_LEN(len) (CMSG_ALIGN(sizeof(struct cmsghdr)) + (len))
#define CMSG_FIRSTHDR(m) \
    ((size_t)(m)->msg_controllen >= sizeof(struct cmsghdr) ? (struct cmsghdr *)(m)->msg_control : (struct cmsghdr *)0)
#define __CMSG_NEXT(c) ((unsigned char *)(c) + CMSG_ALIGN((c)->cmsg_len))
#define __CMSG_END(m) ((unsigned char *)(m)->msg_control + (m)->msg_controllen)
/* Dalsi hlavicka, jen kdyz se cela vejde za aktualni (jako musl). */
#define CMSG_NXTHDR(m, c) \
    ((c)->cmsg_len < sizeof(struct cmsghdr) || \
     CMSG_ALIGN((c)->cmsg_len) + sizeof(struct cmsghdr) >= (size_t)(__CMSG_END(m) - (unsigned char *)(c)) \
         ? (struct cmsghdr *)0 \
         : (struct cmsghdr *)__CMSG_NEXT(c))

int socket(int domain, int type, int protocol);
int socketpair(int domain, int type, int protocol, int fds[2]);
int bind(int fd, const struct sockaddr *addr, socklen_t len);
int listen(int fd, int backlog);
int accept(int fd, struct sockaddr *addr, socklen_t *len);
int accept4(int fd, struct sockaddr *addr, socklen_t *len, int flags);
int connect(int fd, const struct sockaddr *addr, socklen_t len);
ssize_t send(int fd, const void *buf, size_t n, int flags);
ssize_t recv(int fd, void *buf, size_t n, int flags);
ssize_t sendto(int fd, const void *buf, size_t n, int flags, const struct sockaddr *to, socklen_t len);
ssize_t recvfrom(int fd, void *buf, size_t n, int flags, struct sockaddr *from, socklen_t *len);
ssize_t sendmsg(int fd, const struct msghdr *m, int flags);
ssize_t recvmsg(int fd, struct msghdr *m, int flags);
int shutdown(int fd, int how);
int getsockname(int fd, struct sockaddr *addr, socklen_t *len);
int getpeername(int fd, struct sockaddr *addr, socklen_t *len);
int setsockopt(int fd, int level, int name, const void *value, socklen_t len);
int getsockopt(int fd, int level, int name, void *value, socklen_t *len);

_CSTD_END

#endif
