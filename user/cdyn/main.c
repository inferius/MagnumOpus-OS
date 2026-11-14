/*
 * cdyn - program PIE v C nad libc.so (docs/cs/cstd.md, kapitola libc.so).
 * Overuje: start pres crt1.o a __libc_start, stdio, getenv a matematiku
 * z libc.so, vlakna s vlastnim TLS a errno, dlopen pluginu v C (libcplug.so)
 * s dynamickym TLS po vlaknech, jeden heap (malloc v pluginu, free tady),
 * soubory a atexit. Vypisuje "ok <krok>", posledni radek "cdyn: hotovo".
 */
#include <dlfcn.h>
#include <errno.h>
#include <math.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails;
#define CHECK(name, cond)                                                   \
    do {                                                                    \
        if (cond) {                                                         \
            printf("ok %s\n", name);                                        \
        } else {                                                            \
            fprintf(stderr, "cdyn: selhalo %s (radek %d)\n", name, __LINE__); \
            fails++;                                                        \
        }                                                                   \
    } while (0)

static _Thread_local long tls_value;
static int (*plug_tick)(void);

struct result {
    long tls;
    int err;
    int ticks;
};

static void *worker(void *arg) {
    long id = (long)arg;
    tls_value = id * 10;
    errno = (int)id + 100;
    int t = 0;
    for (int i = 0; i < 100; i++) t = plug_tick();
    struct result *r = malloc(sizeof *r);
    r->tls = tls_value;
    r->err = errno;
    r->ticks = t;
    return r;
}

static void bye(void) { printf("cdyn: hotovo\n"); }

int main(int argc, char **argv) {
    atexit(bye);
    CHECK("args", argc == 2 && strcmp(argv[1], "test") == 0);
    const char *dir = getenv("CDYN_DIR");
    volatile double two = 2.0;
    char buf[64];
    snprintf(buf, sizeof buf, "%.4f", sqrt(two));
    CHECK("libc", dir != NULL && strcmp(buf, "1.4142") == 0);
    /* mem* a long double z libc.so (definovane v C, export verzovacim skriptem). */
    char big[300], copy[300];
    memset(big, 'x', sizeof big);
    memcpy(copy, big, sizeof copy);
    memmove(copy + 1, copy, 100);
    volatile long double ld = 2.0L;
    CHECK("mem-ld", memcmp(big, copy, sizeof big) == 0 && powl(ld, 3.0L) == 8.0L);

    void *h = dlopen("libcplug.so", RTLD_NOW);
    if (h == NULL) fprintf(stderr, "cdyn: dlopen: %s\n", dlerror());
    char *(*hello)(const char *) = h ? (char *(*)(const char *))dlsym(h, "plug_hello") : NULL;
    plug_tick = h ? (int (*)(void))dlsym(h, "plug_tick") : NULL;
    CHECK("dlopen", h != NULL && hello != NULL && plug_tick != NULL);
    if (h == NULL || hello == NULL || plug_tick == NULL) return 1;
    char *s = hello("cdyn");
    CHECK("plugin", strcmp(s, "ahoj cdyn z pluginu") == 0);
    free(s); /* malloc v pluginu, free tady: jeden heap v libc.so */
    CHECK("dlsym-chyba", dlsym(h, "neexistuje") == NULL && dlerror() != NULL);

    /* Vlakna: vlastni TLS programu, errno a TLS pluginu (dynamicke, DTV). */
    pthread_t t[4];
    for (long i = 0; i < 4; i++) pthread_create(&t[i], NULL, worker, (void *)(i + 1));
    int ok = 1;
    for (long i = 0; i < 4; i++) {
        struct result *r;
        pthread_join(t[i], (void **)&r);
        if (r->tls != (i + 1) * 10 || r->err != (int)(i + 1) + 100 || r->ticks != 100) ok = 0;
        free(r);
    }
    CHECK("vlakna-tls", ok && tls_value == 0 && plug_tick() == 1);

    /* Soubory pres libc.so (relace FS od rodice). */
    char path[256];
    snprintf(path, sizeof path, "%s/cdyn.txt", dir);
    FILE *f = fopen(path, "w");
    int wrote = f != NULL && fputs("pres libc.so\n", f) >= 0 && fclose(f) == 0;
    f = fopen(path, "r");
    char line[32] = {0};
    CHECK("soubory", wrote && f != NULL && fgets(line, sizeof line, f) != NULL && strcmp(line, "pres libc.so\n") == 0);
    if (f) fclose(f);
    remove(path);

    CHECK("dlclose", dlclose(h) == 0);
    fflush(stdout);
    return fails == 0 ? 0 : 1;
}
