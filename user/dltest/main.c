// Testovaci program PIE (dltest): vola obe knihovny, cte jejich TLS
// a vrati kod: bity 0-7 = chyby (0 = vse sedi), od bitu 8 cast adresy
// programu (test ASLR porovna dva behy).

extern int dl_fn(int);
extern int dl_init_ran;
extern int dl_tls_get(void);
extern int dl2_tls_bump(void);
extern __thread int dl2_tls;

__thread int main_tls = 9;
static int (*fp)(int) = dl_fn;

long program_main(unsigned long bootstrap, unsigned long *sp) {
    (void)bootstrap;
    (void)sp;
    long bad = 0;
    if (dl_fn(1) != 1 + 40 + 2) {
        bad |= 1;
    }
    if (fp(0) != 0 + 40 + 1) {
        bad |= 2;
    }
    if (dl_init_ran != 40) {
        bad |= 4;
    }
    if (dl_tls_get() != 101) {
        bad |= 8;
    }
    if (dl2_tls_bump() != 6 || dl2_tls != 6) {
        bad |= 16;
    }
    if (main_tls != 9) {
        bad |= 32;
    }
    main_tls = 10;
    if (main_tls != 10 || dl2_tls != 6 || &main_tls == &dl2_tls) {
        bad |= 64;
    }
    unsigned long base = (unsigned long)&program_main;
    return bad | (long)(((base >> 16) & 0xFFFFFF) << 8);
}
