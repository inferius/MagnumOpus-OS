/*
 * mmap souboru pres okna objektu pameti z fsd (MAP_VMO): MAP_PRIVATE =
 * snimek s kopii pri zapisu, MAP_SHARED = tytez ramce ve vsech procesech
 * a zapis do souboru; koherence s read/write v obou smerech, ftruncate
 * zvetsi namapovany soubor, msync, MAP_FIXED do vlastni rezervace
 * PROT_NONE, mapovani pres hranici okna (16 MiB) a sdileni s potomkem
 * (ctest child fmap). Vraci 0 = ok, jinak krok.
 */
#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

/* Okno fsd (fsproto::v2::WINDOW). */
#define WINDOW (16L * 1024 * 1024)

/* Ceka (nejvys ~10 s), az bude bajt *p roven v. */
static int wait_byte(volatile unsigned char *p, unsigned char v) {
    for (int i = 0; i < 10000; i++) {
        if (*p == v) return 1;
        struct timespec ts = {0, 1000000};
        nanosleep(&ts, NULL);
    }
    return 0;
}

/* Potomek "child fmap PATH": sdilene mapovani tehoz souboru; ceka na 'P'
 * od rodice (pres jeho mapovani), odpovi 'C' do bajtu 1 (pres sve). */
int fmap_child_main(const char *path) {
    long page = sysconf(_SC_PAGESIZE);
    int fd = open(path, O_RDWR);
    if (fd < 0) return 1;
    volatile unsigned char *m = mmap(NULL, (size_t)page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (m == MAP_FAILED) return 2;
    if (!wait_byte(m, 'P')) return 3;
    m[1] = 'C';
    if (msync((void *)m, (size_t)page, MS_SYNC) != 0) return 4;
    munmap((void *)m, (size_t)page);
    close(fd);
    return 0;
}

/* MAP_PRIVATE: snimek, zapis jen do kopie. */
static int private_maps(const char *path, long page) {
    size_t size = 3 * (size_t)page + 100;
    static unsigned char data[4 * 65536];
    if (size > sizeof data) return 1;
    for (size_t i = 0; i < size; i++) data[i] = (unsigned char)(i * 13 + 7);
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    if (fd < 0 || write(fd, data, size) != (ssize_t)size) return 2;
    unsigned char *p = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
    if (p == MAP_FAILED) return 3;
    if (memcmp(p, data, size) != 0) return 4;
    p[0] = 0xAA;
    p[size - 1] = 0xBB;
    unsigned char b[2];
    if (pread(fd, b, 1, 0) != 1 || b[0] != data[0]) return 5;
    unsigned char *q = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    if (q == MAP_FAILED || q[0] != data[0] || q[size - 1] != data[size - 1]) return 6;
    /* Od offsetu stranky. */
    unsigned char *o = mmap(NULL, (size_t)page, PROT_READ, MAP_PRIVATE, fd, page);
    if (o == MAP_FAILED || memcmp(o, data + page, (size_t)page) != 0) return 7;
    /* Zmena souboru: dalsi mapovani ji vidi, starsi snimek ne. */
    if (pwrite(fd, "ZMENA", 5, 0) != 5) return 8;
    unsigned char *n = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    if (n == MAP_FAILED || memcmp(n, "ZMENA", 5) != 0 || n[size - 1] != data[size - 1]) return 9;
    if (q[1] != data[1]) return 10;
    /* Rozsah za koncem souboru jde namapovat (posledni stranka s nulami). */
    unsigned char *big = mmap(NULL, 8 * (size_t)page, PROT_READ, MAP_PRIVATE, fd, 0);
    if (big == MAP_FAILED || memcmp(big, "ZMENA", 5) != 0 || big[size] != 0) return 11;
    /* Zavreni fd mapovani nerusi. */
    close(fd);
    if (q[1] != data[1] || p[0] != 0xAA) return 12;
    munmap(p, size);
    munmap(q, size);
    munmap(o, (size_t)page);
    munmap(n, size);
    munmap(big, 8 * (size_t)page);
    return 0;
}

/* MAP_SHARED se zapisem: koherence, ftruncate, msync, trvalost. */
static int shared_maps(const char *path, long page) {
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    if (fd < 0 || write(fd, "abcdefghij", 10) != 10) return 20;
    /* Jen pro cteni otevreny fd: sdileny zapis = EACCES. */
    int ro = open(path, O_RDONLY);
    if (ro < 0 || mmap(NULL, (size_t)page, PROT_READ | PROT_WRITE, MAP_SHARED, ro, 0) != MAP_FAILED || errno != EACCES) return 21;
    close(ro);
    /* Mapovani 4 stranek souboru o 10 bajtech (za koncem = SIGBUS). */
    size_t len = 4 * (size_t)page;
    unsigned char *s = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (s == MAP_FAILED || memcmp(s, "abcdefghij", 10) != 0 || s[10] != 0) return 22;
    /* mmap -> read: zapis do mapovani vidi pread hned. */
    s[0] = 'S';
    unsigned char b[4];
    if (pread(fd, b, 1, 0) != 1 || b[0] != 'S') return 23;
    /* write -> mmap: pwrite vidi mapovani hned. */
    if (pwrite(fd, "W", 1, 5) != 1 || s[5] != 'W') return 24;
    /* Druhe sdilene mapovani tehoz souboru = tytez ramce. */
    unsigned char *s2 = mmap(NULL, (size_t)page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (s2 == MAP_FAILED || s2[0] != 'S') return 25;
    s2[2] = 'D';
    if (s[2] != 'D') return 26;
    munmap(s2, (size_t)page);
    /* Zapis za konec pres write zvetsi soubor i mapovani. */
    if (pwrite(fd, "X", 1, (off_t)page + 1) != 1 || s[page + 1] != 'X') return 27;
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size != page + 2) return 28;
    /* ftruncate: zvetseni = dalsi stranky v mapovani (nuly), zapis jde do souboru. */
    if (ftruncate(fd, 3 * (off_t)page) != 0) return 29;
    if (s[2 * page + 7] != 0) return 30;
    s[2 * page + 7] = 'T';
    if (pread(fd, b, 1, 2 * (off_t)page + 7) != 1 || b[0] != 'T') return 31;
    /* Zmenseni: zbytek posledni stranky za koncem jsou nuly. */
    s[page] = 'Y';
    if (ftruncate(fd, (off_t)page + 1) != 0 || s[page + 1] != 0 || s[page] != 'Y') return 32;
    if (fstat(fd, &st) != 0 || st.st_size != page + 1) return 33;
    /* msync: MS_SYNC zapise, MS_ASYNC je zadost, obe naraz = EINVAL. */
    if (msync(s, len, MS_SYNC) != 0 || msync(s, len, MS_ASYNC) != 0) return 34;
    if (msync(s, len, MS_SYNC | MS_ASYNC) == 0 || errno != EINVAL) return 35;
    /* Zmena pres mapovani po close fd a munmap zustane v souboru. */
    close(fd);
    s[3] = 'M';
    if (munmap(s, len) != 0) return 36;
    fd = open(path, O_RDONLY);
    if (fd < 0 || pread(fd, b, 4, 0) != 4 || memcmp(b, "SbDM", 4) != 0) return 37;
    close(fd);
    return 0;
}

static long ms_since(const struct timespec *a) {
    struct timespec b;
    clock_gettime(CLOCK_MONOTONIC, &b);
    return (b.tv_sec - a->tv_sec) * 1000 + (b.tv_nsec - a->tv_nsec) / 1000000;
}

/* MAP_FIXED do vlastni rezervace PROT_NONE (Wine), mapovani, read/write
 * a msync pres hranici okna fsd; cena prvniho mapovani plneho okna. */
static int fixed_and_windows(const char *path, long page) {
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    if (fd < 0) return 40;
    /* Plne okno dat (ne ridky soubor: plneni cte skutecne bloky). */
    static unsigned char chunk[1024 * 1024];
    for (size_t i = 0; i < sizeof chunk; i++) chunk[i] = (unsigned char)(i * 7 + 1);
    for (long at = 0; at < WINDOW; at += (long)sizeof chunk)
        if (pwrite(fd, chunk, sizeof chunk, at) != (ssize_t)sizeof chunk) return 48;
    if (fsync(fd) != 0) return 53;
    /* Prvni mapovani nacte cele okno (16 MiB) z FS, druhe uz ne. */
    struct timespec t0;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    unsigned char *f1 = mmap(NULL, (size_t)page, PROT_READ, MAP_SHARED, fd, 0);
    long first = ms_since(&t0);
    clock_gettime(CLOCK_MONOTONIC, &t0);
    unsigned char *f2 = mmap(NULL, (size_t)page, PROT_READ, MAP_SHARED, fd, page);
    long second = ms_since(&t0);
    if (f1 == MAP_FAILED || f2 == MAP_FAILED || f1[1] != chunk[1] || f2[0] != chunk[page]) return 49;
    printf("ctest fmap: prvni mapovani okna 16 MiB %ld ms, dalsi %ld ms\n", first, second);
    munmap(f1, (size_t)page);
    munmap(f2, (size_t)page);
    /* Soubor pres hranici okna: znacky tesne pred a za ni. */
    if (pwrite(fd, "pred", 4, WINDOW - 4) != 4 || pwrite(fd, "za", 2, WINDOW) != 2) return 41;
    unsigned char *r = mmap(NULL, 4 * (size_t)page, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (r == MAP_FAILED) return 42;
    unsigned char *a = mmap(r + page, (size_t)page, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_FIXED, fd, WINDOW - page);
    if (a != r + page || memcmp(a + page - 4, "pred", 4) != 0) return 43;
    unsigned char *c = mmap(r + 2 * page, (size_t)page, PROT_READ, MAP_PRIVATE | MAP_FIXED, fd, WINDOW);
    if (c != r + 2 * page || memcmp(c, "za", 2) != 0) return 44;
    /* Sdilene mapovani pres hranici okna je souvisle. */
    unsigned char *w = mmap(NULL, 2 * (size_t)page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, WINDOW - page);
    if (w == MAP_FAILED || memcmp(w + page - 4, "predza", 6) != 0) return 45;
    w[page - 1] = 'Q';
    w[page] = 'R';
    if (a[page - 1] != 'Q') return 46;
    unsigned char b[2];
    if (pread(fd, b, 2, WINDOW - 1) != 2 || b[0] != 'Q' || b[1] != 'R') return 47;
    /* write pres hranici oken (obe nactena) vidi mapovani v obou. */
    if (pwrite(fd, "xy", 2, WINDOW - 1) != 2 || w[page - 1] != 'x' || w[page] != 'y') return 50;
    /* msync rozsahu pres hranici zapise obe okna. */
    w[page - 2] = 'm';
    w[page + 1] = 'n';
    if (msync(w, 2 * (size_t)page, MS_SYNC) != 0) return 51;
    munmap(w, 2 * (size_t)page);
    /* Po munmap (zapis zpet) jsou zmeny v souboru i pro nove otevreni. */
    int fd2 = open(path, O_RDONLY);
    unsigned char e[4];
    if (fd2 < 0 || pread(fd2, e, 4, WINDOW - 2) != 4 || memcmp(e, "mxyn", 4) != 0) return 52;
    close(fd2);
    munmap(r, 4 * (size_t)page);
    close(fd);
    return 0;
}

/* Sdileni s jinym procesem: tytez ramce, zapis potomka vidi rodic. */
static int two_processes(const char *path, long page) {
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    if (fd < 0 || write(fd, "--", 2) != 2) return 60;
    volatile unsigned char *m = mmap(NULL, (size_t)page, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (m == MAP_FAILED) return 61;
    char *argv[] = {"ctest", "child", "fmap", (char *)path, NULL};
    pid_t pid = 0;
    if (posix_spawnp(&pid, "ctest", NULL, NULL, argv, environ) != 0) return 62;
    m[0] = 'P';
    if (!wait_byte(m + 1, 'C')) return 63;
    int st = 0;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0) return 64;
    unsigned char b[2];
    if (pread(fd, b, 2, 0) != 2 || b[0] != 'P' || b[1] != 'C') return 65;
    munmap((void *)m, (size_t)page);
    close(fd);
    return 0;
}

int fmap_test(const char *dir) {
    long page = sysconf(_SC_PAGESIZE);
    char path[300];
    snprintf(path, sizeof path, "%s/fmap.bin", dir);
    int r = private_maps(path, page);
    if (r == 0) r = shared_maps(path, page);
    if (r == 0) r = two_processes(path, page);
    /* Novy soubor (jiny objekt): fsd nema nactene okno, mereni plneni. */
    char wpath[300];
    snprintf(wpath, sizeof wpath, "%s/fmapw.bin", dir);
    if (r == 0) r = fixed_and_windows(wpath, page);
    if (r != 0) return r;
    return unlink(path) == 0 && unlink(wpath) == 0 ? 0 : 99;
}
