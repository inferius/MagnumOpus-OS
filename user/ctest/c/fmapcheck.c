/*
 * mmap souboru pres objekt pameti z fsd (MAP_VMO): MAP_PRIVATE = klon
 * s kopii pri zapisu (zapis do mapovani soubor nemeni), MAP_SHARED jen pro
 * cteni, mapovani od offsetu, zmena souboru = dalsi mapovani vidi novy
 * obsah, rozsah za koncem jde dosavadni kopii. Vraci 0 = ok, jinak krok.
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

int fmap_test(const char *dir) {
    long page = sysconf(_SC_PAGESIZE);
    char path[300];
    snprintf(path, sizeof path, "%s/fmap.bin", dir);
    size_t size = 3 * (size_t)page + 100;
    static unsigned char data[4 * 65536];
    if (size > sizeof data) return 1;
    for (size_t i = 0; i < size; i++) data[i] = (unsigned char)(i * 13 + 7);
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    if (fd < 0 || write(fd, data, size) != (ssize_t)size) return 2;
    /* MAP_PRIVATE pro cteni i zapis: obsah souboru, zapis jen do kopie. */
    unsigned char *p = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
    if (p == MAP_FAILED) return 3;
    if (memcmp(p, data, size) != 0) return 4;
    p[0] = 0xAA;
    p[size - 1] = 0xBB;
    unsigned char b[2];
    if (pread(fd, b, 1, 0) != 1 || b[0] != data[0]) return 5;
    /* Druhe mapovani vidi soubor, ne zapis prvniho. */
    unsigned char *q = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    if (q == MAP_FAILED || q[0] != data[0] || q[size - 1] != data[size - 1]) return 6;
    /* Od offsetu stranky a sdilene jen pro cteni. */
    unsigned char *o = mmap(NULL, (size_t)page, PROT_READ, MAP_SHARED, fd, page);
    if (o == MAP_FAILED || memcmp(o, data + page, (size_t)page) != 0) return 7;
    /* Zmena souboru: dalsi mapovani dostane novy obsah. */
    if (pwrite(fd, "ZMENA", 5, 0) != 5) return 8;
    unsigned char *n = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    if (n == MAP_FAILED || memcmp(n, "ZMENA", 5) != 0 || n[size - 1] != data[size - 1]) return 9;
    /* Rozsah za koncem objektu: kopie jako drive (nuly za koncem souboru). */
    unsigned char *big = mmap(NULL, 8 * (size_t)page, PROT_READ, MAP_PRIVATE, fd, 0);
    if (big == MAP_FAILED || memcmp(big, "ZMENA", 5) != 0 || big[6 * page] != 0) return 10;
    /* Sdileny zapis zatim nejde. */
    if (mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0) != MAP_FAILED || errno != ENOTSUP) return 11;
    /* Zavreni fd mapovani nerusi. */
    close(fd);
    if (q[1] != data[1] || p[0] != 0xAA) return 12;
    munmap(p, size);
    munmap(q, size);
    munmap(o, (size_t)page);
    munmap(n, size);
    munmap(big, 8 * (size_t)page);
    return unlink(path) == 0 ? 0 : 13;
}
