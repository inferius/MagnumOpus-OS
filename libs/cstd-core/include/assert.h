/* assert.h - kontrola podminky; selhani ukonci program (cstd). */
#ifndef _ASSERT_H
#define _ASSERT_H

#include "_cstd.h"

_CSTD_BEGIN
_CSTD_NORETURN void __cstd_assert_fail(const char *expr, const char *file, int line);
_CSTD_END

#ifdef NDEBUG
#define assert(e) ((void)0)
#else
#define assert(e) ((e) ? (void)0 : __cstd_assert_fail(#e, __FILE__, __LINE__))
#endif

/* C11: static_assert je makro (v C23 klicove slovo, v C++ take). */
#if !defined(__cplusplus) && (!defined(__STDC_VERSION__) || __STDC_VERSION__ < 202311L)
#define static_assert _Static_assert
#endif

#endif
