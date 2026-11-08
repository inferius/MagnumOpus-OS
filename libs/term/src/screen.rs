//! Textova obrazovka a kresleni vlastnim pismem do framebufferu.
//!
//! [`Screen`] je mrizka znaku s barvou (rozumi textu z [`crate::vt`]:
//! `\n`, `\r`, `\t` a SGR: zakladni barvy, 24bitove `38;2;r;g;b`, tucne,
//! podtrzene); [`paint`] ji vykresli pismem Noto Sans
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
    /// Barva textu (RGB).
    pub fg: (u8, u8, u8),
    pub bold: bool,
    pub underline: bool,
    /// Pozadi (`None` = pozadi obrazovky).
    pub bg: Option<(u8, u8, u8)>,
}

const BLANK: Cell = Cell { ch: ' ', fg: PALETTE[1], bold: false, underline: false, bg: None };

pub struct Screen {
    pub cols: usize,
    pub rows: usize,
    cells: Vec<Cell>,
    pub x: usize,
    pub y: usize,
    fg: (u8, u8, u8),
    /// Vychozi barva textu (po SGR 0/39): z motivu okna, jinak paleta
    /// konzole.
    ink: (u8, u8, u8),
    bold: bool,
    underline: bool,
    bg: Option<(u8, u8, u8)>,
    reverse: bool,
    /// Rozpracovana escape sekvence.
    esc: Option<alloc::string::String>,
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Self {
            cols,
            rows,
            cells: alloc::vec![BLANK; cols * rows],
            x: 0,
            y: 0,
            fg: PALETTE[1],
            ink: PALETTE[1],
            bold: false,
            underline: false,
            bg: None,
            reverse: false,
            esc: None,
        }
    }

    /// Vychozi barva textu podle motivu (svetle pozadi okna: paleta
    /// konzole by byla temer neviditelna).
    pub fn set_ink(&mut self, ink: (u8, u8, u8)) {
        self.ink = ink;
        self.fg = ink;
    }

    /// Bunka se znakem a aktualnim stylem (obracene = prohozene barvy).
    fn styled(&self, ch: char) -> Cell {
        let (fg, bg) = if self.reverse { (self.bg.unwrap_or(PALETTE[0]), Some(self.fg)) } else { (self.fg, self.bg) };
        Cell { ch, fg, bold: self.bold, underline: self.underline, bg }
    }

    pub fn cell(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.cols + x]
    }

    /// Bunky radku (porovnani snimku: co se zmenilo).
    pub fn row_cells(&self, y: usize) -> &[Cell] {
        &self.cells[y * self.cols..(y + 1) * self.cols]
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
        let ps: Vec<&str> = params.split(';').collect();
        let mut i = 0;
        while i < ps.len() {
            match ps[i] {
                "" | "0" => {
                    self.fg = self.ink;
                    self.bold = false;
                    self.underline = false;
                    self.bg = None;
                    self.reverse = false;
                }
                "7" => self.reverse = true,
                "27" => self.reverse = false,
                "49" => self.bg = None,
                // 24bitova barva pozadi.
                "48" if ps.get(i + 1) == Some(&"2") && i + 4 < ps.len() => {
                    let c = |k: usize| ps[i + k].parse::<u8>().unwrap_or(0);
                    self.bg = Some((c(2), c(3), c(4)));
                    i += 4;
                }
                "1" => self.bold = true,
                "2" => self.fg = PALETTE[2],
                "4" => self.underline = true,
                "22" => self.bold = false,
                "24" => self.underline = false,
                "31" => self.fg = PALETTE[3],
                "33" => self.fg = PALETTE[4],
                "36" => self.fg = PALETTE[5],
                "39" => self.fg = self.ink,
                // 24bitova barva textu.
                "38" if ps.get(i + 1) == Some(&"2") && i + 4 < ps.len() => {
                    let c = |k: usize| ps[i + k].parse::<u8>().unwrap_or(0);
                    self.fg = (c(2), c(3), c(4));
                    i += 4;
                }
                _ => {}
            }
            i += 1;
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
                    } else if seq == "[2J" {
                        self.clear();
                    } else if seq == "[H" {
                        self.x = 0;
                        self.y = 0;
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
                    self.cells[self.y * self.cols + self.x] = self.styled(c);
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
    let bgc = s.encode(PALETTE[0]);
    for y in 0..s.height {
        for x in 0..s.width {
            s.put(x, y, bgc);
        }
    }
    let rows: alloc::vec::Vec<usize> = (0..screen.rows).collect();
    paint_rows(screen, s, &rows, false);
}

/// Rozsah pixelovych radku textoveho radku `row` (pro kopii jen zmenenych
/// radku do framebufferu).
pub fn row_pixels(s_height: usize, row: usize) -> core::ops::Range<usize> {
    let (_, ch) = cell_size(font_for(s_height));
    let top = 8 + row * ch;
    top..top + ch
}

/// Vykresli jen radky `rows` (`clear` = nejdriv jejich pas vyplnit
/// pozadim). Konzole tak pri psani prekresli jen radek editoru.
pub fn paint_rows(screen: &Screen, s: &mut Surface, rows: &[usize], clear: bool) {
    let font = font_for(s.height);
    let (cw, ch) = cell_size(font);
    let bg = PALETTE[0];
    let bgc = s.encode(bg);
    for &row in rows {
        if row >= screen.rows {
            continue;
        }
        if clear {
            for y in row_pixels(s.height, row) {
                for x in 0..s.width {
                    s.put(x, y, bgc);
                }
            }
        }
        for col in 0..screen.cols {
            let c = screen.cell(col, row);
            let (ox, oy) = (8 + col * cw, 8 + row * ch);
            let cbg = c.bg.unwrap_or(bg);
            if let Some(b) = c.bg {
                let px = s.encode(b);
                for dy in 0..ch {
                    for dx in 0..cw {
                        s.put(ox + dx, oy + dy, px);
                    }
                }
            }
            if c.ch == ' ' {
                continue;
            }
            // Krouzek cinnosti a dilci bunka pruhu (pismo je nema).
            if let Some(d) = crate::activity::drawn(c.ch) {
                for dy in 0..ch {
                    for dx in 0..cw {
                        let a = crate::activity::coverage(d, cw as u32, ch as u32, dx as u32, dy as u32);
                        if a > 0 {
                            s.put(ox + dx, oy + dy, s.encode(blend(c.fg, cbg, a)));
                        }
                    }
                }
                continue;
            }
            let weight = FontWeight::Regular;
            let Some(r) = get_raster(c.ch, weight, font).or_else(|| get_raster('?', weight, font)) else { continue };
            let fg = c.fg;
            for (dy, line) in r.rows().enumerate() {
                // Tucne: glyf a jeho kopie o pixel vpravo, pruhlednost
                // slucena maximem (bez rozmazani slabych okraju).
                let n = line.len() + c.bold as usize;
                for dx in 0..n {
                    let here = line.get(dx).copied().unwrap_or(0);
                    let left = if c.bold && dx > 0 { line[dx - 1] } else { 0 };
                    let a = here.max(left);
                    if a == 0 {
                        continue;
                    }
                    s.put(ox + dx, oy + dy, s.encode(blend(fg, cbg, a)));
                }
            }
            if c.underline {
                let px = s.encode(fg);
                for dx in 0..cw {
                    s.put(ox + dx, oy + ch - 2, px);
                }
            }
        }
    }
}
