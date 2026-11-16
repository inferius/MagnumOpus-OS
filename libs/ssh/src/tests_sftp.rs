//! Server SFTP v3 nad FS v pameti: zapis a cteni (jako scp tam a zpet),
//! vypis slozky, prejmenovani, mazani, chyby jako stavove kody, cesty.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use abi::Error;

use crate::sftp::{Attrs, Sftp, SftpFs, resolve, status};

#[derive(Default)]
struct Mem {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
}

impl SftpFs for Mem {
    fn stat(&mut self, p: &str) -> Result<Attrs, Error> {
        if self.dirs.contains(p) {
            return Ok(Attrs { dir: true, ..Default::default() });
        }
        self.files.get(p).map(|f| Attrs { size: f.len() as u64, ..Default::default() }).ok_or(Error::NotFound)
    }
    fn list(&mut self, p: &str) -> Result<Vec<(String, Attrs)>, Error> {
        if !self.dirs.contains(p) {
            return Err(Error::NotFound);
        }
        let pre = alloc::format!("{}/", p.trim_end_matches('/'));
        Ok(self
            .files
            .iter()
            .filter_map(|(k, v)| k.strip_prefix(&pre).filter(|n| !n.contains('/')).map(|n| (String::from(n), Attrs { size: v.len() as u64, ..Default::default() })))
            .collect())
    }
    fn read(&mut self, p: &str, off: u64, len: usize) -> Result<Vec<u8>, Error> {
        let f = self.files.get(p).ok_or(Error::NotFound)?;
        let s = (off as usize).min(f.len());
        Ok(f[s..(s + len).min(f.len())].to_vec())
    }
    fn write(&mut self, p: &str, off: u64, data: &[u8]) -> Result<(), Error> {
        let f = self.files.get_mut(p).ok_or(Error::NotFound)?;
        let end = off as usize + data.len();
        if f.len() < end {
            f.resize(end, 0);
        }
        f[off as usize..end].copy_from_slice(data);
        Ok(())
    }
    fn create(&mut self, p: &str) -> Result<(), Error> {
        if p.starts_with("/system") {
            return Err(Error::AccessDenied);
        }
        self.files.insert(p.into(), Vec::new());
        Ok(())
    }
    fn truncate(&mut self, p: &str, size: u64) -> Result<(), Error> {
        self.files.get_mut(p).ok_or(Error::NotFound)?.truncate(size as usize);
        Ok(())
    }
    fn mkdir(&mut self, p: &str) -> Result<(), Error> {
        self.dirs.insert(p.into());
        Ok(())
    }
    fn remove(&mut self, p: &str) -> Result<(), Error> {
        if self.files.remove(p).is_some() || self.dirs.remove(p) { Ok(()) } else { Err(Error::NotFound) }
    }
    fn rename(&mut self, a: &str, b: &str) -> Result<(), Error> {
        let v = self.files.remove(a).ok_or(Error::NotFound)?;
        self.files.insert(b.into(), v);
        Ok(())
    }
}

fn pkt(t: u8, id: Option<u32>, body: &[u8]) -> Vec<u8> {
    let mut v = vec![t];
    if let Some(id) = id {
        v.extend_from_slice(&id.to_be_bytes());
    }
    v.extend_from_slice(body);
    let mut out = (v.len() as u32).to_be_bytes().to_vec();
    out.extend(v);
    out
}

fn s(x: &str) -> Vec<u8> {
    let mut v = (x.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(x.as_bytes());
    v
}

/// Jedna odpoved: (typ, id, telo za id).
fn reply(out: &[u8]) -> (u8, u32, Vec<u8>) {
    let len = u32::from_be_bytes(out[..4].try_into().unwrap()) as usize;
    assert_eq!(out.len(), 4 + len, "jedna odpoved");
    (out[4], u32::from_be_bytes(out[5..9].try_into().unwrap()), out[9..].to_vec())
}

fn status_of(body: &[u8]) -> u32 {
    u32::from_be_bytes(body[..4].try_into().unwrap())
}

fn session() -> (Sftp, Mem) {
    let mut fs = Mem::default();
    fs.dirs.insert("/home/alice".into());
    let mut sf = Sftp::new("/home/alice");
    let out = sf.input(&pkt(1, None, &3u32.to_be_bytes()), &mut fs).unwrap();
    assert_eq!(out[4], 2, "VERSION");
    (sf, fs)
}

#[test]
fn zapis_a_cteni_jako_scp() {
    let (mut sf, mut fs) = session();
    // OPEN zapis + vytvorit + zkratit (relativni cesta vuci domovu).
    let mut b = s("data.bin");
    b.extend_from_slice(&(2u32 | 8 | 0x10).to_be_bytes());
    b.extend_from_slice(&0u32.to_be_bytes());
    let (t, id, h) = reply(&sf.input(&pkt(3, Some(7), &b), &mut fs).unwrap());
    assert_eq!((t, id), (102, 7));
    let handle = h.clone();
    // Dva zapisy, jeden paket rozdeleny na dve casti (prijde po kouscich).
    for (i, chunk) in [&b"ahoj "[..], &b"svete"[..]].iter().enumerate() {
        let mut w = handle.clone();
        w.extend_from_slice(&((i * 5) as u64).to_be_bytes());
        w.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
        w.extend_from_slice(chunk);
        let p = pkt(6, Some(8 + i as u32), &w);
        let (a, z) = p.split_at(5);
        assert!(sf.input(a, &mut fs).unwrap().is_empty());
        let (_, _, body) = reply(&sf.input(z, &mut fs).unwrap());
        assert_eq!(status_of(&body), status::OK);
    }
    let (_, _, body) = reply(&sf.input(&pkt(4, Some(10), &handle), &mut fs).unwrap());
    assert_eq!(status_of(&body), status::OK);
    assert_eq!(fs.files.get("/home/alice/data.bin").map(Vec::as_slice), Some(&b"ahoj svete"[..]));
    // Cteni zpet, pak EOF.
    let mut b = s("/home/alice/data.bin");
    b.extend_from_slice(&1u32.to_be_bytes());
    b.extend_from_slice(&0u32.to_be_bytes());
    let (_, _, h) = reply(&sf.input(&pkt(3, Some(11), &b), &mut fs).unwrap());
    let mut rd = h.clone();
    rd.extend_from_slice(&0u64.to_be_bytes());
    rd.extend_from_slice(&1000u32.to_be_bytes());
    let (t, _, body) = reply(&sf.input(&pkt(5, Some(12), &rd), &mut fs).unwrap());
    assert_eq!(t, 103);
    assert_eq!(&body[4..], b"ahoj svete");
    let mut rd = h.clone();
    rd.extend_from_slice(&10u64.to_be_bytes());
    rd.extend_from_slice(&1000u32.to_be_bytes());
    let (t, _, body) = reply(&sf.input(&pkt(5, Some(13), &rd), &mut fs).unwrap());
    assert_eq!((t, status_of(&body)), (101, status::EOF));
}

#[test]
fn vypis_prejmenovani_mazani_a_chyby() {
    let (mut sf, mut fs) = session();
    fs.files.insert("/home/alice/a.txt".into(), b"123".to_vec());
    let (_, _, h) = reply(&sf.input(&pkt(11, Some(1), &s(".")), &mut fs).unwrap());
    let (t, _, body) = reply(&sf.input(&pkt(12, Some(2), &h), &mut fs).unwrap());
    assert_eq!(t, 104);
    assert_eq!(u32::from_be_bytes(body[..4].try_into().unwrap()), 3, ". .. a.txt");
    let (t, _, body) = reply(&sf.input(&pkt(12, Some(3), &h), &mut fs).unwrap());
    assert_eq!((t, status_of(&body)), (101, status::EOF));
    // REALPATH, RENAME, REMOVE.
    let (t, _, body) = reply(&sf.input(&pkt(16, Some(4), &s("../alice/./x/..")), &mut fs).unwrap());
    assert_eq!(t, 104);
    assert!(body.windows(11).any(|w| w == b"/home/alice"));
    let mut b = s("a.txt");
    b.extend(s("b.txt"));
    let (_, _, body) = reply(&sf.input(&pkt(18, Some(5), &b), &mut fs).unwrap());
    assert_eq!(status_of(&body), status::OK);
    assert!(fs.files.contains_key("/home/alice/b.txt"));
    let (_, _, body) = reply(&sf.input(&pkt(13, Some(6), &s("b.txt")), &mut fs).unwrap());
    assert_eq!(status_of(&body), status::OK);
    // Chyby FS jako stavove kody.
    let (t, _, body) = reply(&sf.input(&pkt(17, Some(7), &s("neni")), &mut fs).unwrap());
    assert_eq!((t, status_of(&body)), (101, status::NO_SUCH_FILE));
    let mut b = s("/system/x");
    b.extend_from_slice(&(2u32 | 8).to_be_bytes());
    b.extend_from_slice(&0u32.to_be_bytes());
    let (_, _, body) = reply(&sf.input(&pkt(3, Some(8), &b), &mut fs).unwrap());
    assert_eq!(status_of(&body), status::PERMISSION_DENIED);
    // Nezname operace a spatny handle.
    let (_, _, body) = reply(&sf.input(&pkt(20, Some(9), &s("x")), &mut fs).unwrap());
    assert_eq!(status_of(&body), status::OP_UNSUPPORTED);
    let (_, _, body) = reply(&sf.input(&pkt(4, Some(10), &s("zz")), &mut fs).unwrap());
    assert_eq!(status_of(&body), status::BAD_MESSAGE);
    // Prilis velky paket = konec kanalu.
    assert!(sf.input(&u32::MAX.to_be_bytes(), &mut fs).is_err());
}

#[test]
fn cesty() {
    assert_eq!(resolve("/home/a", "."), "/home/a");
    assert_eq!(resolve("/home/a", "x/../y"), "/home/a/y");
    assert_eq!(resolve("/home/a", "/../../etc"), "/etc");
    assert_eq!(resolve("/home/a", ".."), "/home");
    assert_eq!(resolve("/home/a", "/"), "/");
}

/// Klient SFTP (`sftp_client.rs`) proti serveru: nahrani a stazeni po
/// kouscich (jako scp), vypis, mazani, chyby jako status.
#[test]
fn klient_proti_serveru() {
    use crate::sftp_client::{CHUNK, Reply, SftpClient, eof, ok};
    let mut fs = Mem::default();
    fs.dirs.insert("/home/a".into());
    let mut srv = Sftp::new("/home/a");
    let mut c = SftpClient::new();
    let ask = |c: &mut SftpClient, srv: &mut Sftp, fs: &mut Mem, p: &[u8]| -> Reply {
        c.feed(&srv.input(p, fs).unwrap());
        c.next_reply().unwrap().expect("cela odpoved")
    };
    let init = c.init();
    assert_eq!(ask(&mut c, &mut srv, &mut fs, &init), Reply::Version(3));
    // Nahrani 100 KiB po kouscich.
    let data: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
    let (_, p) = c.open("soubor.bin", true);
    let Reply::Handle { handle, .. } = ask(&mut c, &mut srv, &mut fs, &p) else { panic!("handle") };
    for (i, ch) in data.chunks(CHUNK).enumerate() {
        let (id, p) = c.write(&handle, (i * CHUNK) as u64, ch);
        let r = ask(&mut c, &mut srv, &mut fs, &p);
        assert!(ok(&r) && r.id() == Some(id), "{r:?}");
    }
    let (_, p) = c.close(&handle);
    assert!(ok(&ask(&mut c, &mut srv, &mut fs, &p)));
    assert_eq!(fs.files["/home/a/soubor.bin"], data);
    // Stazeni zpet.
    let (_, p) = c.open("/home/a/soubor.bin", false);
    let Reply::Handle { handle, .. } = ask(&mut c, &mut srv, &mut fs, &p) else { panic!("handle") };
    let mut back = Vec::new();
    loop {
        let (_, p) = c.read(&handle, back.len() as u64, CHUNK as u32);
        match ask(&mut c, &mut srv, &mut fs, &p) {
            Reply::Data { data, .. } => back.extend(data),
            r if eof(&r) => break,
            r => panic!("{r:?}"),
        }
    }
    assert_eq!(back, data);
    // Atributy, vypis, chyby.
    let (_, p) = c.stat("soubor.bin");
    assert!(matches!(ask(&mut c, &mut srv, &mut fs, &p), Reply::Attrs { attrs, .. } if attrs.size == Some(100_000)));
    let (_, p) = c.opendir(".");
    let Reply::Handle { handle, .. } = ask(&mut c, &mut srv, &mut fs, &p) else { panic!("dir") };
    let (_, p) = c.readdir(&handle);
    assert!(matches!(ask(&mut c, &mut srv, &mut fs, &p), Reply::Name { entries, .. } if entries.iter().any(|(n, _)| n == "soubor.bin")));
    let (_, p) = c.open("/system/x", true);
    let r = ask(&mut c, &mut srv, &mut fs, &p);
    assert!(matches!(r, Reply::Status { code: status::PERMISSION_DENIED, .. }), "{r:?}");
    let (_, p) = c.remove("soubor.bin");
    assert!(ok(&ask(&mut c, &mut srv, &mut fs, &p)));
    // Rozdelena odpoved: cela az po vsech bajtech.
    let (_, p) = c.stat("/home/a");
    let out = srv.input(&p, &mut fs).unwrap();
    c.feed(&out[..3]);
    assert_eq!(c.next_reply(), Ok(None));
    c.feed(&out[3..]);
    assert!(matches!(c.next_reply(), Ok(Some(Reply::Attrs { attrs, .. })) if attrs.is_dir()));
}
