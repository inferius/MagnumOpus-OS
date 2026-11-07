//! Testy protokolu render uzlu (`cargo test -p gpuproto`).

use alloc::vec;

use crate::*;

#[test]
fn request_roundtrip() {
    for m in [
        Request::Info,
        Request::GetCapset { id: 4, version: 0 },
        Request::CreateContext { capset: 4 },
        Request::CreateBlob {
            mem: mem::HOST3D,
            flags: flags::MAPPABLE,
            blob_id: 0,
            size: 1 << 20,
        },
        Request::DestroyBlob { resource: 7 },
        Request::ScanoutBlob { resource: 7, w: 64, h: 64, format: 2, stride: 256 },
        Request::Submit {
            ring_idx: 0,
            fence: true,
            data: vec![1, 0, 0, 0, 2, 0, 0, 0],
        },
    ] {
        assert_eq!(Request::decode(&m.encode()), Ok(m));
    }
}

#[test]
fn reply_roundtrip() {
    for m in [
        Reply::Info {
            capsets: vec![Capset { id: 4, version: 0, size: 160 }],
            shm_size: 256 << 20,
        },
        Reply::Capset(vec![1, 2, 3]),
        Reply::Context,
        Reply::Blob { resource: 3, size: 65536, map_info: 1 },
        Reply::Done { op: op::DESTROY_BLOB },
        Reply::Submitted,
        Reply::Error { op: op::SUBMIT, code: Error::InvalidArgs },
    ] {
        assert_eq!(Reply::decode(&m.encode()), Ok(m));
    }
}

#[test]
fn submit_limits() {
    let enc = |n: usize| Request::Submit { ring_idx: 0, fence: false, data: vec![0; n] }.encode();
    // Prazdny proud, proud mimo nasobek 4 a prilis velky proud neprojdou.
    assert_eq!(Request::decode(&enc(0)), Err(Error::InvalidArgs));
    assert_eq!(Request::decode(&enc(6)), Err(Error::InvalidArgs));
    assert_eq!(Request::decode(&enc(MAX_SUBMIT + 4)), Err(Error::InvalidArgs));
    assert!(Request::decode(&enc(MAX_SUBMIT)).is_ok());
    // Plot jen 0 nebo 1.
    let mut b = enc(4);
    b[5] = 2;
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
}

#[test]
fn bad_messages() {
    // Neznamy priznak blobu, data navic, neznama operace, kratka zprava.
    let b = Request::CreateBlob { mem: 2, flags: 0x100, blob_id: 0, size: 4096 }.encode();
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
    let mut b = Request::Info.encode();
    b.push(0);
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
    assert_eq!(Request::decode(&[77, 0, 0, 0]), Err(Error::NotSupported));
    assert_eq!(Request::decode(&[2, 0]), Err(Error::InvalidArgs));
    assert_eq!(op_of(&[2, 0, 0, 0, 9]), 2);
    assert_eq!(op_of(&[2]), 0);
    // Prilis mnoho capsetu.
    let mut b = Reply::Info { capsets: vec![], shm_size: 0 }.encode();
    b[12..16].copy_from_slice(&1000u32.to_le_bytes());
    assert_eq!(Reply::decode(&b), Err(Error::InvalidArgs));
}
