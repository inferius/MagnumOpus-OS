/*
 * Symbolicke odkazy v cstd podle Wine 11 (dosdevices/c: -> ../drive_c,
 * z: -> /, com1 -> /dev/ttyS0): symlink relativne k cwd, jmena s ':',
 * visici odkaz, prepis = unlink + symlink, nasledovani v prostredni slozce
 * i pres openat/fstatat/fchdir, lstat a fstatat(AT_SYMLINK_NOFOLLOW) =
 * S_IFLNK, readlink (presny text, zkraceni), st_dev/st_ino cile, readdir
 * "/" a ".." pres z:, realpath, O_NOFOLLOW a smycka = ELOOP, st_nlink 1.
 * Vraci 0 = ok, jinak cislo kroku.
 */
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int put(const char *path, const char *text) {
    FILE *f = fopen(path, "w");
    if (f == NULL) return -1;
    fputs(text, f);
    return fclose(f);
}

/* Je jmeno `name` ve vypisu adresare `dir`? */
static int listed(const char *dir, const char *name) {
    DIR *d = opendir(dir);
    if (d == NULL) return 0;
    struct dirent *e;
    int found = 0;
    while ((e = readdir(d)) != NULL)
        if (strcmp(e->d_name, name) == 0) found = 1;
    closedir(d);
    return found;
}

int link_test(const char *dir) {
    char base[256], cwd[PATH_MAX], buf[64], rp[PATH_MAX], p[600];
    struct stat st, st2;
    if (getcwd(cwd, sizeof cwd) == NULL) return 1;
    snprintf(base, sizeof base, "%s/prefix", dir);
    if (mkdir(base, 0700) != 0 && errno != EEXIST) return 2;
    if (chdir(base) != 0) return 3;
    if (mkdir("drive_c", 0700) != 0 && errno != EEXIST) return 4;
    if (mkdir("drive_c/windows", 0700) != 0 && errno != EEXIST) return 5;
    if (mkdir("drive_c/windows/system32", 0700) != 0 && errno != EEXIST) return 6;
    if (put("drive_c/windows/system32/kernel32.dll", "MZ") != 0 || put("drive_c/win.ini", "[windows]\n") != 0) return 7;
    if (mkdir("dosdevices", 0700) != 0 && errno != EEXIST) return 8;
    unlink("dosdevices/c:");
    unlink("dosdevices/c::");
    unlink("dosdevices/z:");
    unlink("dosdevices/com1");
    /* 1, 2: relativne k cwd, jmena s ':', cil od slozky odkazu. */
    if (symlink("../drive_c", "dosdevices/c:") != 0) return 9;
    if (symlink("../drive_c", "dosdevices/c::") != 0) return 10;
    if (symlink("/", "dosdevices/z:") != 0) return 11;
    /* 3: visici odkaz: vytvoreni, lstat, readlink jdou, stat = ENOENT. */
    if (symlink("/dev/ttyS0", "dosdevices/com1") != 0) return 12;
    if (lstat("dosdevices/com1", &st) != 0 || !S_ISLNK(st.st_mode) || (st.st_mode & 0777) != 0777) return 13;
    if (readlink("dosdevices/com1", buf, sizeof buf) != 10 || memcmp(buf, "/dev/ttyS0", 10) != 0) return 14;
    if (stat("dosdevices/com1", &st) != -1 || errno != ENOENT) return 15;
    /* 4: symlink na existujici jmeno = EEXIST, prepis = unlink + symlink. */
    if (symlink("../jinam", "dosdevices/c:") != -1 || errno != EEXIST) return 16;
    if (unlink("dosdevices/c::") != 0 || symlink("../drive_c/windows", "dosdevices/c::") != 0) return 17;
    if (readlink("dosdevices/c::", buf, sizeof buf) != 18 || memcmp(buf, "../drive_c/windows", 18) != 0) return 18;
    /* 5: readlink presny text bez nuly, zkraceni na maly buffer. */
    memset(buf, 'x', sizeof buf);
    if (readlink("dosdevices/c:", buf, sizeof buf) != 10 || memcmp(buf, "../drive_c", 10) != 0 || buf[10] != 'x') return 19;
    if (readlink("dosdevices/c:", buf, 3) != 3 || memcmp(buf, "../", 3) != 0) return 20;
    if (readlink("drive_c/win.ini", buf, sizeof buf) != -1 || errno != EINVAL) return 21;
    /* 5: prostredni slozka, openat/fstatat pres dirfd, lstat/NOFOLLOW. */
    if (stat("dosdevices/c:/windows/system32/kernel32.dll", &st) != 0 || !S_ISREG(st.st_mode)) return 22;
    int d = open("dosdevices", O_RDONLY | O_DIRECTORY);
    if (d < 0) return 23;
    int fd = openat(d, "c:/windows/system32/kernel32.dll", O_RDONLY);
    if (fd < 0 || read(fd, buf, sizeof buf) != 2 || memcmp(buf, "MZ", 2) != 0) return 24;
    close(fd);
    if (fstatat(d, "c:/windows", &st, 0) != 0 || !S_ISDIR(st.st_mode)) return 25;
    if (fstatat(d, "c:", &st, AT_SYMLINK_NOFOLLOW) != 0 || !S_ISLNK(st.st_mode)) return 26;
    if (lstat("dosdevices/c:", &st) != 0 || !S_ISLNK(st.st_mode)) return 27;
    if (readlinkat(d, "z:", buf, sizeof buf) != 1 || buf[0] != '/') return 28;
    /* 6: stat pres odkaz = st_dev/st_ino cile. */
    if (stat("dosdevices/c:", &st) != 0 || stat("drive_c", &st2) != 0) return 29;
    if (st.st_dev != st2.st_dev || st.st_ino != st2.st_ino || !S_ISDIR(st.st_mode)) return 30;
    /* 8: bezny soubor st_nlink 1 (registry.c Wine). */
    if (stat("drive_c/win.ini", &st) != 0 || st.st_nlink != 1) return 31;
    /* 1: z: = koren jmenneho prostoru relace; readdir "/" a ".." pres z:. */
    const char *top = dir[0] == '/' ? dir + 1 : dir;
    char first[64];
    snprintf(first, sizeof first, "%.*s", (int)strcspn(top, "/"), top);
    if (!listed("dosdevices/z:", first)) return 32;
    if (!listed("dosdevices/z:/..", first)) return 33;
    snprintf(p, sizeof p, "dosdevices/z:%s/prefix/drive_c/win.ini", dir);
    if (stat(p, &st) != 0 || !S_ISREG(st.st_mode)) return 34;
    /* 5: O_DIRECTORY + fchdir pres odkaz: cwd je fyzicka cesta cile. */
    int cd = open("dosdevices/c:", O_RDONLY | O_DIRECTORY);
    if (cd < 0 || fchdir(cd) != 0) return 35;
    close(cd);
    char here[PATH_MAX], want[PATH_MAX];
    snprintf(want, sizeof want, "%s/drive_c", base);
    if (getcwd(here, sizeof here) == NULL || strcmp(here, want) != 0) return 36;
    if (chdir(base) != 0) return 37;
    /* 7: realpath pres odkazy a .. */
    snprintf(want, sizeof want, "%s/drive_c/win.ini", base);
    if (realpath("dosdevices/c:/../dosdevices/c:/win.ini", rp) == NULL || strcmp(rp, want) != 0) return 38;
    /* c:: -> ../drive_c/windows, ".." od cile = drive_c (ne dosdevices). */
    snprintf(want, sizeof want, "%s/drive_c", base);
    if (realpath("dosdevices/c::/..", rp) == NULL || strcmp(rp, want) != 0) return 39;
    /* O_NOFOLLOW na odkaz = ELOOP; smycka = ELOOP. */
    if (open("dosdevices/c:", O_RDONLY | O_NOFOLLOW) != -1 || errno != ELOOP) return 40;
    unlink("l1");
    unlink("l2");
    if (symlink("l2", "l1") != 0 || symlink("l1", "l2") != 0) return 41;
    if (stat("l1", &st) != -1 || errno != ELOOP) return 42;
    /* *at: odkaz relativne k adresari, unlink pusobi na odkaz. */
    unlinkat(d, "d:", 0);
    if (symlinkat("../drive_c/win.ini", d, "d:") != 0) return 43;
    if (fstatat(d, "d:", &st, 0) != 0 || !S_ISREG(st.st_mode)) return 44;
    if (unlinkat(d, "d:", 0) != 0 || stat("drive_c/win.ini", &st) != 0) return 45;
    close(d);
    /* Uklid (rmdir odkazu na adresar = ENOTDIR, unlink ho smaze). */
    if (rmdir("dosdevices/c:") != -1 || errno != ENOTDIR) return 46;
    const char *links[] = {"l1", "l2", "dosdevices/c:", "dosdevices/c::", "dosdevices/z:", "dosdevices/com1"};
    for (size_t i = 0; i < sizeof links / sizeof *links; i++)
        if (unlink(links[i]) != 0) return 47;
    unlink("drive_c/windows/system32/kernel32.dll");
    unlink("drive_c/win.ini");
    rmdir("drive_c/windows/system32");
    rmdir("drive_c/windows");
    rmdir("drive_c");
    rmdir("dosdevices");
    if (chdir(cwd) != 0) return 48;
    return rmdir(base) == 0 ? 0 : 49;
}
