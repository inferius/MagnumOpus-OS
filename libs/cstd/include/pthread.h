/*
 * pthread.h - vlakna a synchronizace (cstd, docs/cs/cstd.md).
 *
 * Vlakna jsou vlakna osrt (TLS jako Rust std). Typy maji vlastni pevne
 * rozlozeni, stejne na x86_64 i aarch64; staticke inicializatory jsou nuly.
 * Zruseni vlakna jen odlozene (PTHREAD_CANCEL_ASYNCHRONOUS = ENOTSUP),
 * PTHREAD_PROCESS_SHARED neni (futex je soukromy procesu).
 */
#ifndef _CSTD_PTHREAD_H
#define _CSTD_PTHREAD_H

#include <_cstd.h>
#include <sys/types.h>
#include <sched.h>

_CSTD_BEGIN

typedef struct __cstd_thread *pthread_t;
typedef struct { unsigned long __stack; int __detach; unsigned char __rest[52]; } pthread_attr_t;
typedef struct { unsigned int __s[10]; } pthread_mutex_t;
typedef struct { int __kind; } pthread_mutexattr_t;
typedef struct { unsigned int __s[12]; } pthread_cond_t;
typedef struct { int __clock; } pthread_condattr_t;
typedef struct { unsigned int __s[14]; } pthread_rwlock_t;
typedef struct { int __unused; } pthread_rwlockattr_t;
typedef struct { unsigned int __s[8]; } pthread_barrier_t;
typedef struct { int __unused; } pthread_barrierattr_t;
typedef int pthread_once_t;
typedef int pthread_spinlock_t;
typedef unsigned int pthread_key_t;

#define PTHREAD_MUTEX_INITIALIZER {{0}}
#define PTHREAD_COND_INITIALIZER {{0}}
#define PTHREAD_RWLOCK_INITIALIZER {{0}}
#define PTHREAD_ONCE_INIT 0

#define PTHREAD_MUTEX_NORMAL 0
#define PTHREAD_MUTEX_RECURSIVE 1
#define PTHREAD_MUTEX_ERRORCHECK 2
#define PTHREAD_MUTEX_DEFAULT PTHREAD_MUTEX_NORMAL

#define PTHREAD_CREATE_JOINABLE 0
#define PTHREAD_CREATE_DETACHED 1
#define PTHREAD_CANCEL_ENABLE 0
#define PTHREAD_CANCEL_DISABLE 1
#define PTHREAD_CANCEL_DEFERRED 0
#define PTHREAD_CANCEL_ASYNCHRONOUS 1
#define PTHREAD_CANCELED ((void *)-1)
#define PTHREAD_BARRIER_SERIAL_THREAD (-1)
#define PTHREAD_PROCESS_PRIVATE 0
#define PTHREAD_STACK_MIN 16384
#define PTHREAD_KEYS_MAX 1024
#define PTHREAD_DESTRUCTOR_ITERATIONS 4

int pthread_create(pthread_t *t, const pthread_attr_t *attr, void *(*start)(void *), void *arg);
int pthread_join(pthread_t t, void **result);
int pthread_detach(pthread_t t);
pthread_t pthread_self(void);
int pthread_equal(pthread_t a, pthread_t b);
int pthread_getschedparam(pthread_t t, int *policy, struct sched_param *param);
int pthread_setschedparam(pthread_t t, int policy, const struct sched_param *param);
_CSTD_NORETURN void pthread_exit(void *result);
int pthread_cancel(pthread_t t);
void pthread_testcancel(void);
int pthread_setcancelstate(int state, int *old);
int pthread_setcanceltype(int type, int *old);

int pthread_attr_init(pthread_attr_t *a);
int pthread_attr_destroy(pthread_attr_t *a);
int pthread_attr_setstacksize(pthread_attr_t *a, size_t size);
int pthread_attr_getstacksize(const pthread_attr_t *a, size_t *size);
int pthread_attr_setdetachstate(pthread_attr_t *a, int state);
int pthread_attr_getdetachstate(const pthread_attr_t *a, int *state);

int pthread_mutexattr_init(pthread_mutexattr_t *a);
int pthread_mutexattr_destroy(pthread_mutexattr_t *a);
int pthread_mutexattr_settype(pthread_mutexattr_t *a, int type);
int pthread_mutexattr_gettype(const pthread_mutexattr_t *a, int *type);
int pthread_mutex_init(pthread_mutex_t *m, const pthread_mutexattr_t *a);
int pthread_mutex_destroy(pthread_mutex_t *m);
int pthread_mutex_lock(pthread_mutex_t *m);
int pthread_mutex_trylock(pthread_mutex_t *m);
int pthread_mutex_timedlock(pthread_mutex_t *m, const struct timespec *abs);
int pthread_mutex_unlock(pthread_mutex_t *m);

int pthread_condattr_init(pthread_condattr_t *a);
int pthread_condattr_destroy(pthread_condattr_t *a);
int pthread_condattr_setclock(pthread_condattr_t *a, clockid_t clock);
int pthread_condattr_getclock(const pthread_condattr_t *a, clockid_t *clock);
int pthread_cond_init(pthread_cond_t *c, const pthread_condattr_t *a);
int pthread_cond_destroy(pthread_cond_t *c);
int pthread_cond_wait(pthread_cond_t *c, pthread_mutex_t *m);
int pthread_cond_timedwait(pthread_cond_t *c, pthread_mutex_t *m, const struct timespec *abs);
int pthread_cond_signal(pthread_cond_t *c);
int pthread_cond_broadcast(pthread_cond_t *c);

int pthread_rwlock_init(pthread_rwlock_t *l, const pthread_rwlockattr_t *a);
int pthread_rwlock_destroy(pthread_rwlock_t *l);
int pthread_rwlock_rdlock(pthread_rwlock_t *l);
int pthread_rwlock_tryrdlock(pthread_rwlock_t *l);
int pthread_rwlock_timedrdlock(pthread_rwlock_t *l, const struct timespec *abs);
int pthread_rwlock_wrlock(pthread_rwlock_t *l);
int pthread_rwlock_trywrlock(pthread_rwlock_t *l);
int pthread_rwlock_timedwrlock(pthread_rwlock_t *l, const struct timespec *abs);
int pthread_rwlock_unlock(pthread_rwlock_t *l);

int pthread_once(pthread_once_t *once, void (*f)(void));

int pthread_barrier_init(pthread_barrier_t *b, const pthread_barrierattr_t *a, unsigned count);
int pthread_barrier_destroy(pthread_barrier_t *b);
int pthread_barrier_wait(pthread_barrier_t *b);

int pthread_spin_init(pthread_spinlock_t *s, int shared);
int pthread_spin_destroy(pthread_spinlock_t *s);
int pthread_spin_lock(pthread_spinlock_t *s);
int pthread_spin_trylock(pthread_spinlock_t *s);
int pthread_spin_unlock(pthread_spinlock_t *s);

int pthread_key_create(pthread_key_t *key, void (*dtor)(void *));
int pthread_key_delete(pthread_key_t key);
void *pthread_getspecific(pthread_key_t key);
int pthread_setspecific(pthread_key_t key, const void *value);

struct __cstd_cleanup {
    void (*__f)(void *);
    void *__arg;
    struct __cstd_cleanup *__prev;
};
void __cstd_cleanup_push(struct __cstd_cleanup *c, void (*f)(void *), void *arg);
void __cstd_cleanup_pop(struct __cstd_cleanup *c, int execute);
#define pthread_cleanup_push(f, arg) \
    { struct __cstd_cleanup __cstd_c; __cstd_cleanup_push(&__cstd_c, (f), (arg));
#define pthread_cleanup_pop(execute) \
    __cstd_cleanup_pop(&__cstd_c, (execute)); }

_CSTD_END

#endif
