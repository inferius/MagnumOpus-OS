//! Testy na hostiteli: `cargo test -p fileops`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::job::{CHUNK, Job, Op, Phase, Resolution, Step};
use crate::mem::MemFs;
use crate::path::{is_within, join, name, numbered, parent, valid_name};
use crate::proto::{ClientMsg, ServerMsg, Status};
use crate::{Error, Vfs};

fn s(v: &str) -> String {
    String::from(v)
}

/// Dobehne ulohu; konflikt rozhodne `answer`.
fn run(job: &mut Job, fs: &mut MemFs, answer: Option<(Resolution, bool)>) -> u32 {
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 100_000, "uloha nekonci");
        match job.step(fs) {
            Step::Worked => {}
            Step::Conflict(_) => {
                let (r, all) = answer.expect("necekany konflikt");
                job.decide(fs, r, all);
            }
            Step::Idle => return steps,
        }
    }
}

fn sample() -> MemFs {
    let mut fs = MemFs::default();
    fs.file("/home/a/doc.txt", b"dokument").file("/home/a/foto/1.bmp", &[7u8; 3000]).file("/home/a/foto/sub/2.bmp", b"dva").dir("/home/a/cil");
    fs
}

#[test]
fn paths() {
    assert_eq!(join("/", "a"), "/a");
    assert_eq!(join("/a/", "b"), "/a/b");
    assert_eq!(parent("/a/b"), "/a");
    assert_eq!(parent("/a"), "/");
    assert_eq!(name("/a/b.txt"), "b.txt");
    assert!(is_within("/a/b/c", "/a/b"));
    assert!(is_within("/A/b", "/a"));
    assert!(!is_within("/a/bc", "/a/b"));
    assert_eq!(numbered("zprava.txt", 2, false), "zprava (2).txt");
    assert_eq!(numbered("zprava (2).txt", 3, false), "zprava (3).txt");
    assert_eq!(numbered("Fotky.2024", 2, true), "Fotky.2024 (2)");
    assert_eq!(numbered(".profile", 2, false), ".profile (2)");
    assert!(valid_name("a b") && !valid_name("a/b") && !valid_name("..") && !valid_name(""));
    assert!(Job::is_temp_name(".doc.txt.kopie-12") && !Job::is_temp_name("doc.kopie-1") && !Job::is_temp_name(".kopie-1"));
}

#[test]
fn copy_uses_instant_clone() {
    let mut fs = sample();
    let mut j = Job::new(1, Op::Copy, vec![s("/home/a/doc.txt"), s("/home/a/foto")], s("/home/a/cil"));
    run(&mut j, &mut fs, None);
    assert_eq!(j.phase, Phase::Done);
    assert_eq!(fs.clones, 2, "dve okamzite kopie, zadne cteni");
    assert_eq!(fs.data("/home/a/cil/doc.txt"), Some(&b"dokument"[..]));
    assert_eq!(fs.data("/home/a/cil/foto/sub/2.bmp"), Some(&b"dva"[..]));
    assert_eq!(j.total_bytes, Some(8 + 3000 + 3));
    assert_eq!((j.done_bytes, j.done_items, j.percent()), (3011, 2, 100));
    assert!(fs.exists("/home/a/doc.txt"), "zdroj zustal");
    assert!(fs.below("/home/a/cil").iter().all(|p| !Job::is_temp_name(name(p))), "zadne docasne jmeno");
}

#[test]
fn fallback_copies_in_chunks() {
    let mut fs = sample();
    fs.clone = false;
    let big: Vec<u8> = (0..(CHUNK * 2 + 17)).map(|i| i as u8).collect();
    fs.file("/home/a/foto/velky.raw", &big);
    let mut j = Job::new(2, Op::Copy, vec![s("/home/a/foto")], s("/home/a/cil"));
    run(&mut j, &mut fs, None);
    assert_eq!(fs.clones, 0);
    assert_eq!(fs.data("/home/a/cil/foto/velky.raw"), Some(&big[..]));
    assert_eq!(fs.data("/home/a/cil/foto/1.bmp").map(<[u8]>::len), Some(3000));
    assert_eq!(fs.data("/home/a/cil/foto/sub/2.bmp"), Some(&b"dva"[..]));
    assert_eq!(j.done_bytes, j.total_bytes.unwrap());
}

#[test]
fn conflicts_skip_rename_overwrite() {
    let mut fs = sample();
    fs.file("/home/a/cil/doc.txt", b"stary");
    // Preskocit.
    let mut j = Job::new(3, Op::Copy, vec![s("/home/a/doc.txt")], s("/home/a/cil"));
    assert_eq!(j.step(&mut fs), Step::Worked);
    let Step::Conflict(c) = j.step(&mut fs) else { panic!("cekal se konflikt") };
    assert_eq!(c.target, "/home/a/cil/doc.txt");
    assert_eq!(j.phase, Phase::Conflict);
    assert_eq!(j.step(&mut fs), Step::Conflict(c.clone()), "bez rozhodnuti stoji");
    j.decide(&mut fs, Resolution::Skip, false);
    run(&mut j, &mut fs, None);
    assert_eq!((j.skipped, fs.data("/home/a/cil/doc.txt")), (1, Some(&b"stary"[..])));
    // Prejmenovat.
    let mut j = Job::new(4, Op::Copy, vec![s("/home/a/doc.txt")], s("/home/a/cil"));
    run(&mut j, &mut fs, Some((Resolution::Rename, false)));
    assert_eq!(fs.data("/home/a/cil/doc (2).txt"), Some(&b"dokument"[..]));
    // Prepsat: puvodni cil do kose.
    let mut j = Job::new(5, Op::Copy, vec![s("/home/a/doc.txt")], s("/home/a/cil"));
    run(&mut j, &mut fs, Some((Resolution::Overwrite, false)));
    assert_eq!(fs.data("/home/a/cil/doc.txt"), Some(&b"dokument"[..]));
    assert_eq!(fs.trashed.last().map(|t| t.0.as_str()), Some("/home/a/cil/doc.txt"));
    assert_eq!(fs.trashed.last().unwrap().1[0].1, crate::mem::Node::File(b"stary".to_vec()));
}

#[test]
fn decision_for_all_is_not_asked_again() {
    let mut fs = sample();
    fs.file("/home/a/cil/doc.txt", b"x").dir("/home/a/cil/foto");
    let mut j = Job::new(6, Op::Copy, vec![s("/home/a/doc.txt"), s("/home/a/foto")], s("/home/a/cil"));
    let mut asked = 0;
    loop {
        match j.step(&mut fs) {
            Step::Worked => {}
            Step::Conflict(_) => {
                asked += 1;
                j.decide(&mut fs, Resolution::Rename, true);
            }
            Step::Idle => break,
        }
    }
    assert_eq!(asked, 1);
    assert!(fs.exists("/home/a/cil/doc (2).txt") && fs.exists("/home/a/cil/foto (2)/sub/2.bmp"));
}

#[test]
fn copy_into_same_folder_gets_free_name_without_asking() {
    let mut fs = sample();
    let mut j = Job::new(7, Op::Copy, vec![s("/home/a/doc.txt")], s("/home/a"));
    run(&mut j, &mut fs, None);
    assert_eq!(fs.data("/home/a/doc (2).txt"), Some(&b"dokument"[..]));
}

#[test]
fn folder_into_itself_is_an_error() {
    let mut fs = sample();
    let mut j = Job::new(8, Op::Copy, vec![s("/home/a/foto"), s("/home/a/doc.txt")], s("/home/a/foto/sub"));
    run(&mut j, &mut fs, None);
    assert_eq!(j.error_count, 1);
    assert_eq!(j.errors[0], (s("/home/a/foto"), Error::InvalidArgs));
    assert!(fs.exists("/home/a/foto/sub/doc.txt"), "ostatni polozky pokracuji");
}

#[test]
fn move_within_dataset_renames_and_across_trashes_source() {
    let mut fs = sample();
    fs.dataset("/home/a/Projekty");
    let mut j = Job::new(9, Op::Move, vec![s("/home/a/doc.txt")], s("/home/a/cil"));
    run(&mut j, &mut fs, None);
    assert_eq!((fs.clones, fs.trashed.len()), (0, 0), "jen prejmenovani");
    assert!(!fs.exists("/home/a/doc.txt") && fs.exists("/home/a/cil/doc.txt"));
    let mut j = Job::new(10, Op::Move, vec![s("/home/a/foto")], s("/home/a/Projekty"));
    run(&mut j, &mut fs, None);
    assert_eq!(fs.clones, 1, "mezi datasety kopie");
    assert!(fs.exists("/home/a/Projekty/foto/sub/2.bmp") && !fs.exists("/home/a/foto"));
    assert_eq!(fs.trashed.last().map(|t| t.0.as_str()), Some("/home/a/foto"), "zdroj v kosi, ne smazany");
}

#[test]
fn errors_are_recorded_and_cancel_removes_temp() {
    let mut fs = sample();
    let mut j = Job::new(11, Op::Copy, vec![s("/home/a/neni"), s("/home/a/doc.txt")], s("/home/a/cil"));
    run(&mut j, &mut fs, None);
    assert_eq!((j.error_count, j.done_items), (1, 1));
    assert_eq!(j.errors[0].1, Error::NotFound);
    // Zruseni uprostred kopie ctenim.
    let mut fs = sample();
    fs.clone = false;
    fs.file("/home/a/velky", &vec![1u8; CHUNK * 3]);
    let mut j = Job::new(12, Op::Copy, vec![s("/home/a/velky")], s("/home/a/cil"));
    for _ in 0..5 {
        j.step(&mut fs);
    }
    assert!(fs.exists("/home/a/cil/.velky.kopie-12"));
    j.cancel(&mut fs);
    assert_eq!(j.phase, Phase::Canceled);
    assert!(fs.below("/home/a/cil").is_empty(), "docasne jmeno pryc, cil nevznikl");
    assert_eq!(j.step(&mut fs), Step::Idle);
}

#[test]
fn pause_and_resume() {
    let mut fs = sample();
    let mut j = Job::new(13, Op::Copy, vec![s("/home/a/doc.txt")], s("/home/a/cil"));
    j.pause();
    assert_eq!(j.step(&mut fs), Step::Idle);
    j.resume();
    run(&mut j, &mut fs, None);
    assert_eq!(j.phase, Phase::Done);
}

/// Preruseni po kazdem kroku: pokracovani z posledniho ulozeneho stavu
/// (vse zapsane do FS zustava, jako po `sync`) da vzdy stejny vysledek
/// a zadne docasne jmeno.
fn interrupted_everywhere(clone: bool) {
    let mut base = sample();
    base.clone = clone;
    base.file("/home/a/velky", &(0..CHUNK * 2 + 5).map(|i| (i * 7) as u8).collect::<Vec<_>>());
    base.file("/home/a/cil/doc.txt", b"stary");
    base.dataset("/home/a/Projekty");
    base.file("/home/a/Projekty/doc.txt", b"stary v cili");
    let sources = vec![s("/home/a/doc.txt"), s("/home/a/foto"), s("/home/a/velky")];
    let make = || Job::new(20, Op::Move, sources.clone(), s("/home/a/Projekty"));
    // Vzor bez preruseni.
    let mut want = base.clone();
    let mut j = make();
    run(&mut j, &mut want, Some((Resolution::Overwrite, true)));
    let total = {
        let mut fs = base.clone();
        let mut j = make();
        run(&mut j, &mut fs, Some((Resolution::Overwrite, true)))
    };
    for stop in 1..total {
        let mut fs = base.clone();
        let mut j = make();
        let mut saved = j.encode();
        for _ in 0..stop {
            if let Step::Conflict(_) = j.step(&mut fs) {
                j.decide(&mut fs, Resolution::Overwrite, true);
            }
            if j.should_save() {
                fs.sync().unwrap();
                saved = j.encode();
                j.saved();
            }
        }
        let mut r = Job::decode(&saved).unwrap();
        r.resumed(&mut fs);
        run(&mut r, &mut fs, Some((Resolution::Overwrite, true)));
        assert_eq!(r.phase, Phase::Done, "preruseni po kroku {stop}");
        assert_eq!(fs.nodes, want.nodes, "preruseni po kroku {stop}");
        assert_eq!(r.error_count, 0, "preruseni po kroku {stop}: {:?}", r.errors);
    }
}

#[test]
fn resume_after_interruption_clone() {
    interrupted_everywhere(true);
}

#[test]
fn resume_after_interruption_fallback() {
    interrupted_everywhere(false);
}

#[test]
fn state_decode_rejects_garbage() {
    let j = Job::new(1, Op::Copy, vec![s("/a")], s("/b"));
    let b = j.encode();
    assert_eq!(Job::decode(&b).unwrap(), j);
    for cut in 0..b.len() {
        assert!(Job::decode(&b[..cut]).is_err(), "zkracene na {cut}");
    }
    let mut bad = b.clone();
    bad[0] = 99;
    assert_eq!(Job::decode(&bad), Err(Error::NotSupported));
}

#[test]
fn proto_roundtrip() {
    for m in [
        ClientMsg::Hello { version: 1 },
        ClientMsg::Start { req: 3, op: Op::Move, sources: vec![s("/a"), s("/b c")], dest: s("/d") },
        ClientMsg::Decide { job: 9, resolution: Resolution::Rename, all: true },
        ClientMsg::Cancel { job: 1 },
        ClientMsg::Pause { job: 1 },
        ClientMsg::Resume { job: 1 },
        ClientMsg::Subscribe,
    ] {
        assert_eq!(ClientMsg::decode(&m.encode()).unwrap(), m);
    }
    let mut fs = sample();
    fs.file("/home/a/cil/doc.txt", b"x");
    let mut j = Job::new(5, Op::Copy, vec![s("/home/a/neni"), s("/home/a/doc.txt")], s("/home/a/cil"));
    while !matches!(j.step(&mut fs), Step::Conflict(_)) {}
    let st = Status::of(&j);
    assert!(st.conflict.is_some() && st.last_error.is_some());
    for m in [ServerMsg::Started { req: 1, job: 5 }, ServerMsg::Rejected { req: 2, error: Error::AccessDenied }, ServerMsg::Status(st)] {
        assert_eq!(ServerMsg::decode(&m.encode()).unwrap(), m);
    }
    assert_eq!(ClientMsg::decode(&ClientMsg::Start { req: 1, op: Op::Copy, sources: vec![], dest: s("/") }.encode()), Err(Error::InvalidArgs));
    assert_eq!(ClientMsg::decode(&[0xff, 0, 0, 0]), Err(Error::InvalidArgs));
}
