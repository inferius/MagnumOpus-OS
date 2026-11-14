/*
 * Test casti 2f knihovny cstd: mmap, munmap, mprotect, madvise, mseal.
 * Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

int mmap_test(const char *dir) {
    long page = sysconf(_SC_PAGESIZE);
    if (page < 4096 || getpagesize() != page) return 1;

    /* Anonymni pamet: nuly, zapis, cast zpet. */
    size_t len = 64 * (size_t)page;
    unsigned char *p = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) return 2;
    for (size_t i = 0; i < len; i += (size_t)page)
        if (p[i] != 0) return 3;
    memset(p, 0xAB, len);
    if (munmap(p + 32 * page, 32 * (size_t)page) != 0) return 4;
    /* MADV_DONTNEED: dalsi cteni vraci nuly. */
    if (madvise(p, (size_t)page, MADV_DONTNEED) != 0 || p[0] != 0 || p[page] != 0xAB) return 5;
    /* Jen pro cteni. */
    if (mprotect(p, (size_t)page, PROT_READ) != 0 || p[1] != 0) return 6;
    /* Spusteni nezapecetene pameti bez opravneni JIT: W^X. */
    if (mprotect(p + page, (size_t)page, PROT_READ | PROT_EXEC) != -1 || errno != EACCES) return 7;
    if (munmap(p, 32 * (size_t)page) != 0) return 8;

    /* PROT_NONE = rezervace; mprotect doplni pamet. */
    unsigned char *r = mmap(NULL, 16 * (size_t)page, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (r == MAP_FAILED) return 10;
    if (mprotect(r + 4 * page, 2 * (size_t)page, PROT_READ | PROT_WRITE) != 0) return 11;
    r[4 * page] = 1;
    r[6 * page - 1] = 2;
    if (r[4 * page] != 1 || r[6 * page - 1] != 2) return 12;
    /* MAP_FIXED do vlastniho rozsahu nahradi obsah. */
    unsigned char *f = mmap(r + 4 * page, (size_t)page, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    if (f != r + 4 * page || f[0] != 0) return 13;
    if (munmap(r, 16 * (size_t)page) != 0) return 14;

    /* Soubor: obsah v pameti, MAP_SHARED se zapisem ne. */
    char path[256];
    snprintf(path, sizeof path, "%s/mmap.bin", dir);
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) return 20;
    char data[5000];
    for (int i = 0; i < (int)sizeof data; i++) data[i] = (char)('a' + i % 26);
    if (write(fd, data, sizeof data) != (ssize_t)sizeof data) return 21;
    char *m = mmap(NULL, sizeof data, PROT_READ, MAP_PRIVATE, fd, 0);
    if (m == MAP_FAILED || memcmp(m, data, sizeof data) != 0) return 22;
    char *m2 = mmap(NULL, (size_t)page, PROT_READ, MAP_PRIVATE, fd, page);
    if (m2 == MAP_FAILED || m2[0] != data[page] || m2[sizeof data - page] != 0) return 23;
    if (mmap(NULL, 10, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0) != MAP_FAILED || errno != ENOTSUP) return 24;
    if (mmap(NULL, 10, PROT_READ, MAP_PRIVATE, fd, 100) != MAP_FAILED || errno != EINVAL) return 25;
    munmap(m, sizeof data);
    munmap(m2, (size_t)page);
    close(fd);
    unlink(path);

    /* mseal: zapecetenou oblast uz nejde odmapovat. */
    unsigned char *s = mmap(NULL, (size_t)page, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (s == MAP_FAILED || mseal(s, (size_t)page, 0) != 0) return 30;
    if (munmap(s, (size_t)page) != -1) return 31;
    return 0;
}
