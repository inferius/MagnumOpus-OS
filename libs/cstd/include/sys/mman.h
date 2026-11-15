/*
 * sys/mman.h - mapovani pameti (cstd nad spravou pameti jadra).
 * Anonymni pamet se strankami na pozadani; soubor se nacte do pameti
 * (MAP_SHARED se zapisem = ENOTSUP). PROT_EXEC jen s opravnenim JIT (W^X).
 */
#ifndef _CSTD_SYS_MMAN_H
#define _CSTD_SYS_MMAN_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

#define PROT_NONE 0
#define PROT_READ 1
#define PROT_WRITE 2
#define PROT_EXEC 4

#define MAP_SHARED 0x01
#define MAP_PRIVATE 0x02
#define MAP_FIXED 0x10
#define MAP_ANONYMOUS 0x20
#define MAP_ANON MAP_ANONYMOUS
#define MAP_NORESERVE 0x4000
#define MAP_FIXED_NOREPLACE 0x100000
#define MAP_FAILED ((void *)-1)

#define MADV_NORMAL 0
#define MADV_RANDOM 1
#define MADV_SEQUENTIAL 2
#define MADV_WILLNEED 3
#define MADV_DONTNEED 4
#define MADV_FREE 8
#define POSIX_MADV_NORMAL MADV_NORMAL
#define POSIX_MADV_RANDOM MADV_RANDOM
#define POSIX_MADV_SEQUENTIAL MADV_SEQUENTIAL
#define POSIX_MADV_WILLNEED MADV_WILLNEED
#define POSIX_MADV_DONTNEED MADV_DONTNEED

#define MS_ASYNC 1
#define MS_INVALIDATE 2
#define MS_SYNC 4

void *mmap(void *addr, size_t len, int prot, int flags, int fd, off_t off);
int munmap(void *addr, size_t len);
int mprotect(void *addr, size_t len, int prot);
int madvise(void *addr, size_t len, int advice);

/* Pametovy fd: zvetsitelny objekt pameti jadra (kapacita 64 MiB,
 * mo_memfd_create v <magnumopus/memfd.h> jinou). mmap(MAP_SHARED) sdili
 * ramce mezi procesy, pristup za velikost = SIGBUS. */
#define MFD_CLOEXEC 0x0001U
#define MFD_ALLOW_SEALING 0x0002U
#define MFD_HUGETLB 0x0004U
int memfd_create(const char *name, unsigned int flags);
int posix_madvise(void *addr, size_t len, int advice);
int mseal(void *addr, size_t len, unsigned long flags);
int msync(void *addr, size_t len, int flags);
int mlock(const void *addr, size_t len);
int munlock(const void *addr, size_t len);

_CSTD_END

#endif
