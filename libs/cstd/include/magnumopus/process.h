/* magnumopus/process.h - procesovy fd (cstd, src/procfd.rs; Windows
 * vrstva W2). Program sveri svou pamet serveru sve aplikace: fd nese jen
 * prava cteni a zapisu pameti (PROCESS_MEMORY), ne ukonceni ani ladeni,
 * ma vzdy FD_CLOEXEC a predava se vyslovne (sendmsg SCM_RIGHTS).
 * Bezpecnost jako ptrace stejneho uzivatele na Linuxu. */
#ifndef _CSTD_MAGNUMOPUS_PROCESS_H
#define _CSTD_MAGNUMOPUS_PROCESS_H

#include <_cstd.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>

_CSTD_BEGIN

#define MO_PROCESS_READ 0
#define MO_PROCESS_WRITE 1

/* Novy procesovy fd vlastniho procesu (FD_CLOEXEC); -1 + errno. */
int mo_self_process_fd(void);
/* Ridici fd vlastniho procesu jen s pravem ukonceni (FD_CLOEXEC). */
int mo_self_control_fd(void);
/* Fd volajiciho vlakna jen s pravem preruseni (signaly, FD_CLOEXEC). */
int mo_self_thread_fd(void);
/* Ukonci proces za ridicim fd (jako SIGKILL); 0 nebo -1 + errno. */
int mo_process_kill(int fd);
/* Posle signal sig (1..63) vlaknu za fd vlakna; 0 nebo -1 + errno. */
int mo_thread_signal(int fd, int sig);
/* Cteni (MO_PROCESS_READ) z addr procesu za fd do buf, nebo zapis
 * (MO_PROCESS_WRITE) buf do addr; prenesene bajty (mene = castecne),
 * -1 + errno. */
ssize_t mo_process_memory(int fd, int op, uint64_t addr, void *buf, size_t len);

_CSTD_END

#endif
