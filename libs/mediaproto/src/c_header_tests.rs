//! Hlavicka protokolu v C (`user/ffdec/src/proto.h`) musi sedet s timto
//! crate: cisla operaci, udalosti, zdroje a limity.

extern crate std;

use std::string::String;

use crate::{MAX_POOL, MAX_STREAMS, TAG_REPLY, ev, op, source};

fn header() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../user/ffdec/src/proto.h")).unwrap()
}

fn define(h: &str, name: &str) -> u64 {
    let line = h.lines().find(|l| l.starts_with(&std::format!("#define {name} "))).unwrap_or_else(|| panic!("{name} chybi"));
    let v = line.split_whitespace().nth(2).unwrap().trim_end_matches('u');
    match v.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).unwrap(),
        None => v.parse().unwrap(),
    }
}

#[test]
fn proto_h_matches() {
    let h = header();
    for (name, v) in [
        ("OP_SELECT", op::SELECT),
        ("OP_POOL", op::POOL),
        ("OP_AUDIO", op::AUDIO),
        ("OP_PLAY", op::PLAY),
        ("OP_PAUSE", op::PAUSE),
        ("OP_SEEK", op::SEEK),
        ("OP_RELEASE", op::RELEASE),
        ("OP_CLOSE", op::CLOSE),
        ("OP_PROBE", op::PROBE),
        ("EV_FRAME", ev::FRAME),
        ("EV_AUDIO_BASE", ev::AUDIO_BASE),
        ("EV_END", ev::END),
        ("EV_ERROR", ev::ERROR),
        ("TAG_REPLY", TAG_REPLY),
        ("SRC_READ_AT", source::op::READ_AT),
        ("SRC_SIZE", source::op::SIZE),
        ("SRC_MAX_READ", source::MAX_READ),
    ] {
        assert_eq!(define(&h, name), v as u64, "{name}");
    }
    assert_eq!(define(&h, "MAX_POOL"), MAX_POOL as u64);
    assert_eq!(define(&h, "MAX_STREAMS"), MAX_STREAMS as u64);
    assert_eq!(define(&h, "FOURCC_NV12"), bufdesc::format::NV12 as u64);
    assert_eq!(define(&h, "BUFDESC_LEN"), bufdesc::BufDesc::linear(bufdesc::format::NV12, 2, 2, 16).unwrap().encode().len() as u64);
    assert_eq!(define(&h, "KIND_VIDEO"), crate::Kind::Video as u64);
    assert_eq!(define(&h, "KIND_AUDIO"), crate::Kind::Audio as u64);
    assert_eq!(define(&h, "KIND_SUBTITLE"), crate::Kind::Subtitle as u64);
}
