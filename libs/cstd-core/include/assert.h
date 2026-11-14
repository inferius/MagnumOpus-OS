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

#endif
