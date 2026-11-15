// Knihovna z disku (DL-4c, libdlorigin.so.1): zavislost libdlorigin2 najde
// pres vlastni DT_RUNPATH $ORIGIN (adresar teto knihovny).

extern int orig2_fn(int);

int orig_fn(int x) {
    return orig2_fn(x) + 1;
}
