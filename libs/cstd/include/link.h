/*
 * link.h - pruchod nahranymi moduly (cstd, docs/cs/cstd.md).
 * Program PIE pod ld.so: moduly zavadece. Staticky program: jediny modul
 * (program) s hlavickami PT_LOAD a PT_GNU_EH_FRAME (tabulky rozbalovani
 * pro libunwind, C++). Rozlozeni dl_phdr_info jako glibc
 * (abi::loader::PhdrInfo).
 */
#ifndef _CSTD_LINK_H
#define _CSTD_LINK_H

#include <_cstd.h>
#include <elf.h>
#include <stddef.h>

_CSTD_BEGIN

#define ElfW(type) Elf64_##type

struct dl_phdr_info {
    ElfW(Addr) dlpi_addr;
    const char *dlpi_name;
    const ElfW(Phdr) *dlpi_phdr;
    ElfW(Half) dlpi_phnum;
    unsigned long long dlpi_adds;
    unsigned long long dlpi_subs;
    size_t dlpi_tls_modid;
    void *dlpi_tls_data;
};

int dl_iterate_phdr(int (*callback)(struct dl_phdr_info *info, size_t size, void *data), void *data);

_CSTD_END

#endif
