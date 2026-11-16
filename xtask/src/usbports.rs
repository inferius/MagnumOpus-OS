//! Porty USB testu na radici `xhci.0` - jedine misto, kde se prideluji.
//! Zarizeni bez portu dostane prvni volny, takze zarizeni pri startu
//! (flash disky medii) by jinak obsadila port, ktery test pripojuje za
//! behu (QMP), a `device_add` by selhal ("usb port 4 not found").
//!
//! Radic ma 8 portu ([`xhci_device`]; vychozi `qemu-xhci` jen 4, cislo
//! portu je spolecne pro USB 2 i USB 3):
//!
//! - porty 1..4: klavesnice pri startu (prvni volny = 1) a test usb za
//!   behu (mys, klavesnice bez portu, hub na [`USB_TEST_HUB`]); zarizeni
//!   bez portu dostane prvni volny, tedy odsud,
//! - porty 5..8: flash disky medii ([`MEDIA_FAT`], ...), disk pripojeny
//!   za behu, slozka hostitele (`--share`, jen bezny beh).

/// Radic xHCI pro QEMU (8 portu USB 2 a 8 USB 3, cisla 1..8).
pub fn xhci_device() -> &'static str {
    "qemu-xhci,id=xhci,p2=8,p3=8"
}

/// Hub testu usb (klavesnice za hubem, `qmp.rs` `usb_steps`).
pub const USB_TEST_HUB: &str = "4";

/// Flash disky vymennych medii (porty USB 3).
pub const MEDIA_FAT: &str = "5";
pub const MEDIA_EXFAT: &str = "6";
pub const MEDIA_NTFS: &str = "7";
/// Disk pripojeny za behu (QMP `device_add`, po `device_del` volny).
pub const MEDIA_HOT: &str = "8";
/// Slozka hostitele (`--share`); jen bezny beh, kde testovaci media nejsou.
pub const MEDIA_SHARE: &str = "5";

/// Porty, ktere test pripojuje nebo obsazuje (kontrola kolizi v testech).
pub const TEST_PORTS: &[&str] = &[USB_TEST_HUB, MEDIA_FAT, MEDIA_EXFAT, MEDIA_NTFS, MEDIA_HOT];
