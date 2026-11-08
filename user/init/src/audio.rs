//! Zvuk: start sluzby `audiod` s ovladaci zvukovych karet.
//!
//! audiod dostane vsechny kanaly k ovladacum (`audio` od devmgr) a sam
//! vybere zarizeni s analogovym vystupem. Bootstrap kanal je prvni klient
//! sluzby; init si ho necha ([`AUDIO`]) a dalsi klienty (termd, testy)
//! vytvari pres `CONNECT`.

use alloc::vec::Vec;
use core::time::Duration;

use audioproto::conn::AudioClient;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, println, time};

pub struct Audio {
    pub process: Process,
    /// Klient je postupny (pozadavek, odpoved): zamek, aby si dve vlakna
    /// initu nebrala odpovedi.
    pub client: spin::Mutex<AudioClient>,
}

/// Bezici zvukova sluzba (`None` v Once = nespustena).
pub static AUDIO: spin::Once<Audio> = spin::Once::new();

/// Novy kanal ke zvukove sluzbe (pro termd a testy).
pub fn connect() -> Result<Channel> {
    AUDIO.get().ok_or(Error::NotFound)?.client.lock().connect()
}

/// Spusti audiod nad ovladaci `devices` (i prazdne: sluzba bezi a streamy
/// odmita, aby klienti dostali jasnou chybu).
pub fn start(startup: &Startup, devices: Vec<Channel>) -> Result<Audio> {
    let n = devices.len();
    let handles = devices.into_iter().map(|c| StartHandle::user(c.into_handle())).collect();
    let app = crate::apps::spawn(startup, "audiod", handles)?;
    let msg = app.control.read(Some(time::deadline_after(Duration::from_secs(10))))?;
    if msg.data != b"ok" {
        let _ = app.process.kill();
        return Err(Error::Corrupted);
    }
    println!("init: zvukova sluzba bezi ({n} zvukovych karet)");
    Ok(Audio { process: app.process, client: spin::Mutex::new(AudioClient::new(app.control)) })
}
