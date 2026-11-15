/*
 * crtdso.o - __dso_handle sdilene knihovny (sysroot C, docs/cs/cpp.md).
 *
 * Kazdy modul ma vlastni skryty __dso_handle: C++ podle nej v __cxa_atexit
 * pozna, ke kteremu modulu destruktor patri (__cxa_finalize pri dlclose).
 * Linkuje se do kazde .so z csysroot::ldflags_so (jako crtbegin.o).
 */

__attribute__((visibility("hidden"))) void *const __dso_handle = (void *)&__dso_handle;
