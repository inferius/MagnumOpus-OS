/*
 * Test doplnku POSIX v cstd (pro porty a Windows vrstvu): posix_spawn
 * a waitpid (ctest spusti sam sebe v rezimu "child"), semafory, vlakno
 * na vlastnim zasobniku, statfs, fchdir, setsid, times/ctime, getifaddrs,
 * SOCK_RAW a CMSG. Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <fcntl.h>
#include <ifaddrs.h>
#include <net/ethernet.h>
#include <net/if.h>
#include <net/if_arp.h>
#include <netinet/tcp.h>
#include <sys/un.h>
#include <netinet/in.h>
#include <pthread.h>
#include <semaphore.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/statfs.h>
#include <sys/times.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

/* Rezim potomka: "child exit N", "child echo TEXT", "child stdin",
 * "child cwd", "child sleep". */
int spawn_child_main(int argc, char **argv) {
    if (argc == 4 && strcmp(argv[2], "exit") == 0) return atoi(argv[3]);
    if (argc == 4 && strcmp(argv[2], "echo") == 0) {
        printf("%s\n", argv[3]);
        return 0;
    }
    if (argc == 3 && strcmp(argv[2], "stdin") == 0) {
        char b[8];
        return read(0, b, sizeof b) == 0 ? 0 : 1;
    }
    if (argc == 3 && strcmp(argv[2], "count") == 0) {
        /* Precte stdin do konce; kod = pocet bajtu mod 251. */
        char b[4096];
        long total = 0;
        ssize_t k;
        while ((k = read(0, b, sizeof b)) > 0) total += k;
        return k < 0 ? 255 : (int)(total % 251);
    }
    if (argc == 3 && strcmp(argv[2], "cwd") == 0) {
        char b[256];
        if (getcwd(b, sizeof b) == NULL) return 1;
        printf("%s\n", b);
        return 0;
    }
    if (argc == 3 && strcmp(argv[2], "overflow") == 0) {
        /* Pojistka sprintf: buffer znamych 4 bajtu, text delsi = abort. */
        char small[4];
        volatile int n = 123456;
        sprintf(small, "%d", n);
        return 0;
    }
    if (argc == 3 && strcmp(argv[2], "sleep") == 0) {
        sleep(60);
        return 0;
    }
    return 100;
}

/* Spusti "ctest child ..." a vrati pid (nebo -kod chyby). */
static pid_t child(posix_spawn_file_actions_t *fa, const char *a2, const char *a3) {
    char *argv[] = {"ctest", "child", (char *)a2, (char *)a3, NULL};
    pid_t pid = 0;
    int r = posix_spawnp(&pid, "ctest", fa, NULL, argv, environ);
    return r == 0 ? pid : -r;
}

/* Vystup potomka pres rouru na jeho stdout. */
static int child_output(const char *a2, const char *a3, const char *cwd, char *out, size_t n) {
    int fds[2];
    if (pipe(fds) != 0) return -1;
    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_adddup2(&fa, fds[1], 1);
    posix_spawn_file_actions_addclose(&fa, fds[0]);
    posix_spawn_file_actions_addclose(&fa, fds[1]);
    if (cwd != NULL) posix_spawn_file_actions_addchdir_np(&fa, cwd);
    pid_t pid = child(&fa, a2, a3);
    posix_spawn_file_actions_destroy(&fa);
    close(fds[1]);
    if (pid <= 0) {
        close(fds[0]);
        return -2;
    }
    size_t got = 0;
    ssize_t k;
    while (got + 1 < n && (k = read(fds[0], out + got, n - 1 - got)) > 0) got += (size_t)k;
    out[got] = 0;
    close(fds[0]);
    int st = -1;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0) return -3;
    return (int)got;
}

static int spawning(const char *dir) {
    int st = 0;
    pid_t p = child(NULL, "exit", "7");
    if (p <= 1 || p == getpid()) return 1;
    if (waitpid(p, &st, 0) != p || !WIFEXITED(st) || WEXITSTATUS(st) != 7) return 2;
    if (waitpid(p, &st, WNOHANG) != -1 || errno != ECHILD) return 3;

    char buf[256];
    if (child_output("echo", "ahoj z potomka", NULL, buf, sizeof buf) < 0 || strcmp(buf, "ahoj z potomka\n") != 0) return 4;
    char want[300];
    snprintf(want, sizeof want, "%s\n", dir);
    if (child_output("cwd", NULL, dir, buf, sizeof buf) < 0 || strcmp(buf, want) != 0) return 5;

    /* Prazdny vstup z /dev/null. */
    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 0, "/dev/null", O_RDONLY, 0);
    p = child(&fa, "stdin", NULL);
    if (p <= 0 || waitpid(p, &st, 0) != p || !WIFEXITED(st) || WEXITSTATUS(st) != 0) return 6;
    posix_spawn_file_actions_destroy(&fa);

    /* Vystup potomka do souboru (proud z fsd) a vstup ze souboru. */
    char out[256];
    snprintf(out, sizeof out, "%s/spawn-out.txt", dir);
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 1, out, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    p = child(&fa, "echo", "do souboru");
    posix_spawn_file_actions_destroy(&fa);
    if (p <= 0 || waitpid(p, &st, 0) != p || !WIFEXITED(st) || WEXITSTATUS(st) != 0) return 7;
    /* Proud zapisuje fsd: obsah je tam, az fsd zpracuje zpravy. */
    int got = -1;
    for (int i = 0; i < 200; i++) {
        FILE *f = fopen(out, "r");
        if (f != NULL) {
            got = (int)fread(buf, 1, sizeof buf - 1, f);
            fclose(f);
        }
        if (got == 11) break;
        usleep(10000);
    }
    buf[got > 0 ? got : 0] = 0;
    if (strcmp(buf, "do souboru\n") != 0) return 18;
    /* 200 000 bajtu na stdin: vic zprav, plna fronta kanalu. */
    char in[256];
    snprintf(in, sizeof in, "%s/spawn-in.txt", dir);
    FILE *f = fopen(in, "w");
    if (f == NULL) return 19;
    for (int i = 0; i < 200000; i++) fputc('a' + i % 26, f);
    fclose(f);
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 0, in, O_RDONLY, 0);
    p = child(&fa, "count", NULL);
    posix_spawn_file_actions_destroy(&fa);
    if (p <= 0 || waitpid(p, &st, 0) != p || !WIFEXITED(st) || WEXITSTATUS(st) != 200000 % 251) return 20;
    unlink(in);
    unlink(out);
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_adddup2(&fa, 1, 5);
    if (child(&fa, "exit", "0") != -ENOTSUP) return 8;
    posix_spawn_file_actions_destroy(&fa);
    char *argv[] = {"neni", NULL};
    if (posix_spawn(&p, "/neni/takovy/program", NULL, NULL, argv, environ) != ENOENT) return 9;

    /* kill na potomka, stav hlasi signal; WNOHANG na bezicim = 0. */
    p = child(NULL, "sleep", NULL);
    if (p <= 0) return 10;
    if (waitpid(p, &st, WNOHANG) != 0) return 11;
    if (kill(p, 0) != 0 || kill(p, SIGTERM) != 0) return 12;
    if (waitpid(-1, &st, 0) != p || !WIFSIGNALED(st) || WTERMSIG(st) != SIGTERM) return 13;
    if (wait(&st) != -1 || errno != ECHILD) return 14;

    /* sprintf s pojistkou: v mezich funguje, preteceni ukonci potomka. */
    char ok[16];
    if (sprintf(ok, "%d-%s", 42, "x") != 4 || strcmp(ok, "42-x") != 0) return 15;
    p = child(NULL, "overflow", NULL);
    if (p <= 0 || waitpid(p, &st, 0) != p || (WIFEXITED(st) && WEXITSTATUS(st) == 0)) return 16;
    char words[] = "a,b,,c";
    if (strcmp(strtok(words, ","), "a") != 0 || strcmp(strtok(NULL, ","), "b") != 0 ||
        strcmp(strtok(NULL, ","), "c") != 0 || strtok(NULL, ",") != NULL)
        return 17;
    return 0;
}

static sem_t sem;
static int sem_seen;

static void *sem_worker(void *arg) {
    (void)arg;
    if (sem_wait(&sem) == 0) sem_seen = 1;
    return NULL;
}

static int semaphores(void) {
    if (sem_init(&sem, 0, 0) != 0) return 20;
    if (sem_trywait(&sem) != -1 || errno != EAGAIN) return 21;
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    t.tv_nsec += 20000000;
    if (t.tv_nsec >= 1000000000) {
        t.tv_sec++;
        t.tv_nsec -= 1000000000;
    }
    if (sem_timedwait(&sem, &t) != -1 || errno != ETIMEDOUT) return 22;
    pthread_t th;
    if (pthread_create(&th, NULL, sem_worker, NULL) != 0) return 23;
    usleep(20000);
    if (sem_post(&sem) != 0 || pthread_join(th, NULL) != 0 || !sem_seen) return 24;
    int v = -1;
    if (sem_getvalue(&sem, &v) != 0 || v != 0) return 25;
    if (sem_open("/x", O_CREAT, 0600, 1) != SEM_FAILED || errno != ENOSYS) return 26;
    return sem_destroy(&sem) == 0 ? 0 : 27;
}

static char *stack_lo, *stack_hi;

static void *on_stack(void *arg) {
    char local;
    *(int *)arg = &local >= stack_lo && &local < stack_hi;
    return NULL;
}

static int own_stack(void) {
    size_t size = 256 * 1024;
    char *mem = aligned_alloc(4096, size);
    if (mem == NULL) return 30;
    stack_lo = mem;
    stack_hi = mem + size;
    pthread_attr_t a;
    pthread_attr_init(&a);
    if (pthread_attr_setstack(&a, mem, size) != 0) return 31;
    void *got = NULL;
    size_t got_size = 0;
    if (pthread_attr_getstack(&a, &got, &got_size) != 0 || got != mem || got_size != size) return 32;
    if (pthread_attr_setscope(&a, PTHREAD_SCOPE_PROCESS) != ENOTSUP) return 33;
    int inside = 0;
    pthread_t th;
    if (pthread_create(&th, &a, on_stack, &inside) != 0 || pthread_join(th, NULL) != 0) return 34;
    pthread_attr_destroy(&a);
    free(mem);
    return inside ? 0 : 35;
}

static int volumes(const char *dir) {
    struct statfs s;
    if (statfs(dir, &s) != 0 || s.f_type != MOFS_SUPER_MAGIC || s.f_bsize <= 0 || s.f_blocks == 0 ||
        s.f_bfree > s.f_blocks || s.f_namelen != 255)
        return 40;
    if (s.f_frsize != s.f_bsize || s.f_bavail > s.f_blocks) return 41;
    int fd = open(dir, O_RDONLY | O_DIRECTORY);
    if (fd < 0 || fstatfs(fd, &s) != 0) return 42;
    if (statfs("/neni/takovy", &s) != -1 || errno != ENOENT) return 43;
    if (chdir("/") != 0 || fchdir(fd) != 0) return 44;
    char cwd[256];
    if (getcwd(cwd, sizeof cwd) == NULL || strcmp(cwd, dir) != 0) return 45;
    close(fd);
    return chdir("/") == 0 ? 0 : 46;
}

static int misc(void) {
    struct tms t;
    clock_t a = times(&t);
    if (a <= 0 || t.tms_utime != 0) return 50;
    char b[26];
    time_t now = time(NULL);
    if (ctime_r(&now, b) == NULL || strlen(b) != 25 || b[24] != '\n') return 51;
    struct tm tm;
    time_t fixed = 116989432; /* 1973-09-16 01:03:52 UTC */
    if (gmtime_r(&fixed, &tm) == NULL || strcmp(asctime(&tm), "Sun Sep 16 01:03:52 1973\n") != 0) return 52;
    if (setsid() != getpid() || getpgrp() != getpid() || setpgid(0, 0) != 0) return 53;
    if (strcmp(strerror(EOWNERDEAD), "Owner died") != 0) return 54;
    char host[HOST_NAME_MAX + 1];
    if (gethostname(host, sizeof host) != 0 || strlen(host) == 0) return 55;
    if (putenv("CTEST_PUTENV=1") != 0 || getenv("CTEST_PUTENV") == NULL) return 56;
    _Static_assert(sizeof(struct sockaddr_un) == 110, "sockaddr_un");
    _Static_assert(sizeof(struct tcp_info) == 232, "tcp_info");
    _Static_assert(sizeof(struct ether_header) == ETH_HLEN, "ether_header");
    _Static_assert(sizeof(struct arphdr) == 8, "arphdr");
    caddr_t cp = host;
    (void)cp;
    return 0;
}

static int network(void) {
    struct ifaddrs *list;
    if (getifaddrs(&list) != 0) return 60;
    int lo = 0;
    for (struct ifaddrs *i = list; i != NULL; i = i->ifa_next) {
        if (strcmp(i->ifa_name, "lo") == 0 && i->ifa_addr != NULL && i->ifa_addr->sa_family == AF_INET &&
            ((struct sockaddr_in *)i->ifa_addr)->sin_addr.s_addr == htonl(INADDR_LOOPBACK) && (i->ifa_flags & IFF_LOOPBACK))
            lo = 1;
    }
    freeifaddrs(list);
    if (!lo) return 61;
    if (if_nametoindex("lo") != 1 || if_nametoindex("wlan9") != 0) return 62;
    if (socket(AF_INET, SOCK_RAW, IPPROTO_ICMP) != -1 || errno != EPERM) return 63;
    if (CMSG_LEN(sizeof(int)) != 20 || CMSG_SPACE(sizeof(int)) != 24) return 64;
    union {
        char buf[64];
        struct cmsghdr align;
    } ctl;
    struct msghdr m;
    memset(&m, 0, sizeof m);
    m.msg_control = ctl.buf;
    m.msg_controllen = CMSG_SPACE(sizeof(int)) * 2;
    struct cmsghdr *c = CMSG_FIRSTHDR(&m);
    if (c == NULL) return 65;
    c->cmsg_len = CMSG_LEN(sizeof(int));
    c->cmsg_level = SOL_SOCKET;
    c->cmsg_type = SCM_RIGHTS;
    *(int *)CMSG_DATA(c) = 0;
    struct cmsghdr *d = CMSG_NXTHDR(&m, c);
    if (d == NULL || (char *)d != ctl.buf + 24) return 66;
    d->cmsg_len = CMSG_LEN(sizeof(int));
    if (CMSG_NXTHDR(&m, d) != NULL) return 67;
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    if (s >= 0) {
        /* Bez relace site socket nevznikne; s ni sendmsg odmitne SCM_RIGHTS. */
        if (sendmsg(s, &m, 0) != -1 || errno != EOPNOTSUPP) return 68;
        close(s);
    }
    return 0;
}

int posix2_test(const char *dir) {
    int r;
    if ((r = spawning(dir)) != 0) return r;
    if ((r = semaphores()) != 0) return r;
    if ((r = own_stack()) != 0) return r;
    if ((r = volumes(dir)) != 0) return r;
    if ((r = misc()) != 0) return r;
    return network();
}
