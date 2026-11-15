/* spawn.h - posix_spawn bez fork (cstd, src/spawn/). Potomka spusti
 * launchd jako std::process; potomek dostane jen fd 0, 1 a 2:
 * addclose(0-2) = proud nepredat, adddup2(stdio nebo roura, 0-2),
 * addopen(0-2, "/dev/null"); jine akce s cilem > 2 a addopen jineho
 * souboru = ENOTSUP. posix_spawnp hleda v PATH, nenalezeny holy nazev
 * dostane launchd (program systemu). Atributy: SETPGROUP jen 0, SETSID,
 * SETSIGDEF, SETSIGMASK a RESETIDS se prijmou (potomek zacina v novem
 * jobu s vychozimi signaly), planovani jen SCHED_OTHER. */
#ifndef _CSTD_SPAWN_H
#define _CSTD_SPAWN_H

#include <_cstd.h>
#include <sys/types.h>
#include <signal.h>
#include <sched.h>

_CSTD_BEGIN

typedef struct {
    int __allocated;
    int __used;
    void *__actions;
    int __pad[16];
} posix_spawn_file_actions_t;

typedef struct {
    short __flags;
    pid_t __pgrp;
    sigset_t __sd;
    sigset_t __ss;
    struct sched_param __sp;
    int __policy;
    int __pad[16];
} posix_spawnattr_t;

#define POSIX_SPAWN_RESETIDS 0x01
#define POSIX_SPAWN_SETPGROUP 0x02
#define POSIX_SPAWN_SETSIGDEF 0x04
#define POSIX_SPAWN_SETSIGMASK 0x08
#define POSIX_SPAWN_SETSCHEDPARAM 0x10
#define POSIX_SPAWN_SETSCHEDULER 0x20
#define POSIX_SPAWN_USEVFORK 0x40
#define POSIX_SPAWN_SETSID 0x80

int posix_spawn(pid_t *pid, const char *path, const posix_spawn_file_actions_t *fa,
                const posix_spawnattr_t *attr, char *const argv[], char *const envp[]);
int posix_spawnp(pid_t *pid, const char *file, const posix_spawn_file_actions_t *fa,
                 const posix_spawnattr_t *attr, char *const argv[], char *const envp[]);

int posix_spawn_file_actions_init(posix_spawn_file_actions_t *fa);
int posix_spawn_file_actions_destroy(posix_spawn_file_actions_t *fa);
int posix_spawn_file_actions_addclose(posix_spawn_file_actions_t *fa, int fd);
int posix_spawn_file_actions_adddup2(posix_spawn_file_actions_t *fa, int fd, int newfd);
int posix_spawn_file_actions_addopen(posix_spawn_file_actions_t *fa, int fd, const char *path, int oflag, mode_t mode);
int posix_spawn_file_actions_addchdir(posix_spawn_file_actions_t *fa, const char *path);
int posix_spawn_file_actions_addchdir_np(posix_spawn_file_actions_t *fa, const char *path);
int posix_spawn_file_actions_addfchdir(posix_spawn_file_actions_t *fa, int fd);
int posix_spawn_file_actions_addfchdir_np(posix_spawn_file_actions_t *fa, int fd);
int posix_spawn_file_actions_addclosefrom_np(posix_spawn_file_actions_t *fa, int from);

int posix_spawnattr_init(posix_spawnattr_t *a);
int posix_spawnattr_destroy(posix_spawnattr_t *a);
int posix_spawnattr_setflags(posix_spawnattr_t *a, short flags);
int posix_spawnattr_getflags(const posix_spawnattr_t *a, short *flags);
int posix_spawnattr_setpgroup(posix_spawnattr_t *a, pid_t pgroup);
int posix_spawnattr_getpgroup(const posix_spawnattr_t *a, pid_t *pgroup);
int posix_spawnattr_setsigmask(posix_spawnattr_t *a, const sigset_t *mask);
int posix_spawnattr_getsigmask(const posix_spawnattr_t *a, sigset_t *mask);
int posix_spawnattr_setsigdefault(posix_spawnattr_t *a, const sigset_t *set);
int posix_spawnattr_getsigdefault(const posix_spawnattr_t *a, sigset_t *set);
int posix_spawnattr_setschedpolicy(posix_spawnattr_t *a, int policy);
int posix_spawnattr_getschedpolicy(const posix_spawnattr_t *a, int *policy);
int posix_spawnattr_setschedparam(posix_spawnattr_t *a, const struct sched_param *p);
int posix_spawnattr_getschedparam(const posix_spawnattr_t *a, struct sched_param *p);

_CSTD_END

#endif
