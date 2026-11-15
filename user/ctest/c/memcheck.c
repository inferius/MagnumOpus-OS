/*
 * Test pametoveho fd (memfd_create, sys/mman.h, magnumopus/memfd.h):
 * read/write/lseek, mmap MAP_SHARED, SIGBUS za koncem, zvetseni
 * a zmenseni za behu, pecete. Rezim "memfd a|b": dva procesy nad jednim
 * objektem pameti od initu (a zapisuje a zvetsi, b jen cte).
 * Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <fcntl.h>
#include <magnumopus/memfd.h>
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#define PAGE 4096
/* Stitek handlu od initu (user/init/src/tests/cstd.rs). */
#define MEMFD_TAG 0x4D460001u

static sigjmp_buf jump;
static volatile sig_atomic_t buses;

static void on_bus(int sig) {
    (void)sig;
    buses++;
    siglongjmp(jump, 1);
}

/* Precte bajt z adresy; 1 = SIGBUS. */
static int bus_read(volatile char *p, char *out) {
    if (sigsetjmp(jump, 1) != 0) return 1;
    *out = *p;
    return 0;
}

static int bus_write(volatile char *p, char v) {
    if (sigsetjmp(jump, 1) != 0) return 1;
    *p = v;
    return 0;
}

int memfd_test(void) {
    int fd = memfd_create("ctest", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (fd < 0) return 1;
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size != 0 || !S_ISREG(st.st_mode)) return 2;
    if (write(fd, "ahoj", 4) != 4 || fstat(fd, &st) != 0 || st.st_size != 4) return 3;
    char b[8] = {0};
    if (lseek(fd, 0, SEEK_SET) != 0 || read(fd, b, sizeof b) != 4 || memcmp(b, "ahoj", 4) != 0) return 4;
    if (ftruncate(fd, PAGE) != 0) return 5;
    /* Mapovani muze pokryt vic, nez je velikost (az do kapacity). */
    char *m = mmap(NULL, 4 * PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (m == MAP_FAILED) return 6;
    if (memcmp(m, "ahoj", 4) != 0) return 7;
    m[10] = 'x';
    if (pread(fd, b, 1, 10) != 1 || b[0] != 'x') return 8;
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_bus;
    if (sigaction(SIGBUS, &sa, NULL) != 0) return 9;
    char c;
    if (!bus_read(m + 2 * PAGE, &c) || buses != 1) return 10;
    /* Zvetseni za behu: stejne mapovani, nova cast je nulova. */
    if (ftruncate(fd, 3 * PAGE) != 0) return 11;
    if (bus_read(m + 2 * PAGE, &c) || c != 0 || bus_write(m + 2 * PAGE, 'z')) return 12;
    if (pread(fd, b, 1, 2 * PAGE) != 1 || b[0] != 'z') return 13;
    /* Zmenseni: stranka za koncem zmizi i z mapovani. */
    if (ftruncate(fd, PAGE) != 0) return 14;
    if (!bus_write(m + 2 * PAGE, 'q') || buses != 2) return 15;
    if (m[10] != 'x') return 16;
    /* Pecete. */
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_GROW) != 0 || fcntl(fd, F_GET_SEALS) != F_SEAL_GROW) return 17;
    if (ftruncate(fd, 2 * PAGE) != -1 || errno != EPERM) return 18;
    if (ftruncate(fd, 100) != 0 || fstat(fd, &st) != 0 || st.st_size != 100) return 19;
    munmap(m, 4 * PAGE);
    close(fd);
    /* Bez MFD_ALLOW_SEALING pecete nejdou. */
    fd = memfd_create("bez", 0);
    if (fd < 0 || fcntl(fd, F_ADD_SEALS, F_SEAL_WRITE) != -1 || errno != EPERM) return 20;
    close(fd);
    return 0;
}

/* Rezim "memfd a|b": objekt pameti od initu (stitek MEMFD_TAG). */
int memfd_main(const char *side) {
    uint32_t h = mo_start_handle(MEMFD_TAG);
    if (h == 0) return 30;
    if (strcmp(side, "a") == 0) {
        int fd = mo_memfd_import(h, O_RDWR);
        if (fd < 0) return 31;
        char *m = mmap(NULL, 2 * PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        if (m == MAP_FAILED) return 32;
        /* Zvetsit za behu a zapsat pres mapovani. */
        if (ftruncate(fd, 2 * PAGE) != 0) return 33;
        memcpy(m, "ahoj", 5);
        memcpy(m + PAGE, "svete", 6);
        printf("ok memfd-a\nctest: hotovo\n");
        return 0;
    }
    int fd = mo_memfd_import(h, O_RDONLY);
    if (fd < 0) return 40;
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size != 2 * PAGE) return 41;
    char *m = mmap(NULL, 2 * PAGE, PROT_READ, MAP_SHARED, fd, 0);
    if (m == MAP_FAILED) return 42;
    if (strcmp(m, "ahoj") != 0 || strcmp(m + PAGE, "svete") != 0) return 43;
    /* Jen pro cteni: zapis, mapovani pro zapis a zmena velikosti nejdou. */
    if (mmap(NULL, PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0) != MAP_FAILED || errno != EACCES) return 44;
    if (ftruncate(fd, PAGE) != -1) return 45;
    if (write(fd, "x", 1) != -1) return 46;
    printf("ok memfd-b\nctest: hotovo\n");
    return 0;
}
