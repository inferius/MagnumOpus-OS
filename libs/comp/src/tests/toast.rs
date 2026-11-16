//! Vyskakovaci oznameni (`toast.rs`): vyjeti, konec zobrazeni do centra,
//! krizek, akce, nahrazeni, fyzicky klik, cela obrazovka.

use super::*;
use notifyproto::{Action as NAction, Closed, Notification, Priority, View, ViewReply};

const UID: u64 = 1000;

fn n(id: u64, p: Priority, actions: &[(&str, &str)]) -> Notification {
    Notification {
        id,
        app_id: "org.example.dl".into(),
        app_name: "Stahování".into(),
        title: "Soubor stažen".into(),
        body: "obraz.iso je ve složce Stažené. Dlouhý text, který se musí zalomit na více řádků karty.".into(),
        priority: p,
        actions: actions.iter().map(|(i, l)| NAction { id: (*i).into(), label: (*l).into() }).collect(),
        time: 0,
        popup: true,
        replaced: false,
    }
}

fn card_px(t: &T) -> u32 {
    // Stred karty vpravo nahore (sirka z tokenu).
    let w = t.c.style.theme.size.notification_w as i32;
    t.px(W as i32 - w / 2 - 20, 40)
}

#[test]
fn toast_slides_in_and_goes_to_center_after_show() {
    let mut t = T::new();
    t.frame();
    let empty = card_px(&t);
    t.c.notify_event(UID, ViewReply::Snapshot { list: vec![], last: true }, t.now);
    t.c.notify_event(UID, ViewReply::Added(n(1, Priority::Normal, &[])), t.now);
    assert_eq!(t.c.toasts().len(), 1);
    // Vyjizdi: probouzi se kazdy snimek, pak je karta na miste.
    assert!(t.c.next_wakeup(t.now).is_some_and(|d| d <= t.now + PERIOD));
    for _ in 0..40 {
        t.frame();
    }
    assert_ne!(card_px(&t), empty, "karta je nakreslena");
    // Po 6 s zmizi do centra (Seen), v seznamu zustane.
    t.now += crate::toast::SHOW;
    t.c.timers(t.now);
    t.frame();
    assert!(t.c.toasts().is_empty());
    assert_eq!(t.c.notifications().len(), 1);
    assert_eq!(t.c.take_notify_requests(), vec![(UID, View::Seen { id: 1 })]);
    assert_eq!(card_px(&t), empty, "karta zmizela z obrazovky");
}

#[test]
fn close_action_and_body_click() {
    let mut t = T::new();
    t.frame();
    t.c.notify_event(UID, ViewReply::Added(n(1, Priority::High, &[("open", "Otevřít"), ("dir", "Složka")])), t.now);
    for _ in 0..40 {
        t.frame();
    }
    // High zustava i po 6 s.
    t.now += 2 * crate::toast::SHOW;
    t.c.timers(t.now);
    assert_eq!(t.c.toasts().len(), 1);
    let l = t.c.toast_layout_for_test(0);
    // Akce (druha): Invoke s jejim id.
    let a = l.1[1];
    t.click(a.x + 4, a.y + 4);
    assert_eq!(t.c.take_notify_requests(), vec![(UID, View::Invoke { id: 1, action: "dir".into() })]);
    assert!(t.c.toasts().is_empty());
    // Krizek: Dismiss.
    t.c.notify_event(UID, ViewReply::Added(n(2, Priority::High, &[])), t.now);
    let (close, _, card) = t.c.toast_layout_for_test(0);
    t.click(close.x + close.w as i32 / 2, close.y + close.h as i32 / 2);
    assert_eq!(t.c.take_notify_requests(), vec![(UID, View::Dismiss { id: 2 })]);
    // Klik do karty: Invoke bez akce.
    t.c.notify_event(UID, ViewReply::Added(n(3, Priority::High, &[])), t.now);
    t.click(card.x + 10, card.y + card.h as i32 - 10);
    assert_eq!(t.c.take_notify_requests(), vec![(UID, View::Invoke { id: 3, action: String::new() })]);
}

#[test]
fn synthetic_click_does_not_invoke() {
    let mut t = T::new();
    t.frame();
    t.c.notify_event(UID, ViewReply::Added(n(1, Priority::High, &[("ok", "Povolit")])), t.now);
    let (_, acts, _) = t.c.toast_layout_for_test(0);
    t.move_to(acts[0].x + 4, acts[0].y + 4);
    t.input(Event::Button { button: Button::Left, action: Action::Press }, true);
    t.input(Event::Button { button: Button::Left, action: Action::Release }, true);
    assert!(t.c.take_notify_requests().is_empty());
    assert_eq!(t.c.toasts().len(), 1);
}

#[test]
fn replace_updates_in_place_and_removed_disappears() {
    let mut t = T::new();
    t.frame();
    t.c.notify_event(UID, ViewReply::Added(n(1, Priority::Normal, &[])), t.now);
    let mut m = n(1, Priority::Normal, &[]);
    m.title = "Stahování 50 %".into();
    m.replaced = true;
    t.c.notify_event(UID, ViewReply::Added(m), t.now);
    assert_eq!(t.c.toasts().len(), 1);
    assert_eq!(t.c.toasts()[0].n.title, "Stahování 50 %");
    assert_eq!(t.c.notifications().len(), 1);
    t.c.notify_event(UID, ViewReply::Removed { id: 1, why: Closed::Sender }, t.now);
    assert!(t.c.toasts().is_empty() && t.c.notifications().is_empty());
    // Nevyskakujici (Low, popup false): jen do centra.
    let mut low = n(2, Priority::Low, &[]);
    low.popup = false;
    t.c.notify_event(UID, ViewReply::Added(low), t.now);
    assert!(t.c.toasts().is_empty());
    assert_eq!(t.c.notifications().len(), 1);
}

#[test]
fn at_most_three_cards_then_queue() {
    let mut t = T::new();
    t.frame();
    for i in 1..=5 {
        t.c.notify_event(UID, ViewReply::Added(n(i, Priority::High, &[])), t.now);
    }
    assert_eq!(t.c.toasts().len(), crate::toast::MAX_SHOWN);
    t.c.notify_dismiss(1, t.now);
    assert_eq!(t.c.toasts().iter().map(|x| x.n.id).collect::<Vec<_>>(), vec![2, 3, 4]);
    // Karty se neprekryvaji.
    let a = t.c.toast_layout_for_test(0).2;
    let b = t.c.toast_layout_for_test(1).2;
    assert!(a.y + a.h as i32 <= b.y);
}

#[test]
fn fullscreen_dnd_keeps_quiet_except_urgent() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Fullscreen, RED);
    t.frame();
    assert!(t.c.do_not_disturb());
    t.c.notify_event(UID, ViewReply::Added(n(1, Priority::High, &[])), t.now);
    assert!(t.c.toasts().is_empty());
    assert_eq!(t.c.take_notify_requests(), vec![(UID, View::Seen { id: 1 })]);
    t.c.notify_event(UID, ViewReply::Added(n(2, Priority::Urgent, &[])), t.now);
    assert_eq!(t.c.toasts().len(), 1);
}

#[test]
fn snapshot_replaces_state_of_user() {
    let mut t = T::new();
    t.c.notify_event(UID, ViewReply::Added(n(1, Priority::High, &[])), t.now);
    t.c.notify_event(UID, ViewReply::Snapshot { list: vec![n(5, Priority::Low, &[])], last: false }, t.now);
    t.c.notify_event(UID, ViewReply::Snapshot { list: vec![n(6, Priority::Low, &[])], last: true }, t.now);
    let ids: Vec<u64> = t.c.notifications().iter().map(|(_, x)| x.id).collect();
    assert_eq!(ids, vec![5, 6]);
    t.c.notify_event(UID, ViewReply::Dnd { until: 99 }, t.now);
    assert_eq!(t.c.notify_dnd(UID), 99);
}
