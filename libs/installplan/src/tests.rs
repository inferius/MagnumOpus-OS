//! Testy planu instalace.

use super::config::Settings;
use super::*;
use diskguard::gpt::Read;

const SS: usize = 512;
const GIB: u64 = 1 << 30;

fn disk(bytes: u64) -> DiskInfo {
    DiskInfo { index: 1, model: String::from("QEMU NVMe Ctrl"), sectors: bytes / 512, sector_size: SS, live: false }
}

/// Tabulka disku s "Windows" v prvnich `win` bajtech (zbytek volny),
/// sestavena jen z hlavicek (obsah oddilu neni potreba).
fn windows_table(d: &DiskInfo, win: u64) -> Table {
    let basic = diskguard::guid(0xEBD0_A0A2, 0xB9E5, 0x4433, [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7]);
    let parts = [
        NewPartition { type_guid: ESP_TYPE, guid: [1; 16], first: 2048, last: 206_847, attrs: 0, name: "EFI system partition" },
        NewPartition { type_guid: basic, guid: [2; 16], first: 206_848, last: win / 512, attrs: 0, name: "Basic data partition" },
    ];
    table_of(d, &parts)
}

fn table_of(d: &DiskInfo, parts: &[NewPartition]) -> Table {
    let ends = gpt::build(d.sectors, SS, [9; 16], parts).unwrap();
    let mut read = |lba: u64, buf: &mut [u8]| {
        let head_end = ends.head.len() as u64 / 512;
        let tail_start = d.sectors - ends.tail.len() as u64 / 512;
        for (i, s) in buf.chunks_mut(512).enumerate() {
            let l = lba + i as u64;
            if l < head_end {
                s.copy_from_slice(&ends.head[l as usize * 512..][..512]);
            } else if l >= tail_start {
                s.copy_from_slice(&ends.tail[(l - tail_start) as usize * 512..][..512]);
            } else {
                s.fill(0);
            }
        }
        true
    };
    match gpt::read_table(d.sectors, SS, &mut read) {
        Read::Table(t) => t,
        _ => panic!("tabulka"),
    }
}

#[test]
fn empty_disk_whole_only() {
    let d = disk(64 * GIB);
    let c = choices(&d, None);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].mode, Mode::WholeDisk);
    assert!(c[0].available);
    let p = plan(&d, None, Mode::WholeDisk, [[1; 16], [2; 16], [3; 16]]).unwrap();
    assert_eq!(p.esp.0, 2048);
    assert_eq!((p.esp.1 - p.esp.0 + 1) * 512, ESP_BYTES);
    assert_eq!(p.data.0, p.esp.1 + 1);
    assert_eq!((p.data.1 + 1) % 2048, 0);
    assert!(matches!(p.gpt, GptWrite::New(_)));
    assert_eq!(p.confirm, Confirm::TypeName(String::from("QEMU NVMe Ctrl")));
    assert!(!p.other_os);
}

#[test]
fn small_disk_refused() {
    let d = disk(4 * GIB);
    assert!(!choices(&d, None)[0].available);
    assert!(plan(&d, None, Mode::WholeDisk, [[0; 16]; 3]).is_err());
}

#[test]
fn next_to_windows() {
    let d = disk(128 * GIB);
    let t = windows_table(&d, 60 * GIB);
    let c = choices(&d, Some(&t));
    let modes: Vec<(Mode, bool)> = c.iter().map(|c| (c.mode, c.available)).collect();
    assert_eq!(modes, vec![(Mode::FreeSpace, true), (Mode::ShrinkOther, false), (Mode::WholeDisk, true)]);
    assert!(c[1].detail.contains("Správě disků Windows"));
    let p = plan(&d, Some(&t), Mode::FreeSpace, [[1; 16], [2; 16], [3; 16]]).unwrap();
    assert!(p.other_os);
    assert!(p.esp.0 > 60 * GIB / 512);
    assert!(matches!(p.gpt, GptWrite::Add(_)));
    assert_eq!(p.preview.iter().filter(|l| matches!(l, Line::Keep(_))).count(), 2);
    assert!(preview_text(&p).contains("EFI jiného systému se nemění"));
    // Celý disk smaže oba oddíly Windows.
    let p = plan(&d, Some(&t), Mode::WholeDisk, [[1; 16]; 3]).unwrap();
    assert_eq!(p.preview.iter().filter(|l| matches!(l, Line::Erase(_))).count(), 2);
    // Zmenšení není.
    assert!(plan(&d, Some(&t), Mode::ShrinkOther, [[0; 16]; 3]).is_err());
}

#[test]
fn disk_map_before_after() {
    use map::SegKind::*;
    let d = disk(128 * GIB);
    let t = windows_table(&d, 60 * GIB);
    let kinds = |s: &[map::Seg]| s.iter().map(|s| s.kind).collect::<Vec<_>>();
    let now = map::now(&d, Some(&t));
    assert_eq!(kinds(&now), vec![Efi, Other, Free]);
    assert!(now[1].label.starts_with("Basic data partition"));
    // Soucet useku = pouzitelna cast disku (mezery pod 1 MiB vynechane).
    let total: u64 = now.iter().map(|s| s.bytes).sum();
    assert!(total <= d.bytes() && total > d.bytes() - (2 << 20));
    let p = plan(&d, Some(&t), Mode::FreeSpace, [[1; 16], [2; 16], [3; 16]]).unwrap();
    let after = map::after(&d, Some(&t), &p);
    assert_eq!(kinds(&after), vec![Efi, Other, OurEfi, Ours]);
    assert_eq!(after.iter().map(|s| s.new).collect::<Vec<_>>(), vec![false, false, true, true]);
    let p = plan(&d, Some(&t), Mode::WholeDisk, [[1; 16]; 3]).unwrap();
    assert_eq!(kinds(&map::after(&d, Some(&t), &p)), vec![OurEfi, Ours]);
    // Prazdny disk: jen volne misto.
    assert_eq!(kinds(&map::now(&d, None)), vec![Free]);
    // Preinstalace: data zustanou (ne nova), EFI se prepise.
    let first = plan(&d, None, Mode::WholeDisk, [[1; 16], [2; 16], [3; 16]]).unwrap();
    let t = table_of(&d, &new_parts(first.esp, first.data, &[[2; 16], [3; 16]]));
    let p = plan(&d, Some(&t), Mode::Reinstall, [[0; 16]; 3]).unwrap();
    let after = map::after(&d, Some(&t), &p);
    assert_eq!(kinds(&after), vec![OurEfi, Ours]);
    assert_eq!(after.iter().map(|s| s.new).collect::<Vec<_>>(), vec![true, false]);
}

#[test]
fn windows_disk_full() {
    let d = disk(64 * GIB);
    let t = windows_table(&d, 60 * GIB);
    let c = choices(&d, Some(&t));
    assert_eq!(c[0].mode, Mode::FreeSpace);
    assert!(!c[0].available, "{}", c[0].detail);
    assert!(plan(&d, Some(&t), Mode::FreeSpace, [[0; 16]; 3]).is_err());
}

#[test]
fn reinstall_keeps_data() {
    let d = disk(64 * GIB);
    let first = plan(&d, None, Mode::WholeDisk, [[1; 16], [2; 16], [3; 16]]).unwrap();
    let parts = new_parts(first.esp, first.data, &[[2; 16], [3; 16]]);
    let t = table_of(&d, &parts);
    let c = choices(&d, Some(&t));
    assert_eq!(c[0].mode, Mode::Reinstall);
    let p = plan(&d, Some(&t), Mode::Reinstall, [[0; 16]; 3]).unwrap();
    assert!(!p.format_data);
    assert!(matches!(p.gpt, GptWrite::Keep));
    assert_eq!(p.data, first.data);
}

#[test]
fn live_disk_refused() {
    let mut d = disk(64 * GIB);
    d.live = true;
    assert!(plan(&d, None, Mode::WholeDisk, [[0; 16]; 3]).is_err());
}

#[test]
fn texts_and_rules() {
    assert_eq!(human(512 << 20), "537 MB");
    assert_eq!(human(511_000_000_000), "511 GB");
    assert!(valid_user_name("jan_novak-2"));
    assert!(!valid_user_name("2jan"));
    assert!(!valid_user_name("Jan"));
    assert!(password_ok("dlouhe heslo"));
    assert!(password_ok("kun pes les dum"));
    assert!(!password_ok("kratke"));
    assert_eq!(Mode::from_key("volne-misto"), Some(Mode::FreeSpace));
}

#[test]
fn settings_roundtrip() {
    let s = Settings {
        language: String::from("cs"),
        keyboards: vec![String::from("cz"), String::from("us")],
        zone: String::from("Europe/Prague"),
        rtc_local: true,
        default_magnum: false,
    };
    assert_eq!(Settings::parse(&s.to_toml()), s);
    assert_eq!(Settings::parse("smeti\npasmo = \"UTC\"\n").zone, "UTC");
    assert_eq!(Settings::parse("").keyboards, vec![String::from("cz")]);
    assert_eq!(s.time_settings(), "zone = Europe/Prague\nrtc = local\n");
}

#[test]
fn proto_roundtrip() {
    use super::proto::*;
    let reqs = [
        Request::Disks,
        Request::Keyboards,
        Request::Preview { disk: 2, mode: Mode::FreeSpace },
        Request::Install {
            disk: 1,
            mode: Mode::WholeDisk,
            confirm: String::from("QEMU NVMe Ctrl"),
            settings: Settings::default().to_toml(),
            account: AccountInput {
                name: String::from("jan"),
                display: String::from("Jan Novák"),
                password: String::from("dlouhe heslo"),
                pin: String::new(),
            },
        },
    ];
    for r in reqs {
        assert_eq!(Request::decode(&r.encode()), Ok(r));
    }
    let d = disk(64 * GIB);
    let replies = [
        Reply::Failed(Error::AccessDenied, String::from("ne")),
        Reply::Disks(vec![DiskEntry {
            index: 1,
            model: d.model.clone(),
            bytes: d.bytes(),
            live: false,
            parts: vec![String::from("EFI (537 MB)")],
            choices: choices(&d, None),
        }]),
        Reply::Keyboards(vec![KeyboardEntry {
            name: String::from("cz"),
            description: String::from("Česká"),
            rows: vec![String::from("qwertz")],
            keys: vec![(21, [Some('z'), Some('Z'), None, None]), (3, [Some('ě'), Some('2'), Some('@'), None])],
        }]),
        Reply::Preview {
            text: String::from("x"),
            type_name: Some(String::from("disk")),
            other_os: true,
            lines: vec![Line::Keep(String::from("a")), Line::New(String::from("b")), Line::Erase(String::from("c"))],
            before: map::now(&d, None),
            after: map::after(&d, None, &plan(&d, None, Mode::WholeDisk, [[1; 16], [2; 16], [3; 16]]).unwrap()),
        },
        Reply::Preview { text: String::from("x"), type_name: None, other_os: false, lines: vec![], before: vec![], after: vec![] },
        Reply::Progress { step: 3, text: String::from("FS") },
        Reply::Done(String::from("hotovo")),
    ];
    for r in replies {
        assert_eq!(Reply::decode(&r.encode()), Ok(r));
    }
    assert!(Request::decode(&[9, 0, 0, 0]).is_err());
}

#[test]
fn answers_file() {
    let t = "disk = \"QEMU NVMe Ctrl\"\nzpusob = \"cely-disk\" # komentar\npotvrzeni = \"QEMU NVMe Ctrl\"\njmeno = \"jan\"\nheslo = \"dlouhe heslo\"\nklavesnice = [\"cz\", \"us\"]\n";
    let a = super::answers::parse(t).unwrap();
    assert_eq!(a.mode, Mode::WholeDisk);
    assert_eq!(a.account.name, "jan");
    assert_eq!(a.account.pin, "");
    assert_eq!(Settings::parse(&a.settings).keyboards, vec![String::from("cz"), String::from("us")]);
    assert!(super::answers::parse("disk = 1\n").is_err());
    assert!(super::answers::parse("disk = 1\nzpusob = nic\n").is_err());
}
