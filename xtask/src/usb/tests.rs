//! Testy ziveho USB: pravidla zapisu na disk, rozbor vypisu disku, zapis
//! obrazu a rozlozeni obrazu.

use std::io::Cursor;

use super::hostdisk::{HostDisk, Refusal, check, confirmation, write_image};
use super::platform::{parse_lsblk, parse_windows};

const GIB: u64 = 1024 * 1024 * 1024;

fn disk(id: &str, bus: &str, size: u64, system: bool) -> HostDisk {
    HostDisk {
        id: id.into(),
        path: format!("/dev/{id}"),
        model: "Model".into(),
        size,
        bus: bus.into(),
        removable: false,
        system,
        mounted: Vec::new(),
    }
}

#[test]
fn refuses_system_internal_small_and_unknown() {
    let disks = [
        disk("0", "NVMe", 512 * GIB, true),
        disk("1", "SATA", 1024 * GIB, false),
        disk("2", "USB", 16 * GIB, false),
        disk("3", "USB", GIB / 2, false),
        disk("4", "USB", 64 * GIB, true),
    ];
    assert_eq!(check(&disks, "0", GIB), Err(Refusal::System));
    assert_eq!(check(&disks, "1", GIB), Err(Refusal::NotRemovable("SATA".into())));
    assert_eq!(check(&disks, "2", GIB).map(|d| d.id.as_str()), Ok("2"));
    assert!(matches!(check(&disks, "3", GIB), Err(Refusal::TooSmall { .. })));
    // USB disk se systemem (Windows To Go, Linux z USB) taky ne.
    assert_eq!(check(&disks, "4", GIB), Err(Refusal::System));
    assert_eq!(check(&disks, "9", GIB), Err(Refusal::NotFound));
    // Linux: priznak removable staci i bez sbernice USB (ctecka SD na PCI).
    let mut sd = disk("mmcblk0", "", 32 * GIB, false);
    sd.removable = true;
    assert!(check(&[sd], "/dev/mmcblk0", GIB).is_ok());
    assert_eq!(confirmation(&disks[2]), "ANO 2");
}

#[test]
fn parses_windows_list() {
    let text = "0|Samsung SSD 980|1000204886016|NVMe|True|True|C:\r\n\
                2|SanDisk Ultra|30752636928|USB|False|False|E:,F:\r\n\
                garbage\r\n";
    let d = parse_windows(text);
    assert_eq!(d.len(), 2);
    assert!(d[0].system);
    assert_eq!(d[1].path, r"\\.\PhysicalDrive2");
    assert_eq!(d[1].bus, "USB");
    assert_eq!(d[1].mounted, vec!["E:", "F:"]);
    assert!(check(&d, "2", GIB).is_ok());
    assert_eq!(check(&d, "0", GIB), Err(Refusal::System));
}

#[test]
fn parses_lsblk_with_nested_root() {
    let text = r#"NAME="nvme0n1" PATH="/dev/nvme0n1" TYPE="disk" SIZE="512110190592" RM="0" TRAN="nvme" MODEL="WD SN770" MOUNTPOINT="" PKNAME=""
NAME="nvme0n1p1" PATH="/dev/nvme0n1p1" TYPE="part" SIZE="536870912" RM="0" TRAN="nvme" MODEL="" MOUNTPOINT="/boot/efi" PKNAME="nvme0n1"
NAME="nvme0n1p2" PATH="/dev/nvme0n1p2" TYPE="part" SIZE="511571771392" RM="0" TRAN="nvme" MODEL="" MOUNTPOINT="" PKNAME="nvme0n1"
NAME="vg-root" PATH="/dev/mapper/vg-root" TYPE="lvm" SIZE="511571771392" RM="0" TRAN="" MODEL="" MOUNTPOINT="/" PKNAME="nvme0n1p2"
NAME="sdb" PATH="/dev/sdb" TYPE="disk" SIZE="15376318464" RM="1" TRAN="usb" MODEL="Flash Disk" MOUNTPOINT="" PKNAME=""
NAME="sdb1" PATH="/dev/sdb1" TYPE="part" SIZE="15375269888" RM="1" TRAN="usb" MODEL="" MOUNTPOINT="/media/u/STICK" PKNAME="sdb"
"#;
    let d = parse_lsblk(text);
    assert_eq!(d.len(), 2);
    assert!(d[0].system, "koren pres LVM na oddilu NVMe");
    assert!(!d[1].system);
    assert!(d[1].removable);
    assert_eq!(d[1].mounted, vec!["/media/u/STICK"]);
    assert_eq!(check(&d, "/dev/nvme0n1", GIB), Err(Refusal::System));
    assert!(check(&d, "/dev/sdb", GIB).is_ok());
}

#[test]
fn write_image_puts_head_last_and_wipes_old_backup_gpt() {
    let dir = std::env::temp_dir().join(format!("xtask-usb-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = dir.join("img");
    let mib = 1024 * 1024;
    let data: Vec<u8> = (0..3 * mib).map(|i| (i / mib + 1) as u8).collect();
    std::fs::write(&img, &data).unwrap();
    // Disk 5 MiB plny 0xEE (stara data a zalozni GPT na konci).
    let mut target = Cursor::new(vec![0xEEu8; 5 * mib]);
    let mut order = Vec::new();
    write_image(&img, &mut target, 5 * mib as u64, |done, _| order.push(done)).unwrap();
    let out = target.into_inner();
    assert_eq!(&out[..3 * mib], &data[..]);
    // Mezi obrazem a poslednim MiB zustava stary obsah, posledni MiB je nulovy.
    assert!(out[3 * mib..4 * mib].iter().all(|&b| b == 0xEE));
    assert!(out[4 * mib..].iter().all(|&b| b == 0));
    assert_eq!(order.len(), 3);
    // Disk mensi nez obraz.
    let mut small = Cursor::new(vec![0u8; 2 * mib]);
    assert!(write_image(&img, &mut small, 2 * mib as u64, |_, _| {}).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn image_layout_has_esp_and_our_partition() {
    let dir = std::env::temp_dir().join(format!("xtask-usb-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("kernel");
    std::fs::write(&f, b"kernel").unwrap();
    let img = dir.join("usb.img");
    let layout = super::image::create(&img, super::image::MIN_MIB, &[("os/kernel.elf", &f)]).unwrap();
    // Cteni po sektorech ze souboru (obraz se nenacita cely do pameti).
    let mut file = std::fs::File::open(&img).unwrap();
    let mut read = |at: u64, buf: &mut [u8]| {
        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(at)).unwrap();
        file.read_exact(buf).unwrap();
    };
    let sectors = layout.size / 512;
    let d = diskguard::decide(sectors, 512, |lba, buf| {
        read(lba * 512, buf);
        true
    });
    assert!(d.writable, "{:?}", d.kind);
    assert_eq!(d.start * 512, layout.data.0);
    assert_eq!(d.sectors * 512, layout.data.1);
    // Superblok naseho FS na zacatku datoveho oddilu (blok 1..=8).
    let mut sb = vec![0u8; 9 * 4096];
    read(layout.data.0, &mut sb);
    assert!(sb.chunks(4096).any(|b| b.starts_with(diskguard::FS_MAGIC_PREFIX)));
    // ESP je FAT32 se souborem jadra.
    let mut esp = vec![0u8; 512];
    read(1024 * 1024, &mut esp);
    assert_eq!(&esp[82..90], b"FAT32   ");
    std::fs::remove_dir_all(&dir).unwrap();
}
