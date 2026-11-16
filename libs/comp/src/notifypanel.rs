//! Centrum oznameni pod zvonkem v liste (design/bubliny-panely.html
//! "Oznameni"): prepinac Nerusit, oznameni uzivatele relace (nejnovejsi
//! nahore) s akcemi a zavrenim, "Vymazat vse". Data a akce jdou pres
//! `notifyd` (`toast.rs`: `notifications`, `notify_*`); otevreni centra
//! oznaci vse jako videne (`notify_mark_seen`), odznak zvonku pak zmizi.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use notifyproto::{Notification, Priority};
use ui::{Node, PanelSize, button, column, label, panel, panel_footer, panel_header, panel_line, panel_section, row, switch};

use crate::Comp;
use crate::barpanel::PanelMsg;

/// Kolik oznameni centrum ukaze (starsi zustanou v notifyd).
const MAX_SHOWN: usize = 8;

/// Duvod vypnuteho tlacitka Nastaveni (aplikace zatim neni).
const NO_SETTINGS: &str = "Nastavení zatím nejsou";

impl Comp {
    /// Cislo uctu aktivni (odemcene) relace.
    pub(crate) fn active_uid(&self) -> Option<u64> {
        self.greeter.sessions.iter().find(|s| !s.locked).map(|s| s.uid)
    }

    /// Nerusit uzivatele relace (od notifyd; 0 = vypnuto).
    pub(crate) fn user_dnd(&self) -> bool {
        self.active_uid().is_some_and(|u| self.notify_dnd(u) != 0)
    }

    /// Pocet nevidenych oznameni pro odznak zvonku.
    pub(crate) fn bell_unseen(&self) -> usize {
        self.active_uid().map_or(0, |u| self.notify_unseen(u))
    }

    /// Pohled centra oznameni.
    pub(crate) fn bell_view(&self, s: &PanelSize) -> Node<PanelMsg> {
        let uid = self.active_uid();
        let mut list: Vec<&Notification> = self.notifications().iter().filter(|(u, _)| Some(*u) == uid).map(|(_, n)| n).collect();
        list.sort_by_key(|n| core::cmp::Reverse(n.time));
        let fullscreen = self.do_not_disturb();
        let dnd = self.user_dnd();
        let unseen = list.iter().filter(|n| !n.seen).count();
        let sub = if dnd || fullscreen {
            String::from("Nerušit")
        } else if unseen > 0 {
            format!("{unseen} nová")
        } else if list.is_empty() {
            String::from("žádná")
        } else {
            String::new()
        };
        let mut items = alloc::vec![panel_header(s, if dnd || fullscreen { "bell-off" } else { "bell" }, "Oznámení", &sub)];
        let desc = match (dnd, fullscreen) {
            (true, _) => "zapnuto; urgentní oznámení se ukážou",
            (false, true) => "při celé obrazovce; oznámení se ukážou po návratu",
            _ => "vypnuto",
        };
        let sw = switch("Nerušit", dnd).on_toggle(PanelMsg::Dnd).panel_row(Some("bell-off"), desc).key("nerusit");
        items.push(if uid.is_some() { sw } else { sw.disabled(true) });
        if list.is_empty() {
            items.push(panel_line(s, alloc::vec![label("Žádná oznámení.").tone(ui::TextTone::Small)]));
        } else {
            items.push(panel_section(s, "Oznámení"));
            // Sirka textu: panel bez okraju, ikony a tlacitka zavrit (jinak by
            // dlouhy text radek roztahl a krizek vyjel z panelu).
            let sz = &self.style.theme.size;
            let tw = s.width.saturating_sub(2 * s.side + sz.icon_m + sz.control_h + 2 * s.gap);
            for n in list.iter().take(MAX_SHOWN) {
                items.push(notice_line(s, n, tw));
            }
            if list.len() > MAX_SHOWN {
                items.push(panel_line(s, alloc::vec![label(&format!("a další ({})", list.len() - MAX_SHOWN)).tone(ui::TextTone::Small)]));
            }
        }
        items.push(panel_footer(
            s,
            alloc::vec![
                button("Vymazat vše").ghost().on_click(PanelMsg::NotifyClearAll).disabled(list.is_empty()),
                button("Nastavení oznámení").icon("settings").disabled(true).tooltip(NO_SETTINGS).grow(),
            ],
        ));
        panel(items)
    }
}

/// Radek oznameni: ikona podle priority, tucny titulek, text, kdo poslal;
/// akce odesilatele (nejvys tri) a zavreni.
fn notice_line(s: &PanelSize, n: &Notification, tw: u32) -> Node<PanelMsg> {
    let icon = match n.priority {
        Priority::Urgent | Priority::High => "alert",
        _ => "bell",
    };
    let mut text = alloc::vec![label(&n.title).tone(if n.seen { ui::TextTone::Normal } else { ui::TextTone::Strong })];
    if !n.body.is_empty() {
        text.push(label(&n.body).tone(ui::TextTone::Small));
    }
    text.push(label(&n.app_name).tone(ui::TextTone::Faint));
    if !n.actions.is_empty() {
        let acts: Vec<Node<PanelMsg>> = n.actions.iter().take(3).map(|a| button(&a.label).on_click(PanelMsg::NotifyInvoke(n.id, a.id.clone()))).collect();
        text.push(row(acts).gap(s.s2));
    }
    let close = button("").icon("close").ghost().tooltip(&format!("Zavřít: {}", n.title)).on_click(PanelMsg::NotifyDismiss(n.id));
    panel_line(s, alloc::vec![ui::icon(icon), column(text).gap(s.s1).width(tw), close])
}
