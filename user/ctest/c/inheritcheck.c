/*
 * Dedeni fd v posix_spawn (T4): potomek dostane fd rodice bez FD_CLOEXEC
 * na stejnych cislech - rouru, soubor (vlastni pozice od spusteni),
 * pametovy fd, dup2 na vysoke cislo a addopen nad 2; fd s FD_CLOEXEC
 * nedostane. Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

/* Cisla, na ktera rodic fd pripravi (potomek je zna). */
enum { FD_PIPE = 20, FD_CLOEXEC_PIPE = 21, FD_FILE = 22, FD_MEM = 23, FD_OPENED = 24, FD_NULL = 25, FD_DUP = 40 };

/* Potomek "child fds": vraci 0, nebo cislo selhane kontroly. */
int inherit_child_main(void) {
    char b[16];
    /* fd s FD_CLOEXEC potomek nema. */
    if (fcntl(FD_CLOEXEC_PIPE, F_GETFD) != -1 || errno != EBADF) return 1;
    /* Soubor od pozice rodice (3). */
    ssize_t n = read(FD_FILE, b, sizeof b);
    if (n != 5 || memcmp(b, "defgh", 5) != 0) return 2;
    struct stat st;
    if (fstat(FD_FILE, &st) != 0 || st.st_size != 8) return 3;
    if (lseek(FD_FILE, 0, SEEK_SET) != 0 || read(FD_FILE, b, 3) != 3 || memcmp(b, "abc", 3) != 0) return 4;
    /* Zapis do zdedeneho souboru je videt i rodici. */
    if (pwrite(FD_FILE, "XY", 2, 0) != 2) return 5;
    /* Pametovy fd: obsah, pozice a zapis. */
    if (pread(FD_MEM, b, 5, 0) != 5 || memcmp(b, "pamet", 5) != 0) return 6;
    if (lseek(FD_MEM, 0, SEEK_CUR) != 5) return 7;
    if (write(FD_MEM, "!", 1) != 1) return 8;
    /* dup2 na vysoke cislo = tentyz soubor, pozice od spusteni. */
    if (pread(FD_DUP, b, 2, 6) != 2 || memcmp(b, "gh", 2) != 0) return 9;
    /* addopen nad 2. */
    if (read(FD_OPENED, b, sizeof b) != 6 || memcmp(b, "druhy\n", 6) != 0) return 10;
    if ((fcntl(FD_OPENED, F_GETFL) & O_ACCMODE) != O_RDONLY) return 11;
    /* addopen(/dev/null) nad 2: cteni konec, zapis se zahodi. */
    if (read(FD_NULL, b, sizeof b) != 0 || write(FD_NULL, "x", 1) != 1) return 13;
    /* Roura: konec rodice zapisu. */
    if (write(FD_PIPE, "ok", 2) != 2) return 12;
    return 0;
}

static int put(const char *path, const char *text) {
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) return -1;
    ssize_t n = write(fd, text, strlen(text));
    close(fd);
    return n == (ssize_t)strlen(text) ? 0 : -1;
}

int inherit_test(const char *dir) {
    char a[256], b[256];
    snprintf(a, sizeof a, "%s/inherit-a.txt", dir);
    snprintf(b, sizeof b, "%s/inherit-b.txt", dir);
    if (put(a, "abcdefgh") != 0 || put(b, "druhy\n") != 0) return 1;

    int p[2], q[2];
    if (pipe2(p, O_CLOEXEC) != 0 || pipe2(q, O_CLOEXEC) != 0) return 2;
    if (dup2(p[1], FD_PIPE) != FD_PIPE) return 3;
    if (dup3(q[1], FD_CLOEXEC_PIPE, O_CLOEXEC) != FD_CLOEXEC_PIPE) return 4;
    close(p[1]);
    int f = open(a, O_RDWR | O_CLOEXEC);
    if (f < 0 || dup2(f, FD_FILE) != FD_FILE || lseek(FD_FILE, 3, SEEK_SET) != 3) return 5;
    close(f);
    int m = memfd_create("inherit", MFD_CLOEXEC);
    if (m < 0 || write(m, "pamet", 5) != 5 || dup2(m, FD_MEM) != FD_MEM) return 6;
    close(m);

    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_adddup2(&fa, FD_FILE, FD_DUP);
    posix_spawn_file_actions_addopen(&fa, FD_OPENED, b, O_RDONLY, 0);
    posix_spawn_file_actions_addopen(&fa, FD_NULL, "/dev/null", O_RDWR, 0);
    char *argv[] = {"ctest", "child", "fds", NULL};
    pid_t pid = 0;
    int r = posix_spawnp(&pid, "ctest", &fa, NULL, argv, environ);
    posix_spawn_file_actions_destroy(&fa);
    /* Konec zapisu ma uz jen potomek: EOF po jeho konci. */
    close(FD_PIPE);
    if (r != 0) return 7;
    char got[8];
    size_t n = 0;
    ssize_t k;
    while (n < sizeof got && (k = read(p[0], got + n, sizeof got - n)) > 0) n += (size_t)k;
    int st = -1;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st)) return 8;
    if (WEXITSTATUS(st) != 0) {
        fprintf(stderr, "ctest inherit: potomek krok %d\n", WEXITSTATUS(st));
        return 9;
    }
    if (n != 2 || memcmp(got, "ok", 2) != 0) return 10;
    /* Pozice rodice se nezmenila (potomek mel vlastni), zapis potomka je videt. */
    if (lseek(FD_FILE, 0, SEEK_CUR) != 3) return 11;
    char c[8];
    if (pread(FD_FILE, c, 8, 0) != 8 || memcmp(c, "XYcdefgh", 8) != 0) return 12;
    if (pread(FD_MEM, c, 6, 0) != 6 || memcmp(c, "pamet!", 6) != 0) return 13;

    /* Socket zdedit nejde: dup2 socketu = ENOTSUP. */
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s >= 0) {
        posix_spawn_file_actions_init(&fa);
        posix_spawn_file_actions_adddup2(&fa, s, 30);
        char *argv2[] = {"ctest", "child", "exit", "0", NULL};
        r = posix_spawnp(&pid, "ctest", &fa, NULL, argv2, environ);
        posix_spawn_file_actions_destroy(&fa);
        close(s);
        if (r != ENOTSUP) return 14;
    }
    close(p[0]);
    close(q[0]);
    close(FD_CLOEXEC_PIPE);
    close(FD_FILE);
    close(FD_MEM);
    unlink(a);
    unlink(b);
    return 0;
}
