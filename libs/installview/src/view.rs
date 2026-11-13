//! Rozhrani pruvodce: kostra okna (kroky, obsah, tlacitka) a obsah
//! jednotlivych kroku. Texty kroku a pravidla jsou z `installplan`.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use installplan::map::Seg;
use installplan::{Line, NO_ENCRYPTION, PROGRESS, STEPS};
use ui::{Cells, Context, Node, SizeClass, button, checkbox, col, column, label, list, password_input, progress, row, scroll, spacer, table, text_input};

use crate::{Installer, Msg, Outcome, step};

pub(crate) fn view(s: &Installer, cx: &Context) -> Node<Msg> {
    let mut pane = vec![scroll(body(s)).grow().key("obsah")];
    if !s.error.is_empty() {
        pane.push(label(&s.error).warn().key("chyba"));
    }
    if s.step != step::PROGRESS {
        pane.push(footer(s));
    }
    let pane = column(pane).grow();
    if cx.class == SizeClass::Compact {
        // Uzke okno: misto seznamu kroku jen "Krok N z 8".
        column([label(&format!("Krok {} z {}: {}", s.step + 1, STEPS.len(), STEPS[s.step])).heading().key("krok"), pane])
    } else {
        // Radek deti svisle stredi podle prirozene vysky: seznam kroku
        // i obsah dostanou vysku okna (rozvrzeni ji orizne na plochu).
        row([steps(s).width(220).height(cx.h), pane.height(cx.h)])
    }
}

/// Seznam kroku vlevo: hotove, aktualni (vybrany), dalsi.
fn steps(s: &Installer) -> Node<Msg> {
    let items = STEPS.iter().enumerate().map(|(i, t)| if i < s.step { format!("{}. {t} - hotovo", i + 1) } else { format!("{}. {t}", i + 1) });
    list(items, Some(s.step)).key("kroky")
}

fn footer(s: &Installer) -> Node<Msg> {
    let next = match s.step {
        step::SUMMARY => "Nainstalovat",
        _ => "Pokračovat",
    };
    row([
        button("Zpět").on_click(Msg::Back).disabled(!s.can_back()).key("zpet"),
        spacer(),
        button(next).on_click(Msg::Next).disabled(!s.can_next()).key("dalsi"),
    ])
    .key("tlacitka")
}

fn body(s: &Installer) -> Node<Msg> {
    // Po instalaci misto nadpisu kroku vysledek.
    let heading = match (&s.outcome, s.step) {
        (Some(Outcome::Done(_)), step::PROGRESS) => "Nainstalováno",
        (Some(Outcome::Failed(_)), step::PROGRESS) => "Instalace selhala",
        _ => STEPS[s.step],
    };
    let title = label(heading).heading().key("nadpis");
    let mut c = vec![title];
    match s.step {
        step::KEYBOARD => keyboard(s, &mut c),
        step::CHECK => check(s, &mut c),
        step::TARGET => target(s, &mut c),
        step::PREVIEW => preview(s, &mut c),
        step::ACCOUNT => account(s, &mut c),
        step::SECURITY => {
            c.push(checkbox("Šifrovat disk", false).disabled(true).key("sifrovani"));
            c.push(label(NO_ENCRYPTION));
        }
        step::SUMMARY => summary(s, &mut c),
        _ => run(s, &mut c),
    }
    column(c)
}

fn keyboard(s: &Installer, c: &mut Vec<Node<Msg>>) {
    c.push(label("Jazyk systému: čeština (další jazyky se připravují)."));
    if s.keyboards.is_empty() {
        c.push(label("Načítám rozložení klávesnice..."));
        return;
    }
    c.push(label("Rozložení klávesnice (přepínání levý Alt + levý Shift)"));
    let kbs: Vec<(String, String)> = s
        .keyboards
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let role = match s.chosen.iter().position(|&c| c == i) {
                Some(0) => "hlavní",
                Some(_) => "přidané",
                None => "",
            };
            (format!("{} ({})", k.description, k.name), String::from(role))
        })
        .collect();
    let n = kbs.len();
    c.push(
        table(vec![col("Rozložení", 0), col("Volba", 110)], n, |i| i as u64, move |i| Cells::new([kbs[i].0.clone(), kbs[i].1.clone()]).icon("keyboard"))
            .header(false)
            .selection(s.kb_cursor.map(|i| vec![i as u64]).unwrap_or_default())
            .on_selection(Msg::KbCursor)
            .on_activate(|_| Msg::KbAdd)
            .height(200)
            .key("rozlozeni"),
    );
    let cur = s.kb_cursor;
    let chosen = cur.is_some_and(|i| s.chosen.contains(&i));
    c.push(
        row([
            button("Přidat").on_click(Msg::KbAdd).disabled(cur.is_none() || chosen).key("pridat"),
            button("Odebrat").on_click(Msg::KbRemove).disabled(!chosen || s.chosen.len() < 2).key("odebrat"),
            button("Nastavit jako hlavní").on_click(Msg::KbMain).disabled(cur.is_none() || s.chosen.first() == cur.as_ref()).key("hlavni"),
        ])
        .key("akce-rozlozeni"),
    );
    // Nahled klavesnice: rady klaves vybraneho rozlozeni.
    if let Some(k) = cur.and_then(|i| s.keyboards.get(i)) {
        c.push(label(&format!("Náhled: {}", k.description)).key("nahled-titulek"));
        for (i, r) in k.rows.iter().enumerate() {
            c.push(label(r).key(format!("rada{i}").as_str()));
        }
    }
    let list: Vec<String> = s.chosen.iter().map(|&i| s.keyboards[i].description.clone()).collect();
    c.push(label(&format!("Zvolené: {}", list.join(", "))).key("zvolene"));
    c.push(label("Časové pásmo"));
    c.push(text_input(&s.zone).placeholder("Europe/Prague").on_change(Msg::Zone).key("pasmo"));
}

fn check(s: &Installer, c: &mut Vec<Node<Msg>>) {
    c.push(label("Co z tohoto počítače systém umí používat. Zkoušíš ho právě teď, takže výsledek je ověřený, ne odhadnutý."));
    let Some(disks) = &s.disks else {
        c.push(label("Zjišťuji disky..."));
        return;
    };
    for d in disks {
        let live = if d.live { " - živé USB, odtud systém běží" } else { "" };
        c.push(label(&format!("Disk {}: {} ({}){live}", d.index, d.model, installplan::human(d.bytes))).key(d.index as u64 * 2));
        c.push(label(&format!("    {}", d.parts.join(", "))).key(d.index as u64 * 2 + 1));
    }
    if s.targets().is_empty() {
        c.push(label("Žádný disk pro instalaci (kromě živého USB).").warn());
    }
    c.push(label("Secure Boot ve firmwaru musí být vypnutý, jinak počítač nainstalovaný systém nespustí.").warn().key("secure-boot"));
}

fn target(s: &Installer, c: &mut Vec<Node<Msg>>) {
    let Some(disks) = &s.disks else { return };
    let t = s.targets();
    let rows: Vec<(String, String)> = t.iter().map(|&i| (disks[i].model.clone(), installplan::human(disks[i].bytes))).collect();
    let ids: Vec<u64> = t.iter().map(|&i| i as u64).collect();
    let ids2 = ids.clone();
    c.push(label("Disk"));
    c.push(
        table(vec![col("Disk", 0), col("Velikost", 100).numeric()], rows.len(), move |i| ids[i], move |i| Cells::new([rows[i].0.clone(), rows[i].1.clone()]).icon("disk"))
            .header(false)
            .selection(s.disk.map(|d| vec![d as u64]).filter(|v| ids2.contains(&v[0])).unwrap_or_default())
            .on_selection(Msg::Disk)
            .height(120)
            .key("disky"),
    );
    let Some(d) = s.disk_entry() else { return };
    let avail = s.available();
    c.push(label("Způsob instalace"));
    if avail.is_empty() {
        c.push(label("Na tento disk teď instalovat nejde (důvody níže).").warn());
    } else {
        let sel = avail.iter().position(|ch| Some(ch.mode) == s.mode);
        c.push(list(avail.iter().map(|ch| ch.title.clone()), sel).on_select(Msg::Mode).key("zpusob"));
        if let Some(ch) = sel.map(|i| avail[i]) {
            c.push(label(&ch.detail).key("popis-zpusobu"));
        }
    }
    for (i, ch) in d.choices.iter().filter(|c| !c.available).enumerate() {
        c.push(label(&format!("Nejde: {} - {}", ch.title, ch.detail)).key(1000 + i as u64));
    }
}

/// Graf oddilu: dokud knihovna UI nema pruhovy graf, useky jako text
/// v poradi na disku.
fn disk_map(title: &str, segs: &[Seg], key: &str) -> Node<Msg> {
    let parts: Vec<String> = segs.iter().map(|g| if g.new { format!("{} (nový)", g.label) } else { g.label.clone() }).collect();
    column([label(title), label(&parts.join("  |  "))]).key(key)
}

fn preview(s: &Installer, c: &mut Vec<Node<Msg>>) {
    c.push(label("Zatím se nic nezměnilo. Disk se zapíše až po kliknutí na Nainstalovat v kroku Souhrn."));
    let Some(p) = &s.preview else {
        if s.error.is_empty() {
            c.push(label("Připravuji náhled..."));
        }
        return;
    };
    c.push(disk_map("Teď", &p.before, "mapa-ted"));
    c.push(disk_map("Po instalaci", &p.after, "mapa-po"));
    for (i, l) in p.lines.iter().enumerate() {
        let (kind, warn) = match l {
            Line::Keep(_) => ("zůstane", false),
            Line::New(_) => ("nový", false),
            Line::Erase(_) => ("smaže se", true),
        };
        let mut k = label(kind).width(110);
        if warn {
            k = k.warn();
        }
        c.push(row([k, label(l.body()).grow()]).key(2000 + i as u64));
    }
    match &p.type_name {
        Some(name) => {
            c.push(label("Všechno na disku se smaže, včetně jiných systémů.").warn());
            c.push(label(&format!("Napiš název disku pro potvrzení: {name}")));
            c.push(text_input(&s.confirm).placeholder(name).on_change(Msg::Confirm).key("potvrzeni"));
        }
        None => c.push(checkbox("Rozumím, tyto změny se zapíšou na disk", s.agree).on_toggle(Msg::Agree).key("souhlas")),
    }
    if p.other_os {
        c.push(label("Výchozí systém při startu"));
        c.push(list([String::from("MagnumOpus"), String::from("druhý systém")], Some(if s.default_magnum { 0 } else { 1 })).on_select(Msg::DefaultOs).key("vychozi"));
        c.push(checkbox("Druhý systém je Windows (hodiny v místním čase)", s.rtc_local).on_toggle(Msg::RtcLocal).key("rtc"));
    }
}

fn account(s: &Installer, c: &mut Vec<Node<Msg>>) {
    if s.keeps_accounts() {
        c.push(label("Přeinstalace: účty a soubory z předchozí instalace zůstanou."));
        return;
    }
    c.push(label("Účet je jen na tomto počítači. Nic se neposílá na internet."));
    let a = &s.account;
    c.push(label("Jméno pro přihlášení (malá písmena, číslice, - a _)"));
    c.push(text_input(&a.name).placeholder("jan").on_change(Msg::Name).error(s.name_error()).key("jmeno"));
    c.push(label("Zobrazované jméno"));
    c.push(text_input(&a.display).placeholder(&a.name).on_change(Msg::Display).key("zobrazovane"));
    c.push(label("Heslo (aspoň 10 znaků nebo 4 slova)"));
    c.push(password_input(&a.password).on_change(Msg::Password).error(s.password_error()).key("heslo"));
    c.push(label("Heslo znovu"));
    c.push(password_input(&s.password2).on_change(Msg::Password2).error(s.password2_error()).key("heslo2"));
    c.push(label("PIN pro rychlé přihlášení (prázdné = bez PIN)"));
    c.push(password_input(&a.pin).on_change(Msg::Pin).key("pin"));
}

fn summary(s: &Installer, c: &mut Vec<Node<Msg>>) {
    c.push(label("Zkontroluj, co se nainstaluje. Po kliknutí na Nainstalovat se zapíše disk."));
    let st = s.settings();
    let disk = s.disk_entry().map_or(String::new(), |d| format!("{} ({})", d.model, s.mode.map_or("", |m| m.key())));
    let account = if s.keeps_accounts() { String::from("stávající") } else { s.account.name.clone() };
    let rows = [
        ("Klávesnice", st.keyboards.join(", ")),
        ("Časové pásmo", st.zone.clone()),
        ("Disk", disk),
        ("Účet", account),
        ("Šifrování", String::from("vypnuto (připravuje se)")),
        ("Výchozí systém", installplan::default_os_text(st.default_magnum)),
    ];
    for (i, (k, v)) in rows.into_iter().enumerate() {
        c.push(row([label(k).width(160), label(&v).grow()]).key(3000 + i as u64));
    }
}

fn run(s: &Installer, c: &mut Vec<Node<Msg>>) {
    match &s.outcome {
        Some(Outcome::Done(summary)) => {
            for (i, l) in summary.lines().enumerate() {
                c.push(label(l).key(4000 + i as u64));
            }
        }
        Some(Outcome::Failed(t)) => {
            c.push(label(t).warn().key("duvod"));
        }
        None => {}
    }
    let pct = s.percent();
    c.push(progress(pct).key("prubeh"));
    c.push(label(&format!("{pct} %")).key("procenta"));
    let cur = s.progress.as_ref().map_or(0, |p| p.0 as usize);
    let done = matches!(s.outcome, Some(Outcome::Done(_)));
    for (i, t) in PROGRESS.iter().enumerate() {
        let state = if done || i < cur {
            "hotovo"
        } else if i == cur && s.outcome.is_none() {
            "probíhá"
        } else {
            ""
        };
        c.push(row([label(state).width(90), label(t).grow()]).key(5000 + i as u64));
    }
    if s.outcome.is_none() {
        c.push(label("Mezitím můžeš systém dál zkoušet. Instalace běží i po zavření okna (průběh ve Správci úloh)."));
    }
}
