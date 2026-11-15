// Program ze souboru (DL-4c, dltest-origin): knihovna libdlorigin.so.1
// z DT_RUNPATH $ORIGIN/../lib (na disku), jeji zavislost pres $ORIGIN
// knihovny, systemova libdltest2 od libd; dlopen absolutni cesty na disku
// a dlopen holeho jmena knihovny z disku (musi selhat: hole jmeno = jen
// /system/lib). Kod: bity chyb, 0 = vse sedi.

#define MO_AT_LOADER 0x4D4F0001ul
#define MO_DLOPEN 40
#define MO_DLSYM 48

typedef void *(*dlopen_fn)(const char *, int);
typedef void *(*dlsym_fn)(void *, const char *);

extern int orig_fn(int);

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

long program_main(unsigned long bootstrap, unsigned long *sp) {
    (void)bootstrap;
    long bad = 0;
    // (2 + 40) * 2 + 1
    if (orig_fn(2) != 85) {
        bad |= 1;
    }
    unsigned long t = table_of(sp);
    if (t == 0) {
        return bad | 64;
    }
    dlopen_fn op = *(dlopen_fn *)(t + MO_DLOPEN);
    dlsym_fn sy = *(dlsym_fn *)(t + MO_DLSYM);
    void *h = op("/dl4c/bin/../lib/libdlorigin3.so.1", 2);
    if (h == 0) {
        bad |= 2;
    } else {
        int (*get)(void) = (int (*)(void))sy(h, "orig3_value");
        // (1 + 40) * 2 + 3
        if (get == 0 || get() != 85) {
            bad |= 4;
        }
        // Tataz cesta jinak zapsana = tentyz modul.
        if (op("/dl4c/lib/libdlorigin3.so.1", 2) != h) {
            bad |= 8;
        }
    }
    // Uz nahrana knihovna z disku jde podle SONAME; nenahrana podle holeho
    // jmena ne (dlopen hleda hole jmeno jen v /system/lib).
    if (op("libdlorigin.so.1", 2) == 0) {
        bad |= 16;
    }
    if (op("libdlorigin-none.so.1", 2) != 0) {
        bad |= 32;
    }
    return bad;
}
