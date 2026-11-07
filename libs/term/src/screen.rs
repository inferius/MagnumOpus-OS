//! Textova obrazovka a kresleni vlastnim pismem do framebufferu.
//!
//! [`Screen`] je mrizka znaku s barvou (rozumi textu z [`crate::vt`]:
//! `\n`, `\r`, `\t` a SGR barvy); [`paint`] ji vykresli pismem Noto Sans
//! Mono (s diakritikou) do linearniho framebufferu, ktery sluzba terminalu
//! dostane jako capability. Stejna obrazovka slouzi jako zachranna
//! textova konzole.

use alloc::vec::Vec;

use noto_sans_mono_bitmap::{FontWeight, RasterHeight, get_raster, get_raster_width};

/// Barvy (pozadi, bezny text, tlumeny, cervena, zluta, azurova).
pub const PALETTE: [(u8, u8, u8); 6] =
    [(0x10, 0x18, 0x20), (0xE0, 0xE6, 0xEA), (0x80, 0x8C, 0x96), (0xF0, 0x6A, 0x6A), (0xE8, 0xC0, 0x5A), (0x6A, 0xC8, 0xE0)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    /// Index do [`PALETTE`] (1 = bezny text).
    pub fg: u8,
    pub bold: bool,
}

const BLANK: Cell = Cell { ch: ' ', fg: 1, bold: false };

pub struct Screen {
    pub cols: usize,
    pub rows: usize,
    cells: Vec<Cell>,
    pub x: usize,
    pub y: usize,
    fg: u8,
    bold: bool,
    /// Rozpracovana escape sekvence.
    esc: Option<alloc::string::String>,
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Self { cols, rows, cells: alloc::vec![BLANK; cols * rows], x: 0, y: 0, fg: 1, bold: false, esc: None }
    }

    pub fn cell(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.cols + x]
    }

    /// Radek jako text (bez koncovych mezer).
    pub fn line(&self, y: usize) -> alloc::string::String {
        let s: alloc::string::String = self.cells[y * self.cols..(y + 1) * self.cols].iter().map(|c| c.ch).collect();
        s.trim_end().into()
    }

    fn scroll(&mut self) {
        self.cells.drain(..self.cols);
        self.cells.extend(core::iter::repeat_n(BLANK, self.cols));
        self.y = self.rows - 1;
    }

    fn newline(&mut self) {
        self.x = 0;
        self.y += 1;
        if self.y >= self.rows {
            self.scroll();
        }
    }

    fn sgr(&mut self, params: &str) {
        for p in params.split(';') {
            match p {
                "" | "0" => {
                    self.fg = 1;
                    self.bold = false;
                }
                "1" => self.bold = true,
                "2" => self.fg = 2,
                "31" => self.fg = 3,
                "33" => self.fg = 4,
                "36" => self.fg = 5,
                "39" => self.fg = 1,
                _ => {}
            }
        }
    }

    pub fn write(&mut self, text: &str) {
        for c in text.chars() {
            if let Some(seq) = &mut self.esc {
                seq.push(c);
                let done = match seq.chars().next() {
                    Some('[') => seq.len() > 1 && ('@'..='~').contains(&c),
                    Some(']') => c == '\x07' || seq.ends_with("\x1b\\"),
                    _ => true,
                };
                if done {
                    let seq = self.esc.take().unwrap();
                    if let Some(p) = seq.strip_prefix('[').and_then(|s| s.strip_suffix('m')) {
                        self.sgr(p);
                    }
                }
                continue;
            }
            match c {
                '\x1b' => self.esc = Some(alloc::string::String::new()),
                '\n' => self.newline(),
                '\r' => self.x = 0,
                '\t' => {
                    let n = 4 - self.x % 4;
                    for _ in 0..n {
                        self.write(" ");
                    }
                }
                c if (c as u32) < 0x20 => {}
                c => {
                    if self.x >= self.cols {
                        self.newline();
                    }
                    self.cells[self.y * self.cols + self.x] = Cell { ch: c, fg: self.fg, bold: self.bold };
                    self.x += 1;
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(BLANK);
        self.x = 0;
        self.y = 0;
    }
}

/// Format pixelu framebufferu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb,
    Bgr,
}

/// Linearni framebuffer (32 bitu na pixel).
pub struct Surface<'a> {
    pub buf: &'a mut [u32],
    pub width: usize,
    pub height: usize,
    /// Pixelu na radek.
    pub stride: usize,
    pub format: PixelFormat,
}

impl Surface<'_> {
    fn encode(&self, (r, g, b): (u8, u8, u8)) -> u32 {
        let (r, g, b) = (r as u32, g as u32, b as u32);
        match self.format {
            PixelFormat::Rgb => r | (g << 8) | (b << 16),
            PixelFormat::Bgr => b | (g << 8) | (r << 16),
        }
    }

    fn put(&mut self, x: usize, y: usize, c: u32) {
        if x < self.width && y < self.height {
            self.buf[y * self.stride + x] = c;
        }
    }
}

/// Velikost pisma podle rozliseni (HiDPI od 1080 radku).
pub fn font_for(height: usize) -> RasterHeight {
    if height >= 1080 { RasterHeight::Size24 } else { RasterHeight::Size16 }
}

/// Velikost bunky (sirka, vyska) v pixelech.
pub fn cell_size(font: RasterHeight) -> (usize, usize) {
    (get_raster_width(FontWeight::Regular, font), font.val() + 2)
}

/// Kolik sloupcu a radku se vejde do framebufferu.
pub fn grid_for(width: usize, height: usize) -> (usize, usize) {
    let (cw, ch) = cell_size(font_for(height));
    ((width.saturating_sub(16)) / cw, (height.saturating_sub(16)) / ch)
}

fn blend(fg: (u8, u8, u8), bg: (u8, u8, u8), a: u8) -> (u8, u8, u8) {
    let m = |f: u8, b: u8| ((f as u32 * a as u32 + b as u32 * (255 - a as u32)) / 255) as u8;
    (m(fg.0, bg.0), m(fg.1, bg.1), m(fg.2, bg.2))
}

/// Vykresli obrazovku (s okrajem 8 px).
pub fn paint(screen: &Screen, s: &mut Surface) {
    let font = font_for(s.height);
    let (cw, ch) = cell_size(font);
    let bg = PALETTE[0];
    let bgc = s.encode(bg);
    for y in 0..s.height {
        for x in 0..s.width {
            s.put(x, y, bgc);
        }
    }
    for row in 0..screen.rows {
        for col in 0..screen.cols {
            let c = screen.cell(col, row);
            if c.ch == ' ' {
                continue;
            }
            let weight = FontWeight::Regular;
            let Some(r) = get_raster(c.ch, weight, font).or_else(|| get_raster('?', weight, font)) else { continue };
            let fg = PALETTE[c.fg as usize % PALETTE.len()];
            let (ox, oy) = (8 + col * cw, 8 + row * ch);
            for (dy, line) in r.rows().enumerate() {
                for (dx, &a) in line.iter().enumerate() {
                    if a == 0 {
                        continue;
                    }
                    // Tucne: o pixel vedle.
                    let px = s.encode(blend(fg, bg, a));
                    s.put(ox + dx, oy + dy, px);
                    if c.bold {
                        s.put(ox + dx + 1, oy + dy, px);
                    }
                }
            }
        }
    }
}
