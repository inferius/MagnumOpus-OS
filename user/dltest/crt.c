// Minimalni start programu PIE v C (DL-2, docs/cs/dynamicke-linkovani.md).
// Zavadec ld.so skoci na _start s handlem bootstrapu v prvnim argumentu
// (rdi / x0) a SP na auxv (abi::auxv). Az bude cstd jako libc.so, prevezme
// tuto roli jeho start (crt1); tady jen program_main a konec procesu.

// Cislo syscallu PROCESS_EXIT (libs/abi/src/syscall.rs).
#define MO_PROCESS_EXIT 18

long program_main(unsigned long bootstrap, unsigned long *sp);

__attribute__((noreturn)) static void mo_exit(long code) {
#if defined(__x86_64__)
    __asm__ volatile("syscall" : : "a"((long)MO_PROCESS_EXIT), "D"(code) : "rcx", "r11", "memory");
#elif defined(__aarch64__)
    register long x8 __asm__("x8") = MO_PROCESS_EXIT;
    register long x0 __asm__("x0") = code;
    __asm__ volatile("svc #0" : : "r"(x8), "r"(x0) : "memory");
#endif
    for (;;) {
    }
}

__attribute__((noreturn, used)) void __mo_start(unsigned long bootstrap, unsigned long *sp) {
    mo_exit(program_main(bootstrap, sp));
}

#if defined(__x86_64__)
__asm__(".globl _start\n"
        "_start:\n"
        "  mov %rsp, %rsi\n"
        "  and $-16, %rsp\n"
        "  xor %ebp, %ebp\n"
        "  call __mo_start\n"
        "  ud2\n");
#elif defined(__aarch64__)
__asm__(".globl _start\n"
        "_start:\n"
        "  mov x1, sp\n"
        "  mov x29, xzr\n"
        "  mov x30, xzr\n"
        "  bl __mo_start\n"
        "  brk #1\n");
#endif
