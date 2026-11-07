//! Testy prevodu formatu a kopie obdelniku (na hostiteli).

use super::*;

/// Plocha nad vektorem pixelu (`stride` v pixelech).
struct Buf {
    px: Vec<u32>,
    w: u32,
    h: u32,
    stride: u32,
}

impl Buf {
    fn new(w: u32, h: u32, stride: u32, fill: u32) -> Self {
        Self {
            px: vec![fill; (stride * h) as usize],
            w,
            h,
            stride,
        }
    }

    fn surface(&mut self) -> Surface {
        Surface {
            base: self.px.as_mut_ptr() as *mut u8,
            width: self.w,
            height: self.h,
            stride: self.stride * 4,
        }
    }

    fn at(&self, x: u32, y: u32) -> u32 {
        self.px[(y * self.stride + x) as usize]
    }
}

/// Zdroj s jedinecnym pixelem na kazde pozici (alfa 0xFF).
fn source(w: u32, h: u32, stride: u32) -> Buf {
    let mut b = Buf::new(w, h, stride, 0);
    for y in 0..h {
        for x in 0..w {
            // B = x, G = y, R = x ^ y.
            b.px[(y * stride + x) as usize] = 0xFF00_0000 | ((x ^ y) & 0xFF) << 16 | (y & 0xFF) << 8 | (x & 0xFF);
        }
    }
    b
}

#[test]
fn convert_bgr_drops_alpha() {
    assert_eq!(convert(0x80_12_34_56, Format::Bgr), 0x00_12_34_56);
    assert_eq!(convert(0xFF_FF_FF_FF, Format::Bgr), 0x00_FF_FF_FF);
}

#[test]
fn convert_rgb_swaps_red_and_blue() {
    // B8G8R8A8: B=0x56, G=0x34, R=0x12 -> bajty R, G, B, 0.
    let p = convert(0xFF_12_34_56, Format::Rgb);
    assert_eq!(p.to_le_bytes(), [0x12, 0x34, 0x56, 0x00]);
    assert_eq!(convert(0xFF_00_00_FF, Format::Rgb), 0x00_FF_00_00);
}

#[test]
fn format_names_roundtrip() {
    for f in [Format::Rgb, Format::Bgr] {
        assert_eq!(Format::parse(f.name()), Some(f));
    }
    assert_eq!(Format::from_abi(1), Some(Format::Rgb));
    assert_eq!(Format::from_abi(2), Some(Format::Bgr));
    assert_eq!(Format::from_abi(0), None);
    assert_eq!(Format::parse("xrgb"), None);
}

#[test]
fn clip_to_surface() {
    assert_eq!(clip(Rect::new(0, 0, 10, 10), 20, 20), Some(Rect::new(0, 0, 10, 10)));
    assert_eq!(clip(Rect::new(-5, -3, 10, 10), 20, 20), Some(Rect::new(0, 0, 5, 7)));
    assert_eq!(clip(Rect::new(15, 18, 10, 10), 20, 20), Some(Rect::new(15, 18, 5, 2)));
    assert_eq!(clip(Rect::new(0, 0, u32::MAX, u32::MAX), 20, 20), Some(Rect::new(0, 0, 20, 20)));
    assert_eq!(clip(Rect::new(20, 0, 5, 5), 20, 20), None);
    assert_eq!(clip(Rect::new(-10, 0, 10, 5), 20, 20), None);
    assert_eq!(clip(Rect::new(i32::MAX, i32::MAX, u32::MAX, u32::MAX), 20, 20), None);
    assert_eq!(clip(Rect::new(3, 3, 0, 4), 20, 20), None);
}

#[test]
fn copy_rect_converts_only_inside() {
    let mut src = source(16, 8, 16);
    // Cil s vetsim krokem (vypln na konci radku se nesmi zmenit).
    let mut dst = Buf::new(16, 8, 20, 0xDEAD_BEEF);
    let (s, d) = (src.surface(), dst.surface());
    // SAFETY: obe plochy jsou cele ve vektorech.
    let done = unsafe { copy_rect(&s, &d, Rect::new(2, 1, 5, 3), Format::Rgb) };
    assert_eq!(done, Some(Rect::new(2, 1, 5, 3)));
    for y in 0..8 {
        for x in 0..20 {
            let got = dst.px[(y * 20 + x) as usize];
            if (2..7).contains(&x) && (1..4).contains(&y) {
                assert_eq!(got, convert(src.at(x, y), Format::Rgb), "pixel {x},{y}");
            } else {
                assert_eq!(got, 0xDEAD_BEEF, "mimo obdelnik {x},{y}");
            }
        }
    }
}

#[test]
fn copy_rect_clips_to_smaller_surface() {
    // Zdroj vetsi nez cil (jiny krok): kopie se orizne na cil.
    let mut src = source(32, 16, 40);
    let mut dst = Buf::new(10, 6, 12, 0);
    let (s, d) = (src.surface(), dst.surface());
    // SAFETY: obe plochy jsou cele ve vektorech.
    let done = unsafe { copy_rect(&s, &d, Rect::new(-4, 4, 100, 100), Format::Bgr) };
    assert_eq!(done, Some(Rect::new(0, 4, 10, 2)));
    for y in 0..6 {
        for x in 0..12 {
            let got = dst.at(x, y);
            if x < 10 && y >= 4 {
                assert_eq!(got, src.at(x, y) & 0x00FF_FFFF, "pixel {x},{y}");
            } else {
                assert_eq!(got, 0, "mimo {x},{y}");
            }
        }
    }
    // Uplne mimo: nic.
    // SAFETY: jako vyse.
    assert_eq!(unsafe { copy_rect(&s, &d, Rect::new(10, 0, 5, 5), Format::Bgr) }, None);
}

#[test]
fn surface_validity() {
    let mut b = Buf::new(4, 4, 4, 0);
    let s = b.surface();
    assert!(s.is_valid());
    assert_eq!(s.bytes(), 64);
    assert!(!Surface { stride: 12, ..s }.is_valid());
    assert!(!Surface { stride: 18, ..s }.is_valid());
    assert!(!Surface { base: core::ptr::null_mut(), ..s }.is_valid());
}

