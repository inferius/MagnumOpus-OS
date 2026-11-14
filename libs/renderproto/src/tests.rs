//! Testy protokolu render (`cargo test -p renderproto`).

use alloc::vec;

use crate::*;

#[test]
fn request_roundtrip() {
    let desc = bufdesc::BufDesc::linear(bufdesc::format::NV12, 64, 32, 16).unwrap();
    for m in [
        Request::Info,
        Request::Query { key: query::CAPSET, a: 4, b: 0 },
        Request::CreateContext { uapi: 4 },
        Request::CreateBuffer { mem: mem::HOST3D, flags: flags::MAPPABLE, driver_id: 9, size: 1 << 20 },
        Request::ImportBuffer { desc },
        Request::Destroy { buffer: 7 },
        Request::Bind { data: vec![1, 2, 3] },
        Request::Submit { queue: 1, wait: vec![5, 6], signal: vec![7], data: vec![1, 0, 0, 0, 2, 0, 0, 0] },
        Request::Submit { queue: 0, wait: vec![], signal: vec![1], data: vec![] },
    ] {
        assert_eq!(Request::decode(&m.encode()), Ok(m));
    }
}

#[test]
fn reply_roundtrip() {
    for m in [
        Reply::Info { class: class::VIRTIO_GPU, uapis: vec![Uapi { id: 4, version: 0, size: 160 }], shm_size: 1 << 30 },
        Reply::Query(vec![9; 32]),
        Reply::Context,
        Reply::Buffer { buffer: 3, size: 4096, map_info: 2 },
        Reply::Done { op: op::DESTROY },
        Reply::Submitted,
        Reply::Error { op: op::SUBMIT, code: Error::FenceFailed },
    ] {
        assert_eq!(Reply::decode(&m.encode()), Ok(m));
    }
}

#[test]
fn bad_requests() {
    // Neznama operace, zbytek za zpravou, neznamy priznak bufferu.
    assert_eq!(Request::decode(&[99, 0, 0, 0]), Err(Error::NotSupported));
    let mut b = Request::Destroy { buffer: 1 }.encode();
    b.push(0);
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
    let b = Request::CreateBuffer { mem: 2, flags: 1 << 9, driver_id: 0, size: 4096 }.encode();
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
    // SUBMIT: prilis mnoho plotu, data nezarovnana na 4 B a nad limit.
    let b = Request::Submit { queue: 0, wait: vec![1; MAX_FENCES + 1], signal: vec![], data: vec![] }.encode();
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
    let b = Request::Submit { queue: 0, wait: vec![], signal: vec![], data: vec![1, 2, 3] }.encode();
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
    let b = Request::Submit { queue: 0, wait: vec![], signal: vec![], data: vec![0; MAX_SUBMIT + 4] }.encode();
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
    // Popis bufferu s nenulovou rezervou.
    let mut b = Request::ImportBuffer { desc: bufdesc::BufDesc::linear(bufdesc::format::ARGB8888, 8, 8, 1).unwrap() }.encode();
    b[4 + 23] = 1;
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
}

#[test]
fn provider_messages() {
    use crate::node;
    assert_eq!(node::Request::decode(&node::Request::Open.encode()), Ok(node::Request::Open));
    for m in [node::Reply::Opened, node::Reply::Error { code: Error::NoResources }] {
        assert_eq!(node::Reply::decode(&m.encode()), Ok(m));
    }
    assert_eq!(node::Request::decode(&[7, 0, 0, 0]), Err(Error::NotSupported));
}
