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

#[test]
fn commands_3d_have_spec_sizes() {
    assert_eq!(ctx_create(1, "compd").len(), 96);
    assert_eq!(ctx_attach_resource(1, 2).len(), 32);
    assert_eq!(resource_create_3d(1, virgl::target::TEXTURE_2D, virgl::format::B8G8R8A8_UNORM, virgl::bind::SAMPLER_VIEW, 4, 4).len(), 72);
    assert_eq!(transfer_3d(true, 1, 2, Rect::default(), 0, 16).len(), 72);
    let s = submit_3d(1, &[1, 2, 3, 4]);
    assert_eq!(s.len(), 32 + 4);
    let u = |o: usize| u32::from_le_bytes(s[o..o + 4].try_into().unwrap());
    assert_eq!((u(0), u(16), u(24)), (cmd::SUBMIT_3D, 1, 4));
}

#[test]
fn virgl_headers_and_lengths() {
    use virgl::*;
    let mut s = Stream::new();
    s.create_blend_premultiplied(5);
    // Hlavicka: cmd 1, obj BLEND 1, delka 11.
    assert_eq!(s.words()[0], 1 | 1 << 8 | 11 << 16);
    assert_eq!(s.words().len(), 12);
    let s2 = s.words()[4];
    assert_eq!(s2 & 1, 1, "blend zapnuty");
    assert_eq!(s2 >> 4 & 0x1F, 1, "src ONE");
    assert_eq!(s2 >> 9 & 0x1F, 0x13, "dst INV_SRC_ALPHA");
    assert_eq!(s2 >> 27, 0xF, "maska RGBA");
    s.reset();
    s.create_shader(7, SHADER_FRAGMENT, "FRAG\nEND\n");
    // Text 9 B + NUL = 10 B -> 3 dwordy; delka = 5 + 3.
    assert_eq!(s.words()[0] >> 16, 8);
    assert_eq!(s.words()[3], 10, "delka textu vcetne NUL");
    assert_eq!(s.words().len(), 9);
    s.reset();
    s.draw_strip(0, 4).set_viewport(800, 600).set_scissor(1, 2, 3, 4).clear_color([0.0; 4]);
    assert_eq!(s.words()[0] >> 16, 12, "DRAW_VBO ma 12 poli");
    assert_eq!(s.words()[13] >> 16, 7, "viewport 7 poli");
    assert_eq!(s.words()[22], 0, "scissor slot");
    assert_eq!(s.words()[23], 1 | 2 << 16);
    s.reset();
    s.inline_write(9, 96, &[0u8; 192]);
    assert_eq!(s.words()[0] >> 16, 11 + 48);
    assert_eq!(s.words()[6], 96, "box x = offset");
    assert_eq!(s.words()[9], 192, "box w = bajty");
}

#[test]
fn quad_vertices_map_pixels_to_ndc() {
    let v = virgl::quad_vertices((0.0, 0.0, 400.0, 300.0), (0.0, 0.0, 1.0, 1.0), [1.0; 4], 800, 600);
    assert_eq!(v.len(), 4 * 48);
    let fl = |i: usize| f32::from_le_bytes(v[i * 4..i * 4 + 4].try_into().unwrap());
    // Prvni vrchol: levy horni roh obrazovky = (-1, 1).
    assert_eq!((fl(0), fl(1)), (-1.0, 1.0));
    // Druhy: levy dolni roh obdelniku (y = 300 -> NDC 0).
    assert_eq!((fl(12), fl(13)), (-1.0, 0.0));
}

#[test]
fn header_carries_ring_idx() {
    let c = blob::submit_3d_fenced(5, 3, 0x1122_3344_5566_7788, &[1, 2, 3, 4]);
    let u = |o: usize| u32::from_le_bytes(c[o..o + 4].try_into().unwrap());
    assert_eq!(u(0), cmd::SUBMIT_3D);
    assert_eq!(u(4), FLAG_FENCE | FLAG_INFO_RING_IDX);
    assert_eq!(u64::from_le_bytes(c[8..16].try_into().unwrap()), 0x1122_3344_5566_7788);
    assert_eq!(u(16), 5);
    // ring_idx je bajt 20, padding 21..24 nula.
    assert_eq!(&c[20..24], &[3, 0, 0, 0]);
    assert_eq!((u(24), u(28)), (4, 0));
    assert_eq!(&c[32..], &[1, 2, 3, 4]);
}

#[test]
fn blob_commands_layout() {
    // virtio_gpu_ctx_create: hlavicka, nlen, context_init, debug_name[64].
    let c = blob::ctx_create_init(2, CAPSET_VENUS, "venus");
    assert_eq!(c.len(), 24 + 8 + 64);
    let u = |c: &[u8], o: usize| u32::from_le_bytes(c[o..o + 4].try_into().unwrap());
    assert_eq!((u(&c, 0), u(&c, 16), u(&c, 24), u(&c, 28)), (cmd::CTX_CREATE, 2, 5, 4));
    assert_eq!(&c[32..37], b"venus");
    // virtio_gpu_resource_create_blob: 24 + 4*4 + 8 + 8, kusy po 16 B.
    let info = blob::BlobInfo {
        resource: 9,
        mem: blob::BLOB_MEM_HOST3D,
        flags: blob::BLOB_FLAG_USE_MAPPABLE,
        blob_id: 0,
        size: 1 << 20,
    };
    let c = blob::resource_create_blob(2, &info, &[]);
    assert_eq!(c.len(), 56);
    assert_eq!((u(&c, 0), u(&c, 16)), (cmd::RESOURCE_CREATE_BLOB, 2));
    assert_eq!((u(&c, 24), u(&c, 28), u(&c, 32), u(&c, 36)), (9, 2, 1, 0));
    assert_eq!(u64::from_le_bytes(c[48..56].try_into().unwrap()), 1 << 20);
    let c = blob::resource_create_blob(2, &info, &[(0x1000, 4096)]);
    assert_eq!((c.len(), u(&c, 36)), (72, 1));
    // virtio_gpu_resource_map_blob: 24 + id + padding + offset.
    let c = blob::resource_map_blob(2, 9, 0x20_0000);
    assert_eq!(c.len(), 40);
    assert_eq!((u(&c, 0), u(&c, 24)), (cmd::RESOURCE_MAP_BLOB, 9));
    assert_eq!(u64::from_le_bytes(c[32..40].try_into().unwrap()), 0x20_0000);
    assert_eq!(blob::resource_unmap_blob(2, 9).len(), 32);
    // virtio_gpu_set_scanout_blob: 24 + rect 16 + 6 x u32 + strides 16 + offsets 16.
    let c = blob::set_scanout_blob(0, 9, 64, 32, FORMAT_B8G8R8X8_UNORM, 256, 0);
    assert_eq!(c.len(), 96);
    assert_eq!((u(&c, 0), u(&c, 32), u(&c, 36)), (cmd::SET_SCANOUT_BLOB, 64, 32));
    assert_eq!((u(&c, 40), u(&c, 44), u(&c, 48), u(&c, 52), u(&c, 56)), (0, 9, 64, 32, 2));
    assert_eq!((u(&c, 64), u(&c, 80)), (256, 0));
    assert_eq!(blob::get_capset(CAPSET_VENUS, 0).len(), 32);
}

#[test]
fn blob_responses() {
    let mut b = vec![0u8; blob::MAP_INFO_SIZE];
    b[0..4].copy_from_slice(&cmd::RESP_OK_MAP_INFO.to_le_bytes());
    b[24..28].copy_from_slice(&blob::MAP_CACHE_CACHED.to_le_bytes());
    assert_eq!(blob::parse_map_info(&b), Some(blob::MAP_CACHE_CACHED));
    assert_eq!(blob::parse_map_info(&b[..26]), None);
    b[0..4].copy_from_slice(&cmd::RESP_ERR_INVALID_PARAMETER.to_le_bytes());
    assert_eq!(blob::parse_map_info(&b), None);
    let mut c = vec![0u8; 24 + 3];
    c[0..4].copy_from_slice(&cmd::RESP_OK_CAPSET.to_le_bytes());
    c[24..].copy_from_slice(&[7, 8, 9]);
    assert_eq!(blob::parse_capset(&c), Some(&[7u8, 8, 9][..]));
}

#[test]
fn shm_ranges_alloc_and_merge() {
    const A: u64 = 64 * 1024;
    let mut a = shm::RangeAlloc::new(4 * A + 100, A);
    assert_eq!(a.free_bytes(), 4 * A);
    let x = a.alloc(1).unwrap();
    let y = a.alloc(A + 1).unwrap();
    assert_eq!((x, y), (0, A));
    assert!(a.alloc(2 * A).is_none());
    assert_eq!(a.alloc(A), Some(3 * A));
    a.release(3 * A, A);
    a.release(x, 1);
    a.release(y, A + 1);
    // Po vraceni vseho jeden souvisly rozsah.
    assert_eq!(a.alloc(4 * A), Some(0));
    assert!(a.alloc(u64::MAX).is_none());
    assert!(shm::RangeAlloc::new(100, A).alloc(1).is_none());
}
