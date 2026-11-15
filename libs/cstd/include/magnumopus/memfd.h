/* magnumopus/memfd.h - pametovy fd mezi procesy (cstd, src/memfd.rs).
 * Handle objektu pameti posle program jinemu procesu vlastnim kanalem
 * (SCM_RIGHTS neni); prijemce si z nej udela fd. */
#ifndef _CSTD_MAGNUMOPUS_MEMFD_H
#define _CSTD_MAGNUMOPUS_MEMFD_H

#include <_cstd.h>
#include <stdint.h>

_CSTD_BEGIN

/* memfd_create s kapacitou (nejvetsi velikost, kterou ftruncate dovoli). */
int mo_memfd_create(const char *name, unsigned int flags, uint64_t capacity);
/* Novy handle objektu pameti za fd; readonly = prijemce nesmi zapisovat
 * ani menit velikost. 0 = ok, -1 + errno. */
int mo_memfd_export(int fd, int readonly, uint32_t *handle);
/* fd z handlu (fd ho prevezme); flags O_RDONLY/O_RDWR, O_CLOEXEC. */
int mo_memfd_import(uint32_t handle, int flags);
/* Handle od rodice ve startovni zprave se stitkem tag (jednou; 0 = neni). */
uint32_t mo_start_handle(uint32_t tag);

_CSTD_END

#endif
