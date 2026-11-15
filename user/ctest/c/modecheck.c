/*
 * Mod POSIX jako pohled na granty FS (rozhodnuti T1A): st_mode, mkdir
 * a open(O_CREAT) s modem po umask, chmod/fchmod (vedome rozsireni pro
 * kazdeho), st_uid = getuid. Kontroly jako wineserver (st_mode & 077 == 0
 * u adresare po mkdir 0700). Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <sys/stat.h>
#include <unistd.h>

int mode_test(const char *dir) {
    char d[256], f[300], g[300];
    snprintf(d, sizeof d, "%s/mode-dir", dir);
    snprintf(f, sizeof f, "%s/soubor", d);
    snprintf(g, sizeof g, "%s/dalsi", d);
    rmdir(d);
    if (mkdir(d, 0700) != 0) return 1;
    struct stat st;
    if (stat(d, &st) != 0 || !S_ISDIR(st.st_mode)) return 2;
    if ((st.st_mode & 077) != 0 || (st.st_mode & 0700) != 0700) return 3;
    if (st.st_uid != getuid()) return 4;
    /* chmod 755: vedome sdileni pro kazdeho (grant EVERYONE); group bez
     * prijemce grantu (zadna skupina na objektu) zustane 0. */
    if (chmod(d, 0755) != 0 || stat(d, &st) != 0 || (st.st_mode & 0777) != 0705) return 5;
    if (chmod(d, 0700) != 0 || stat(d, &st) != 0 || (st.st_mode & 0777) != 0700) return 6;
    /* Soubor: vlastnik rw-, umask ubere ostatnim. */
    mode_t old = umask(077);
    int fd = open(f, O_WRONLY | O_CREAT | O_EXCL, 0666);
    umask(old);
    if (fd < 0) return 7;
    if (fstat(fd, &st) != 0 || !S_ISREG(st.st_mode) || (st.st_mode & 0777) != 0600) return 8;
    if (fchmod(fd, 0644) != 0 || fstat(fd, &st) != 0 || (st.st_mode & 0777) != 0604) return 9;
    close(fd);
    if (stat(f, &st) != 0 || (st.st_mode & 0777) != 0604) return 10;
    /* Bity setuid/sticky se ignoruji; mkdir 0777 nic neprida. */
    if (chmod(f, 04600) != 0 || stat(f, &st) != 0 || (st.st_mode & 07777) != 0600) return 11;
    if (mkdir(g, 0777) != 0 || stat(g, &st) != 0 || (st.st_mode & 077) != 0) return 12;
    if (chmod("/neni/takovy", 0700) != -1 || errno != ENOENT) return 13;
    unlink(f);
    rmdir(g);
    return rmdir(d) == 0 ? 0 : 14;
}
