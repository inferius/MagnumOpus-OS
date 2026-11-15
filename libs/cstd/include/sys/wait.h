/* sys/wait.h - cekani na potomky z posix_spawn (cstd, src/spawn/wait.rs).
 * Cisla potomku prideluje cstd v procesu; skupiny procesu nejsou
 * (waitpid(0) a waitpid(-pgid) = kterykoli potomek). Stav jako Linux;
 * konec od systemu (kill, vyjimka CPU) = signal. SIGCHLD rodic nedostava. */
#ifndef _CSTD_SYS_WAIT_H
#define _CSTD_SYS_WAIT_H

#include <_cstd.h>
#include <sys/types.h>
#include <signal.h>

_CSTD_BEGIN

#define WNOHANG 1
#define WUNTRACED 2
#define WSTOPPED 2
#define WEXITED 4
#define WCONTINUED 8
#define WNOWAIT 0x01000000

#define WEXITSTATUS(s) (((s) & 0xff00) >> 8)
#define WTERMSIG(s) ((s) & 0x7f)
#define WSTOPSIG(s) WEXITSTATUS(s)
#define WCOREDUMP(s) ((s) & 0x80)
#define WIFEXITED(s) (WTERMSIG(s) == 0)
#define WIFSIGNALED(s) (((signed char)(((s) & 0x7f) + 1) >> 1) > 0)
#define WIFSTOPPED(s) (((s) & 0xff) == 0x7f)
#define WIFCONTINUED(s) ((s) == 0xffff)

typedef enum { P_ALL = 0, P_PID = 1, P_PGID = 2 } idtype_t;
typedef unsigned int id_t;

struct rusage;

pid_t wait(int *status);
pid_t waitpid(pid_t pid, int *status, int options);
int waitid(idtype_t idtype, id_t id, siginfo_t *info, int options);
pid_t wait3(int *status, int options, struct rusage *ru);
pid_t wait4(pid_t pid, int *status, int options, struct rusage *ru);

_CSTD_END

#endif
