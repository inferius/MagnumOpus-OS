//! Testy faze 3: scrub na pozadi (overeni obou kopii, oprava, hlaseni
//! neopravitelnych chyb, pokracovani po restartu).

use super::data::{noise, text};
use super::{assert_clean, new_fs, remount};
use crate::datarec::{DataSettings, DedupMode, SCRUB_IDLE, SCRUB_RUNNING};
use crate::layout::{BLOCK_SIZE, Timestamp};
use crate::{Fs, Kind, MemDevice};

fn t(secs: i64) -> Timestamp {
    Timestamp { secs, nanos: 0 }
}

/// Dobehne scrub (po malych krocich); vraci pocet kroku.
fn run(fs: &mut Fs<MemDevice>, budget: u64) -> u32 {
    let mut steps = 0;
    while !fs.scrub_step(budget, t(2000)).unwrap() {
        steps += 1;
        assert!(steps < 100_000, "scrub se nezastavil");
    }
    steps
}

fn volume(copies: u8) -> Fs<MemDevice> {
    let mut fs = new_fs(8192);
    fs.set_data_settings("/", &DataSettings { compress: None, copies, dedup: DedupMode::Off }).unwrap();
    fs.create("/dir", Kind::Dir).unwrap();
    for i in 0..20 {
        let p = alloc::format!("/dir/f{i}");
        fs.create(&p, Kind::File).unwrap();
        fs.write(&p, 0, &noise(20_000 + i * 3000, i as u64)).unwrap();
    }
    fs.commit().unwrap();
    fs.snapshot("s1").unwrap();
    fs.write("/dir/f3", 0, &text(50_000, 1)).unwrap();
    fs.create_dataset("/ds").unwrap();
    fs.create("/ds/g", Kind::File).unwrap();
    fs.write("/ds/g", 0, &noise(90_000, 77)).unwrap();
    fs.commit().unwrap();
    fs
}

#[test]
fn scrub_clean_volume() {
    let mut fs = volume(1);
    fs.scrub_start(t(1000)).unwrap();
    assert_eq!(fs.scrub_status().unwrap().state.state, SCRUB_RUNNING);
    let steps = run(&mut fs, 64);
    assert!(steps > 3, "rozpocet kroku se nedodrzel");
    let st = fs.scrub_status().unwrap();
    assert_eq!(st.state.state, SCRUB_IDLE);
    assert_eq!((st.state.bad, st.state.lost), (0, 0));
    assert!(st.state.scanned > 300);
    assert_eq!(st.state.finished, t(2000));
    assert!(st.errors.is_empty());
    assert_clean(&mut fs);
}

#[test]
fn scrub_repairs_bad_copies() {
    let mut fs = volume(2);
    let (_, e) = fs.test_extents("/dir/f5")[0].clone();
    let (_, e2) = fs.test_extents("/ds/g")[0].clone();
    let mut dev = fs.into_device();
    // Prvni kopie jednoho souboru a druha kopie jineho.
    dev.data[e.phys as usize * BLOCK_SIZE + 3] ^= 1;
    dev.data[(e2.copy + 2) as usize * BLOCK_SIZE + 9] ^= 1;
    let mut fs = Fs::mount(dev).unwrap();
    fs.scrub_start(t(1000)).unwrap();
    run(&mut fs, 256);
    let st = fs.scrub_status().unwrap().state;
    assert_eq!(st.bad, 2);
    assert_eq!(st.repaired, 2);
    assert_eq!(st.lost, 0);
    // Opraveno na disku: dalsi beh uz nic nenajde.
    fs.commit().unwrap();
    let mut fs = remount(fs);
    fs.scrub_start(t(3000)).unwrap();
    run(&mut fs, 256);
    let st2 = fs.scrub_status().unwrap();
    assert_eq!(st2.state.bad, 0, "{st2:?} prvni: {st:?}");
    assert_clean(&mut fs);
}

#[test]
fn scrub_reports_lost_data_with_path() {
    let mut fs = volume(1);
    let (_, e) = fs.test_extents("/dir/f9")[0].clone();
    let mut dev = fs.into_device();
    dev.data[(e.phys + 1) as usize * BLOCK_SIZE] ^= 0x80;
    let mut fs = Fs::mount(dev).unwrap();
    fs.scrub_start(t(1000)).unwrap();
    run(&mut fs, 128);
    let st = fs.scrub_status().unwrap();
    assert_eq!(st.state.lost, 1);
    assert_eq!(st.errors.len(), 1);
    assert!(st.errors[0].contains("/dir/f9"), "chyba bez cesty: {}", st.errors[0]);
}

#[test]
fn scrub_resumes_after_remount_and_stop() {
    let mut fs = volume(2);
    fs.scrub_start(t(1000)).unwrap();
    assert!(!fs.scrub_step(40, t(1100)).unwrap());
    let before = fs.scrub_status().unwrap().state;
    fs.commit().unwrap();
    // Restart: pozice zustala.
    let mut fs = remount(fs);
    let st = fs.scrub_status().unwrap().state;
    assert_eq!((st.state, st.dataset, st.key), (SCRUB_RUNNING, before.dataset, before.key));
    fs.scrub_stop().unwrap();
    assert!(fs.scrub_step(40, t(1200)).unwrap(), "zastaveny scrub pokracoval");
    fs.scrub_start(t(1300)).unwrap();
    // Pokracovani, ne novy beh: zacatek zustava.
    assert_eq!(fs.scrub_status().unwrap().state.started, t(1000));
    run(&mut fs, 64);
    let st = fs.scrub_status().unwrap().state;
    assert_eq!((st.state, st.bad), (SCRUB_IDLE, 0));
    assert!(st.scanned >= before.scanned);
}
