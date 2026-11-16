/*
 * Test casti 2e knihovny cstd: sockety BSD, getaddrinfo, poll, select
 * a epoll proti pomocnym serverum hostitele (ctest net, sada "cstd sit").
 * Prostredi: ECHO, UDP_ECHO, CLOSED (host:port), NAME (jmeno echo serveru
 * v DNS hostitele), FORWARDED (port naslouchani), CALLBACK (host:port
 * serveru, ktery se pripoji zpet). Vypisuje "ok <krok>". Krok "share":
 * socket TCP na loopbacku zdedeny v posix_spawn a poslany pres AF_UNIX
 * (SCM_RIGHTS) jinemu procesu. Krok "netrole": potomek s MO_NET ma vlastni
 * relaci site (role Net), bez ni zadnou.
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
#include <spawn.h>
#include <sys/socket.h>
#include <sys/uio.h>
#include <sys/wait.h>
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
    /* AF_UNIX je jen SOCK_STREAM (unixcheck.c); jina rodina neni. */
    if (socket(AF_UNIX, SOCK_DGRAM, 0) != -1 || errno != EPROTONOSUPPORT) return 3;
    if (socket(5, SOCK_STREAM, 0) != -1 || errno != EAFNOSUPPORT) return 6; /* AF_APPLETALK */
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

/* --- predani socketu jinemu procesu (W4b) --- */

/* Port naslouchani na loopbacku (relace testu ho smi navazat). */
#define SHARE_PORT 47100
/* Cislo, na kterem potomek dostane socket (dedeni) nebo konec AF_UNIX. */
#define SHARE_FD 30

extern char **environ;

/* Spojeni na loopbacku: *srv = prijata strana, vraci klientskou. */
static int lo_pair(int l, int *srv) {
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(SHARE_PORT);
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    int c = socket(AF_INET, SOCK_STREAM, 0);
    if (c < 0 || connect(c, (struct sockaddr *)&a, sizeof a) != 0) return -1;
    *srv = accept(l, NULL, NULL);
    return *srv < 0 ? -1 : c;
}

/* Precte presne `want` a pak konec proudu. */
static int expect_then_eof(int fd, const char *want) {
    char b[64];
    size_t n = strlen(want), got = 0;
    while (got < n) {
        ssize_t k = read(fd, b + got, n - got);
        if (k <= 0) return 1;
        got += (size_t)k;
    }
    if (memcmp(b, want, n) != 0) return 2;
    return read(fd, b, sizeof b) == 0 ? 0 : 3;
}

static int spawn_child(const char *mode, int fd, pid_t *pid) {
    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_adddup2(&fa, fd, SHARE_FD);
    /* Stdio potomka prazdne: ctest net nema stdin (a stdout je kanal testu). */
    posix_spawn_file_actions_addopen(&fa, 0, "/dev/null", O_RDONLY, 0);
    posix_spawn_file_actions_addopen(&fa, 1, "/dev/null", O_WRONLY, 0);
    char *argv[] = {"ctest", "child", (char *)mode, NULL};
    int r = posix_spawnp(pid, "ctest", &fa, NULL, argv, environ);
    posix_spawn_file_actions_destroy(&fa);
    if (r != 0) fprintf(stderr, "cnet: posix_spawn %s (fd %d): %s\n", mode, fd, strerror(r));
    return r;
}

static int wait_ok(pid_t pid) {
    int st;
    return waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 0 ? 0 : 1;
}

/* Socket TCP zdedeny v posix_spawn: potomek pise, rodic po zavreni vlastni
 * kopie cte data a konec proudu (socket zanikne s poslednim kanalem). */
static int share_inherit(int l) {
    int srv, c = lo_pair(l, &srv);
    if (c < 0) return 1;
    pid_t pid;
    if (spawn_child("sockw", c, &pid) != 0) return 2;
    close(c);
    if (wait_ok(pid) != 0) return 3;
    int r = expect_then_eof(srv, "ahoj-dedeni");
    close(srv);
    return r ? 10 + r : 0;
}

/* Socket TCP poslany pres AF_UNIX (SCM_RIGHTS) jinemu procesu. */
static int share_scm(int l) {
    int u[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, u) != 0) return 1;
    pid_t pid;
    if (spawn_child("sockr", u[1], &pid) != 0) return 2;
    close(u[1]);
    int srv, c = lo_pair(l, &srv);
    if (c < 0) return 3;
    char tag = 's';
    struct iovec v = {&tag, 1};
    union {
        char buf[CMSG_SPACE(sizeof(int))];
        struct cmsghdr align;
    } ctl;
    struct msghdr m;
    memset(&m, 0, sizeof m);
    m.msg_iov = &v;
    m.msg_iovlen = 1;
    m.msg_control = ctl.buf;
    m.msg_controllen = sizeof ctl.buf;
    struct cmsghdr *cm = CMSG_FIRSTHDR(&m);
    cm->cmsg_len = CMSG_LEN(sizeof(int));
    cm->cmsg_level = SOL_SOCKET;
    cm->cmsg_type = SCM_RIGHTS;
    memcpy(CMSG_DATA(cm), &c, sizeof c);
    if (sendmsg(u[0], &m, 0) != 1) return 4;
    close(c);
    close(u[0]);
    if (wait_ok(pid) != 0) return 5;
    int r = expect_then_eof(srv, "ahoj-scm");
    close(srv);
    return r ? 10 + r : 0;
}

/* Potomek: "sockw" = zapsat do zdedeneho socketu, "sockr" = prijmout
 * socket pres AF_UNIX a zapsat do nej. */
int net_child_main(const char *mode) {
    int fd = SHARE_FD;
    const char *msg = "ahoj-dedeni";
    if (strcmp(mode, "sockr") == 0) {
        char tag;
        char buf[256];
        struct iovec v = {&tag, 1};
        struct msghdr m;
        memset(&m, 0, sizeof m);
        m.msg_iov = &v;
        m.msg_iovlen = 1;
        m.msg_control = buf;
        m.msg_controllen = sizeof buf;
        if (recvmsg(SHARE_FD, &m, 0) != 1) return 1;
        struct cmsghdr *cm = CMSG_FIRSTHDR(&m);
        if (cm == NULL || cm->cmsg_type != SCM_RIGHTS) return 2;
        memcpy(&fd, CMSG_DATA(cm), sizeof fd);
        msg = "ahoj-scm";
    }
    size_t n = strlen(msg);
    if (write(fd, msg, n) != (ssize_t)n) return 3;
    close(fd);
    return 0;
}

static int share(void) {
    int l = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(SHARE_PORT);
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (l < 0 || bind(l, (struct sockaddr *)&a, sizeof a) != 0 || listen(l, 4) != 0) return 1;
    int r = share_inherit(l);
    if (r) return 100 + r;
    r = share_scm(l);
    close(l);
    return r ? 200 + r : 0;
}

/* --- role Net: potomek s MO_NET ma sit, bez ni ne (W4b) --- */

/* Potomek "netconn": spojeni na loopback (0 = spojeno, 7 = bez site). */
int net_conn_child_main(void) {
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(SHARE_PORT);
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    int c = socket(AF_INET, SOCK_STREAM, 0);
    if (c < 0) return 7;
    if (connect(c, (struct sockaddr *)&a, sizeof a) != 0) return 7;
    close(c);
    return 0;
}

/* Prostredi bez MO_NET (kopie environ). */
static char **env_without_net(void) {
    static char *out[256];
    int n = 0;
    for (char **e = environ; *e != NULL && n < 255; e++) {
        if (strncmp(*e, "MO_NET=", 7) != 0) out[n++] = *e;
    }
    out[n] = NULL;
    return out;
}

static int netrole(void) {
    int l = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(SHARE_PORT);
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (l < 0 || bind(l, (struct sockaddr *)&a, sizeof a) != 0 || listen(l, 4) != 0) return 1;
    if (getenv("MO_NET") == NULL) return 2;
    char *argv[] = {"ctest", "child", "netconn", NULL};
    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 0, "/dev/null", O_RDONLY, 0);
    posix_spawn_file_actions_addopen(&fa, 1, "/dev/null", O_WRONLY, 0);
    pid_t pid;
    int st;
    /* S MO_NET (zdedene prostredi): spojeni projde. */
    if (posix_spawnp(&pid, "ctest", &fa, NULL, argv, environ) != 0) return 3;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0) return 4;
    int srv = accept(l, NULL, NULL);
    if (srv < 0) return 5;
    close(srv);
    /* Bez MO_NET: potomek relaci nema. */
    if (posix_spawnp(&pid, "ctest", &fa, NULL, argv, env_without_net()) != 0) return 6;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 7) return 7;
    posix_spawn_file_actions_destroy(&fa);
    close(l);
    return 0;
}

int net_main(void) {
    STEP("tcp", tcp_echo());
    STEP("udp", udp_echo());
    STEP("dns", resolve());
    STEP("refused", refused());
    STEP("accept", listen_accept());
    STEP("share", share());
    STEP("netrole", netrole());
    if (fails == 0) puts("ctest: hotovo");
    fflush(stdout);
    return fails == 0 ? 0 : 1;
}
