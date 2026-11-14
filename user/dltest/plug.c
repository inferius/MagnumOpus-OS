// Knihovna pro dlopen (DL-4b, libdlplug.so.1): TLS bez initial-exec, tedy
// liny blok pres DTV a dynamicky deskriptor TLSDESC (aarch64 vychozi,
// x86_64 -mtls-dialect=gnu2); zavisi na libdltest2 (uz nahrane).

extern int dl2_add(int);

__thread int plug_tls = 7;
// Vetsi blok se zarovnanim: kopie sablony i nuly za ni.
__thread long plug_buf[16] __attribute__((aligned(64))) = {3};

static int plug_init_ran;

__attribute__((constructor)) static void plug_init(void) {
    plug_init_ran = dl2_add(2);
}

int plug_init_get(void) {
    return plug_init_ran;
}

int plug_tls_bump(void) {
    return ++plug_tls;
}

int *plug_tls_addr(void) {
    return &plug_tls;
}

long plug_buf_sum(void) {
    long s = 0;
    for (int i = 0; i < 16; i++) {
        s += plug_buf[i];
    }
    return s + (long)((unsigned long)plug_buf & 63);
}
