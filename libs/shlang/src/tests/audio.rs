//! Prikaz `audio` (zapis B i C) nad testovacim hostitelem.

use super::host::{fails, run_lines};
use crate::host::AudioReq;
use crate::value::Value;

fn reqs(line: &str) -> Vec<AudioReq> {
    let (h, _, o) = run_lines(&[line]);
    if let Err(e) = &o.result {
        panic!("{}", e.report(line));
    }
    h.audio
}

#[test]
fn tone_defaults_and_flags() {
    assert_eq!(reqs("audio tone"), vec![AudioReq::Tone { freq: 440, ms: 1_000, level: 50 }]);
    assert_eq!(reqs("audio tone 1000 --duration 2s --level 20"), vec![AudioReq::Tone { freq: 1_000, ms: 2_000, level: 20 }]);
    assert_eq!(reqs("audio tone 880 -d 250ms"), vec![AudioReq::Tone { freq: 880, ms: 250, level: 50 }]);
    assert_eq!(reqs("audio(\"tone\", 500)"), vec![AudioReq::Tone { freq: 500, ms: 1_000, level: 50 }]);
}

#[test]
fn volume_mute_status() {
    assert_eq!(reqs("audio volume 60"), vec![AudioReq::Volume(Some(60))]);
    assert_eq!(reqs("audio mute; audio unmute; audio status"), vec![AudioReq::Mute(true), AudioReq::Mute(false), AudioReq::Status]);
    let (h, _, o) = run_lines(&["audio volume"]);
    assert!(o.result.is_ok());
    assert_eq!(h.out, vec![Value::Int(70)]);
    assert_eq!(h.audio, vec![AudioReq::Volume(None)]);
}

#[test]
fn play_reads_file_through_session_fs() {
    let (h, _, o) = run_lines(&["audio play a.txt"]);
    assert!(o.result.is_ok());
    assert_eq!(h.audio, vec![AudioReq::Play { wav: b"alfa\n".to_vec() }]);
    // Neexistujici soubor = chyba FS, ne pozadavek.
    let e = fails("audio play /nic.wav");
    assert_eq!(e.code, "NOT_FOUND");
}

#[test]
fn bad_arguments() {
    for (line, code) in [
        ("audio tone 5", "SH_ARGS"),
        ("audio tone 440 --level 101", "SH_ARGS"),
        ("audio tone 440 --duration 90s", "SH_ARGS"),
        ("audio tone 440 --duration 5", "SH_TYPE"),
        ("audio volume 150", "SH_ARGS"),
        ("audio hraj", "SH_ARGS"),
        ("audio play", "SH_ARGS"),
    ] {
        assert_eq!(fails(line).code, code, "{line}");
    }
}
