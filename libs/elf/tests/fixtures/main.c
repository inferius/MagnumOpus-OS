// Zdroj testovaciho programu PIE pro `libs/elf`: PT_INTERP, DT_NEEDED na
// libfix.so.1, RUNPATH $ORIGIN. Preklad: `gen.sh`.

extern int exported_fn(int);
extern int exported_var;

void _start(void) {
    exported_var = exported_fn(1);
    for (;;) {
    }
}
