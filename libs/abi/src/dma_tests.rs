//! Testy `abi::dma`: kodovani useku a slouceni stranek bez IOMMU.

use crate::dma::{Segment, merge_pages};

#[test]
fn segment_roundtrip() {
    let s = Segment { addr: 0x1_2345_6000, len: 0x3000 };
    assert_eq!(Segment::decode(&s.encode()), s);
}

#[test]
fn adjacent_pages_merge() {
    let pages = [0x1000, 0x2000, 0x5000, 0x6000, 0x7000, 0x3000];
    let segs: std::vec::Vec<_> = merge_pages(pages, 0x1000).collect();
    assert_eq!(
        segs,
        [
            Segment { addr: 0x1000, len: 0x2000 },
            Segment { addr: 0x5000, len: 0x3000 },
            Segment { addr: 0x3000, len: 0x1000 },
        ]
    );
    assert_eq!(merge_pages([], 0x1000).count(), 0);
}
