/*
 * sys/epoll.h - API epoll Linuxu (cstd nad poll a udalostmi pripravenosti).
 * EPOLLET hlasi jen nove nastavene bity, EPOLLONESHOT vypne polozku.
 */
#ifndef _CSTD_SYS_EPOLL_H
#define _CSTD_SYS_EPOLL_H

#include <_cstd.h>
#include <stdint.h>

_CSTD_BEGIN

#define EPOLLIN 0x001
#define EPOLLPRI 0x002
#define EPOLLOUT 0x004
#define EPOLLERR 0x008
#define EPOLLHUP 0x010
#define EPOLLRDHUP 0x2000
#define EPOLLONESHOT (1u << 30)
#define EPOLLET (1u << 31)
#define EPOLL_CTL_ADD 1
#define EPOLL_CTL_DEL 2
#define EPOLL_CTL_MOD 3
#define EPOLL_CLOEXEC 02000000

typedef union epoll_data {
    void *ptr;
    int fd;
    uint32_t u32;
    uint64_t u64;
} epoll_data_t;

struct epoll_event {
    uint32_t events;
    epoll_data_t data;
}
#ifdef __x86_64__
__attribute__((packed))
#endif
;

int epoll_create(int size);
int epoll_create1(int flags);
int epoll_ctl(int epfd, int op, int fd, struct epoll_event *ev);
int epoll_wait(int epfd, struct epoll_event *events, int max, int timeout);

_CSTD_END

#endif
