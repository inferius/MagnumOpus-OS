//! Testy popisu bufferu na hostiteli: `cargo test -p bufdesc`.

extern crate std;

use std::vec;

use crate::{BufDesc, DescError, MAX_DIM, Placement, Plane, cache, format, modifier};

#[test]
fn linear_layouts() {
    let d = BufDesc::linear(format::ARGB8888, 1920, 1080, 64).unwrap();
    assert_eq!(d.planes, [Plane { offset: 0, pitch: 7680 }]);
    assert_eq!(d.min_size(), Ok(7680 * 1080));
    // NV12: Y 1920x1080, UV 960 dvojic x 540 radku = 1920 B na radek.
    let n = BufDesc::linear(format::NV12, 1920, 1080, 64).unwrap();
    assert_eq!(n.planes, [Plane { offset: 0, pitch: 1920 }, Plane { offset: 1920 * 1080, pitch: 1920 }]);
    assert_eq!(n.min_size(), Ok(1920 * 1080 * 3 / 2));
    // I420 s lichou sirkou: chroma zaokrouhlena nahoru.
    let i = BufDesc::linear(format::YUV420, 641, 481, 1).unwrap();
    assert_eq!(i.planes.len(), 3);
    assert_eq!(i.planes[1].pitch, 321);
    assert_eq!(i.planes[2].offset, 641 * 481 + 321 * 241);
    // P010: 2 B na vzorek.
    let p = BufDesc::linear(format::P010, 64, 64, 1).unwrap();
    assert_eq!((p.planes[0].pitch, p.planes[1].pitch), (128, 128));
    assert_eq!(BufDesc::linear(fourcc_unknown(), 8, 8, 1).err(), Some(DescError::Format));
    assert_eq!(BufDesc::linear(format::NV12, 0, 8, 1).err(), Some(DescError::Size));
    assert_eq!(BufDesc::linear(format::NV12, 8, 8, 3).err(), Some(DescError::Encoding));
}

fn fourcc_unknown() -> u32 {
    crate::fourcc(b"ZZZZ")
}

#[test]
fn validate_against_vmo() {
    let d = BufDesc::linear(format::NV12, 1280, 720, 256).unwrap();
    let need = d.min_size().unwrap();
    assert_eq!(d.validate(need), Ok(()));
    assert_eq!(d.validate(need - 1), Err(DescError::Bounds { plane: 1 }));
    // Krok radku mensi nez radek pixelu.
    let mut bad = d.clone();
    bad.planes[0].pitch = 1279;
    assert_eq!(bad.validate(u64::MAX), Err(DescError::Pitch { plane: 0 }));
    // Spatny pocet rovin pro format.
    let mut bad = d.clone();
    bad.planes.pop();
    assert_eq!(bad.validate(u64::MAX), Err(DescError::Planes));
    let mut bad = d.clone();
    bad.planes.clear();
    assert_eq!(bad.validate(u64::MAX), Err(DescError::Planes));
    // Rozmery.
    let mut bad = d.clone();
    bad.width = MAX_DIM + 1;
    assert_eq!(bad.validate(u64::MAX), Err(DescError::Size));
    // Neznamy format s LINEAR, neplatny modifikator, typ cache.
    let mut bad = d.clone();
    bad.fourcc = fourcc_unknown();
    assert_eq!(bad.validate(u64::MAX), Err(DescError::Format));
    let mut bad = d.clone();
    bad.modifier = modifier::INVALID;
    assert_eq!(bad.validate(u64::MAX), Err(DescError::Format));
    let mut bad = d.clone();
    bad.cache = 3;
    assert_eq!(bad.validate(u64::MAX), Err(DescError::Encoding));
}

#[test]
fn overflow_is_rejected_not_wrapped() {
    // Nepratelsky popis: offset tesne pod u64::MAX, obri pitch.
    let d = BufDesc {
        fourcc: format::ARGB8888,
        modifier: modifier::LINEAR,
        width: 16,
        height: 16,
        planes: vec![Plane { offset: u64::MAX - 10, pitch: 64 }],
        placement: Placement::System,
        cache: cache::CACHED,
    };
    assert_eq!(d.validate(u64::MAX), Err(DescError::Bounds { plane: 0 }));
    // Dlazdicovy modifikator: hrube meze pitch * vyska.
    let mut t = d.clone();
    t.modifier = 0x0100_0000_0000_0001;
    t.planes[0] = Plane { offset: 4096, pitch: u32::MAX };
    assert_eq!(t.validate(1 << 32), Err(DescError::Bounds { plane: 0 }));
    t.planes[0] = Plane { offset: 0, pitch: 64 };
    assert_eq!(t.validate(64 * 16), Ok(()));
    assert_eq!(t.validate(64 * 16 - 1), Err(DescError::Bounds { plane: 0 }));
    t.planes[0].pitch = 0;
    assert_eq!(t.validate(1 << 20), Err(DescError::Pitch { plane: 0 }));
}

#[test]
fn encoding_roundtrip_and_strictness() {
    let mut d = BufDesc::linear(format::YUV420, 320, 240, 16).unwrap();
    d.placement = Placement::Device;
    d.cache = cache::WRITE_COMBINING;
    let b = d.encode();
    assert_eq!(b.len(), 72);
    assert_eq!(BufDesc::decode(&b), Ok(d.clone()));
    // Zkracene, s nenulovou nepouzitou rovinou, s rezervou, s neznamym
    // umistenim, s 0 nebo 5 rovinami.
    assert!(BufDesc::decode(&b[..71]).is_err());
    let mut x = b.clone();
    x.push(0);
    assert!(BufDesc::decode(&x).is_err());
    let mut x = b.clone();
    x[24 + 3 * 12] = 1; // offset ctvrte (nepouzite) roviny
    assert_eq!(BufDesc::decode(&x), Err(DescError::Encoding));
    let mut x = b.clone();
    x[23] = 1; // rezerva
    assert_eq!(BufDesc::decode(&x), Err(DescError::Encoding));
    let mut x = b.clone();
    x[21] = 2; // umisteni
    assert_eq!(BufDesc::decode(&x), Err(DescError::Encoding));
    for n in [0u8, 5] {
        let mut x = b.clone();
        x[20] = n;
        assert_eq!(BufDesc::decode(&x), Err(DescError::Encoding), "{n} rovin");
    }
}
