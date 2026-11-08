//! Barevny motiv terminalu: semanticke tridy -> barvy.
//!
//! Protokol nese jen tridy (zvyrazneni syntaxe [`shlang::highlight::Class`],
//! druh souboru [`FileClass`], skala velikosti a stari). Barvu prideli az
//! motiv v okne. Vestavene jsou tmavy a svetly; uzivatel je prebije
//! souborem `~/.config/term/theme.toml` (podmnozina TOML):
//!
//! ```toml
//! base = "dark"            # nebo "light"
//! [syntax]
//! cmd = "#7ee2a8"
//! unknown = "#ff8a80 underline"
//! [files]
//! dir = "#7fb4ff bold"
//! [ui]
//! bg = "#15171c"
//! [ext]
//! image = "jpg png heic"   # pripony k druhu souboru
//! ```
//!
//! Stejne barvy generuji tridy CSS dokumentace ([`Theme::css`]).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use shlang::highlight::Class;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(s: &str) -> Option<Rgb> {
        let h = s.strip_prefix('#')?;
        if h.len() != 6 || !h.is_ascii() {
            return None;
        }
        let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        Some(Rgb(b(0)?, b(2)?, b(4)?))
    }

    pub fn hex(self) -> String {
        alloc::format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub fg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Style {
    pub const fn fg(fg: Rgb) -> Self {
        Self { fg, bold: false, italic: false, underline: false }
    }

    /// `"#rrggbb [bold] [italic] [underline]"`.
    pub fn parse(s: &str) -> Option<Style> {
        let mut it = s.split_whitespace();
        let mut st = Style::fg(Rgb::parse(it.next()?)?);
        for w in it {
            match w {
                "bold" => st.bold = true,
                "italic" => st.italic = true,
                "underline" => st.underline = true,
                _ => return None,
            }
        }
        Some(st)
    }
}

/// Druh polozky vypisu souboru (barva jmena).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClass {
    Plain,
    Dir,
    Dataset,
    Symlink,
    Exec,
    Image,
    Archive,
    Document,
    Media,
    Hidden,
    /// Bez prava cist.
    NoAccess,
}

impl FileClass {
    pub const ALL: &'static [FileClass] = &[
        FileClass::Plain,
        FileClass::Dir,
        FileClass::Dataset,
        FileClass::Symlink,
        FileClass::Exec,
        FileClass::Image,
        FileClass::Archive,
        FileClass::Document,
        FileClass::Media,
        FileClass::Hidden,
        FileClass::NoAccess,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FileClass::Plain => "file",
            FileClass::Dir => "dir",
            FileClass::Dataset => "dataset",
            FileClass::Symlink => "symlink",
            FileClass::Exec => "exec",
            FileClass::Image => "image",
            FileClass::Archive => "archive",
            FileClass::Document => "document",
            FileClass::Media => "media",
            FileClass::Hidden => "hidden",
            FileClass::NoAccess => "noaccess",
        }
    }

    fn index(self) -> usize {
        FileClass::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

/// Barvy prvku terminalu mimo syntaxi a soubory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ui {
    pub bg: Rgb,
    pub fg: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub ok: Rgb,
    pub warn: Rgb,
    pub err: Rgb,
    /// Vybrany radek, pozadi nabidky.
    pub select: Rgb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    pub ui: Ui,
    pub(crate) syntax: Vec<Style>,
    pub(crate) files: Vec<Style>,
    /// Skala 0 (male/nove) .. 3 (obri/stare).
    pub scale: [Rgb; 4],
    /// Pripony k druhum souboru: (druh, pripona malymi pismeny).
    pub ext: Vec<(FileClass, String)>,
}

const DEFAULT_EXT: &[(FileClass, &str)] = &[
    (FileClass::Image, "jpg jpeg png gif webp bmp svg heic tif tiff ico"),
    (FileClass::Archive, "zip tar gz tgz zst xz bz2 7z rar img iso"),
    (FileClass::Document, "pdf md txt doc docx odt rtf html csv json toml"),
    (FileClass::Media, "mp3 flac ogg wav opus mp4 mkv webm mov avi"),
    (FileClass::Exec, "sh mos elf exe"),
];

fn ext_list() -> Vec<(FileClass, String)> {
    let mut out = Vec::new();
    for (c, list) in DEFAULT_EXT {
        for e in list.split_whitespace() {
            out.push((*c, e.to_string()));
        }
    }
    out
}

fn st(hex: &str) -> Style {
    Style::parse(hex).unwrap_or(Style::fg(Rgb(0x80, 0x80, 0x80)))
}

impl Theme {
    /// Tmavy motiv (vychozi).
    pub fn dark() -> Theme {
        let syntax = [
            "#d8dae0", "#7ee2a8", "#ff8a80 underline", "#ff9ac1", "#e0b46c", "#c7a6ff", "#a5d6ff", "#ffcf70", "#79d0c8",
            "#8f97a6 italic", "#9aa3b5", "#9aa3b5 bold", "#e6c1ff", "#9ccbff",
        ];
        let files = [
            "#d8dae0", "#7fb4ff bold", "#5fd7d7 bold", "#79d0c8 italic", "#7ee2a8 bold", "#f59ac9", "#f0b45a", "#d8dae0",
            "#c7a6ff", "#7c8291", "#ff8a80",
        ];
        Theme {
            name: "dark".into(),
            ui: Ui {
                bg: Rgb(0x15, 0x17, 0x1c),
                fg: Rgb(0xd8, 0xda, 0xe0),
                muted: Rgb(0x9a, 0xa1, 0xb0),
                accent: Rgb(0x7f, 0xb4, 0xff),
                ok: Rgb(0x7e, 0xe2, 0xa8),
                warn: Rgb(0xf0, 0xb4, 0x5a),
                err: Rgb(0xff, 0x8a, 0x80),
                select: Rgb(0x2a, 0x31, 0x40),
            },
            syntax: syntax.iter().map(|s| st(s)).collect(),
            files: files.iter().map(|s| st(s)).collect(),
            scale: [Rgb(0x7c, 0x82, 0x91), Rgb(0x79, 0xd0, 0xc8), Rgb(0xf0, 0xb4, 0x5a), Rgb(0xff, 0x8a, 0x80)],
            ext: ext_list(),
        }
    }

    /// Svetly motiv.
    pub fn light() -> Theme {
        let syntax = [
            "#24262b", "#1f7a4a", "#c0392b underline", "#b5246b", "#9a5b00", "#6b3fc9", "#1f5fa8", "#a8590a", "#0f6e66",
            "#646a76 italic", "#5f6672", "#5f6672 bold", "#7a3fa8", "#1f5fa8",
        ];
        let files = [
            "#24262b", "#1f5fa8 bold", "#0f6e66 bold", "#0f6e66 italic", "#1f7a4a bold", "#a8326e", "#8a5200", "#24262b",
            "#6b3fc9", "#646a76", "#c0392b",
        ];
        Theme {
            name: "light".into(),
            ui: Ui {
                bg: Rgb(0xfb, 0xfb, 0xf9),
                fg: Rgb(0x24, 0x26, 0x2b),
                muted: Rgb(0x58, 0x5d, 0x6b),
                accent: Rgb(0x1f, 0x5f, 0xa8),
                ok: Rgb(0x1a, 0x6b, 0x40),
                warn: Rgb(0x9a, 0x5b, 0x00),
                err: Rgb(0xc0, 0x39, 0x2b),
                select: Rgb(0xe3, 0xec, 0xf9),
            },
            syntax: syntax.iter().map(|s| st(s)).collect(),
            files: files.iter().map(|s| st(s)).collect(),
            scale: [Rgb(0x64, 0x6a, 0x76), Rgb(0x0f, 0x6e, 0x66), Rgb(0x9a, 0x5b, 0x00), Rgb(0xc0, 0x39, 0x2b)],
            ext: ext_list(),
        }
    }

    pub fn syntax(&self, c: Class) -> Style {
        self.syntax[c as usize]
    }

    pub fn file(&self, c: FileClass) -> Style {
        self.files[c.index()]
    }

    /// Druh souboru podle pripony (malymi pismeny); `None` = bezny soubor.
    pub fn ext_class(&self, ext: &str) -> Option<FileClass> {
        let e = ext.to_lowercase();
        self.ext.iter().find(|(_, x)| *x == e).map(|(c, _)| *c)
    }

    /// Motiv ze souboru (prebije `base` nebo vychozi tmavy). Chyba =
    /// (cislo radku od 1, popis).
    pub fn parse_toml(src: &str) -> Result<Theme, (usize, String)> {
        let mut t = Theme::dark();
        let mut section = String::new();
        for (n, raw) in src.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            let err = |m: &str| Err((n + 1, String::from(m)));
            if let Some(s) = line.strip_prefix('[') {
                let Some(name) = s.strip_suffix(']') else { return err("chybi ']'") };
                section = name.trim().into();
                if !matches!(section.as_str(), "syntax" | "files" | "ui" | "ext") {
                    return err("neznama sekce (syntax, files, ui, ext)");
                }
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { return err("cekam klic = \"hodnota\"") };
            let (k, v) = (k.trim(), v.trim());
            let Some(v) = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
                return err("hodnota patri do uvozovek");
            };
            match section.as_str() {
                "" if k == "base" => {
                    let ext = core::mem::take(&mut t.ext);
                    t = match v {
                        "dark" => Theme::dark(),
                        "light" => Theme::light(),
                        _ => return err("base je dark nebo light"),
                    };
                    t.ext = ext;
                }
                "" if k == "name" => t.name = v.into(),
                "syntax" => {
                    let Some(c) = Class::ALL.iter().find(|c| c.name() == k) else { return err("neznama trida syntaxe") };
                    let Some(s) = Style::parse(v) else { return err("styl je \"#rrggbb [bold] [italic] [underline]\"") };
                    t.syntax[*c as usize] = s;
                }
                "files" => {
                    let Some(c) = FileClass::ALL.iter().find(|c| c.name() == k) else { return err("neznamy druh souboru") };
                    let Some(s) = Style::parse(v) else { return err("styl je \"#rrggbb [bold] [italic] [underline]\"") };
                    t.files[c.index()] = s;
                }
                "ui" => {
                    let Some(rgb) = Rgb::parse(v) else { return err("barva je \"#rrggbb\"") };
                    let u = &mut t.ui;
                    match k {
                        "bg" => u.bg = rgb,
                        "fg" => u.fg = rgb,
                        "muted" => u.muted = rgb,
                        "accent" => u.accent = rgb,
                        "ok" => u.ok = rgb,
                        "warn" => u.warn = rgb,
                        "err" => u.err = rgb,
                        "select" => u.select = rgb,
                        k if k.starts_with("scale") => {
                            let Some(i) = k[5..].parse::<usize>().ok().filter(|i| *i < 4) else {
                                return err("scale0 az scale3");
                            };
                            t.scale[i] = rgb;
                        }
                        _ => return err("neznama barva ui"),
                    }
                }
                "ext" => {
                    let Some(c) = FileClass::ALL.iter().find(|c| c.name() == k) else { return err("neznamy druh souboru") };
                    for e in v.split_whitespace() {
                        let e = e.trim_start_matches('.').to_lowercase();
                        t.ext.retain(|(_, x)| *x != e);
                        t.ext.push((*c, e));
                    }
                }
                _ => return err("neznamy klic"),
            }
        }
        Ok(t)
    }

    /// Tridy CSS pro dokumentaci (`.sh-<trida>`, `.fk-<druh>`) uvnitr
    /// selektoru `scope` (napr. `:root` nebo `[data-theme="dark"]`).
    pub fn css_vars(&self, scope: &str) -> String {
        let mut out = alloc::format!("{scope} {{\n");
        for c in Class::ALL {
            out.push_str(&alloc::format!("  --sh-{}: {};\n", c.name(), self.syntax(*c).fg.hex()));
        }
        for c in FileClass::ALL {
            out.push_str(&alloc::format!("  --fk-{}: {};\n", c.name(), self.file(*c).fg.hex()));
        }
        out.push_str("}\n");
        out
    }

    /// Pravidla trid (bez barev - ty jsou v promennych z [`Theme::css_vars`]).
    pub fn css_rules(&self) -> String {
        let mut out = String::new();
        let deco = |s: Style| {
            let mut d = String::new();
            if s.bold {
                d.push_str(" font-weight: 600;");
            }
            if s.italic {
                d.push_str(" font-style: italic;");
            }
            if s.underline {
                d.push_str(" text-decoration: underline wavy; text-underline-offset: 3px;");
            }
            d
        };
        for c in Class::ALL {
            out.push_str(&alloc::format!(".sh-{0} {{ color: var(--sh-{0});{1} }}\n", c.name(), deco(self.syntax(*c))));
        }
        for c in FileClass::ALL {
            out.push_str(&alloc::format!(".fk-{0} {{ color: var(--fk-{0});{1} }}\n", c.name(), deco(self.file(*c))));
        }
        out
    }
}

fn strip_comment(line: &str) -> &str {
    // `#` uvnitr uvozovek (barva "#rrggbb") komentar neni.
    let mut q = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => q = !q,
            '#' if !q => return &line[..i],
            _ => {}
        }
    }
    line
}
