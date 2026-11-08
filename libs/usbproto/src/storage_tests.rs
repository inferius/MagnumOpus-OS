//! Testy tridy mass storage (BOT, SCSI).

use crate::storage::scsi::{self, key};
use crate::storage::*;

#[test]
fn cbw_layout() {
    let c = Cbw::new(0x1234_5678, 1, 4096, true, &scsi::rw(false, 100, 8).0[..10]);
    let b = c.encode();
    assert_eq!(&b[0..4], b"USBC");
    assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 0x1234_5678);
    assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 4096);
    assert_eq!(b[12], 0x80);
    assert_eq!(b[13], 1);
    assert_eq!(b[14], 10);
    assert_eq!(b[15], scsi::READ_10);
    assert_eq!(&b[17..21], &100u32.to_be_bytes());
    assert_eq!(&b[22..24], &8u16.to_be_bytes());
}

#[test]
fn csw_roundtrip_and_invalid() {
    let c = Csw {
        tag: 7,
        residue: 512,
        status: CswStatus::Failed,
    };
    assert_eq!(Csw::parse(&c.encode()), Some(c));
    let mut bad = c.encode();
    bad[0] = b'X';
    assert_eq!(Csw::parse(&bad), None);
    let mut bad = c.encode();
    bad[12] = 9;
    assert_eq!(Csw::parse(&bad), None);
    assert_eq!(Csw::parse(&c.encode()[..12]), None);
}

#[test]
fn rw_picks_16_for_large_lba() {
    let (c, n) = scsi::rw(true, 0x1_0000_0000, 8);
    assert_eq!(n, 16);
    assert_eq!(c[0], scsi::WRITE_16);
    assert_eq!(&c[2..10], &0x1_0000_0000u64.to_be_bytes());
    assert_eq!(&c[10..14], &8u32.to_be_bytes());
    // Posledni sektor pod 2^32 jeste (10).
    let (_, n) = scsi::rw(false, 0xFFFF_FFF8, 8);
    assert_eq!(n, 10);
    let (_, n) = scsi::rw(false, 0xFFFF_FFF9, 8);
    assert_eq!(n, 16);
}

#[test]
fn capacity_parsing() {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&0x003F_FFFFu32.to_be_bytes());
    b[4..].copy_from_slice(&512u32.to_be_bytes());
    assert_eq!(scsi::parse_capacity_10(&b), Some((0x003F_FFFF, 512)));
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&0x1_2345_6789u64.to_be_bytes());
    b[8..12].copy_from_slice(&4096u32.to_be_bytes());
    assert_eq!(scsi::parse_capacity_16(&b), Some((0x1_2345_6789, 4096)));
    assert_eq!(scsi::parse_capacity_10(&[0; 4]), None);
}

#[test]
fn inquiry_and_sense() {
    let mut b = [b' '; 36];
    b[0] = 0;
    b[1] = 0x80;
    b[8..12].copy_from_slice(b"QEMU");
    b[16..29].copy_from_slice(b"QEMU HARDDISK");
    b[32..36].copy_from_slice(b"2.5+");
    let i = scsi::parse_inquiry(&b).unwrap();
    assert_eq!(i.vendor, "QEMU");
    assert_eq!(i.product, "QEMU HARDDISK");
    assert!(i.removable);

    let mut s = [0u8; 18];
    s[0] = 0x70;
    s[2] = key::NOT_READY;
    s[12] = 0x3A;
    let sense = scsi::parse_sense(&s).unwrap();
    assert!(sense.no_medium());
    assert!(!sense.retryable());
    s[2] = key::UNIT_ATTENTION;
    s[12] = 0x28;
    assert!(scsi::parse_sense(&s).unwrap().retryable());
    assert_eq!(scsi::parse_sense(&[0x72, 0x05, 0x24, 0x00]).unwrap().key, key::ILLEGAL_REQUEST);
}

#[test]
fn class_requests() {
    assert_eq!(reset(2).to_bytes(), [0x21, 0xFF, 0, 0, 2, 0, 0, 0]);
    assert_eq!(get_max_lun(0).to_bytes(), [0xA1, 0xFE, 0, 0, 0, 0, 1, 0]);
    assert_eq!(clear_halt(0x81).to_bytes(), [0x02, 0x01, 0, 0, 0x81, 0, 0, 0]);
    assert!(is_bot_disk(0x08, 0x06, 0x50));
    assert!(!is_bot_disk(0x08, 0x06, PROTOCOL_UAS));
    assert!(scsi::write_protected(&[3, 0, 0x80, 0]));
}
