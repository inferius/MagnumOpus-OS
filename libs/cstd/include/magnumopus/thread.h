/*
 * magnumopus/thread.h - rozsireni cstd pro vlakna (ne POSIX).
 *
 * Druhy ukazatel vlakna: x86_64 baze GS programu (TEB Windows vrstvy,
 * docs/cs/windows-vrstva.md), jadro ji uklada pro kazde vlakno
 * (THREAD_SET_POINTER se SECOND). aarch64 ho nema: -1 a errno ENOTSUP.
 */
#ifndef _CSTD_MAGNUMOPUS_THREAD_H
#define _CSTD_MAGNUMOPUS_THREAD_H

#include <_cstd.h>

_CSTD_BEGIN

/* Nastavi druhy ukazatel aktualniho vlakna; 0 = ok, -1 = chyba (errno). */
int mos_thread_set_second_pointer(void *value);

/* Posledni hodnota nastavena timto vlaknem (nove vlakno NULL). */
void *mos_thread_get_second_pointer(void);

_CSTD_END

#endif
