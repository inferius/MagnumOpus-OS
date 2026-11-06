//! Jednoducha textova konzole nad linearnim framebufferem.
//!
//! Font je predrasterizovany Noto Sans Mono s antialiasingem (crate
//! `noto-sans-mono-bitmap`), takze funguje i pro diakritiku. Velikost pisma
//! se voli podle rozliseni (zaklad pro HiDPI).

use core::fmt;

use boot_info::{Framebuffer, PixelFormat};
use noto_sans_mono_bitmap::{FontWeight, RasterHeight, get_raster, get_raster_width};

const MARGIN: usize = 8;
const LINE_SPACING: usize = 2;

const BACKGROUND: (u8, u8, u8) = (0x10, 0x18, 0x20);
const FOREGROUND: (u8, u8, u8) = (0xE0, 0xE6, 0xEA);

pub struct FbConsole {
    base: *mut u32,
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
    font_height: RasterHeight,
    char_width: usize,
    x: usize,
    y: usize,
}

// SAFETY: FbConsole je jediny vlastnik framebufferu a pristup k nemu je
// serializovany mutexem v `console`.
unsafe impl Send for FbConsole {}

impl FbConsole {
    /// # Safety
    ///
    /// `fb` musi popisovat platny, namapovany framebuffer s 32 bity na pixel
    /// a nikdo jiny do nej nesmi soucasne zapisovat.
    pub unsafe fn new(fb: &Framebuffer) -> Self {
        let font_height = if fb.height >= 1080 {
            RasterHeight::Size24
        } else {
            RasterHeight::Size16
        };
        let mut console = Self {
            base: fb.virt_addr as *mut u32,
            width: fb.width as usize,
            height: fb.height as usize,
            stride: fb.stride as usize,
            format: fb.format,
            font_height,
            char_width: get_raster_width(FontWeight::Regular, font_height),
            x: MARGIN,
            y: MARGIN,
        };
        console.clear();
        console
    }

    fn line_height(&self) -> usize {
        self.font_height.val() + LINE_SPACING
    }

    fn encode(&self, (r, g, b): (u8, u8, u8)) -> u32 {
        let (r, g, b) = (r as u32, g as u32, b as u32);
        match self.format {
            PixelFormat::Rgb => r | (g << 8) | (b << 16),
            _ => b | (g << 8) | (r << 16),
        }
    }

    fn put_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            // SAFETY: x < width <= stride a y < height, takze offset lezi
            // uvnitr framebufferu (viz `new`). Volatile, protoze jde o MMIO.
            unsafe { self.base.add(y * self.stride + x).write_volatile(color) };
        }
    }

    fn fill_rows(&mut self, from_y: usize, to_y: usize, color: u32) {
        for y in from_y..to_y.min(self.height) {
            for x in 0..self.width {
                self.put_pixel(x, y, color);
            }
        }
    }

    pub fn clear(&mut self) {
        let bg = self.encode(BACKGROUND);
        self.fill_rows(0, self.height, bg);
        self.x = MARGIN;
        self.y = MARGIN;
    }

    fn newline(&mut self) {
        self.x = MARGIN;
        self.y += self.line_height();
        if self.y + self.line_height() > self.height - MARGIN {
            self.scroll();
        }
    }

    fn scroll(&mut self) {
        let lh = self.line_height();
        let rows = self.height - lh;
        // SAFETY: zdroj i cil lezi uvnitr framebufferu (rows + lh = height),
        // `copy` zvlada prekryv.
        unsafe {
            core::ptr::copy(
                self.base.add(lh * self.stride),
                self.base,
                rows * self.stride,
            )
        };
        let bg = self.encode(BACKGROUND);
        self.fill_rows(rows, self.height, bg);
        self.y -= lh;
    }

    fn write_char(&mut self, c: char) {
        match c {
            '\n' => return self.newline(),
            '\r' => {
                self.x = MARGIN;
                return;
            }
            _ => {}
        }
        if self.x + self.char_width > self.width - MARGIN {
            self.newline();
        }
        let raster = get_raster(c, FontWeight::Regular, self.font_height)
            .or_else(|| get_raster('?', FontWeight::Regular, self.font_height))
            .expect("font neobsahuje '?'");
        for (dy, row) in raster.rows().enumerate() {
            for (dx, &intensity) in row.iter().enumerate() {
                let color = self.encode(blend(intensity));
                self.put_pixel(self.x + dx, self.y + dy, color);
            }
        }
        self.x += self.char_width;
    }
}

/// Smicha barvu popredi a pozadi podle intenzity pixelu fontu.
fn blend(intensity: u8) -> (u8, u8, u8) {
    let mix = |fg: u8, bg: u8| -> u8 {
        let i = intensity as u16;
        ((fg as u16 * i + bg as u16 * (255 - i)) / 255) as u8
    };
    (
        mix(FOREGROUND.0, BACKGROUND.0),
        mix(FOREGROUND.1, BACKGROUND.1),
        mix(FOREGROUND.2, BACKGROUND.2),
    )
}

impl fmt::Write for FbConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c);
        }
        Ok(())
    }
}
