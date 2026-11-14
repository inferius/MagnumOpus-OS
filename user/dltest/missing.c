// Program zavisly na knihovne, ktera v obrazu neni (libdlnone.so.1):
// zavadec musi skoncit srozumitelnou chybou, ne padem.

extern int dl_none(void);

long program_main(unsigned long bootstrap, unsigned long *sp) {
    (void)bootstrap;
    (void)sp;
    return dl_none();
}
