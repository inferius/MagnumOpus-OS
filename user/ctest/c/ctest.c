/*
 * Test knihovny cstd: program v C nad std (docs/cs/cstd.md). Kazda
 * kontrola vypise "ok <jmeno>"; init (sada "cstd") overi vystup a kod.
 * Posledni radek "ctest: hotovo" vypise funkce atexit az po navratu
 * z main - overuje tak atexit i vyprazdneni stdout pri konci.
 */
#include <assert.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;

int posix_test(const char *dir);
int threads_test(void);
int time_test(void);
int math_test(void);
int net_main(void);
int mmap_test(const char *dir);
int signal_test(void);
int lock_main(const char *side, const char *dir);
int wchar_test(const char *dir);
int posix2_test(const char *dir);
int inherit_test(const char *dir);
int unix_test(const char *dir);
int mode_test(const char *dir);
int link_test(const char *dir);
int fmap_test(const char *dir);
int termios_test(const char *dir);
int memfd_test(void);
int memfd_main(const char *side);
int spawn_child_main(int argc, char **argv);

#define CHECK(name, cond)                                              \
    do {                                                               \
        if (cond) {                                                    \
            printf("ok %s\n", name);                                   \
        } else {                                                       \
            fprintf(stderr, "ctest: selhalo %s (%s:%d)\n", name,       \
                    __FILE__, __LINE__);                               \
            failures++;                                                \
        }                                                              \
    } while (0)

static void last_words(void) {
    /* Bez konce radku: vyprazdnit musi az konec procesu. */
    printf("ctest: hotovo");
}

static void first_registered(void) {
    printf("\n");
}

static int args(int argc, char **argv) {
    return argc == 3 && strcmp(argv[0], "ctest") == 0 &&
           strcmp(argv[1], "jedna") == 0 && strcmp(argv[2], "dva") == 0 &&
           argv[3] == NULL;
}

static int formatting(void) {
    char buf[64];
    int n = snprintf(buf, sizeof buf, "%d|%5s|%-3x|%lu|%c|%%", -42, "ab", 255,
                     (unsigned long)123456789, 'z');
    return n == (int)strlen(buf) && strcmp(buf, "-42|   ab|ff |123456789|z|%") == 0;
}

static int memory(void) {
    char *p = malloc(10);
    if (p == NULL) return 0;
    memcpy(p, "123456789", 10);
    p = realloc(p, 100000);
    if (p == NULL || strcmp(p, "123456789") != 0) return 0;
    free(p);
    int *z = calloc(1000, sizeof(int));
    for (int i = 0; i < 1000; i++)
        if (z[i] != 0) return 0;
    free(z);
    return 1;
}

static int files(const char *dir) {
    char path[256];
    snprintf(path, sizeof path, "%s/ctest.txt", dir);
    remove(path);
    FILE *f = fopen(path, "w");
    if (f == NULL) return 0;
    for (int i = 0; i < 1000; i++) fprintf(f, "radek %d\n", i);
    if (fputs("konec\n", f) != 0 || fclose(f) != 0) return 0;

    f = fopen(path, "r");
    if (f == NULL) return 0;
    char line[64];
    int count = 0;
    while (fgets(line, sizeof line, f) != NULL) {
        if (count < 1000) {
            char want[32];
            snprintf(want, sizeof want, "radek %d\n", count);
            if (strcmp(line, want) != 0) return 0;
        }
        count++;
    }
    if (count != 1001 || !feof(f) || strcmp(line, "konec\n") != 0) return 0;
    if (fseek(f, 6, SEEK_SET) != 0 || fgetc(f) != '0' || ftell(f) != 7) return 0;
    fclose(f);

    /* Pripojeni na konec a cteni po blocich. */
    f = fopen(path, "a+");
    if (f == NULL || fwrite("XYZ", 1, 3, f) != 3) return 0;
    fseek(f, -4, SEEK_END);
    char tail[8] = {0};
    if (fread(tail, 1, 8, f) != 4 || memcmp(tail, "\nXYZ", 4) != 0) return 0;
    fclose(f);

    char renamed[256];
    snprintf(renamed, sizeof renamed, "%s/ctest2.txt", dir);
    remove(renamed);
    if (rename(path, renamed) != 0) return 0;
    if (fopen(path, "r") != NULL || errno != ENOENT) return 0;
    return remove(renamed) == 0;
}

static int errors(void) {
    errno = 0;
    FILE *f = fopen("/neexistuje/soubor", "r");
    if (f != NULL || errno != ENOENT) return 0;
    if (strcmp(strerror(ENOENT), "No such file or directory") != 0) return 0;
    errno = 0;
    if (fopen("/x", "q") != NULL || errno != EINVAL) return 0;
    perror("ctest perror");
    return 1;
}

int main(int argc, char **argv) {
    /* Rezim "net": jen sitovy test (sada "cstd sit", relace site). */
    if (argc == 2 && strcmp(argv[1], "net") == 0) return net_main();
    /* Rezim "lock a|b DIR": zamky mezi dvema procesy. */
    if (argc == 4 && strcmp(argv[1], "lock") == 0) return lock_main(argv[2], argv[3]);
    /* Rezim "child ...": potomek testu posix_spawn. */
    if (argc >= 3 && strcmp(argv[1], "child") == 0) return spawn_child_main(argc, argv);
    /* Rezim "memfd a|b": dva procesy nad objektem pameti od initu. */
    if (argc == 3 && strcmp(argv[1], "memfd") == 0) return memfd_main(argv[2]);
    atexit(first_registered);
    atexit(last_words);

    CHECK("args", args(argc, argv));
    const char *dir = getenv("CTEST_DIR");
    CHECK("getenv", dir != NULL && getenv("NEEXISTUJE") == NULL &&
                        strcmp(getenv("CTEST_HELLO"), "svet") == 0);
    CHECK("format", formatting());
    CHECK("memory", memory());
    CHECK("files", dir != NULL && files(dir));
    CHECK("errno", errors());
    CHECK("posix", dir != NULL && posix_test(dir) == 0);
    int th = threads_test();
    if (th != 0) fprintf(stderr, "ctest threads: krok %d\n", th);
    CHECK("threads", th == 0);
    int tm = time_test();
    if (tm != 0) fprintf(stderr, "ctest time: krok %d errno %d\n", tm, errno);
    CHECK("time", tm == 0);
    int mt = math_test();
    if (mt != 0) fprintf(stderr, "ctest math: krok %d\n", mt);
    CHECK("math", mt == 0);
    int mm = dir != NULL ? mmap_test(dir) : -1;
    if (mm != 0) fprintf(stderr, "ctest mmap: krok %d errno %d\n", mm, errno);
    CHECK("mmap", mm == 0);
    int sg = signal_test();
    if (sg != 0) fprintf(stderr, "ctest signal: krok %d errno %d\n", sg, errno);
    CHECK("signal", sg == 0);
    int wc = dir != NULL ? wchar_test(dir) : -1;
    if (wc != 0) fprintf(stderr, "ctest wchar: krok %d errno %d\n", wc, errno);
    CHECK("wchar", wc == 0);
    int p2 = dir != NULL ? posix2_test(dir) : -1;
    if (p2 != 0) fprintf(stderr, "ctest posix2: krok %d errno %d\n", p2, errno);
    CHECK("posix2", p2 == 0);
    int ih = dir != NULL ? inherit_test(dir) : -1;
    if (ih != 0) fprintf(stderr, "ctest inherit: krok %d errno %d\n", ih, errno);
    CHECK("inherit", ih == 0);
    int ux = dir != NULL ? unix_test(dir) : -1;
    if (ux != 0) fprintf(stderr, "ctest unix: krok %d errno %d\n", ux, errno);
    CHECK("unix", ux == 0);
    int md = dir != NULL ? mode_test(dir) : -1;
    if (md != 0) fprintf(stderr, "ctest mode: krok %d errno %d\n", md, errno);
    CHECK("mode", md == 0);
    int lk = dir != NULL ? link_test(dir) : -1;
    if (lk != 0) fprintf(stderr, "ctest link: krok %d errno %d\n", lk, errno);
    CHECK("link", lk == 0);
    int fm = dir != NULL ? fmap_test(dir) : -1;
    if (fm != 0) fprintf(stderr, "ctest fmap: krok %d errno %d\n", fm, errno);
    CHECK("fmap", fm == 0);
    int tt = dir != NULL ? termios_test(dir) : -1;
    if (tt != 0) fprintf(stderr, "ctest termios: krok %d errno %d\n", tt, errno);
    CHECK("termios", tt == 0);
    int mf = memfd_test();
    if (mf != 0) fprintf(stderr, "ctest memfd: krok %d errno %d\n", mf, errno);
    CHECK("memfd", mf == 0);
    assert(failures == 0);
    fputs("chyba na stderr\n", stderr);
    fflush(stdout);
    return failures == 0 ? 0 : 1;
}
