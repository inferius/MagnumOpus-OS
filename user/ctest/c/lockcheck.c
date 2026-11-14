/*
 * Test zamku souboru mezi dvema procesy (ctest lock a|b DIR, sada "cstd"):
 * zamky drzi fsd, vlastnik fcntl = proces. Procesy se domlouvaji
 * znackovymi soubory v DIR.
 *
 *  A: zapis [0,100) -> znacka a-ready -> ceka na b-ready -> F_SETLKW zapis
 *     [200,300) (drzi ho B pro cteni) -> po ziskani a-got, konec (uvolni vse).
 *  B: ceka na a-ready -> F_SETLK zapis [0,100) = EAGAIN, F_GETLK ukaze A,
 *     cteni [200,300) -> b-ready -> chvili pocka (A ceka na B) -> F_SETLKW
 *     zapis [0,100) = EDEADLK (cyklus) -> odemkne [200,300) -> po konci A
 *     F_SETLKW [0,100) uspeje; flock LOCK_EX|LOCK_NB na dvou fd.
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/file.h>
#include <unistd.h>

static char dir[200];

static void path(char *out, size_t n, const char *name) { snprintf(out, n, "%s/%s", dir, name); }

static void mark(const char *name) {
    char p[256];
    path(p, sizeof p, name);
    int fd = open(p, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd >= 0) close(fd);
}

static int wait_mark(const char *name) {
    char p[256];
    path(p, sizeof p, name);
    for (int i = 0; i < 2000; i++) {
        if (access(p, F_OK) == 0) return 0;
        usleep(5000);
    }
    return -1;
}

static int lock(int fd, int cmd, short type, off_t start, off_t len) {
    struct flock l = {0};
    l.l_type = type;
    l.l_whence = SEEK_SET;
    l.l_start = start;
    l.l_len = len;
    return fcntl(fd, cmd, &l);
}

static int side_a(int fd) {
    if (lock(fd, F_SETLK, F_WRLCK, 0, 100) != 0) return 1;
    mark("a-ready");
    if (wait_mark("b-ready") != 0) return 2;
    /* Ceka na B (ten drzi [200,300) pro cteni). */
    if (lock(fd, F_SETLKW, F_WRLCK, 200, 100) != 0) return 3;
    mark("a-got");
    return 0;
}

static int side_b(int fd) {
    if (wait_mark("a-ready") != 0) return 10;
    if (lock(fd, F_SETLK, F_WRLCK, 0, 100) != -1 || errno != EAGAIN) return 11;
    struct flock q = {0};
    q.l_type = F_WRLCK;
    q.l_whence = SEEK_SET;
    q.l_start = 50;
    q.l_len = 1;
    if (fcntl(fd, F_GETLK, &q) != 0 || q.l_type != F_WRLCK || q.l_start != 0 || q.l_len != 100) return 12;
    if (lock(fd, F_SETLK, F_RDLCK, 200, 100) != 0) return 13;
    /* Vlastni zamky se nebiji: zapis na casti vlastniho cteni. */
    if (lock(fd, F_SETLK, F_WRLCK, 250, 10) != 0 || lock(fd, F_SETLK, F_RDLCK, 250, 10) != 0) return 14;
    mark("b-ready");
    usleep(300 * 1000);
    /* A ceka na B, B by cekal na A: cyklus. */
    if (lock(fd, F_SETLKW, F_WRLCK, 0, 100) != -1 || errno != EDEADLK) return 15;
    if (lock(fd, F_SETLK, F_UNLCK, 200, 100) != 0) return 16;
    if (wait_mark("a-got") != 0) return 17;
    /* A konci a jeho zamky zmizi: cekani uspeje. */
    if (lock(fd, F_SETLKW, F_WRLCK, 0, 100) != 0) return 18;
    /* flock: druhy fd stejneho souboru je jiny vlastnik. */
    char p[256];
    path(p, sizeof p, "lock.db");
    int fd2 = open(p, O_RDWR);
    if (flock(fd, LOCK_EX | LOCK_NB) != 0) return 19;
    if (flock(fd2, LOCK_SH | LOCK_NB) != -1 || errno != EAGAIN) return 20;
    if (flock(fd, LOCK_UN) != 0 || flock(fd2, LOCK_SH | LOCK_NB) != 0) return 21;
    close(fd2);
    return 0;
}

int lock_main(const char *side, const char *d) {
    snprintf(dir, sizeof dir, "%s", d);
    char p[256];
    path(p, sizeof p, "lock.db");
    int fd = open(p, O_RDWR | O_CREAT, 0644);
    if (fd < 0) {
        perror("ctest lock: open");
        return 1;
    }
    int r = strcmp(side, "a") == 0 ? side_a(fd) : side_b(fd);
    if (r != 0) {
        fprintf(stderr, "ctest lock %s: krok %d errno %d\n", side, r, errno);
        return 1;
    }
    printf("ok lock-%s\nctest: hotovo\n", side);
    fflush(stdout);
    return 0;
}
