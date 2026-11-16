//! Pilulka slozky ve vyzve okna GUI (`.cwd` z `design/terminal.html`):
//! rozmery a kde za ni zacina text radku vstupu.
//!
//! Vyzva radku vstupu je v mrizce znaku (`App::cwd_lead` bunek, kratka
//! slozka, oddelovac `" > "`), okno GUI ale slozku kresli jako pilulku
//! proporcionalnim pismem, ktere je uzsi nez stejny pocet bunek. Kdyby
//! text radku zacinal az na bunce za vyzvou, mezera za pilulkou by rostla
//! s delkou cesty. Proto okno radek vstupu posune (`indent`): bunka `end`
//! (prvni znak textu) lezi pevnou mezeru `text_gap` za pravym okrajem
//! nakreslene pilulky. Kurzor, klik mysi i vyber pocitaji x bunky stejne
//! (levy okraj textu radku + sloupec * sirka bunky), takze sedi se znaky.

/// Tokeny pilulky (px).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PillMetrics {
    /// Vnitrni okraj vlevo i vpravo (`s3`).
    pub pad: u32,
    /// Ikona slozky (`icon_s`).
    pub icon: u32,
    /// Mezera ikona -> jmeno (`s1`).
    pub icon_gap: u32,
    /// Mezera pilulka -> text radku (`s3`).
    pub text_gap: u32,
}

/// Rozmery pilulky a posun radku vstupu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pill {
    /// Sirka pilulky (px).
    pub width: u32,
    /// Odsazeni textu radku vstupu (px, obvykle zaporne): bunka `end`
    /// lezi na `indent + end * cw` = `width + text_gap`.
    pub indent: i32,
}

/// Pilulka pro jmeno sirky `name_w` (px, zmerene pismem, kterym se jmeno
/// kresli) a vyzvu, ktera v mrizce konci bunkou `end`, pri sirce bunky
/// `cw`. Sirka je nejvys `end` bunek bez `text_gap` (delsi jmeno se pri
/// kresleni zkrati na sirku pilulky), takze text radku se nikdy neposune
/// doprava za puvodni bunku `end`.
pub fn cwd_pill(name_w: u32, end: usize, cw: u32, m: &PillMetrics) -> Pill {
    let cells = end as u32 * cw;
    let max = cells.saturating_sub(m.text_gap);
    let width = (2 * m.pad + m.icon + m.icon_gap + name_w).min(max);
    Pill { width, indent: (width + m.text_gap) as i32 - cells as i32 }
}

/// Misto pro jmeno uvnitr pilulky sirky `width` (pro zkraceni "...").
pub fn name_room(width: u32, m: &PillMetrics) -> u32 {
    width.saturating_sub(2 * m.pad + m.icon + m.icon_gap)
}
