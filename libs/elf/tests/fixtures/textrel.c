// Zdroj knihovny s DT_TEXTREL (kod bez PIC, relokace v textu) pro test
// odmitnuti (W^X). Preklad: `gen.sh` (`-fno-pic`, `-z notext`).

extern int ext_var;

int read_ext(void) {
    return ext_var;
}
