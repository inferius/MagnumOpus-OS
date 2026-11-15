/* semaphore.h - nepojmenovane semafory nad futexem (cstd, src/semaphore.rs).
 * pshared = 1 se prijme, sdileni mezi procesy ale nejde (system nema
 * sdilenou pamet mezi procesy). sem_open/sem_close/sem_unlink = ENOSYS. */
#ifndef _CSTD_SEMAPHORE_H
#define _CSTD_SEMAPHORE_H

#include <_cstd.h>
#include <time.h>

_CSTD_BEGIN

typedef union {
    char __size[32];
    long __align;
} sem_t;

#define SEM_FAILED ((sem_t *)0)
#define SEM_VALUE_MAX 0x7fffffff

int sem_init(sem_t *s, int pshared, unsigned int value);
int sem_destroy(sem_t *s);
int sem_post(sem_t *s);
int sem_wait(sem_t *s);
int sem_trywait(sem_t *s);
int sem_timedwait(sem_t *s, const struct timespec *abs);
int sem_clockwait(sem_t *s, clockid_t clock, const struct timespec *abs);
int sem_getvalue(sem_t *s, int *value);
sem_t *sem_open(const char *name, int flags, ...);
int sem_close(sem_t *s);
int sem_unlink(const char *name);

_CSTD_END

#endif
