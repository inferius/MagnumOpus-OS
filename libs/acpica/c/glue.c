/*
 * Cast vrstvy OS ACPICA, ktera musi byt v C: funkce s promennym poctem
 * argumentu (AcpiOsPrintf) a globalni zamek ACPI nad FACS (atomicke
 * operace na sdilene pameti s firmwarem). Zbytek AcpiOs* je v Rustu
 * (user/acpid/src/osl).
 */
#include "acpi.h"
#include "accommon.h"
#include <string.h>
#include <stdio.h>

/* Formatovany vystup ACPICA jde do logu programu (cstd printf). */
void ACPI_INTERNAL_VAR_XFACE AcpiOsPrintf(const char *Format, ...)
{
    va_list Args;
    va_start(Args, Format);
    vprintf(Format, Args);
    va_end(Args);
}

void AcpiOsVprintf(const char *Format, va_list Args)
{
    vprintf(Format, Args);
}

/*
 * Globalni zamek (ACPI 6.5, 5.2.10.1): bit 0 = cekajici (pending), bit 1 =
 * vlastneny (owned). Ziskani: nastavit owned, a kdyz uz owned byl, nastavit
 * i pending. Vraci 1 = zamek ziskan, 0 = drzi ho firmware (prijde GBL_RLS).
 */
int AcpiMagnumAcquireGlobalLock(void *Facs)
{
    volatile UINT32 *Lock = &((ACPI_TABLE_FACS *) Facs)->GlobalLock;
    UINT32 Old, New;

    Old = __atomic_load_n(Lock, __ATOMIC_SEQ_CST);
    do {
        New = (Old & ~3u) | 2u | ((Old >> 1) & 1u);
    } while (!__atomic_compare_exchange_n(Lock, &Old, New, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
    return (New & 3u) == 2u;
}

/* Uvolneni: smazat owned i pending; vraci 1, kdyz firmware ceka (pending
 * byl nastaven) - pak se nastavi GBL_RLS. */
int AcpiMagnumReleaseGlobalLock(void *Facs)
{
    volatile UINT32 *Lock = &((ACPI_TABLE_FACS *) Facs)->GlobalLock;
    UINT32 Old, New;

    Old = __atomic_load_n(Lock, __ATOMIC_SEQ_CST);
    do {
        New = Old & ~3u;
    } while (!__atomic_compare_exchange_n(Lock, &Old, New, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
    return (int) (Old & 1u);
}

/* ------------------------------------------------------------------------
 * Pomocne funkce pro Rust (user/acpid): cteni struktur ACPICA, jejichz
 * rozlozeni je slozite (FADT, zdroje _CRS). Rust vidi jen jednoduche
 * struktury nize (stejne rozlozeni v libs/acpica/src/lib.rs).
 * --------------------------------------------------------------------- */

/* Rozlozeni ACPI_OBJECT, ACPI_BUFFER a ACPI_OBJECT_LIST, na ktere spoleha
 * Rust (libs/acpica/src/lib.rs). Zmena ve verzi ACPICA = chyba prekladu. */
_Static_assert(sizeof(ACPI_OBJECT) == 24, "ACPI_OBJECT");
_Static_assert(__builtin_offsetof(ACPI_OBJECT, Integer.Value) == 8, "Integer.Value");
_Static_assert(__builtin_offsetof(ACPI_OBJECT, String.Length) == 4, "String.Length");
_Static_assert(__builtin_offsetof(ACPI_OBJECT, String.Pointer) == 8, "String.Pointer");
_Static_assert(__builtin_offsetof(ACPI_OBJECT, Package.Count) == 4, "Package.Count");
_Static_assert(__builtin_offsetof(ACPI_OBJECT, Package.Elements) == 8, "Package.Elements");
_Static_assert(__builtin_offsetof(ACPI_OBJECT, Reference.Handle) == 8, "Reference.Handle");
_Static_assert(sizeof(ACPI_BUFFER) == 16, "ACPI_BUFFER");
_Static_assert(sizeof(ACPI_OBJECT_LIST) == 16, "ACPI_OBJECT_LIST");
_Static_assert(sizeof(ACPI_PCI_ID) == 8, "ACPI_PCI_ID");

/* Udaje z FADT pro sluzbu (po AcpiInitializeTables). */
typedef struct {
    UINT32 HwReduced;
    UINT32 ResetRegister;   /* FADT ma registr reset (AcpiReset ho pouzije) */
    UINT32 SciInterrupt;
    UINT32 S5Supported;     /* \_S5 existuje (typ spanku pro vypnuti) */
    UINT32 SleepRegisters;  /* HW-reduced: registry Sleep Control/Status */
    UINT8 Revision;
    char OemId[7];
} MAGNUM_PLATFORM;

ACPI_STATUS AcpiMagnumPlatform(MAGNUM_PLATFORM *Out)
{
    ACPI_TABLE_HEADER *Fadt;
    ACPI_STATUS Status;
    UINT8 TypA, TypB;

    memset(Out, 0, sizeof(*Out));
    Status = AcpiGetTable(ACPI_SIG_FADT, 1, &Fadt);
    if (ACPI_FAILURE(Status)) {
        return Status;
    }
    Out->Revision = Fadt->Revision;
    memcpy(Out->OemId, Fadt->OemId, 6);
    Out->OemId[6] = 0;
    Out->HwReduced = AcpiGbl_ReducedHardware ? 1 : 0;
    Out->ResetRegister = (AcpiGbl_FADT.Flags & ACPI_FADT_RESET_REGISTER) && AcpiGbl_FADT.ResetRegister.Address != 0;
    Out->SciInterrupt = AcpiGbl_FADT.SciInterrupt;
    Out->SleepRegisters = AcpiGbl_FADT.SleepControl.Address != 0 && AcpiGbl_FADT.SleepStatus.Address != 0;
    Out->S5Supported = ACPI_SUCCESS(AcpiGetSleepTypeData(ACPI_STATE_S5, &TypA, &TypB));
    return AE_OK;
}

/* Preruseni zarizeni z _CRS (Generic Event Device a podobne). */
typedef struct {
    UINT32 Number;
    UINT8 Edge;
    UINT8 ActiveLow;
    UINT16 Reserved;
} MAGNUM_IRQ;

typedef struct {
    MAGNUM_IRQ *Out;
    UINT32 Max;
    UINT32 Count;
} MAGNUM_IRQ_WALK;

static ACPI_STATUS CollectIrq(ACPI_RESOURCE *Res, void *Context)
{
    MAGNUM_IRQ_WALK *W = Context;
    UINT32 i;

    if (Res->Type == ACPI_RESOURCE_TYPE_EXTENDED_IRQ) {
        for (i = 0; i < Res->Data.ExtendedIrq.InterruptCount && W->Count < W->Max; i++) {
            W->Out[W->Count].Number = Res->Data.ExtendedIrq.Interrupts[i];
            W->Out[W->Count].Edge = Res->Data.ExtendedIrq.Triggering == ACPI_EDGE_SENSITIVE;
            W->Out[W->Count].ActiveLow = Res->Data.ExtendedIrq.Polarity == ACPI_ACTIVE_LOW;
            W->Count++;
        }
    } else if (Res->Type == ACPI_RESOURCE_TYPE_IRQ) {
        for (i = 0; i < Res->Data.Irq.InterruptCount && W->Count < W->Max; i++) {
            W->Out[W->Count].Number = Res->Data.Irq.Interrupts[i];
            W->Out[W->Count].Edge = Res->Data.Irq.Triggering == ACPI_EDGE_SENSITIVE;
            W->Out[W->Count].ActiveLow = Res->Data.Irq.Polarity == ACPI_ACTIVE_LOW;
            W->Count++;
        }
    }
    return AE_OK;
}

/* Vrati pocet preruseni zarizeni (nejvys Max) nebo zaporny kod pri chybe. */
INT32 AcpiMagnumCrsInterrupts(ACPI_HANDLE Device, MAGNUM_IRQ *Out, UINT32 Max)
{
    MAGNUM_IRQ_WALK W = { Out, Max, 0 };
    ACPI_STATUS Status = AcpiWalkResources(Device, METHOD_NAME__CRS, CollectIrq, &W);

    if (ACPI_FAILURE(Status)) {
        return -(INT32) Status;
    }
    return (INT32) W.Count;
}
