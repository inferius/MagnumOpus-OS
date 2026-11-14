/*
 * crt1.o - start programu PIE v C nad libc.so (sysroot C, docs/cs/cstd.md).
 *
 * Zavadec ld.so skoci na _start s handlem bootstrapu (rdi / x0), SP na
 * auxv (rsi / x1) a ENTRY_MAGIC (rdx / x2). _start je preda spolu s
 * adresou main do __libc_start (osrt v libc.so): ta nacte startovni
 * zpravu, pripravi stdio, soubory a sit a zavola main.
 */

int main(int argc, char **argv);

#if defined(__x86_64__)
__asm__(".text\n"
        ".globl _start\n"
        ".type _start,@function\n"
        "_start:\n"
        "  xor %ebp, %ebp\n"
        "  mov %rdx, %rcx\n"     /* magic */
        "  mov %rsi, %rdx\n"     /* sp (auxv) */
        "  mov %rdi, %rsi\n"     /* bootstrap */
        "  lea main(%rip), %rdi\n"
        "  and $-16, %rsp\n"
        "  call *__libc_start@GOTPCREL(%rip)\n"
        "  ud2\n");
#elif defined(__aarch64__)
__asm__(".text\n"
        ".globl _start\n"
        ".type _start,%function\n"
        "_start:\n"
        "  mov x3, x2\n"         /* magic */
        "  mov x2, x1\n"         /* sp (auxv) */
        "  mov x1, x0\n"         /* bootstrap */
        "  adrp x0, main\n"
        "  add x0, x0, :lo12:main\n"
        "  mov x9, sp\n"
        "  and x9, x9, #-16\n"
        "  mov sp, x9\n"
        "  mov x29, xzr\n"
        "  mov x30, xzr\n"
        "  adrp x16, :got:__libc_start\n"
        "  ldr x16, [x16, :got_lo12:__libc_start]\n"
        "  blr x16\n"
        "  brk #1\n");
#endif
