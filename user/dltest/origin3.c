// Knihovna pro dlopen absolutni cesty (DL-4c, libdlorigin3.so.1): zavisi na
// libdlorigin2 pres DT_RUNPATH $ORIGIN.

extern int orig2_fn(int);

int orig3_value(void) {
    return orig2_fn(1) + 3;
}
