/*
 * Konfigurace ACPICA pro MagnumOpus (vybira ji acenv.h podle __MAGNUMOPUS__,
 * jediny zasah do prevzatych zdroju, viz third_party/acpica/README.md).
 *
 * ACPICA bezi jako sluzba v userspace (user/acpid). Vrstva OS (AcpiOs*) je
 * v Rustu (user/acpid/src/osl), knihovna C je libs/cstd.
 */
#ifndef __ACMAGNUM_H__
#define __ACMAGNUM_H__

#include <stdint.h>
#include <stdarg.h>
#include <stdbool.h>

/* 64bitove cile: x86_64 a aarch64 (LP64). */
#define ACPI_MACHINE_WIDTH          64
#define COMPILER_DEPENDENT_INT64    long long
#define COMPILER_DEPENDENT_UINT64   unsigned long long
#define ACPI_USE_NATIVE_DIVIDE
#define ACPI_USE_NATIVE_MATH64

/* Knihovna C je cstd (string.h, stdlib.h, ctype.h). */
#define ACPI_USE_SYSTEM_CLIBRARY
#define ACPI_USE_STANDARD_HEADERS
#define ACPI_USE_DO_WHILE_0

/* Cache objektu ma ACPICA vlastni (seznamy nad AcpiOsAllocate). */
#define ACPI_USE_LOCAL_CACHE

/* Typy objektu vrstvy OS: ukazatele na objekty v Rustu. */
#define ACPI_SPINLOCK               void *
#define ACPI_SEMAPHORE              void *
#define ACPI_MUTEX_TYPE             ACPI_BINARY_SEMAPHORE
#define ACPI_CPU_FLAGS              unsigned long long

/*
 * Zapis do registru spanku: cache CPU se v userspace vyprazdnit neda
 * (wbinvd je privilegovana). Pro S5 (vypnuti) a restart to nevadi - obsah
 * RAM se zahazuje. S3 (uspani) zatim neni; az bude, musi cache vyprazdnit
 * jadro (syscall) - viz docs/cs/acpi.md, "Omezeni".
 */
#define ACPI_FLUSH_CPU_CACHE()

/* Globalni zamek ACPI (FACS): atomicky algoritmus ze specifikace, c/glue.c. */
struct acpi_table_facs;
int AcpiMagnumAcquireGlobalLock(void *Facs);
int AcpiMagnumReleaseGlobalLock(void *Facs);
#define ACPI_ACQUIRE_GLOBAL_LOCK(FacsPtr, Acq) ((Acq) = AcpiMagnumAcquireGlobalLock(FacsPtr))
#define ACPI_RELEASE_GLOBAL_LOCK(FacsPtr, Pnd) ((Pnd) = AcpiMagnumReleaseGlobalLock(FacsPtr))

/* aarch64: nezarovnany pristup do pameti tabulek prekladac nepouzije
 * (-mstrict-align), ACPICA pak cte po bajtech. */
#if defined(__aarch64__)
#define ACPI_MISALIGNMENT_NOT_SUPPORTED
#endif

/* Chybove hlasky ano (do logu), ladici vystup a debugger ne. */
#undef ACPI_DEBUG_OUTPUT
#undef ACPI_DEBUGGER
#undef ACPI_DISASSEMBLER

#endif /* __ACMAGNUM_H__ */
