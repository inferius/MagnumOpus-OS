/*
 * signal.h - signaly bez podpory jadra (cstd, docs/cs/cstd.md):
 * synchronni (SIGSEGV, SIGBUS, SIGFPE, SIGILL, SIGTRAP) z obsluhy vyjimek
 * ve vlakne, raise/kill na sebe hned, casovace (alarm, setitimer) ve
 * vlakne signalu. Signaly od jinych procesu zatim ne, SIGPIPE se nevytvari.
 */
#ifndef _CSTD_SIGNAL_H
#define _CSTD_SIGNAL_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

typedef int sig_atomic_t;
typedef void (*sighandler_t)(int);
typedef struct {
    unsigned long __bits[16];
} sigset_t;

#define NSIG 65
#define SIGHUP 1
#define SIGINT 2
#define SIGQUIT 3
#define SIGILL 4
#define SIGTRAP 5
#define SIGABRT 6
#define SIGIOT SIGABRT
#define SIGBUS 7
#define SIGFPE 8
#define SIGKILL 9
#define SIGUSR1 10
#define SIGSEGV 11
#define SIGUSR2 12
#define SIGPIPE 13
#define SIGALRM 14
#define SIGTERM 15
#define SIGCHLD 17
#define SIGCONT 18
#define SIGSTOP 19
#define SIGTSTP 20
#define SIGTTIN 21
#define SIGTTOU 22
#define SIGURG 23
#define SIGVTALRM 26
#define SIGPROF 27
#define SIGWINCH 28
/* Cisla jako Linux; cstd je samo nevytvari (vychozi akce = konec). */
#define SIGSTKFLT 16
#define SIGXCPU 24
#define SIGXFSZ 25
#define SIGIO 29
#define SIGPOLL SIGIO
#define SIGPWR 30
#define SIGSYS 31

#define SIG_DFL ((sighandler_t)0)
#define SIG_IGN ((sighandler_t)1)
#define SIG_ERR ((sighandler_t)-1)

#define SA_SIGINFO 4
#define SA_ONSTACK 0x08000000
#define SA_RESTART 0x10000000
#define SA_NODEFER 0x40000000
#define SA_RESETHAND 0x80000000

#define SIG_BLOCK 0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

#define SI_USER 0
#define SI_TIMER (-2)
/* si_code (hodnoty jako Linux). */
#define SEGV_MAPERR 1
#define SEGV_ACCERR 2
#define BUS_ADRALN 1
#define BUS_ADRERR 2
#define BUS_OBJERR 3
#define FPE_INTDIV 1
#define FPE_INTOVF 2
#define FPE_FLTDIV 3
#define FPE_FLTOVF 4
#define FPE_FLTUND 5
#define FPE_FLTRES 6
#define FPE_FLTINV 7
#define FPE_FLTSUB 8
#define ILL_ILLOPC 1
#define ILL_ILLOPN 2
#define ILL_ILLADR 3
#define ILL_ILLTRP 4
#define ILL_PRVOPC 5
#define ILL_PRVREG 6
#define ILL_COPROC 7
#define ILL_BADSTK 8
#define TRAP_BRKPT 1
#define TRAP_TRACE 2

#define SS_ONSTACK 1
#define SS_DISABLE 2
#define MINSIGSTKSZ 8192
#define SIGSTKSZ 32768

/* 128 B jako Linux: od offsetu 16 adresa (vyjimky) nebo potomek (waitid). */
typedef struct {
    int si_signo;
    int si_errno;
    int si_code;
    int __pad;
    union {
        void *__addr;
        struct {
            int __pid;
            unsigned int __uid;
            int __status;
        } __chld;
    } __si;
    unsigned long __rest[12];
} siginfo_t;
#define si_addr __si.__addr
#define si_pid __si.__chld.__pid
#define si_uid __si.__chld.__uid
#define si_status __si.__chld.__status

#define CLD_EXITED 1
#define CLD_KILLED 2
#define CLD_DUMPED 3

struct sigaction {
    union {
        void (*sa_handler)(int);
        void (*sa_sigaction)(int, siginfo_t *, void *);
    };
    sigset_t sa_mask;
    int sa_flags;
    void (*sa_restorer)(void);
};

typedef struct {
    void *ss_sp;
    int ss_flags;
    size_t ss_size;
} stack_t;

sighandler_t signal(int sig, sighandler_t handler);
int sigaction(int sig, const struct sigaction *act, struct sigaction *old);
int raise(int sig);
int kill(pid_t pid, int sig);
int sigprocmask(int how, const sigset_t *set, sigset_t *old);
int pthread_sigmask(int how, const sigset_t *set, sigset_t *old);
int sigpending(sigset_t *set);
int sigsuspend(const sigset_t *mask);
int sigemptyset(sigset_t *set);
int sigfillset(sigset_t *set);
int sigaddset(sigset_t *set, int sig);
int sigdelset(sigset_t *set, int sig);
int sigismember(const sigset_t *set, int sig);
int sigaltstack(const stack_t *ss, stack_t *old);
char *strsignal(int sig);
void psignal(int sig, const char *s);

_CSTD_END

#endif
