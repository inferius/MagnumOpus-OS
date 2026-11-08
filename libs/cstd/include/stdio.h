/*
 * stdio.h - formatovany vystup (cstd). Standardni vystup je log programu
 * (po radcich). sprintf zamerne chybi: pouzijte snprintf.
 */
#ifndef _STDIO_H
#define _STDIO_H

#include <stdarg.h>
#include <stddef.h>
#include "_cstd.h"

_CSTD_BEGIN

_CSTD_PRINTF(1, 2) int printf(const char *fmt, ...);
int vprintf(const char *fmt, va_list ap);
_CSTD_PRINTF(3, 4) int snprintf(char *buf, size_t n, const char *fmt, ...);
int vsnprintf(char *buf, size_t n, const char *fmt, va_list ap);
int puts(const char *s);
int putchar(int c);

_CSTD_END

#endif
