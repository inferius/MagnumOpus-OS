//! Testy prikazu virtio-gpu (na hostiteli: `cargo test -p vgpu`).

use crate::*;

#[test]
fn commands_have_spec_sizes() {
    assert_eq!(get_display_info().len(), 24);
    assert_eq!(resource_create_2d(1, FORMAT_B8G8R8A8_UNORM, 640, 480).len(), 40);
    assert_eq!(set_scanout(0, 1, Rect::default()).len(), 48);
    assert_eq!(resource_flush(1, Rect::default()).len(), 48);
    assert_eq!(transfer_to_host_2d(1, Rect::default(), 0).len(), 56);
    assert_eq!(resource_attach_backing(1, &[(0x1000, 4096), (0x9000, 8192)]).len(), 32 + 32);
    assert_eq!(resource_unref(1).len(), 32);
}

#[test]
fn create_2d_fields() {
    let c = resource_create_2d(7, FORMAT_B8G8R8X8_UNORM, 1280, 800);
    let u = |o: usize| u32::from_le_bytes(c[o..o + 4].try_into().unwrap());
    assert_eq!(u(0), cmd::RESOURCE_CREATE_2D);
    assert_eq!((u(24), u(28), u(32), u(36)), (7, 2, 1280, 800));
}

#[test]
fn transfer_offset_and_rect() {
    let c = transfer_to_host_2d(3, Rect { x: 10, y: 20, w: 30, h: 40 }, 0x1_0000_0004);
    let u = |o: usize| u32::from_le_bytes(c[o..o + 4].try_into().unwrap());
    assert_eq!((u(24), u(28), u(32), u(36)), (10, 20, 30, 40));
    assert_eq!(u64::from_le_bytes(c[40..48].try_into().unwrap()), 0x1_0000_0004);
    assert_eq!(u(48), 3);
}

#[test]
fn display_info_parse() {
    let mut b = vec![0u8; DISPLAY_INFO_SIZE];
    b[0..4].copy_from_slice(&cmd::RESP_OK_DISPLAY_INFO.to_le_bytes());
    let o = HEADER_SIZE;
    b[o + 8..o + 12].copy_from_slice(&1280u32.to_le_bytes());
    b[o + 12..o + 16].copy_from_slice(&800u32.to_le_bytes());
    b[o + 16..o + 20].copy_from_slice(&1u32.to_le_bytes());
    let s = parse_display_info(&b, 1).unwrap();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].rect.w, 1280);
    assert!(s[0].enabled);
    // Kratka odpoved nebo chyba.
    assert!(parse_display_info(&b[..30], 1).is_none());
    b[0..4].copy_from_slice(&cmd::RESP_ERR_UNSPEC.to_le_bytes());
    assert!(parse_display_info(&b, 1).is_none());
    assert!(!is_ok(&b));
    // Zarizeni hlasi vic vystupu nez 16: precte se 16.
    b[0..4].copy_from_slice(&cmd::RESP_OK_DISPLAY_INFO.to_le_bytes());
    assert_eq!(parse_display_info(&b, 1000).unwrap().len(), MAX_SCANOUTS);
}

#[test]
fn chunks_are_whole_rows() {
    assert_eq!(rows_per_chunk(5120, 4 << 20), 819);
    assert_eq!(rows_per_chunk(0, 4 << 20), 0);
}
