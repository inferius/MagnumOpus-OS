/*
 * sys/random.h - nahodne bajty z generatoru jadra (cstd, docs/cs/cstd.md).
 * getentropy ceka, dokud generator neni nasazeny (jako Linux).
 */
#ifndef _CSTD_SYS_RANDOM_H
#define _CSTD_SYS_RANDOM_H

#include <_cstd.h>
#include <stddef.h>

_CSTD_BEGIN

#ifndef GETENTROPY_MAX
#define GETENTROPY_MAX 256
#endif
int getentropy(void *buf, size_t len);

_CSTD_END

#endif
