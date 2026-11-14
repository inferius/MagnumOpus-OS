/*
 * Test casti 2e knihovny cstd: sockety BSD, getaddrinfo, poll, select
 * a epoll proti pomocnym serverum hostitele (ctest net, sada "cstd sit").
 * Prostredi: ECHO, UDP_ECHO, CLOSED (host:port), NAME (jmeno echo serveru
 * v DNS hostitele), FORWARDED (port naslouchani), CALLBACK (host:port
 * serveru, ktery se pripoji zpet). Vypisuje "ok <krok>".
 */
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/epoll.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <unistd.h>

static int fails;

#define STEP(name, expr)                                                         \
    do {                                                                         \
        int _r = (expr);                                                         \
        if (_r == 0) {                                                           \
            printf("ok %s\n", name);                                             \
        } else {                                                                 \
            fprintf(stderr, "cnet: %s selhalo v kroku %d (errno %d)\n", name, _r, \
                    errno);                                                      \
            fails++;                                                             \
        }                                                                        \
    } while (0)

/* "a.b.c.d:port" z promenne prostredi -> sockaddr_in. */
static int env_addr(const char *var, struct sockaddr_in *a) {
    const char *v = getenv(var);
    if (v == NULL) return -1;
    char host[64];
    const char *colon = strrchr(v, ':');
    if (colon == NULL || (size_t)(colon - v) >= sizeof host) return -1;
    memcpy(host, v, (size_t)(colon - v));
    host[colon - v] = 0;
    memset(a, 0, sizeof *a);
    a->sin_family = AF_INET;
    a->sin_port = htons((uint16_t)atoi(colon + 1));
    return inet_pton(AF_INET, host, &a->sin_addr) == 1 ? 0 : -1;
}

static int write_all(int fd, const char *p, size_t n) {
    while (n > 0) {
        ssize_t k = write(fd, p, n);
        if (k <= 0) return -1;
        p += k;
        n -= (size_t)k;
    }
    return 0;
}

static int tcp_echo(void) {
    struct sockaddr_in a;
    if (env_addr("ECHO", &a) != 0) return 1;
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 3) return 2;
    if (connect(s, (struct sockaddr *)&a, sizeof a) != 0) return 3;
    struct sockaddr_in peer;
    socklen_t len = sizeof peer;
    if (getpeername(s, (struct sockaddr *)&peer, &len) != 0 || peer.sin_port != a.sin_port) return 4;
    const char *msg = "ahoj z C\n";
    if (send(s, msg, strlen(msg), 0) != (ssize_t)strlen(msg)) return 5;
    /* poll: odpoved prijde. */
    struct pollfd p = {s, POLLIN, 0};
    if (poll(&p, 1, 10000) != 1 || !(p.revents & POLLIN)) return 6;
    char buf[64];
    size_t got = 0;
    while (got < strlen(msg)) {
        ssize_t k = recv(s, buf + got, sizeof buf - got, 0);
        if (k <= 0) return 7;
        got += (size_t)k;
    }
    if (memcmp(buf, msg, strlen(msg)) != 0) return 8;
    /* FILE nad socketem (fdopen) a write/read. */
    if (write_all(s, "radek\n", 6) != 0) return 9;
    FILE *f = fdopen(dup(s), "r");
    if (f == NULL || fgets(buf, sizeof buf, f) == NULL || strcmp(buf, "radek\n") != 0) return 10;
    fclose(f);
    /* Neblokujici cteni bez dat = EAGAIN, epoll pak data ohlasi. */
    if (fcntl(s, F_SETFL, fcntl(s, F_GETFL) | O_NONBLOCK) != 0) return 11;
    if (recv(s, buf, sizeof buf, 0) != -1 || errno != EAGAIN) return 12;
    int ep = epoll_create1(EPOLL_CLOEXEC);
    struct epoll_event ev = {.events = EPOLLIN, .data.u64 = 77};
    if (ep < 0 || epoll_ctl(ep, EPOLL_CTL_ADD, s, &ev) != 0) return 13;
    if (send(s, "x", 1, 0) != 1) return 14;
    struct epoll_event out[2];
    if (epoll_wait(ep, out, 2, 10000) != 1 || out[0].data.u64 != 77 || !(out[0].events & EPOLLIN)) return 15;
    if (recv(s, buf, sizeof buf, 0) != 1 || buf[0] != 'x') return 16;
    /* select: zapis je pripraveny. */
    fd_set w;
    FD_ZERO(&w);
    FD_SET(s, &w);
    struct timeval tv = {5, 0};
    if (select(s + 1, NULL, &w, NULL, &tv) != 1 || !FD_ISSET(s, &w)) return 17;
    /* shutdown zapisu: echo server zavre, cteni vrati 0. */
    fcntl(s, F_SETFL, 0);
    if (shutdown(s, SHUT_WR) != 0) return 18;
    if (recv(s, buf, sizeof buf, 0) != 0) return 19;
    close(ep);
    return close(s) == 0 ? 0 : 20;
}

static int udp_echo(void) {
    struct sockaddr_in a;
    if (env_addr("UDP_ECHO", &a) != 0) return 1;
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    if (s < 0) return 2;
    struct timeval tv = {10, 0};
    if (setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv) != 0) return 3;
    if (sendto(s, "datagram", 8, 0, (struct sockaddr *)&a, sizeof a) != 8) return 4;
    char buf[64];
    struct sockaddr_in from;
    socklen_t len = sizeof from;
    ssize_t n = recvfrom(s, buf, sizeof buf, 0, (struct sockaddr *)&from, &len);
    if (n != 8 || memcmp(buf, "datagram", 8) != 0 || from.sin_port != a.sin_port) return 5;
    /* connect u UDP: send/recv jen s cilem. */
    if (connect(s, (struct sockaddr *)&a, sizeof a) != 0 || send(s, "znovu", 5, 0) != 5) return 6;
    if (recv(s, buf, sizeof buf, 0) != 5) return 7;
    int type = 0;
    len = sizeof type;
    if (getsockopt(s, SOL_SOCKET, SO_TYPE, &type, &len) != 0 || type != SOCK_DGRAM) return 8;
    return close(s);
}

static int resolve(void) {
    const char *name = getenv("NAME");
    const char *echo = getenv("ECHO");
    if (name == NULL || echo == NULL) return 1;
    struct addrinfo hints = {0}, *res = NULL;
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    int r = getaddrinfo(name, strrchr(echo, ':') + 1, &hints, &res);
    if (r != 0) {
        fprintf(stderr, "cnet: getaddrinfo: %s\n", gai_strerror(r));
        return 2;
    }
    /* Pripojit se na prvni adresu ze seznamu. */
    int s = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (s < 0 || connect(s, res->ai_addr, res->ai_addrlen) != 0) return 3;
    char host[INET_ADDRSTRLEN];
    struct sockaddr_in *in = (struct sockaddr_in *)res->ai_addr;
    if (inet_ntop(AF_INET, &in->sin_addr, host, sizeof host) == NULL) return 4;
    freeaddrinfo(res);
    close(s);
    /* Ciselna adresa bez DNS, neznama sluzba, neexistujici jmeno. */
    hints.ai_flags = AI_NUMERICHOST;
    if (getaddrinfo("10.0.2.2", "80", &hints, &res) != 0 || res->ai_addrlen != sizeof(struct sockaddr_in)) return 5;
    freeaddrinfo(res);
    if (getaddrinfo("10.0.2.2", "neznama-sluzba", &hints, &res) != EAI_SERVICE) return 6;
    hints.ai_flags = 0;
    if (getaddrinfo("neexistuje.invalid", NULL, &hints, &res) == 0) return 7;
    return 0;
}

static int refused(void) {
    struct sockaddr_in a;
    if (env_addr("CLOSED", &a) != 0) return 1;
    int s = socket(AF_INET, SOCK_STREAM, 0);
    struct timeval tv = {10, 0};
    setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    int r = connect(s, (struct sockaddr *)&a, sizeof a);
    /* Odmitnuti zalezi na hostiteli (REFUSES=1), jinak aspon chyba. */
    const char *refuses = getenv("REFUSES");
    if (r != -1 || (refuses != NULL && refuses[0] == '1' && errno != ECONNREFUSED)) return 2;
    close(s);
    /* Port mimo politiku relace: EACCES. */
    a.sin_port = htons(1);
    s = socket(AF_INET, SOCK_STREAM, 0);
    if (connect(s, (struct sockaddr *)&a, sizeof a) != -1 || errno != EACCES) return 5;
    close(s);
    if (socket(AF_UNIX, SOCK_STREAM, 0) != -1 || errno != EAFNOSUPPORT) return 3;
    char b[4];
    if (recv(0, b, 1, 0) != -1 || errno != ENOTSOCK) return 4;
    return 0;
}

static int listen_accept(void) {
    const char *fw = getenv("FORWARDED");
    struct sockaddr_in cb;
    if (fw == NULL || env_addr("CALLBACK", &cb) != 0) return 1;
    int l = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    struct sockaddr_in a = {0};
    a.sin_family = AF_INET;
    a.sin_port = htons((uint16_t)atoi(fw));
    a.sin_addr.s_addr = htonl(INADDR_ANY);
    int one = 1;
    if (setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one) != 0) return 2;
    if (bind(l, (struct sockaddr *)&a, sizeof a) != 0 || listen(l, 4) != 0) return 3;
    struct sockaddr_in local;
    socklen_t len = sizeof local;
    if (getsockname(l, (struct sockaddr *)&local, &len) != 0 || local.sin_port != a.sin_port) return 4;
    /* Hostitel se pripoji zpet (callback server). */
    int c = socket(AF_INET, SOCK_STREAM, 0);
    if (connect(c, (struct sockaddr *)&cb, sizeof cb) != 0 || write_all(c, "connect\n", 8) != 0) return 5;
    struct pollfd p = {l, POLLIN, 0};
    if (poll(&p, 1, 10000) != 1) return 6;
    struct sockaddr_in from;
    len = sizeof from;
    int conn = accept(l, (struct sockaddr *)&from, &len);
    if (conn < 0 || from.sin_family != AF_INET) return 7;
    char buf[64] = {0};
    size_t got = 0;
    ssize_t k;
    while ((k = read(conn, buf + got, sizeof buf - 1 - got)) > 0) got += (size_t)k;
    if (strcmp(buf, "ahoj z hostitele\n") != 0) return 8;
    close(conn);
    close(c);
    return close(l);
}

int net_main(void) {
    STEP("tcp", tcp_echo());
    STEP("udp", udp_echo());
    STEP("dns", resolve());
    STEP("refused", refused());
    STEP("accept", listen_accept());
    if (fails == 0) puts("ctest: hotovo");
    fflush(stdout);
    return fails == 0 ? 0 : 1;
}
