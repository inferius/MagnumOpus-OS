//! Pilulka vzdaleneho pristupu v liste (SSH, `docs/cs/ssh.md`): videt je,
//! dokud je nekdo pripojeny; text nese kdo a odkud.

use super::*;

#[test]
fn pilulka_vzdaleneho_pristupu() {
    let mut t = T::new();
    assert!(t.c.bar_layout().remote.is_none(), "bez vzdalene relace neni");
    t.c.set_remote(alloc::vec![String::from("alice@10.0.2.2")]);
    let one = t.c.bar_layout().remote.expect("pilulka");
    t.frame();
    assert_eq!(crate::render::remote_label(&t.c.status.remote), "Vzdáleně: alice@10.0.2.2");
    t.c.set_remote(alloc::vec![String::from("alice@10.0.2.2"), String::from("bob@10.0.2.3")]);
    assert_eq!(crate::render::remote_label(&t.c.status.remote), "Vzdáleně: alice@10.0.2.2 +1");
    assert!(t.c.bar_layout().remote.unwrap().w > one.w);
    t.frame();
    t.c.set_remote(Vec::new());
    assert!(t.c.bar_layout().remote.is_none(), "po odpojeni zmizi");
}
