/*
 * Spolecne makra hlavicek cstd (zaklad libc systemu MagnumOpus).
 *
 * Implementace funkci je v Rustu (libs/cstd/src), hlavicky jen deklaruji
 * rozhrani C. Typy size_t, stdint, stdarg a stdbool dodava clang
 * (-ffreestanding -nostdlibinc ponecha jeho vestavene hlavicky).
 */
#ifndef _CSTD_H
#define _CSTD_H

#ifdef __cplusplus
#define _CSTD_BEGIN extern "C" {
#define _CSTD_END }
#else
#define _CSTD_BEGIN
#define _CSTD_END
#endif

#define _CSTD_NORETURN __attribute__((__noreturn__))
#define _CSTD_PRINTF(f, a) __attribute__((__format__(__printf__, f, a)))

#endif
