//! Surove deklarace ACPICA (`acpixf.h`, `actypes.h`) a pomocnych funkci
//! z `c/glue.c`. Jen to, co system pouziva; rozlozeni struktur overuje
//! `_Static_assert` v `c/glue.c`.

#![allow(non_snake_case, non_camel_case_types)]

use core::ffi::{c_char, c_void};

pub type Status = u32;
pub type Handle = *mut c_void;

pub const AE_OK: Status = 0;
pub const AE_ERROR: Status = 0x0001;
pub const AE_NO_MEMORY: Status = 0x0004;
pub const AE_NOT_FOUND: Status = 0x0005;
pub const AE_NOT_EXIST: Status = 0x0006;
pub const AE_ALREADY_EXISTS: Status = 0x0007;
pub const AE_NOT_IMPLEMENTED: Status = 0x000E;
pub const AE_SUPPORT: Status = 0x000F;
pub const AE_LIMIT: Status = 0x0010;
pub const AE_TIME: Status = 0x0011;
pub const AE_NOT_ACQUIRED: Status = 0x0014;
pub const AE_NO_HARDWARE_RESPONSE: Status = 0x0016;
pub const AE_ACCESS: Status = 0x001D;
pub const AE_BAD_PARAMETER: Status = 0x1001;
pub const AE_BAD_ADDRESS: Status = 0x1009;

pub const ACPI_FULL_INITIALIZATION: u32 = 0;
pub const ACPI_EVENT_POWER_BUTTON: u32 = 2;
pub const ACPI_EVENT_SLEEP_BUTTON: u32 = 3;
pub const ACPI_INTERRUPT_HANDLED: u32 = 1;
pub const ACPI_SYSTEM_NOTIFY: u32 = 1;
pub const ACPI_DEVICE_NOTIFY: u32 = 2;
pub const ACPI_ALL_NOTIFY: u32 = 3;

pub const ACPI_TYPE_INTEGER: u32 = 0x01;
pub const ACPI_TYPE_STRING: u32 = 0x02;
pub const ACPI_TYPE_BUFFER: u32 = 0x03;
pub const ACPI_TYPE_PACKAGE: u32 = 0x04;
pub const ACPI_TYPE_DEVICE: u32 = 0x06;
pub const ACPI_TYPE_PROCESSOR: u32 = 0x0C;
pub const ACPI_TYPE_THERMAL: u32 = 0x0D;

pub const ACPI_FULL_PATHNAME: u32 = 0;
pub const ACPI_UINT32_MAX: u32 = u32::MAX;
/// Buffer alokuje ACPICA (uvolnit `AcpiOsFree`).
pub const ACPI_ALLOCATE_BUFFER: u64 = u64::MAX;
pub const ACPI_STATE_S5: u8 = 5;

/// Koren jmenneho prostoru (`ACPI_ROOT_OBJECT`).
pub const ROOT: Handle = usize::MAX as Handle;

/// `ACPI_OBJECT` (24 B): spolecne pole `kind`, zbytek podle typu -
/// Integer: `b` = hodnota; String/Buffer: `a` = delka, `b` = ukazatel;
/// Package: `a` = pocet, `b` = ukazatel na prvky; Reference: `b` = handle.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Object {
    pub kind: u32,
    pub a: u32,
    pub b: u64,
    pub c: u32,
    pub _pad: u32,
}

impl Object {
    pub const fn integer(v: u64) -> Self {
        Self { kind: ACPI_TYPE_INTEGER, a: 0, b: v, c: 0, _pad: 0 }
    }
}

#[repr(C)]
pub struct Buffer {
    pub length: u64,
    pub pointer: *mut c_void,
}

#[repr(C)]
pub struct ObjectList {
    pub count: u32,
    pub pointer: *mut Object,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PciId {
    pub segment: u16,
    pub bus: u16,
    pub device: u16,
    pub function: u16,
}

/// `MAGNUM_PLATFORM` z `c/glue.c`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct Platform {
    pub hw_reduced: u32,
    pub reset_register: u32,
    pub sci_interrupt: u32,
    pub s5_supported: u32,
    pub sleep_registers: u32,
    pub revision: u8,
    pub oem_id: [u8; 7],
}

/// `MAGNUM_IRQ` z `c/glue.c`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct Irq {
    pub number: u32,
    pub edge: u8,
    pub active_low: u8,
    pub _reserved: u16,
}

pub type EventHandler = unsafe extern "C" fn(context: *mut c_void) -> u32;
pub type NotifyHandler = unsafe extern "C" fn(device: Handle, value: u32, context: *mut c_void);
pub type WalkCallback =
    unsafe extern "C" fn(object: Handle, level: u32, context: *mut c_void, ret: *mut *mut c_void) -> Status;
pub type OsdHandler = unsafe extern "C" fn(context: *mut c_void) -> u32;
pub type OsdExecCallback = unsafe extern "C" fn(context: *mut c_void);

unsafe extern "C" {
    pub fn AcpiInitializeSubsystem() -> Status;
    pub fn AcpiInitializeTables(initial: *mut c_void, count: u32, allow_resize: u8) -> Status;
    pub fn AcpiLoadTables() -> Status;
    pub fn AcpiEnableSubsystem(flags: u32) -> Status;
    pub fn AcpiInitializeObjects(flags: u32) -> Status;
    pub fn AcpiUpdateAllGpes() -> Status;
    pub fn AcpiInstallInterface(name: *const c_char) -> Status;

    pub fn AcpiEnterSleepStatePrep(state: u8) -> Status;
    pub fn AcpiEnterSleepState(state: u8) -> Status;
    pub fn AcpiReset() -> Status;

    pub fn AcpiInstallFixedEventHandler(event: u32, handler: EventHandler, context: *mut c_void) -> Status;
    pub fn AcpiInstallNotifyHandler(device: Handle, kind: u32, handler: NotifyHandler, context: *mut c_void) -> Status;

    pub fn AcpiGetDevices(hid: *const c_char, cb: WalkCallback, context: *mut c_void, ret: *mut *mut c_void) -> Status;
    pub fn AcpiWalkNamespace(
        kind: u32,
        start: Handle,
        depth: u32,
        down: Option<WalkCallback>,
        up: Option<WalkCallback>,
        context: *mut c_void,
        ret: *mut *mut c_void,
    ) -> Status;
    pub fn AcpiGetHandle(parent: Handle, path: *const c_char, out: *mut Handle) -> Status;
    pub fn AcpiGetName(object: Handle, kind: u32, out: *mut Buffer) -> Status;
    pub fn AcpiEvaluateObject(object: Handle, path: *const c_char, args: *mut ObjectList, out: *mut Buffer) -> Status;

    pub fn AcpiOsFree(p: *mut c_void);

    /// `c/glue.c`: udaje z FADT.
    pub fn AcpiMagnumPlatform(out: *mut Platform) -> Status;
    /// `c/glue.c`: preruseni z `_CRS`; vraci pocet nebo zaporny kod chyby.
    pub fn AcpiMagnumCrsInterrupts(device: Handle, out: *mut Irq, max: u32) -> i32;
}
