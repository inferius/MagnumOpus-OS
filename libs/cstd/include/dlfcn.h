/*
 * dlfcn.h - nacitani knihoven za behu (cstd nad zavadecem ld.so, DL-4b).
 * Knihovny z /system/lib (jmeno nebo cela cesta); staticky program dlopen
 * neumi (NULL, dlerror rekne proc).
 */
#ifndef _CSTD_DLFCN_H
#define _CSTD_DLFCN_H

#include <_cstd.h>
#include <stddef.h>

_CSTD_BEGIN

#define RTLD_LAZY 1
#define RTLD_NOW 2
#define RTLD_NOLOAD 4
#define RTLD_GLOBAL 0x100
#define RTLD_LOCAL 0
#define RTLD_NODELETE 0x1000
#define RTLD_DEFAULT ((void *)0)
#define RTLD_NEXT ((void *)-1)

typedef struct {
    const char *dli_fname;
    void *dli_fbase;
    const char *dli_sname;
    void *dli_saddr;
} Dl_info;

void *dlopen(const char *path, int flags);
void *dlsym(void *handle, const char *name);
int dlclose(void *handle);
char *dlerror(void);
int dladdr(const void *addr, Dl_info *info);

_CSTD_END

#endif
