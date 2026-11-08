//! Prikaz `audio` (zapis B i C) nad testovacim hostitelem.

use super::host::{TestHost, fails, interp, run_lines};
use crate::host::AudioReq;
use crate::value::Value;

fn reqs(line: &str) -> Vec<AudioReq> {
    let (h, _, o) = run_lines(&[line]);
    if let Err(e) = &o.result {
        panic!("{}", e.report(line));
    }
    h.audio
}

/// Pozadavky bez dat (data jen delka).
fn shape(r: &[AudioReq]) -> Vec<String> {
    r.iter()
        .map(|q| match q {
            AudioReq::Write { data } => format!("write {}", data.len()),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn tone_streams_in_chunks() {
    // 1 s mono 48 kHz s16 = 96 000 bajtu.
    let r = reqs("audio tone");
    assert_eq!(shape(&r), vec!["Open { rate: 48000, channels: 1, sample: 2 }", "write 96000", "Finish"]);
    let r = reqs("audio tone 1000 --duration 250ms --level 20");
    assert_eq!(shape(&r)[1], "write 24000");
    assert_eq!(reqs("audio(\"tone\", 500)").len(), 3);
}

#[test]
fn volume_mute_status_devices() {
    assert_eq!(reqs("audio volume 60"), vec![AudioReq::Volume(Some(60))]);
    assert_eq!(reqs("audio mute; audio unmute; audio status"), vec![AudioReq::Mute(true), AudioReq::Mute(false), AudioReq::Status]);
    assert_eq!(reqs("audio devices; audio device 1"), vec![AudioReq::Devices, AudioReq::Select(1)]);
    let (h, _, o) = run_lines(&["audio volume"]);
    assert!(o.result.is_ok());
    assert_eq!(h.out, vec![Value::Int(70)]);
    assert_eq!(h.audio, vec![AudioReq::Volume(None)]);
}

fn wav_file(rate: u32, channels: u16, data: &[u8]) -> Vec<u8> {
    let mut f = audiomix::wav::header_s16(rate, channels, data.len() as u32);
    f.extend_from_slice(data);
    f
}

#[test]
fn play_reads_file_in_parts() {
    let mut h = TestHost::new();
    let data: Vec<u8> = (0..44_100u32 * 4).map(|i| i as u8).collect();
    h.fs.put("/home/a.wav", &wav_file(22_050, 2, &data));
    let mut it = interp();
    let o = it.run_line(&mut h, "audio play a.wav");
    assert!(o.result.is_ok(), "{:?}", o.result);
    assert_eq!(h.audio[0], AudioReq::Open { rate: 22_050, channels: 2, sample: 2 });
    assert_eq!(h.audio[1], AudioReq::Write { data: data.clone() });
    assert_eq!(h.audio[2], AudioReq::Finish);
}

#[test]
fn play_errors() {
    assert_eq!(fails("audio play /nic.wav").code, "NOT_FOUND");
    assert_eq!(fails("audio play a.txt").code, "SH_TYPE");
}

#[test]
fn ctrl_c_aborts_playback() {
    let mut h = TestHost::new();
    h.interrupt_after = Some(3);
    let mut it = interp();
    let o = it.run_line(&mut h, "audio tone 440 --duration 10s");
    assert_eq!(o.result.unwrap_err().code, "SH_INTERRUPTED");
    assert_eq!(h.audio.last(), Some(&AudioReq::Abort));
    assert!(!h.audio.contains(&AudioReq::Finish));
}

#[test]
fn bad_arguments() {
    for (line, code) in [
        ("audio tone 5", "SH_ARGS"),
        ("audio tone 440 --level 101", "SH_ARGS"),
        ("audio tone 440 --duration 2h", "SH_ARGS"),
        ("audio tone 440 --duration 5", "SH_TYPE"),
        ("audio volume 150", "SH_ARGS"),
        ("audio device", "SH_ARGS"),
        ("audio hraj", "SH_ARGS"),
        ("audio play", "SH_ARGS"),
    ] {
        assert_eq!(fails(line).code, code, "{line}");
    }
}
