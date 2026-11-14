/*
 * Test casti 2g knihovny cstd: signaly (raise, maska, SIGSEGV z vyjimky
 * s opravou pameti a pokracovanim, SIGFPE se siglongjmp, casovac a pause)
 * a setjmp/longjmp. Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <setjmp.h>
#include <signal.h>
#include <stdint.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/time.h>
#include <unistd.h>

static volatile sig_atomic_t usr1, alrm, segv;
static unsigned char *guard_page;
static long page;
#ifdef __x86_64__
static sigjmp_buf fpe_jump;
#endif

static void on_usr1(int s) { usr1 += s; }
static void on_alrm(int s) { (void)s; alrm++; }

static void on_segv(int s, siginfo_t *info, void *uc) {
    (void)s;
    (void)uc;
    /* Opravit: stranka dostane pamet, instrukce se zopakuje (vzor GC/JIT). */
    if ((unsigned char *)info->si_addr >= guard_page && (unsigned char *)info->si_addr < guard_page + page) {
        segv++;
        mprotect(guard_page, (size_t)page, PROT_READ | PROT_WRITE);
    }
}

#ifdef __x86_64__
static void on_fpe(int s) { siglongjmp(fpe_jump, s); }
#endif

static jmp_buf jb;
static void jump_deep(int n) {
    if (n == 0) longjmp(jb, 5);
    jump_deep(n - 1);
}

int signal_test(void) {
    /* setjmp/longjmp: navrat se zachovanymi lokalnimi promennymi (volatile). */
    volatile int local = 1;
    int r = setjmp(jb);
    if (r == 0) {
        local = 2;
        jump_deep(10);
        return 1;
    }
    if (r != 5 || local != 2) return 2;
    if (setjmp(jb) == 0) longjmp(jb, 0);
    /* longjmp(.., 0) vraci 1: sem se dojde az po nem. */

    /* raise a maska. */
    if (signal(SIGUSR1, on_usr1) != SIG_DFL || raise(SIGUSR1) != 0 || usr1 != SIGUSR1) return 10;
    sigset_t set, pend, old;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR1);
    sigprocmask(SIG_BLOCK, &set, &old);
    kill(getpid(), SIGUSR1);
    sigpending(&pend);
    if (usr1 != SIGUSR1 || !sigismember(&pend, SIGUSR1)) return 11;
    sigprocmask(SIG_SETMASK, &old, NULL);
    if (usr1 != 2 * SIGUSR1) return 12;
    if (signal(SIGUSR2, SIG_IGN) != SIG_DFL || raise(SIGUSR2) != 0) return 13;
    if (kill(getpid() + 1, SIGUSR1) != -1 || errno != ESRCH) return 14;
    if (getpid() <= 0 || getpid() != getpid()) return 15;

    /* SIGSEGV z vypadku stranky: obsluha opravi a vypocet pokracuje. */
    page = sysconf(_SC_PAGESIZE);
    guard_page = mmap(NULL, (size_t)page, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (guard_page == MAP_FAILED) return 20;
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = on_segv;
    sa.sa_flags = SA_SIGINFO;
    if (sigaction(SIGSEGV, &sa, NULL) != 0) return 21;
    guard_page[100] = 42;
    if (segv != 1 || guard_page[100] != 42) return 22;
    munmap(guard_page, (size_t)page);

#ifdef __x86_64__
    /* SIGFPE (deleni nulou) a odchod z obsluhy pres siglongjmp. */
    signal(SIGFPE, on_fpe);
    volatile int zero = 0;
    int got = sigsetjmp(fpe_jump, 1);
    if (got == 0) {
        volatile int x = 10 / zero;
        (void)x;
        return 30;
    }
    if (got != SIGFPE) return 31;
#endif

    /* Casovac: SIGALRM ve vlakne signalu probudi pause(). */
    signal(SIGALRM, on_alrm);
    struct itimerval it = {{0, 0}, {0, 50000}};
    if (setitimer(ITIMER_REAL, &it, NULL) != 0) return 40;
    if (pause() != -1 || errno != EINTR) return 41;
    if (alrm != 1) return 42;
    if (alarm(10) != 0 || alarm(0) == 0) return 43;
    return 0;
}
