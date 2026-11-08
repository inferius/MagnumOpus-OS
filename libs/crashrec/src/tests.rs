//! Testy formatu zaznamu padu.

use super::*;

fn header() -> Header {
    Header {
        flags: flags::TEST,
        uptime_ns: 12_345_678_901,
        realtime_s: 1_800_000_000,
        cpu: 2,
        reason: fixed("KERNEL_PANIC"),
        version: fixed("0.1.0"),
    }
}

#[test]
fn roundtrip_and_tail_kept() {
    let mut area = [0u8; 256];
    let a = [b'a'; 100];
    let b = [b'b'; 100];
    // 200 B textu do 144 B: zustane konec (44 x 'a' a 100 x 'b').
    let n = write(&mut area, &header(), &[&a, &b]);
    assert_eq!(n, 256 - HEADER);
    let (h, text, crc) = parse(&area).unwrap();
    assert_eq!(h, header());
    assert_eq!(text.len(), 144);
    assert!(text[..44].iter().all(|&c| c == b'a'));
    assert!(text[44..].iter().all(|&c| c == b'b'));
    assert_eq!(field_str(&h.reason), "KERNEL_PANIC");
    assert_ne!(crc, 0);
}

#[test]
fn corrupted_or_cleared_is_none() {
    let mut area = [0u8; 512];
    write(&mut area, &header(), &[b"zprava\n"]);
    assert!(parse(&area).is_some());
    let mut bad = area;
    bad[HEADER] ^= 1;
    assert!(parse(&bad).is_none(), "CRC");
    let mut bad = area;
    bad[16..20].copy_from_slice(&10_000u32.to_le_bytes());
    assert!(parse(&bad).is_none(), "delka");
    clear(&mut area);
    assert!(parse(&area).is_none());
    assert!(parse(&[0u8; 10]).is_none());
}

#[test]
fn id_format() {
    assert_eq!(&crash_id(0x7F3A_21C9), b"MO-7F3A-21C9");
    assert_eq!(crc32_update(0, b"123456789"), 0xCBF4_3926);
}

#[cfg(feature = "alloc")]
#[test]
fn render_has_summary() {
    let mut area = [0u8; 512];
    write(&mut area, &header(), &[b"KERNEL PANIC na cpu2: x\n"]);
    let (h, text, crc) = parse(&area).unwrap();
    let s = render(&h, text, crc);
    assert!(s.contains("duvod:   KERNEL_PANIC"));
    assert!(s.contains("KERNEL PANIC na cpu2"));
    assert!(s.contains("#MO-"));
}
