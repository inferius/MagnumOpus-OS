//! Testy klicu: skutecny klic, otisk a podpis z OpenSSH 9.7
//! (`ssh-keygen -t ed25519`, `ssh-keygen -lf`, `ssh-keygen -Y sign -n test`
//! nad zpravou `ahoj`), klic FIDO sestaveny podle `PROTOCOL.u2f`.

use alloc::string::String;
use alloc::vec::Vec;

use abi::Error;
use base64ct::{Base64, Encoding};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256, Sha512};

use crate::keys::{Algo, PublicKey, SK_ED25519, SK_USER_PRESENT, SK_USER_VERIFIED, fingerprint};

const LINE: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFQjqHvaUYABCSypj2Ez9B8EiUxh9lgUmivprux8DUOp alice@test";
const FINGERPRINT: &str = "SHA256:Deo9ROWjLr/A0zGsrW6nGtm5Tdy5k9dl73OTIuoEgK0";
const SSHSIG: &str = "U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAgVCOoe9pRgAEJLKmPYTP0HwSJTG\
H2WBSaK+mu7HwNQ6kAAAAEdGVzdAAAAAAAAAAGc2hhNTEyAAAAUwAAAAtzc2gtZWQyNTUx\
OQAAAECSyqnI+RJpqgbZI3PY/YPz3Yx5+ogQkbXTVO4JSqY0mQNp3D+YdUmd9CwTYMtkKq\
ICWsCf+R3/jpKP+T6jm70O";

fn put(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}

fn take<'a>(b: &mut &'a [u8]) -> &'a [u8] {
    let n = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
    let v = &b[4..4 + n];
    *b = &b[4 + n..];
    v
}

#[test]
fn radek_a_otisk_jako_openssh() {
    let k = PublicKey::parse_line(LINE).unwrap();
    assert_eq!(k.algo, Algo::Ed25519);
    assert_eq!(k.comment, "alice@test");
    assert_eq!(k.fingerprint(), FINGERPRINT);
    assert_eq!(k.line(), LINE);
    // CRLF a mezery na koncich (soubor z Windows) nevadi.
    let crlf = alloc::format!("  {LINE}\r\n");
    assert_eq!(PublicKey::parse_line(&crlf).unwrap(), k);
}

#[test]
fn podpis_z_openssh_projde() {
    // SSHSIG: "SSHSIG", verze, klic, jmenny prostor, rezerva, hash, podpis.
    let raw = Base64::decode_vec(SSHSIG).unwrap();
    assert_eq!(&raw[..6], b"SSHSIG");
    let mut b = &raw[10..];
    let key = take(&mut b).to_vec();
    let ns = take(&mut b).to_vec();
    let _reserved = take(&mut b);
    let hash = take(&mut b).to_vec();
    let sig = take(&mut b).to_vec();
    assert_eq!((ns.as_slice(), hash.as_slice()), (&b"test"[..], &b"sha512"[..]));
    // Podepsana data (PROTOCOL.sshsig).
    let mut data = Vec::from(&b"SSHSIG"[..]);
    put(&mut data, b"test");
    put(&mut data, b"");
    put(&mut data, b"sha512");
    put(&mut data, &Sha512::digest(b"ahoj"));
    let k = PublicKey::from_blob(&key).unwrap();
    assert_eq!(k.fingerprint(), FINGERPRINT);
    assert_eq!(k.verify(&data, &sig, false), Ok(()));
    // Jina data, zmeneny podpis: neprojde.
    let mut other = data.clone();
    *other.last_mut().unwrap() ^= 1;
    assert_eq!(k.verify(&other, &sig, false), Err(Error::AccessDenied));
    let mut bad = sig.clone();
    *bad.last_mut().unwrap() ^= 1;
    assert_eq!(k.verify(&data, &bad, false), Err(Error::AccessDenied));
}

/// Klic FIDO a jeho podpis jako od autentikatoru (PROTOCOL.u2f).
fn sk(seed: u8) -> (SigningKey, PublicKey) {
    let s = SigningKey::from_bytes(&[seed; 32]);
    let k = PublicKey {
        algo: Algo::SkEd25519 { application: String::from("ssh:") },
        key: s.verifying_key().to_bytes(),
        comment: String::new(),
    };
    (s, k)
}

fn sk_sig(s: &SigningKey, data: &[u8], flags: u8, counter: u32) -> Vec<u8> {
    let mut m = Vec::new();
    m.extend_from_slice(&Sha256::digest(b"ssh:"));
    m.push(flags);
    m.extend_from_slice(&counter.to_be_bytes());
    m.extend_from_slice(&Sha256::digest(data));
    let mut sig = Vec::new();
    put(&mut sig, SK_ED25519.as_bytes());
    put(&mut sig, &s.sign(&m).to_bytes());
    sig.push(flags);
    sig.extend_from_slice(&counter.to_be_bytes());
    sig
}

#[test]
fn klic_fido_vyzaduje_pritomnost() {
    let (s, k) = sk(7);
    let blob = k.blob();
    assert_eq!(PublicKey::from_blob(&blob).unwrap(), k);
    assert!(k.fingerprint().starts_with("SHA256:"));
    assert_eq!(fingerprint(&blob), k.fingerprint());
    let data = b"session id a zadost";
    assert_eq!(k.verify(data, &sk_sig(&s, data, SK_USER_PRESENT, 5), false), Ok(()));
    // Bez doteku (pritomnosti) se neprihlasi, s need_uv chce i PIN.
    assert_eq!(k.verify(data, &sk_sig(&s, data, 0, 5), false), Err(Error::AccessDenied));
    assert_eq!(k.verify(data, &sk_sig(&s, data, SK_USER_PRESENT, 5), true), Err(Error::AccessDenied));
    assert_eq!(k.verify(data, &sk_sig(&s, data, SK_USER_PRESENT | SK_USER_VERIFIED, 6), true), Ok(()));
    // Podpis jinym klicem.
    let (s2, _) = sk(8);
    assert_eq!(k.verify(data, &sk_sig(&s2, data, SK_USER_PRESENT, 5), false), Err(Error::AccessDenied));
    // Radek s klicem FIDO.
    let line = alloc::format!("{} {} yubikey", SK_ED25519, Base64::encode_string(&blob));
    let p = PublicKey::parse_line(&line).unwrap();
    assert_eq!(p.comment, "yubikey");
    assert_eq!(p.key, k.key);
}

#[test]
fn spatne_a_nepodporovane_klice() {
    for line in ["", "ssh-ed25519", "ssh-ed25519 !!!", "ssh-rsa AAAAC3NzaC1lZDI1NTE5AAAAIFQjqHvaUYABCSypj2Ez9B8EiUxh9lgUmivprux8DUOp x"] {
        assert_eq!(PublicKey::parse_line(line).err(), Some(Error::InvalidArgs), "{line}");
    }
    // Volby pred typem se nepodporuji.
    let opts = alloc::format!("from=\"10.0.0.1\" {LINE}");
    assert!(PublicKey::parse_line(&opts).is_err());
    // RSA: NOT_SUPPORTED (srozumitelna chyba pro uzivatele).
    let mut rsa = Vec::new();
    put(&mut rsa, b"ssh-rsa");
    put(&mut rsa, &[1, 0, 1]);
    put(&mut rsa, &[0xC0; 256]);
    assert_eq!(PublicKey::from_blob(&rsa).err(), Some(Error::NotSupported));
    // Zbytek za klicem, kratky klic, prilis dlouhy blob.
    let k = PublicKey::parse_line(LINE).unwrap();
    let mut extra = k.blob();
    extra.push(0);
    assert!(PublicKey::from_blob(&extra).is_err());
    let mut short = Vec::new();
    put(&mut short, b"ssh-ed25519");
    put(&mut short, &[1; 31]);
    assert!(PublicKey::from_blob(&short).is_err());
    assert!(PublicKey::from_blob(&[0u8; 2000]).is_err());
}


/// Soukromy klic OpenSSH 9.x (`ssh-keygen -t ed25519 -N ""`), vygenerovany
/// jen pro tento test.
const PRIVATE: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACB7EhIo4RIjqW8DPAz7K0lIKiWaqPrPF9myB8nl4+2QvAAAAJBfq6LKX6ui
ygAAAAtzc2gtZWQyNTUxOQAAACB7EhIo4RIjqW8DPAz7K0lIKiWaqPrPF9myB8nl4+2QvA
AAAEBiqYOuHqsPulWOY2cgRUQfLsjB8DbA3AXrmAuNUxhhjnsSEijhEiOpbwM8DPsrSUgq
JZqo+s8X2bIHyeXj7ZC8AAAAC3Zla3RvckB0ZXN0AQI=
-----END OPENSSH PRIVATE KEY-----
";
const PRIVATE_PUB: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHsSEijhEiOpbwM8DPsrSUgqJZqo+s8X2bIHyeXj7ZC8 vektor@test";
/// Tentyz format sifrovany heslem souboru (aes256-ctr + bcrypt).
const PRIVATE_ENC: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABARXRGNb5
rjSM6JFj9g3YW5AAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHNHHkXn/N5ECoMt
ahejSqul9XYjHwV+QnPLuLOqTI59AAAAkA4KSaMqyjdhekQWQi6vjqY86dcWoVbZS5Q0xc
sVzQj34icYSX1iVn8HwIqOfGWySrpTaMpm5ymXxW3ao8y3qQQ+pvRky8KPdECXlvVTqhXz
dSsKXn/GStlaAj6USNDeDZBNr+erEo9Ti7RjaG4YqmcajwYkL4EiZYMpmp2sruhSvYK2Qi
WvnAROjntT7setwQ==
-----END OPENSSH PRIVATE KEY-----
";

#[test]
fn soukromy_klic_openssh() {
    use crate::keys::{ed25519_sign, parse_private};
    let (seed, pk) = parse_private(PRIVATE).unwrap();
    assert_eq!(pk.line(), PRIVATE_PUB);
    assert_eq!(pk.comment, "vektor@test");
    // Podpis seedem overi verejny klic.
    let sig = ed25519_sign(&seed, b"data");
    let mut blob = alloc::vec::Vec::new();
    for part in [crate::keys::ED25519.as_bytes(), &sig[..]] {
        blob.extend_from_slice(&(part.len() as u32).to_be_bytes());
        blob.extend_from_slice(part);
    }
    pk.verify(b"data", &blob, false).unwrap();
    // CRLF a okolni text nevadi.
    let crlf = alloc::format!("komentar\r\n{}", PRIVATE.replace('\n', "\r\n"));
    assert_eq!(parse_private(&crlf).unwrap().0, seed);
    // Sifrovany soubor: zatim ne. Poskozeny: chyba, ne panika.
    assert_eq!(parse_private(PRIVATE_ENC).err(), Some(abi::Error::NotSupported));
    assert_eq!(parse_private("nic").err(), Some(abi::Error::InvalidArgs));
    let broken = PRIVATE.replacen("BiqYOuHqsPulWOY", "BiqYOuHqsPulWOZ", 1);
    assert_ne!(broken, PRIVATE);
    assert!(parse_private(&broken).is_err());
}
