//! gui-tasks: spravce uloh v GUI (milnik 6c).
//!
//! Model a rozhrani jsou v knihovne `taskview` (testuje se na hostiteli).
//! Dokud sluzba `taskd` (vlakno shellu 6b) neni v systemu, bezi nad
//! testovacimi daty (`taskview::TestBackend`); pak dostane od initu kanal
//! na taskd a zdroj se vymeni (protokol `libs/taskproto`).
//!
//! Startovni zprava (kind USER): kanal okna ke kompozitoru.

#![no_std]
#![no_main]

extern crate alloc;

use guiproto::window::Mode;
use libos::{Channel, Startup, println};
use taskview::{TaskManager, TestBackend};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let Some(h) = startup.handles.into_iter().next() else {
        println!("gui-tasks: chybi kanal okna");
        return 1;
    };
    let app = TaskManager::new(TestBackend::default());
    match ui::client::run(Channel::from_handle(h), app, Mode::Standard, 560, 620) {
        Ok(()) => 0,
        Err(e) => {
            println!("gui-tasks: {e}");
            2
        }
    }
}
