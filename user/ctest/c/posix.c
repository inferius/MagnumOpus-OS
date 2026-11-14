/*
 * Test casti 2a knihovny cstd: cisla fd (open, read, write, lseek, dup,
 * pipe, fcntl), FILE nad fd (fdopen, fileno), stat, adresare (mkdir,
 * opendir, readdir, rmdir), pracovni adresar a fork bez podpory.
 * Vraci 0 = ok, jinak cislo kroku, ktery selhal.
 */
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/uio.h>
#include <unistd.h>

static int fds(const char *dir) {
    char path[256];
    snprintf(path, sizeof path, "%s/posix.bin", dir);
    unlink(path);

    int fd = open(path, O_RDWR | O_CREAT | O_EXCL, 0644);
    if (fd < 3) return 1;
    if (open(path, O_RDWR | O_CREAT | O_EXCL, 0644) != -1 || errno != EEXIST) return 2;
    if (write(fd, "0123456789", 10) != 10) return 3;
    if (lseek(fd, 0, SEEK_CUR) != 10 || lseek(fd, 2, SEEK_SET) != 2) return 4;

    /* dup sdili pozici. */
    int d = dup(fd);
    char b[8] = {0};
    if (d < 0 || read(d, b, 3) != 3 || memcmp(b, "234", 3) != 0) return 5;
    if (lseek(fd, 0, SEEK_CUR) != 5) return 6;

    /* pread/pwrite pozici nemeni. */
    if (pwrite(fd, "AB", 2, 0) != 2 || pread(fd, b, 4, 0) != 4 || memcmp(b, "AB23", 4) != 0) return 7;
    if (lseek(fd, 0, SEEK_CUR) != 5) return 8;

    /* writev jednim zapisem, fstat velikost. */
    struct iovec v[2] = {{"xy", 2}, {"z", 1}};
    if (lseek(fd, 0, SEEK_END) != 10 || writev(fd, v, 2) != 3) return 9;
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size != 13 || !S_ISREG(st.st_mode)) return 10;
    if (stat(path, &st) != 0 || st.st_size != 13) return 11;

    /* fcntl: priznaky a FD_CLOEXEC, F_DUPFD od minima. */
    if ((fcntl(fd, F_GETFL) & O_ACCMODE) != O_RDWR) return 12;
    if (fcntl(fd, F_SETFD, FD_CLOEXEC) != 0 || fcntl(fd, F_GETFD) != FD_CLOEXEC) return 13;
    int hi = fcntl(fd, F_DUPFD, 50);
    if (hi < 50) return 14;
    if (dup2(fd, 40) != 40 || close(40) != 0 || close(40) != -1 || errno != EBADF) return 15;

    /* FILE nad fd: fdopen cte od aktualni pozice duplikatu. */
    if (lseek(hi, 0, SEEK_SET) != 0) return 16;
    FILE *f = fdopen(hi, "r");
    if (f == NULL || fileno(f) != hi) return 17;
    char line[32];
    if (fgets(line, sizeof line, f) == NULL || strcmp(line, "AB23456789xyz") != 0) return 18;
    if (fclose(f) != 0 || close(hi) != -1) return 19;

    /* ftruncate a cteni na konci. */
    if (ftruncate(fd, 4) != 0 || fstat(fd, &st) != 0 || st.st_size != 4) return 20;
    if (lseek(fd, 0, SEEK_END) != 4 || read(fd, b, 1) != 0) return 21;
    close(fd);
    close(d);
    if (unlink(path) != 0 || access(path, F_OK) != -1 || errno != ENOENT) return 22;
    return 0;
}

static int pipes(void) {
    int p[2];
    if (pipe(p) != 0) return 30;
    if (write(p[1], "roura", 5) != 5) return 31;
    char b[8] = {0};
    if (read(p[0], b, sizeof b) != 5 || memcmp(b, "roura", 5) != 0) return 32;
    if (lseek(p[0], 0, SEEK_SET) != -1 || errno != ESPIPE) return 33;
    if (write(p[0], "x", 1) != -1 || errno != EBADF) return 34;
    close(p[1]);
    if (read(p[0], b, sizeof b) != 0) return 35;
    close(p[0]);
    return 0;
}

static int dirs(const char *dir) {
    char sub[256], file[300];
    snprintf(sub, sizeof sub, "%s/posix-dir", dir);
    snprintf(file, sizeof file, "%s/a.txt", sub);
    unlink(file);
    rmdir(sub);
    if (mkdir(sub, 0755) != 0) return 40;
    if (mkdir(sub, 0755) != -1 || errno != EEXIST) return 41;
    int fd = open(file, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0 || close(fd) != 0) return 42;
    if (rmdir(sub) != -1 || errno != ENOTEMPTY) return 43;

    DIR *d = opendir(sub);
    if (d == NULL) return 44;
    int dots = 0, found = 0;
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        if (strcmp(e->d_name, ".") == 0 || strcmp(e->d_name, "..") == 0) dots++;
        else if (strcmp(e->d_name, "a.txt") == 0 && e->d_type == DT_REG) found++;
        else return 45;
    }
    if (dots != 2 || found != 1 || closedir(d) != 0) return 46;
    if (opendir(file) != NULL || errno != ENOTDIR) return 47;

    /* Pracovni adresar: relativni cesty. */
    char cwd[256];
    if (getcwd(cwd, sizeof cwd) == NULL) return 48;
    if (chdir(sub) != 0) return 49;
    struct stat st;
    if (stat("a.txt", &st) != 0 || !S_ISREG(st.st_mode)) return 50;
    char now[256];
    if (getcwd(now, sizeof now) == NULL || strcmp(now, sub) != 0) return 51;
    if (chdir(cwd) != 0 || chdir(file) != -1 || errno != ENOTDIR) return 52;
    if (stat(sub, &st) != 0 || !S_ISDIR(st.st_mode)) return 53;

    if (unlink(file) != 0 || rmdir(sub) != 0) return 54;
    return 0;
}

int posix_test(const char *dir) {
    int r = fds(dir);
    if (r == 0) r = pipes();
    if (r == 0) r = dirs(dir);
    if (r == 0 && (fork() != -1 || errno != ENOSYS)) r = 60;
    if (r == 0 && (isatty(-1) != 0 || errno != EBADF)) r = 61;
    if (r == 0 && sysconf(_SC_NPROCESSORS_ONLN) < 1) r = 62;
    if (r != 0) fprintf(stderr, "ctest posix: krok %d, errno %d\n", r, errno);
    return r;
}
