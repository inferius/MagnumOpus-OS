/*
 * Test casti 2b knihovny cstd: pthread (vlakna, mutex, podminka, rwlock,
 * bariera, once, klice s destruktory, pthread_exit, odlozene zruseni
 * s uklidem) a TLS ve vlaknech z C (errno, _Thread_local).
 * Vraci 0 = ok, jinak cislo kroku.
 */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>

#define N 4

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t cond = PTHREAD_COND_INITIALIZER;
static long counter;
static int started;
static _Thread_local int tls_value;
static int tls_ok;

static void *adder(void *arg) {
    long id = (long)arg;
    tls_value = (int)id + 100;
    errno = (int)id + 1000;
    for (int i = 0; i < 10000; i++) {
        pthread_mutex_lock(&lock);
        counter++;
        pthread_mutex_unlock(&lock);
    }
    pthread_mutex_lock(&lock);
    started++;
    pthread_cond_broadcast(&cond);
    if (tls_value == (int)id + 100 && errno == (int)id + 1000) tls_ok++;
    pthread_mutex_unlock(&lock);
    return (void *)(id * 2);
}

static pthread_key_t key;
static int destroyed;

static void key_dtor(void *v) {
    pthread_mutex_lock(&lock);
    destroyed += (int)(long)v;
    pthread_mutex_unlock(&lock);
}

static void *with_key(void *arg) {
    pthread_setspecific(key, arg);
    if (arg == (void *)2) pthread_exit((void *)55);
    return NULL;
}

static int cleaned;
static void cleanup(void *arg) { cleaned = (int)(long)arg; }

static void *waiter(void *arg) {
    (void)arg;
    pthread_mutex_lock(&lock);
    pthread_cleanup_push(cleanup, (void *)9);
    for (;;) pthread_cond_wait(&cond, &lock); /* bod zruseni */
    pthread_cleanup_pop(0);
    return NULL;
}

static pthread_barrier_t barrier;
static int serial;
static void *at_barrier(void *arg) {
    (void)arg;
    if (pthread_barrier_wait(&barrier) == PTHREAD_BARRIER_SERIAL_THREAD) {
        pthread_mutex_lock(&lock);
        serial++;
        pthread_mutex_unlock(&lock);
    }
    return NULL;
}

static int once_count;
static pthread_once_t once = PTHREAD_ONCE_INIT;
static void once_fn(void) { once_count++; }

int threads_test(void) {
    pthread_t t[N];
    errno = 0;
    for (long i = 0; i < N; i++)
        if (pthread_create(&t[i], NULL, adder, (void *)i) != 0) return 1;
    for (long i = 0; i < N; i++) {
        void *r;
        if (pthread_join(t[i], &r) != 0 || r != (void *)(i * 2)) return 2;
    }
    if (counter != N * 10000 || started != N) return 3;
    if (tls_ok != N || errno != 0 || tls_value != 0) return 4;

    /* Klice: destruktor po navratu i po pthread_exit. */
    if (pthread_key_create(&key, key_dtor) != 0) return 10;
    pthread_t a, b;
    void *r;
    pthread_create(&a, NULL, with_key, (void *)1);
    pthread_create(&b, NULL, with_key, (void *)2);
    if (pthread_join(a, &r) != 0 || r != NULL) return 11;
    if (pthread_join(b, &r) != 0 || r != (void *)55) return 12;
    if (destroyed != 3) return 13;

    /* Odlozene zruseni v cekani na podminku, uklid s drzenym mutexem. */
    pthread_t w;
    pthread_create(&w, NULL, waiter, NULL);
    if (pthread_cancel(w) != 0) return 20;
    if (pthread_join(w, &r) != 0 || r != PTHREAD_CANCELED || cleaned != 9) return 21;
    /* Zruseny cekal s mutexem: uklid ho neuvolnil, uvolnime ho tu. */
    if (pthread_mutex_trylock(&lock) != EBUSY) return 22;
    pthread_mutex_unlock(&lock);

    /* Bariera: prave jedno vlakno dostane SERIAL. */
    pthread_barrier_init(&barrier, NULL, N);
    for (int i = 0; i < N; i++) pthread_create(&t[i], NULL, at_barrier, NULL);
    for (int i = 0; i < N; i++) pthread_join(t[i], NULL);
    if (serial != 1) return 30;

    /* rwlock, once, odpojene vlakno, chyby. */
    pthread_rwlock_t rw = PTHREAD_RWLOCK_INITIALIZER;
    if (pthread_rwlock_rdlock(&rw) || pthread_rwlock_rdlock(&rw)) return 40;
    if (pthread_rwlock_trywrlock(&rw) != EBUSY) return 41;
    pthread_rwlock_unlock(&rw);
    pthread_rwlock_unlock(&rw);
    if (pthread_rwlock_wrlock(&rw) || pthread_rwlock_tryrdlock(&rw) != EBUSY) return 42;
    pthread_rwlock_unlock(&rw);
    pthread_once(&once, once_fn);
    pthread_once(&once, once_fn);
    if (once_count != 1) return 43;
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED);
    pthread_attr_setstacksize(&attr, 64 * 1024);
    pthread_t d;
    if (pthread_create(&d, &attr, adder, (void *)7) != 0) return 44;
    if (pthread_join(pthread_self(), NULL) != EDEADLK) return 45;
    if (pthread_setcanceltype(PTHREAD_CANCEL_ASYNCHRONOUS, NULL) != ENOTSUP) return 46;
    return 0;
}
