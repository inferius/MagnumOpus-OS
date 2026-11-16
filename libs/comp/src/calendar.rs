//! Panel hodin (O5, design/bubliny-panely.html "Hodiny"): kalendar mesice
//! s dneskem, listovani mesici a svetovy cas. Datum a casy jinych pasem
//! dava sluzba (`compd`, pasma z `libs/tz`); kalendar (dny v mesici, den
//! v tydnu) se pocita tady.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::{Node, PanelSize, button, column, label, panel, panel_footer, panel_header, panel_line, panel_section, row};

use crate::Comp;
use crate::barpanel::PanelMsg;

/// Dnesni datum a svetovy cas od sluzby.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CalendarInfo {
    pub year: i32,
    /// 1-12.
    pub month: u8,
    /// 1-31.
    pub day: u8,
    /// Mesta: jmeno, mistni cas ("13:05"), rozdil proti mistnimu ("-1 h").
    pub world: Vec<(String, String, String)>,
}

const MONTHS: [&str; 12] = ["leden", "únor", "březen", "duben", "květen", "červen", "červenec", "srpen", "září", "říjen", "listopad", "prosinec"];
const WEEKDAYS: [&str; 7] = ["Po", "Út", "St", "Čt", "Pá", "So", "Ne"];

fn leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Pocet dni v mesici `m` (1-12) roku `y`.
pub fn days_in_month(y: i32, m: u8) -> u8 {
    match m {
        2 if leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Den v tydnu 1. dne mesice (0 = pondeli), Zellerova shoda.
pub fn first_weekday(y: i32, m: u8) -> u8 {
    let (y, m) = if m < 3 { (y - 1, m as i32 + 12) } else { (y, m as i32) };
    let k = y.rem_euclid(100);
    let j = y.div_euclid(100);
    // h: 0 = sobota.
    let h = (1 + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7);
    ((h + 5) % 7) as u8
}

/// Mesic posunuty o `delta` (rok, mesic).
pub fn shift_month(y: i32, m: u8, delta: i32) -> (i32, u8) {
    let i = y * 12 + (m as i32 - 1) + delta;
    (i.div_euclid(12), (i.rem_euclid(12) + 1) as u8)
}

impl Comp {
    /// Datum a svetovy cas pro panel hodin (sluzba kazdou minutu).
    pub fn set_calendar(&mut self, info: Option<CalendarInfo>) {
        if self.calendar != info {
            self.calendar = info;
            self.bar_panel_changed();
        }
    }

    /// Pohled panelu hodin.
    pub(crate) fn clock_view(&self, s: &PanelSize) -> Node<PanelMsg> {
        let t = &self.style.theme;
        let Some(c) = &self.calendar else {
            return panel(alloc::vec![panel_header(s, "calendar", "Kalendář", ""), panel_line(s, alloc::vec![label("Čas zatím neznám (hodiny se nastavují).")])]);
        };
        let (y, m) = shift_month(c.year, c.month, self.calendar_offset);
        let title = format!("{} {y}", MONTHS[(m - 1) as usize]);
        let nav = row([
            button("").icon("left").ghost().tooltip("Předchozí měsíc").on_click(PanelMsg::Month(-1)),
            button("").icon("right").ghost().tooltip("Další měsíc").on_click(PanelMsg::Month(1)),
        ])
        .gap(0);
        let header = row([ui::icon("calendar"), label(&title).tone(ui::TextTone::Strong).grow(), nav]).gap(s.gap).pad(s.s3, s.side, s.s2, s.side);
        // Mrizka 7 x 6: dny predchoziho mesice, mesic, dalsiho mesice.
        let cell = (s.width - 2 * s.side) / 7;
        // Bunka: text na stredu (`.cal span`, place-items: center).
        let mid = |n: Node<PanelMsg>| row([ui::spacer(), n, ui::spacer()]).gap(0).width(cell).height(t.size.control_h_compact);
        let head: Vec<Node<PanelMsg>> = WEEKDAYS.iter().map(|d| mid(label(d).tone(ui::TextTone::Small))).collect();
        let first = first_weekday(y, m) as i32;
        let dim = days_in_month(y, m) as i32;
        let (py, pm) = shift_month(y, m, -1);
        let prev = days_in_month(py, pm) as i32;
        let today = (y, m) == (c.year, c.month);
        let mut weeks: Vec<Node<PanelMsg>> = alloc::vec![row(head).gap(0)];
        for w in 0..6 {
            let mut days: Vec<Node<PanelMsg>> = Vec::new();
            for d in 0..7 {
                let k = w * 7 + d - first + 1;
                let (text, other) = if k < 1 { (prev + k, true) } else if k > dim { (k - dim, true) } else { (k, false) };
                let mut n = mid(label(&format!("{text}")).tone(if other { ui::TextTone::Faint } else { ui::TextTone::Normal }));
                if !other && today && text == c.day as i32 {
                    // Dnes: kruh `accent`, text `on_accent`, tucne.
                    n = row([ui::spacer(), label(&format!("{text}")).tone(ui::TextTone::OnAccent), ui::spacer()])
                        .gap(0)
                        .width(cell)
                        .height(t.size.control_h_compact)
                        .deco(ui::Deco { fill: Some(ui::Fill::Accent), radius: t.size.control_h_compact / 2, top_border: false });
                }
                days.push(n);
            }
            weeks.push(row(days).gap(0));
        }
        let mut items = alloc::vec![header, column(weeks).gap(0).pad(0, s.side, s.s2, s.side)];
        if !c.world.is_empty() {
            items.push(panel_section(s, "Světový čas"));
            for (city, time, diff) in &c.world {
                items.push(panel_line(
                    s,
                    alloc::vec![ui::icon("globe"), column([label(city), label(diff).tone(ui::TextTone::Small)]).gap(0).grow(), label(time).tone(ui::TextTone::Strong)],
                ));
            }
        }
        items.push(panel_section(s, "Události"));
        items.push(panel_line(s, alloc::vec![label("Kalendář událostí připravujeme.").tone(ui::TextTone::Small)]));
        items.push(panel_footer(s, alloc::vec![button("Nastavení času").icon("time-sync").disabled(true).tooltip("Nastavení zatím nejsou").grow()]));
        panel(items)
    }
}
