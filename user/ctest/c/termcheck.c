/*
 * Test termios.h v cstd proti testovacimu terminalu initu (ridici kanal
 * TTY, protokol ttyproto): isatty, tcgetattr/tcsetattr, cfmakeraw,
 * TIOCGWINSZ/TIOCSWINSZ a SIGWINCH, ENOTTY u souboru, ffs.
 * Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <strings.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

static volatile sig_atomic_t winch;

static void on_winch(int sig) {
    (void)sig;
    winch++;
}

int termios_test(const char *dir) {
    /* Init ctest nedava stdin (jen stdout a stderr): terminal je za 1 a 2. */
    if (!isatty(1) || !isatty(2)) return 1;
    struct termios t, raw, back;
    if (tcgetattr(1, &t) != 0 || !(t.c_lflag & ICANON) || !(t.c_lflag & ECHO) || t.c_cc[VINTR] != 3) return 2;
    raw = t;
    cfmakeraw(&raw);
    if (tcsetattr(1, TCSANOW, &raw) != 0) return 3;
    if (tcgetattr(2, &back) != 0 || (back.c_lflag & (ICANON | ECHO)) || (back.c_oflag & OPOST)) return 4;
    if (tcsetattr(1, TCSADRAIN, &t) != 0 || tcgetattr(1, &back) != 0 || !(back.c_lflag & ICANON)) return 5;
    if (tcsetattr(1, 42, &t) != -1 || errno != EINVAL) return 6;
    if (tcflush(1, TCIFLUSH) != 0 || tcdrain(1) != 0) return 7;

    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_winch;
    if (sigaction(SIGWINCH, &sa, NULL) != 0) return 8;
    struct winsize w;
    if (ioctl(1, TIOCGWINSZ, &w) != 0 || w.ws_row != 24 || w.ws_col != 80) return 9;
    struct winsize big = {30, 100, 0, 0};
    if (ioctl(1, TIOCSWINSZ, &big) != 0) return 10;
    for (int i = 0; i < 200 && winch == 0; i++) usleep(10000);
    if (winch != 1) return 11;
    if (ioctl(2, TIOCGWINSZ, &w) != 0 || w.ws_row != 30 || w.ws_col != 100) return 12;

    /* Soubor terminal neni. */
    char path[256];
    snprintf(path, sizeof path, "%s/tty.txt", dir);
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) return 13;
    if (isatty(fd) || errno != ENOTTY) return 14;
    if (tcgetattr(fd, &back) != -1 || errno != ENOTTY) return 15;
    close(fd);
    unlink(path);
    if (ffs(0) != 0 || ffs(0x18) != 4 || ffsll(1LL << 40) != 41) return 16;
    return 0;
}
