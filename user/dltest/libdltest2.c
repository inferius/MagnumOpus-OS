// Spodni testovaci knihovna (libdltest2.so.1): data, funkce a TLS, ktere
// pouziva libdltest i program; definuje i symboly ochrany zasobniku
// (cstd jako libc.so zatim neni).

unsigned long __stack_chk_guard = 0x2d5a7b11c3e49f60ul;

void __stack_chk_fail(void) {
    __builtin_trap();
}

int dl2_value = 40;
__thread int dl2_tls = 5;

int dl2_add(int x) {
    return x + dl2_value;
}

int dl2_tls_bump(void) {
    return ++dl2_tls;
}
