// Program pro test rezervy statickeho TLS (DL-4b, dltest-dl): dlopen
// libdlbig.so.1 (4 KiB TLS initial-exec). Kod: 0 = nahrano a TLS sedi,
// 1 = odmitnuto s radou static_tls_reserve v dlerror, 2 = jina chyba.

// Typ auxv s tabulkou zavadece (abi::loader::AT_MO_LOADER) a posuny
// funkci v tabulce (abi::loader::Interface).
#define MO_AT_LOADER 0x4D4F0001ul
#define MO_DLOPEN 40
#define MO_DLSYM 48
#define MO_DLERROR 64

typedef void *(*dlopen_fn)(const char *, int);
typedef void *(*dlsym_fn)(void *, const char *);
typedef const char *(*dlerror_fn)(void);

static unsigned long table_of(unsigned long *sp) {
    unsigned long *p = sp + 1 + sp[0] + 1;
    while (*p != 0) {
        p++;
    }
    for (p++; p[0] != 0; p += 2) {
        if (p[0] == MO_AT_LOADER) {
            return p[1];
        }
    }
    return 0;
}

static int contains(const char *s, const char *w) {
    for (; *s; s++) {
        int i = 0;
        while (w[i] && s[i] == w[i]) {
            i++;
        }
        if (!w[i]) {
            return 1;
        }
    }
    return 0;
}

long program_main(unsigned long bootstrap, unsigned long *sp) {
    (void)bootstrap;
    unsigned long t = table_of(sp);
    if (t == 0) {
        return 2;
    }
    dlopen_fn op = *(dlopen_fn *)(t + MO_DLOPEN);
    dlsym_fn sy = *(dlsym_fn *)(t + MO_DLSYM);
    dlerror_fn er = *(dlerror_fn *)(t + MO_DLERROR);
    void *h = op("libdlbig.so.1", 2);
    if (h == 0) {
        const char *m = er();
        return m != 0 && contains(m, "static_tls_reserve") ? 1 : 2;
    }
    int (*get)(int) = (int (*)(int))sy(h, "big_get");
    if (get == 0 || get(0) != 5 || get(4095) != 0) {
        return 2;
    }
    return 0;
}
