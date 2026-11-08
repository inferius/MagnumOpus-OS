//! Jmenny prostor ACPI: hledani zarizeni a vyhodnoceni metod s vysledkem
//! jako vlastni hodnota v Rustu ([`Value`]). Buffery od ACPICA se uvolni
//! hned po prevodu.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{CStr, c_void};

use crate::ffi::{self, Buffer, Handle, Object, ObjectList, Status};

/// Vysledek metody ACPI (kopie, nezavisla na pameti ACPICA).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Integer(u64),
    String(String),
    Buffer(Vec<u8>),
    Package(Vec<Value>),
    /// Odkaz na objekt jmenneho prostoru (napr. procesor v `_PSL`).
    Reference(usize),
    /// Jiny typ (cislo typu ACPICA).
    Other(u32),
}

impl Value {
    pub fn as_integer(&self) -> Option<u64> {
        match self {
            Value::Integer(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_package(&self) -> Option<&[Value]> {
        match self {
            Value::Package(v) => Some(v),
            _ => None,
        }
    }
}

/// Nejvetsi hloubka vnoreni balicku pri prevodu (ochrana zasobniku pred
/// spatnym AML).
const MAX_DEPTH: u32 = 8;

/// Prevede objekt ACPICA na [`Value`].
///
/// # Safety
///
/// `o` je platny objekt z vysledku `AcpiEvaluateObject`.
unsafe fn convert(o: &Object, depth: u32) -> Value {
    match o.kind {
        ffi::ACPI_TYPE_INTEGER => Value::Integer(o.b),
        ffi::ACPI_TYPE_STRING => {
            // SAFETY: String.Pointer ukazuje na `Length` bajtu (kontrakt ACPICA).
            let s = unsafe { core::slice::from_raw_parts(o.b as *const u8, o.a as usize) };
            Value::String(String::from_utf8_lossy(s).into_owned())
        }
        ffi::ACPI_TYPE_BUFFER => {
            // SAFETY: dtto pro Buffer.
            let s = unsafe { core::slice::from_raw_parts(o.b as *const u8, o.a as usize) };
            Value::Buffer(s.to_vec())
        }
        ffi::ACPI_TYPE_PACKAGE if depth < MAX_DEPTH => {
            // SAFETY: Package.Elements ma `Count` prvku.
            let items = unsafe { core::slice::from_raw_parts(o.b as *const Object, o.a as usize) };
            Value::Package(items.iter().map(|i| unsafe { convert(i, depth + 1) }).collect())
        }
        0x14 => Value::Reference(o.b as usize),
        k => Value::Other(k),
    }
}

/// Vyhodnoti objekt nebo metodu `path` (relativne k `h`, nebo absolutne
/// s `\`), s argumenty `args`.
pub fn eval(h: Handle, path: &CStr, args: &[Object]) -> Result<Value, Status> {
    let mut args = args.to_vec();
    let mut list = ObjectList { count: args.len() as u32, pointer: args.as_mut_ptr() };
    let mut out = Buffer { length: ffi::ACPI_ALLOCATE_BUFFER, pointer: core::ptr::null_mut() };
    let list_ptr = if args.is_empty() { core::ptr::null_mut() } else { &mut list as *mut ObjectList };
    // SAFETY: platne struktury; ACPICA alokuje vysledek (uvolni se nize).
    let st = unsafe { ffi::AcpiEvaluateObject(h, path.as_ptr(), list_ptr, &mut out) };
    if st != ffi::AE_OK {
        return Err(st);
    }
    if out.pointer.is_null() {
        return Ok(Value::Other(0));
    }
    // SAFETY: vysledek je jeden ACPI_OBJECT na zacatku bufferu.
    let v = unsafe { convert(&*(out.pointer as *const Object), 0) };
    // SAFETY: buffer alokovala ACPICA pres AcpiOsAllocate.
    unsafe { ffi::AcpiOsFree(out.pointer) };
    Ok(v)
}

/// Vyhodnoti a vrati cele cislo (`_TMP`, `_STA`, ...).
pub fn eval_integer(h: Handle, path: &CStr) -> Option<u64> {
    eval(h, path, &[]).ok()?.as_integer()
}

/// Existuje objekt `path` pod `h`?
pub fn exists(h: Handle, path: &CStr) -> bool {
    let mut out = core::ptr::null_mut();
    // SAFETY: platne argumenty.
    unsafe { ffi::AcpiGetHandle(h, path.as_ptr(), &mut out) == ffi::AE_OK }
}

/// Plna cesta objektu (`\_SB_.PCI0`).
pub fn path(h: Handle) -> String {
    let mut out = Buffer { length: ffi::ACPI_ALLOCATE_BUFFER, pointer: core::ptr::null_mut() };
    // SAFETY: ACPICA alokuje retezec zakonceny nulou.
    if unsafe { ffi::AcpiGetName(h, ffi::ACPI_FULL_PATHNAME, &mut out) } != ffi::AE_OK || out.pointer.is_null() {
        return String::from("?");
    }
    // SAFETY: dtto.
    let s = unsafe { CStr::from_ptr(out.pointer as *const core::ffi::c_char) }.to_string_lossy().into_owned();
    unsafe { ffi::AcpiOsFree(out.pointer) };
    s
}

unsafe extern "C" fn collect(object: Handle, _level: u32, context: *mut c_void, _ret: *mut *mut c_void) -> Status {
    // SAFETY: context je `&mut Vec<Handle>` z `devices`/`objects`.
    let v = unsafe { &mut *(context as *mut Vec<usize>) };
    if v.len() < 256 {
        v.push(object as usize);
    }
    ffi::AE_OK
}

/// Zarizeni s danym `_HID` nebo `_CID` (napr. `PNP0C0C` = tlacitko napajeni).
/// Jen zarizeni, ktera jsou pritomna (`_STA`).
pub fn devices(hid: &CStr) -> Vec<Handle> {
    let mut v: Vec<usize> = Vec::new();
    // SAFETY: callback dostane `&mut v` jako context jen behem volani.
    unsafe {
        ffi::AcpiGetDevices(hid.as_ptr(), collect, &mut v as *mut Vec<usize> as *mut c_void, core::ptr::null_mut());
    }
    v.into_iter().map(|h| h as Handle).collect()
}

/// Vsechny objekty daneho typu (`ffi::ACPI_TYPE_THERMAL`, ...).
pub fn objects(kind: u32) -> Vec<Handle> {
    let mut v: Vec<usize> = Vec::new();
    // SAFETY: dtto.
    unsafe {
        ffi::AcpiWalkNamespace(
            kind,
            ffi::ROOT,
            ffi::ACPI_UINT32_MAX,
            Some(collect),
            None,
            &mut v as *mut Vec<usize> as *mut c_void,
            core::ptr::null_mut(),
        );
    }
    v.into_iter().map(|h| h as Handle).collect()
}

/// Preruseni zarizeni z `_CRS` (nejvys 8).
pub fn interrupts(h: Handle) -> Result<Vec<ffi::Irq>, Status> {
    let mut out = [ffi::Irq::default(); 8];
    // SAFETY: pole o 8 prvcich.
    let n = unsafe { ffi::AcpiMagnumCrsInterrupts(h, out.as_mut_ptr(), out.len() as u32) };
    if n < 0 {
        return Err((-n) as Status);
    }
    Ok(out[..n as usize].to_vec())
}
