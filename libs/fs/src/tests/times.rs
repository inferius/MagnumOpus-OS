//! Casy objektu: tvar i64 sekund + u32 ns (pred rokem 1970, za rokem 2262),
//! prehrani intent logu, svazky ve starem tvaru (u64 ns).

use abi::Error;

use super::{assert_clean, new_fs, opts, remount};
use crate::layout::{INODE_SIZE, Inode, KIND_FILE, TimeFormat, Timestamp, feature, u64_at};
use crate::{Fs, Kind, MemDevice};

fn ts(secs: i64, nanos: u32) -> Timestamp {
    Timestamp { secs, nanos }
}

/// 1601-01-01 (pocatek NTFS FILETIME) a 1. 1. 3000.
const NTFS_EPOCH: i64 = -11_644_473_600;
const YEAR_3000: i64 = 32_503_680_000;

#[test]
fn inode_times_both_formats() {
    let i = Inode {
        kind: KIND_FILE,
        mtime: ts(NTFS_EPOCH, 123),
        ctime: ts(-1, 999_999_999),
        btime: ts(YEAR_3000, 5),
        atime: ts(0, 0),
        ..Inode::default()
    };
    let b = i.encode(TimeFormat::Secs);
    assert_eq!(b.len(), INODE_SIZE);
    assert_eq!(Inode::decode(&b, TimeFormat::Secs).unwrap(), i);
    // Nanosekundy mimo rozsah = poskozeny zaznam.
    let mut bad = b.clone();
    bad[112..116].copy_from_slice(&1_000_000_000u32.to_le_bytes());
    assert_eq!(Inode::decode(&bad, TimeFormat::Secs), Err(Error::Corrupted));
    // Stary tvar: u64 ns, pred 1970 se orizne na 0, ns v rezerve nejsou.
    let old = i.encode(TimeFormat::LegacyNanos);
    assert_eq!(u64_at(&old, 16), 0);
    assert_eq!(u64_at(&old, 32), u64::MAX);
    assert!(old[112..128].iter().all(|&x| x == 0));
    let back = Inode::decode(&old, TimeFormat::LegacyNanos).unwrap();
    assert_eq!(back.mtime, ts(0, 0));
}

#[test]
fn times_before_1970_and_after_2262_survive_remount_and_replay() {
    let mut fs = new_fs(4096);
    fs.set_time(ts(NTFS_EPOCH, 1));
    fs.create("/archiv", Kind::File).unwrap();
    fs.set_time(ts(YEAR_3000, 2));
    fs.write("/archiv", 0, b"stary dokument").unwrap();
    fs.commit().unwrap();
    // Zmena jen v intent logu (bez commitu): cas se musi prehrat presne.
    fs.set_time(ts(-86_400, 3));
    fs.create("/log", Kind::File).unwrap();
    fs.write("/log", 0, b"x").unwrap();
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    let a = fs.stat("/archiv").unwrap();
    assert_eq!(a.btime, ts(NTFS_EPOCH, 1));
    assert_eq!(a.mtime, ts(YEAR_3000, 2));
    let l = fs.stat("/log").unwrap();
    assert_eq!(l.btime, ts(-86_400, 3));
    assert_eq!(l.mtime, ts(-86_400, 3));
    assert_clean(&mut fs);
}

#[test]
fn new_volume_has_time_feature_and_root_time() {
    let mut o = opts("cas");
    o.time = ts(1_794_054_615, 7);
    let dev = Fs::format(MemDevice::new(4096), &o).unwrap();
    let mut fs = Fs::mount(dev).unwrap();
    assert_eq!(fs.test_store().sb.features_incompat & feature::INCOMPAT_TIME_SECS, feature::INCOMPAT_TIME_SECS);
    assert_eq!(fs.stat("/").unwrap().btime, ts(1_794_054_615, 7));
    assert_clean(&mut fs);
}

/// Svazek ve starem tvaru casu (z doby pred `INCOMPAT_TIME_SECS`).
fn legacy_fs() -> Fs<MemDevice> {
    let mut fs = new_fs(4096);
    // Koren ma cas 0 - stejne bajty v obou tvarech.
    fs.test_store().sb.features_incompat &= !feature::INCOMPAT_TIME_SECS;
    fs.create("/stary", Kind::Dir).unwrap();
    fs.commit().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.test_store().time_format(), TimeFormat::LegacyNanos);
    fs
}

#[test]
fn legacy_volume_keeps_old_time_format() {
    let mut fs = legacy_fs();
    fs.set_time(ts(1_700_000_000, 42));
    fs.create("/a", Kind::File).unwrap();
    fs.write("/a", 0, b"data").unwrap();
    fs.commit().unwrap();
    // Cas pred 1970 se ve starem tvaru orizne (jinak by se nevesel).
    fs.set_time(ts(-5, 0));
    fs.create("/b", Kind::File).unwrap();
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.test_store().time_format(), TimeFormat::LegacyNanos);
    assert_eq!(fs.stat("/a").unwrap().mtime, ts(1_700_000_000, 42));
    assert_eq!(fs.stat("/b").unwrap().btime, ts(0, 0));
    assert_clean(&mut fs);
}

#[test]
fn set_times_on_path_and_handle_survive_replay() {
    use crate::{SetTimes, access, share};
    let mut fs = new_fs(4096);
    fs.set_time(ts(1_800_000_000, 0));
    fs.create("/d", Kind::Dir).unwrap();
    fs.create("/d/f", Kind::File).unwrap();
    fs.create("/d/g", Kind::File).unwrap();
    fs.commit().unwrap();
    fs.set_time(ts(1_800_000_100, 0));
    // Kopie ze stareho archivu: puvodni casy vcetne roku 1601.
    let t = SetTimes { mtime: Some(ts(NTFS_EPOCH, 9)), btime: Some(ts(-1, 1)), atime: None };
    fs.set_times("/d/f", t).unwrap();
    let h = fs.open("/d/g", access::READ | access::WRITE, share::ALL).unwrap();
    fs.set_times_h(h, SetTimes { atime: Some(ts(5, 5)), ..SetTimes::default() }).unwrap();
    fs.close(h).unwrap();
    // Handle bez prava zapisu casy nemeni.
    let ro = fs.open("/d/f", access::READ, share::ALL).unwrap();
    assert_eq!(fs.set_times_h(ro, t).err(), Some(Error::AccessDenied));
    fs.close(ro).unwrap();
    fs.fsync().unwrap();
    // Jen z intent logu.
    let mut fs = remount(fs);
    let f = fs.stat("/d/f").unwrap();
    assert_eq!(f.mtime, ts(NTFS_EPOCH, 9));
    assert_eq!(f.btime, ts(-1, 1));
    // ctime = okamzik zmeny, nastavit ho nejde.
    assert_eq!(f.ctime, ts(1_800_000_100, 0));
    let (_, g) = fs.test_inode("/d/g").unwrap();
    assert_eq!(g.atime, ts(5, 5));
    assert_eq!(g.mtime, ts(1_800_000_000, 0));
    // Adresar jde nastavit pres cestu.
    fs.set_times("/d", SetTimes { btime: Some(ts(0, 1)), ..SetTimes::default() }).unwrap();
    assert_eq!(fs.stat("/d").unwrap().btime, ts(0, 1));
    assert_eq!(fs.set_times("/neni", t).err(), Some(Error::NotFound));
    assert_clean(&mut fs);
}
