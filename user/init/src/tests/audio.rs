//! Zvuk: ovladac Intel HDA (QEMU `intel-hda` + `hda-duplex`) a sluzba
//! audiod.
//!
//! Testy hraji roli aplikaci: vlastni kanaly k audiod (`CONNECT`),
//! streamy se sdilenym bufferem. Test `ton 1 kHz do WAV` prehraje ton
//! pres prevod frekvence 44,1 -> 48 kHz; xtask ho po konci QEMU hleda
//! v souboru WAV (backend `wav`), takze se overi cela cesta az do
//! "reproduktoru" emulatoru.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use core::time::Duration;

use audioproto::conn::AudioClient;
use audioproto::{Direction, Error, Format, Sample};
use audiomix::tone::Tone;
use libos::{Startup, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("audiod nasel zarizeni HDA a vystup", device_found),
    ("stream bezi, DMA posouva pozici", stream_runs),
    ("ton 1 kHz do WAV", tone_to_wav),
    ("mix dvou klientu, hlasitost a ztlumeni", two_clients),
    ("neplatne pozadavky", bad_requests),
    ("nahravani: vstup bezi a data prichazeji", recording),
    ("dve zvukove karty: seznam, vyber, nahravani z druhe", devices),
    ("shell: audio tone, volume, status", shell_audio),
];

fn client() -> Result<AudioClient, String> {
    crate::audio::connect().map(AudioClient::new).map_err(|e| fail("pripojeni k audiod", e))
}

/// `ms` milisekund tonu `freq` ve formatu s16 s `channels` kanaly.
fn tone_pcm(freq: u32, rate: u32, channels: u8, ms: u32, percent: u8) -> vec::Vec<u8> {
    let mut t = Tone::new(freq, rate, percent);
    let frames = (rate * ms / 1000) as usize;
    let mut out = vec::Vec::with_capacity(frames * channels as usize * 2);
    for _ in 0..frames {
        let [l, _] = t.next_frame();
        for _ in 0..channels {
            out.extend_from_slice(&((l >> 8) as i16).to_le_bytes());
        }
    }
    out
}

fn device_found(_: &Startup) -> TestResult {
    let a = client()?;
    let s = a.status().map_err(|e| fail("STATUS", e))?;
    check!(!s.device.is_empty(), "audiod nema zadne zarizeni (ovladac hda nenasel kodek nebo vystup)");
    check!(s.device.starts_with("HDA "), "neocekavane zarizeni");
    check!(!s.outputs.is_empty(), "zadny aktivni vystup");
    check_eq!(s.format, Format::DEVICE_DEFAULT, "format zarizeni");
    check!(s.period > 0 && s.latency_us > 0 && s.latency_us < 25_000, "latence mimo rozpocet");
    libos::println!(
        "zvuk: {} vystupy {} perioda {} snimku latence mixu {} us",
        s.device, s.outputs, s.period, s.latency_us
    );
    Ok(())
}

fn stream_runs(_: &Startup) -> TestResult {
    let a = client()?;
    let before = a.status().map_err(|e| fail("STATUS", e))?;
    let s = a
        .open(Direction::Output, Format::DEVICE_DEFAULT, 50_000)
        .map_err(|e| fail("OPEN", e))?;
    check!(s.info().capacity >= s.info().period * 2, "buffer klienta mensi nez dve periody");
    let pcm = tone_pcm(440, 48_000, 2, 400, 20);
    let deadline = time::deadline_after(Duration::from_secs(10));
    s.write_all(&pcm, true, Some(deadline)).map_err(|e| fail("zapis do bufferu", e))?;
    time::sleep(Duration::from_millis(200));
    let mid = a.status().map_err(|e| fail("STATUS", e))?;
    check!(mid.running >= 1, "stream nebezi");
    s.drain().map_err(|e| fail("DRAIN", e))?;
    let after = a.status().map_err(|e| fail("STATUS", e))?;
    let info = s.stream_info().map_err(|e| fail("STREAM_INFO", e))?;
    // 400 ms pri 48 kHz = 19 200 snimku; DMA se musi posunout aspon o ne.
    let moved = after.hw_pos.saturating_sub(before.hw_pos);
    check!(moved >= 19_200, format!("DMA se posunulo jen o {moved} snimku"));
    check!(info.played >= 19_000, format!("prehrano jen {} snimku", info.played));
    check!(!info.running, "stream po DRAIN porad bezi");
    check!(after.irqs > before.irqs || after.irqs == 0, "preruseni od radice nechodi");
    libos::println!(
        "zvuk: DMA +{moved} snimku, preruseni {} -> {}, podteceni streamu {}, zarizeni {}",
        before.irqs, after.irqs, info.underruns, after.device_underruns
    );
    s.close().map_err(|e| fail("CLOSE", e))
}

fn tone_to_wav(_: &Startup) -> TestResult {
    let a = client()?;
    a.master_volume(100).map_err(|e| fail("hlasitost", e))?;
    a.mute(false).map_err(|e| fail("ztlumeni", e))?;
    // Mono 44,1 kHz: sluzba prevede na stereo 48 kHz (prevod frekvence).
    let f = Format::new(44_100, 1, Sample::S16);
    let before = a.status().map_err(|e| fail("STATUS", e))?;
    let s = a.open(Direction::Output, f, 100_000).map_err(|e| fail("OPEN", e))?;
    let pcm = tone_pcm(1_000, 44_100, 1, 1_500, 50);
    let deadline = time::deadline_after(Duration::from_secs(20));
    s.write_all(&pcm, true, Some(deadline)).map_err(|e| fail("zapis", e))?;
    s.drain().map_err(|e| fail("DRAIN", e))?;
    let info = s.stream_info().map_err(|e| fail("STREAM_INFO", e))?;
    let st = a.status().map_err(|e| fail("STATUS", e))?;
    // Radek cte xtask (kontrola WAV): podteceni zarizeni behem tonu =
    // sluzba nestihala (zatizeny hostitel), mezery v souboru jsou pak
    // vysvetlene. Format radku neprejmenovat bez upravy xtask/src/audio.rs.
    libos::println!(
        "zvuk: ton prehran {} snimku, podteceni streamu {}, zarizeni behem tonu {}, latence {} us",
        info.played,
        info.underruns,
        st.device_underruns.saturating_sub(before.device_underruns),
        st.latency_us
    );
    // Zaznet musi vsechno (1,5 s pri 44,1 kHz).
    check!(info.played >= 66_150, format!("prehrano jen {} z 66150 snimku", info.played));
    s.close().map_err(|e| fail("CLOSE", e))
}

fn two_clients(_: &Startup) -> TestResult {
    let a = client()?;
    let b = client()?;
    let sa = a.open(Direction::Output, Format::DEVICE_DEFAULT, 200_000).map_err(|e| fail("OPEN a", e))?;
    let sb = b
        .open(Direction::Output, Format::new(22_050, 2, Sample::S16), 200_000)
        .map_err(|e| fail("OPEN b", e))?;
    let pa = tone_pcm(440, 48_000, 2, 150, 30);
    let pb = tone_pcm(660, 22_050, 2, 150, 30);
    let deadline = Some(time::deadline_after(Duration::from_secs(10)));
    sa.write_all(&pa, false, deadline).map_err(|e| fail("zapis a", e))?;
    sb.write_all(&pb, false, deadline).map_err(|e| fail("zapis b", e))?;
    sa.start().map_err(|e| fail("START a", e))?;
    sb.start().map_err(|e| fail("START b", e))?;
    sb.volume(50).map_err(|e| fail("hlasitost streamu", e))?;
    let st = a.status().map_err(|e| fail("STATUS", e))?;
    check!(st.running >= 2, format!("bezi jen {} streamu", st.running));
    a.master_volume(40).map_err(|e| fail("hlavni hlasitost", e))?;
    a.mute(true).map_err(|e| fail("ztlumit", e))?;
    let st = b.status().map_err(|e| fail("STATUS", e))?;
    check_eq!((st.master, st.muted), (40, true), "hlasitost a ztlumeni z druheho klienta");
    sa.drain().map_err(|e| fail("DRAIN a", e))?;
    sb.drain().map_err(|e| fail("DRAIN b", e))?;
    check_eq!(sb.stream_info().map_err(|e| fail("STREAM_INFO", e))?.volume, 50, "hlasitost streamu");
    a.mute(false).map_err(|e| fail("zrusit ztlumeni", e))?;
    a.master_volume(100).map_err(|e| fail("hlasitost", e))?;
    // Zavreni kanalu klienta zavre i jeho streamy.
    drop(sb);
    drop(b);
    time::sleep(Duration::from_millis(50));
    let st = a.status().map_err(|e| fail("STATUS", e))?;
    check!(st.streams <= 1, format!("po zavreni klienta zustalo {} streamu", st.streams));
    Ok(())
}

fn recording(_: &Startup) -> TestResult {
    let a = client()?;
    let st = a.status().map_err(|e| fail("STATUS", e))?;
    check!(st.input, "sluzba nehlasi vstup (druha karta s hda-duplex ma linkovy vstup)");
    // Mono 16 kHz: sluzba prevede ze stereo 48 kHz (prevod dolu, filtr).
    let f = Format::new(16_000, 1, Sample::S16);
    let s = a.open(Direction::Input, f, 100_000).map_err(|e| fail("OPEN vstup", e))?;
    s.start().map_err(|e| fail("START vstup", e))?;
    let mut got = 0u64;
    let mut buf = vec![0u8; 3_200];
    let end = time::deadline_after(Duration::from_secs(10));
    while got < 4_800 && time::now() < end {
        let _ = s.wait(Some(time::deadline_after(Duration::from_millis(200))));
        got += s.read(&mut buf) as u64;
    }
    check!(got >= 4_800, format!("za 10 s prislo jen {got} snimku nahravky"));
    let info = s.stream_info().map_err(|e| fail("STREAM_INFO", e))?;
    check!(info.running, "nahravani nebezi");
    s.stop().map_err(|e| fail("STOP", e))?;
    libos::println!("zvuk: nahrano {got} snimku 16 kHz mono, preteceni {}", info.underruns);
    s.close().map_err(|e| fail("CLOSE", e))
}

fn devices(_: &Startup) -> TestResult {
    let a = client()?;
    let list = a.devices().map_err(|e| fail("DEVICES", e))?;
    // QEMU: prvni radic s hda-output (jen vystup, do WAV), druhy
    // s hda-duplex (vystup i vstup).
    check_eq!(list.len(), 2, "pocet zvukovych karet");
    check!(list[0].active && !list[0].input, format!("prvni karta: {:?}", list[0]));
    check!(!list[1].active && list[1].input, format!("druha karta: {:?}", list[1]));
    // Prepnout vystup na druhou kartu, kratce prehrat a vratit.
    a.select(1).map_err(|e| fail("SELECT 1", e))?;
    let st = a.status().map_err(|e| fail("STATUS", e))?;
    check_eq!(st.device_index, 1, "aktivni zarizeni po vyberu");
    let s = a.open(Direction::Output, Format::DEVICE_DEFAULT, 50_000).map_err(|e| fail("OPEN", e))?;
    s.write_all(&tone_pcm(500, 48_000, 2, 100, 10), true, Some(time::deadline_after(Duration::from_secs(10))))
        .map_err(|e| fail("zapis", e))?;
    s.drain().map_err(|e| fail("DRAIN na druhe karte", e))?;
    drop(s);
    a.select(0).map_err(|e| fail("SELECT 0", e))?;
    check_eq!(a.status().map_err(|e| fail("STATUS", e))?.device_index, 0, "zpet na prvni");
    check_eq!(a.select(9), Err(Error::NotFound), "neexistujici zarizeni");
    Ok(())
}

fn bad_requests(_: &Startup) -> TestResult {
    use audioproto::client::Request;
    let ch = crate::audio::connect().map_err(|e| fail("pripojeni k audiod", e))?;
    let call = |r: Request| -> Result<(), Error> {
        ch.write(&r.encode(), vec::Vec::new())?;
        let m = ch.read(Some(time::deadline_after(Duration::from_secs(5))))?;
        audioproto::decode_reply(&m.data).map(|_| ())
    };
    check_eq!(call(Request::Start { stream: 9_999 }), Err(Error::NotFound), "cizi stream");
    check_eq!(call(Request::Close { stream: 9_999 }), Err(Error::NotFound), "cizi stream (CLOSE)");
    // Rozbita zprava: chyba, ne pad sluzby.
    ch.write(&[1, 0, 0, 0, 0, 0x80, 0xBB], vec::Vec::new()).map_err(|e| fail("zapis", e))?;
    let m = ch.read(Some(time::deadline_after(Duration::from_secs(5)))).map_err(|e| fail("odpoved", e))?;
    check_eq!(audioproto::decode_reply(&m.data).map(|_| ()), Err(Error::InvalidArgs), "zkracena zprava");
    // Ridici zprava `device` od obycejneho klienta se ignoruje (ne pad).
    ch.write(b"device", vec::Vec::new()).map_err(|e| fail("zapis", e))?;
    let m = ch.read(Some(time::deadline_after(Duration::from_secs(5)))).map_err(|e| fail("odpoved", e))?;
    check_eq!(audioproto::decode_reply(&m.data).map(|_| ()), Err(Error::NotSupported), "device od klienta");
    call(Request::Status).map_err(|e| fail("STATUS po chybach", e))?;
    Ok(())
}

/// Prikaz shellu `audio` pres termd a relaci shsess (kanal ke zvuku dostane
/// termd jako treti handle, relace vlastni pres CONNECT).
fn shell_audio(s: &Startup) -> TestResult {
    use shlang::value::Value;
    use term::ctl::Registry;
    use term::proto::{ClientMsg, ServerMsg};

    use super::shell::{Reg, start_termd_with};

    let audio = crate::audio::connect().map_err(|e| fail("pripojeni k audiod", e))?;
    let mut reg = Reg { store: Registry::default(), chan: None };
    let (proc, w) = start_termd_with(s, &mut reg, Some(audio))?;
    w.send(ClientMsg::Attach { session: 0 })?;
    let id = match w.recv()? {
        ServerMsg::Attached { session, .. } => session,
        other => return Err(format!("cekal jsem Attached: {other:?}")),
    };
    let r = (|| -> TestResult {
        w.ok(id, "audio volume 35")?;
        check_eq!(w.value(id, "audio volume")?, Value::Int(35), "hlasitost ze shellu");
        let st = w.value(id, "audio status")?;
        let Value::Record(fields) = &st else { return Err(format!("status neni zaznam: {st:?}")) };
        check!(fields.iter().any(|(k, v)| k == "device" && matches!(v, Value::Str(d) if d.starts_with("HDA "))), "status bez zarizeni");
        let t = w.value(id, "audio tone 880 --duration 300ms --level 20")?;
        let Value::Record(f) = &t else { return Err(format!("tone nevratil zaznam: {t:?}")) };
        let played = f.iter().find(|(k, _)| k == "duration").map(|(_, v)| v.clone());
        check!(matches!(played, Some(Value::Duration(ns)) if ns >= 250_000_000), format!("ton prehral {played:?}"));
        let (_, ok) = w.run(id, "audio volume 200")?;
        check!(!ok, "hlasitost 200 % mela selhat");
        w.ok(id, "audio volume 100")?;
        Ok(())
    })();
    let _ = proc.kill();
    r
}
