//! Identita zarizeni PCI za handlem UEFI (vyrobce a typ), pres protokol
//! `EFI_PCI_IO_PROTOCOL`. Knihovna `uefi` ho neobaluje, tady je jen minimum:
//! cteni konfiguracniho prostoru.
//!
//! Pouziti: loader nechce jako framebuffer GOP virtio-gpu. Jeho obraz
//! firmware posila do zarizeni prikazy virtio; po ukonceni boot services
//! by ho uz nikdo neposilal a obrazovka by zustala prazdna.

use core::ffi::c_void;

use uefi::boot::{self, OpenProtocolAttributes, OpenProtocolParams};
use uefi::proto::device_path::DevicePath;
use uefi::proto::unsafe_protocol;
use uefi::{Handle, Status};

/// Cteni nebo zapis konfiguracniho prostoru (`EFI_PCI_IO_PROTOCOL_CONFIG`).
type ConfigFn = unsafe extern "efiapi" fn(
    this: *mut PciIo,
    width: u32,
    offset: u32,
    count: usize,
    buffer: *mut c_void,
) -> Status;

/// Pristup Read/Write (`EFI_PCI_IO_PROTOCOL_ACCESS`).
#[repr(C)]
struct Access {
    read: *const c_void,
    write: *const c_void,
}

#[repr(C)]
struct ConfigAccess {
    read: ConfigFn,
    write: ConfigFn,
}

/// Zacatek struktury `EFI_PCI_IO_PROTOCOL` (UEFI spec 2.10, kap. 14.4) az po
/// pristup ke konfiguraci; dalsi polozky nepotrebujeme a struktura se jen
/// cte pres ukazatel od firmwaru.
#[repr(C)]
#[unsafe_protocol("4cf5b200-68b8-4ca5-9eec-b23e3f50029a")]
pub struct PciIo {
    poll_mem: *const c_void,
    poll_io: *const c_void,
    mem: Access,
    io: Access,
    pci: ConfigAccess,
}

/// `EfiPciIoWidthUint32`
const WIDTH_U32: u32 = 2;

/// (vyrobce, zarizeni) PCI zarizeni, ke kteremu patri `handle` (napr.
/// vystup GOP - hleda se rodic s PciIo podle cesty zarizeni). `None`, kdyz
/// handle neni pod PCI nebo cteni selze.
pub fn pci_ids_of(handle: Handle) -> Option<(u16, u16)> {
    let params = OpenProtocolParams { handle, agent: boot::image_handle(), controller: None };
    // SAFETY: GetProtocol jen cte cestu zarizeni, nikoho neodpoji.
    let dp = unsafe { boot::open_protocol::<DevicePath>(params, OpenProtocolAttributes::GetProtocol) }.ok()?;
    let mut path: &DevicePath = &dp;
    let pci_handle = boot::locate_device_path::<PciIo>(&mut path).ok()?;
    let params = OpenProtocolParams { handle: pci_handle, agent: boot::image_handle(), controller: None };
    // SAFETY: GetProtocol; konfiguraci jen cteme.
    let mut io = unsafe { boot::open_protocol::<PciIo>(params, OpenProtocolAttributes::GetProtocol) }.ok()?;
    let mut id: u32 = 0;
    let this: *mut PciIo = &mut *io;
    // SAFETY: ukazatel na protokol od firmwaru, buffer 4 B pro jedno cteni
    // sirky 32 bitu z offsetu 0 (vyrobce a zarizeni).
    let st = unsafe { ((*this).pci.read)(this, WIDTH_U32, 0, 1, (&mut id as *mut u32).cast()) };
    if st.is_error() {
        return None;
    }
    Some((id as u16, (id >> 16) as u16))
}

/// Je to virtio-gpu (moderni 1af4:1050 nebo prechodny 1af4:1010)?
pub fn is_virtio_gpu(ids: Option<(u16, u16)>) -> bool {
    matches!(ids, Some((0x1af4, 0x1050)) | Some((0x1af4, 0x1010)))
}
