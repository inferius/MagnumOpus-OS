//! gui-tasks: spravce uloh v GUI (milnik 6c).
//!
//! Model a rozhrani jsou v knihovne `taskview` (testuje se na hostiteli).
//! Dokud sluzba `taskd` (vlakno shellu 6b) neni v systemu, bezi nad
//! testovacimi daty (`taskview::TestBackend`); pak dostane od initu kanal
//! na taskd a zdroj se vymeni (protokol `libs/taskproto`).
//!
//! Startovni zprava (kind USER): 1. kanal okna ke kompozitoru, 2. volitelne
//! kanal na taskd, ktery razi init (identita uzivatele, akce jen nad
//! vlastnimi ulohami). Bez nej bezi nad testovacimi daty.

#![no_std]
#![no_main]

extern crate alloc;

use guiproto::window::Mode;
use libos::{Channel, Startup, println};
use taskview::{ChannelBackend, TaskManager, TestBackend};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let Some(h) = handles.next() else {
        println!("gui-tasks: chybi kanal okna");
        return 1;
    };
    let r = match handles.next().map(Channel::from_handle) {
        Some(taskd) => {
            // Zapis pres druhy handle stejneho kanalu (cteni zustane run_with).
            let writer = match taskd.handle().duplicate(None) {
                Ok(d) => Channel::from_handle(d),
                Err(e) => {
                    println!("gui-tasks: kanal taskd: {e}");
                    return 3;
                }
            };
            let backend = ChannelBackend::new(move |data| {
                let _ = writer.write(&data, alloc::vec::Vec::new());
            });
            let _ = taskd.write(&taskproto::Request::Hello { version: taskproto::VERSION }.encode(), alloc::vec::Vec::new());
            let app = TaskManager::new(backend);
            ui::client::run_with(Channel::from_handle(h), app, Mode::Standard, 560, 620, Some(taskd))
        }
        None => {
            let app = TaskManager::new(TestBackend::default());
            ui::client::run(Channel::from_handle(h), app, Mode::Standard, 560, 620)
        }
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            println!("gui-tasks: {e}");
            2
        }
    }
}
