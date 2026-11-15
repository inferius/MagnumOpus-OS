/*
 * Sockety AF_UNIX (T1, vzor wineserveru): socketpair, bind/listen/accept
 * na relativni cestu, neblokujici posluchac v poll, connect z jineho
 * procesu, SCM_RIGHTS oboma smery (roura, soubor, adresar, konec
 * socketpair), dedeni konce socketpair pres posix_spawn, SO_PEERCRED,
 * shutdown, konec druhe strany (POLLHUP, recv 0, EPIPE), chyby connect
 * a bind. Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

/* Cislo, na kterem potomek "child unixpair" dostane konec socketpair. */
enum { FD_PAIR = 30 };

/* Posle data a jeden fd (jako wine_server_send_fd). */
static int send_fd(int s, const void *data, size_t n, int fd) {
    struct iovec v = {(void *)data, n};
    union {
        char buf[CMSG_SPACE(sizeof(int))];
        struct cmsghdr align;
    } ctl;
    struct msghdr m;
    memset(&m, 0, sizeof m);
    m.msg_iov = &v;
    m.msg_iovlen = 1;
    if (fd >= 0) {
        m.msg_control = ctl.buf;
        m.msg_controllen = sizeof ctl.buf;
        struct cmsghdr *c = CMSG_FIRSTHDR(&m);
        c->cmsg_len = CMSG_LEN(sizeof(int));
        c->cmsg_level = SOL_SOCKET;
        c->cmsg_type = SCM_RIGHTS;
        memcpy(CMSG_DATA(c), &fd, sizeof fd);
    }
    return (int)sendmsg(s, &m, 0);
}

/* Prijme data a fd (jako wine_server_receive_fd); *fd = -1 bez fd. */
static int recv_fd(int s, void *data, size_t n, int *fd) {
    struct iovec v = {data, n};
    char buf[256];
    struct msghdr m;
    memset(&m, 0, sizeof m);
    m.msg_iov = &v;
    m.msg_iovlen = 1;
    m.msg_control = buf;
    m.msg_controllen = sizeof buf;
    *fd = -1;
    int r = (int)recvmsg(s, &m, MSG_CMSG_CLOEXEC);
    if (r < 0) return r;
    for (struct cmsghdr *c = CMSG_FIRSTHDR(&m); c != NULL; c = CMSG_NXTHDR(&m, c))
        if (c->cmsg_level == SOL_SOCKET && c->cmsg_type == SCM_RIGHTS) memcpy(fd, CMSG_DATA(c), sizeof(int));
    return r;
}

/* Potomek "child unix CESTA": pripoji se, posle soubor, dostane adresar. */
int unix_child_main(const char *path) {
    int s = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (s < 0) return 1;
    struct sockaddr_un a;
    memset(&a, 0, sizeof a);
    a.sun_family = AF_UNIX;
    strncpy(a.sun_path, path, sizeof a.sun_path - 1);
    if (connect(s, (struct sockaddr *)&a, sizeof a) != 0) return 2;
    struct ucred cr;
    socklen_t cl = sizeof cr;
    if (getsockopt(s, SOL_SOCKET, SO_PEERCRED, &cr, &cl) != 0 || cl != sizeof cr) return 3;
    /* Soubor otevreny tady, obsah precte rodic. */
    char file[300];
    snprintf(file, sizeof file, "%s.data", path);
    int f = open(file, O_RDONLY);
    if (f < 0) return 4;
    if (send_fd(s, "soubor", 6, f) != 6) return 5;
    close(f);
    char b[16];
    int d = -1;
    if (recv_fd(s, b, sizeof b, &d) != 8 || memcmp(b, "adresar!", 8) != 0 || d < 0) return 6;
    struct stat st;
    if (fstat(d, &st) != 0 || !S_ISDIR(st.st_mode)) return 7;
    if ((fcntl(d, F_GETFD) & FD_CLOEXEC) == 0) return 8;
    /* shutdown: rodic docte a dostane 0. */
    if (send(s, "konec", 5, 0) != 5 || shutdown(s, SHUT_WR) != 0) return 9;
    if (recv(s, b, sizeof b, 0) != 0) return 10;
    return 0;
}

/* Potomek "child unixpair": zdedeny konec socketpair na FD_PAIR. */
int unixpair_child_main(void) {
    if (write(FD_PAIR, "pair", 4) != 4) return 1;
    char b[8];
    if (read(FD_PAIR, b, sizeof b) != 2 || memcmp(b, "ok", 2) != 0) return 2;
    return 0;
}

static pid_t spawn_child(const char *mode, const char *arg, posix_spawn_file_actions_t *fa) {
    char *argv[] = {"ctest", "child", (char *)mode, (char *)arg, NULL};
    pid_t pid = 0;
    return posix_spawnp(&pid, "ctest", fa, NULL, argv, environ) == 0 ? pid : -1;
}

static int child_ok(pid_t pid) {
    int st = -1;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st)) return 0;
    if (WEXITSTATUS(st) != 0) fprintf(stderr, "ctest unix: potomek krok %d\n", WEXITSTATUS(st));
    return WEXITSTATUS(st) == 0;
}

static int pairs(void) {
    int sv[2];
    if (socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, sv) != 0) return 1;
    /* Roura pres SCM_RIGHTS: prijemce pise do prijateho konce. */
    int p[2];
    if (pipe(p) != 0) return 2;
    if (send_fd(sv[0], "x", 1, p[1]) != 1) return 3;
    close(p[1]);
    char b[16];
    int got = -1;
    if (recv_fd(sv[1], b, sizeof b, &got) != 1 || b[0] != 'x' || got < 0) return 4;
    if (write(got, "roura", 5) != 5) return 5;
    close(got);
    if (read(p[0], b, sizeof b) != 5 || memcmp(b, "roura", 5) != 0) return 6;
    close(p[0]);
    /* Hranice zprav s fd a data bez fd. */
    if (send(sv[0], "abc", 3, 0) != 3 || send(sv[0], "de", 2, 0) != 2) return 7;
    int n = 0, k;
    while (n < 5 && (k = (int)recv(sv[1], b + n, sizeof b - n, 0)) > 0) n += k;
    if (n != 5 || memcmp(b, "abcde", 5) != 0) return 8;
    struct ucred cr;
    socklen_t cl = sizeof cr;
    if (getsockopt(sv[0], SOL_SOCKET, SO_PEERCRED, &cr, &cl) != 0 || cr.uid != getuid()) return 9;
    int ty = 0;
    cl = sizeof ty;
    if (getsockopt(sv[0], SOL_SOCKET, SO_TYPE, &ty, &cl) != 0 || ty != SOCK_STREAM) return 10;
    /* Neblokujici prazdny = EAGAIN; konec druhe strany = POLLHUP, 0, EPIPE. */
    fcntl(sv[1], F_SETFL, O_NONBLOCK);
    if (recv(sv[1], b, sizeof b, 0) != -1 || errno != EAGAIN) return 11;
    close(sv[0]);
    struct pollfd pf = {sv[1], POLLIN, 0};
    if (poll(&pf, 1, 1000) != 1 || !(pf.revents & (POLLHUP | POLLIN))) return 12;
    if (recv(sv[1], b, sizeof b, 0) != 0) return 13;
    signal(SIGPIPE, SIG_IGN);
    if (send(sv[1], "x", 1, 0) != -1 || errno != EPIPE) return 14;
    close(sv[1]);
    if (socket(AF_UNIX, SOCK_SEQPACKET, 0) != -1 || errno != EPROTONOSUPPORT) return 15;
    return 0;
}

/* process.c:800: konec socketpair zdedi potomek, druhy se preda pres
 * SCM_RIGHTS (tady sam sobe) a mluvi se s potomkem pres prijaty konec. */
static int handoff(void) {
    int sv[2], ctl[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) != 0 || socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, ctl) != 0) return 20;
    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_adddup2(&fa, sv[1], FD_PAIR);
    pid_t pid = spawn_child("unixpair", NULL, &fa);
    posix_spawn_file_actions_destroy(&fa);
    close(sv[1]);
    if (pid <= 0) return 21;
    if (send_fd(ctl[0], "s", 1, sv[0]) != 1) return 22;
    close(sv[0]);
    char b[8];
    int s = -1;
    if (recv_fd(ctl[1], b, sizeof b, &s) != 1 || s < 0) return 23;
    if (read(s, b, sizeof b) != 4 || memcmp(b, "pair", 4) != 0) return 24;
    if (write(s, "ok", 2) != 2) return 25;
    if (!child_ok(pid)) return 26;
    close(s);
    close(ctl[0]);
    close(ctl[1]);
    return 0;
}

static int named(const char *dir) {
    char cwd[256];
    if (getcwd(cwd, sizeof cwd) == NULL || chdir(dir) != 0) return 30;
    unlink("socket");
    char data[300];
    snprintf(data, sizeof data, "%s/socket.data", dir);
    FILE *f = fopen(data, "w");
    if (f == NULL) return 31;
    fputs("obsah souboru", f);
    fclose(f);
    int l = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un a;
    memset(&a, 0, sizeof a);
    a.sun_family = AF_UNIX;
    strcpy(a.sun_path, "socket");
    /* Relativni cesta vuci pracovnimu adresari. */
    if (l < 0 || bind(l, (struct sockaddr *)&a, sizeof a) != 0 || listen(l, 5) != 0) return 32;
    int l2 = socket(AF_UNIX, SOCK_STREAM, 0);
    if (bind(l2, (struct sockaddr *)&a, sizeof a) != -1 || errno != EADDRINUSE) return 33;
    close(l2);
    struct stat st;
    if (lstat("socket", &st) != 0 || !S_ISSOCK(st.st_mode) || st.st_uid != getuid()) return 34;
    fcntl(l, F_SETFL, O_NONBLOCK);
    if (accept(l, NULL, NULL) != -1 || errno != EAGAIN) return 35;
    struct pollfd pf = {l, POLLIN, 0};
    if (poll(&pf, 1, 0) != 0) return 36;

    char path[300];
    snprintf(path, sizeof path, "%s/socket", dir);
    pid_t pid = spawn_child("unix", path, NULL);
    if (pid <= 0) return 37;
    if (poll(&pf, 1, 10000) != 1 || !(pf.revents & POLLIN)) return 38;
    int c = accept4(l, NULL, NULL, SOCK_CLOEXEC);
    if (c < 0) return 39;
    struct ucred cr;
    socklen_t cl = sizeof cr;
    if (getsockopt(c, SOL_SOCKET, SO_PEERCRED, &cr, &cl) != 0 || cr.uid != getuid() || cr.pid <= 0) return 40;
    char b[32];
    int fd = -1;
    if (recv_fd(c, b, sizeof b, &fd) != 6 || memcmp(b, "soubor", 6) != 0 || fd < 0) return 41;
    int n = (int)read(fd, b, sizeof b);
    if (n != 13 || memcmp(b, "obsah souboru", 13) != 0) return 42;
    close(fd);
    int d = open(dir, O_RDONLY | O_DIRECTORY);
    if (d < 0 || send_fd(c, "adresar!", 8, d) != 8) return 43;
    close(d);
    n = 0;
    int k;
    while ((k = (int)recv(c, b + n, sizeof b - n, 0)) > 0) n += k;
    if (k != 0 || n != 5 || memcmp(b, "konec", 5) != 0) return 44;
    close(c);
    if (!child_ok(pid)) return 45;
    struct sockaddr_un got;
    socklen_t gl = sizeof got;
    if (getsockname(l, (struct sockaddr *)&got, &gl) != 0 || strcmp(got.sun_path, path) != 0) return 46;

    /* Posluchac skoncil: uzel zustal, connect = ECONNREFUSED; bez uzlu ENOENT. */
    close(l);
    int s = socket(AF_UNIX, SOCK_STREAM, 0);
    if (connect(s, (struct sockaddr *)&a, sizeof a) != -1 || errno != ECONNREFUSED) return 47;
    unlink("socket");
    if (connect(s, (struct sockaddr *)&a, sizeof a) != -1 || errno != ENOENT) return 48;
    close(s);
    unlink("socket.data");
    return chdir(cwd) == 0 ? 0 : 49;
}

int unix_test(const char *dir) {
    int r;
    if ((r = pairs()) != 0) return r;
    if ((r = handoff()) != 0) return r;
    return named(dir);
}
