// Testovaci knihovna (libdltest.so.1): zavisi na libdltest2, TLS
// (general-dynamic / TLSDESC), tabulka ukazatelu (relativni relokace),
// inicializator (bezi az po relokaci zavislosti).

extern int dl2_add(int);
extern int dl2_value;

__thread int dl_tls = 100;
static int a = 1;
static int b = 2;
int *dl_table[2] = {&a, &b};
int dl_init_ran = 0;

__attribute__((constructor)) static void dl_init(void) {
    dl_init_ran = dl2_value;
}

int dl_fn(int x) {
    dl_tls += x;
    return dl2_add(x) + *dl_table[x & 1];
}

int dl_tls_get(void) {
    return dl_tls;
}

// Finalizator (DL-3): zavadec ho zavola pri normalnim konci (run_fini).
static int dl_fini_ran = 0;

__attribute__((destructor)) static void dl_fini(void) {
    dl_fini_ran = 1;
}

int dl_fini_get(void) {
    return dl_fini_ran;
}
