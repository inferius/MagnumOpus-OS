/*
 * setjmp.h - nelokalni skoky (cstd). jmp_buf ma 320 B na obou
 * architekturach; sigsetjmp ulozi masku signalu pred setjmp (makro).
 */
#ifndef _CSTD_SETJMP_H
#define _CSTD_SETJMP_H

#include <_cstd.h>

_CSTD_BEGIN

typedef long jmp_buf[40];
typedef long sigjmp_buf[40];

int setjmp(jmp_buf env) __attribute__((__returns_twice__));
_CSTD_NORETURN void longjmp(jmp_buf env, int val);
_CSTD_NORETURN void siglongjmp(sigjmp_buf env, int val);
_CSTD_NORETURN void _longjmp(jmp_buf env, int val);
void __cstd_sigsave(sigjmp_buf env, int save);

#define _setjmp(env) setjmp(env)
#define sigsetjmp(env, save) (__cstd_sigsave((env), (save)), setjmp(env))

_CSTD_END

#endif
