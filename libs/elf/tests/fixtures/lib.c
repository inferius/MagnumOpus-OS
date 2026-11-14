// Zdroj testovacich sdilenych knihoven pro `libs/elf` (host testy).
// Preklad: `gen.sh` v teto slozce (clang + ld.lld), vystup se commituje.
// Pokryva: export funkce a promenne, nedefinovany symbol z DT_NEEDED,
// TLS (general-dynamic / TLSDESC), tabulku ukazatelu (RELATIVE nebo RELR),
// GOT (GLOB_DAT), volani pres PLT (JUMP_SLOT).

extern int dep_value;
extern int dep_call(int);

__thread int tls_counter = 7;
int exported_var = 42;
static int table_target_a = 1;
static int table_target_b = 2;
int *pointer_table[3] = {&table_target_a, &table_target_b, &exported_var};

int exported_fn(int x) {
    tls_counter += x;
    return dep_call(x) + dep_value + *pointer_table[x & 1] + tls_counter;
}

__attribute__((visibility("hidden"))) int hidden_fn(void) {
    return 3;
}
