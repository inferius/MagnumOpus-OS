/*
 * memcpy, memmove, memset, memcmp, bcmp pro libc.so (cil -dyn).
 *
 * V Rustu je dava compiler_builtins, ale cdylib je neexportuje; programy
 * a knihovny v C (i prekladac sam pri kopirovani struktur) je potrebuji
 * z libc.so. Preklada se s -ffreestanding (cbuild), takze prekladac smycky
 * nenahradi volanim teze funkce. Kopiruje se po slovech, kdyz to
 * zarovnani dovoli.
 */
#include <stddef.h>
#include <stdint.h>

typedef uint64_t __attribute__((__may_alias__)) word;

void *memcpy(void *dst, const void *src, size_t n) {
    unsigned char *d = dst;
    const unsigned char *s = src;
    if ((((uintptr_t)d | (uintptr_t)s) & 7) == 0) {
        for (; n >= 8; n -= 8, d += 8, s += 8) *(word *)d = *(const word *)s;
    }
    while (n--) *d++ = *s++;
    return dst;
}

void *memmove(void *dst, const void *src, size_t n) {
    unsigned char *d = dst;
    const unsigned char *s = src;
    if (d == s || n == 0) return dst;
    if (d < s || d >= s + n) return memcpy(dst, src, n);
    /* Prekryv s cilem za zdrojem: odzadu. */
    d += n;
    s += n;
    if ((((uintptr_t)d | (uintptr_t)s) & 7) == 0) {
        for (; n >= 8; n -= 8) {
            d -= 8;
            s -= 8;
            *(word *)d = *(const word *)s;
        }
    }
    while (n--) *--d = *--s;
    return dst;
}

void *memset(void *dst, int c, size_t n) {
    unsigned char *d = dst;
    if (((uintptr_t)d & 7) == 0 && n >= 8) {
        word w = (unsigned char)c;
        w |= w << 8;
        w |= w << 16;
        w |= w << 32;
        for (; n >= 8; n -= 8, d += 8) *(word *)d = w;
    }
    while (n--) *d++ = (unsigned char)c;
    return dst;
}

int memcmp(const void *a, const void *b, size_t n) {
    const unsigned char *x = a, *y = b;
    for (; n; n--, x++, y++) {
        if (*x != *y) return *x - *y;
    }
    return 0;
}

int bcmp(const void *a, const void *b, size_t n) { return memcmp(a, b, n); }
