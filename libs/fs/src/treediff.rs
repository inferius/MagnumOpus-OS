//! Porovnani dvou B+stromu s preskakovanim sdilenych podstromu.
//!
//! Strom je copy-on-write: klon a snapshot, ze ktereho vznikl (nebo dve
//! verze tehoz datasetu), sdileji vsechny nezmenene uzly - potomek ve
//! vnitrnim uzlu je v obou stromech tentyz `Child::Disk(BlockPtr)`.
//! Ukazatel nese kontrolni soucet uzlu a uzel obsahuje ukazatele svych
//! potomku (vcetne jejich souctu), takze shodny ukazatel = shodny cely
//! podstrom (Merkleho strom). Takovy podstrom se neprochazi.
//!
//! Oba stromy se prochazi soucasne dvema kurzory (zasobnik uzlu od korene).
//! Pozice kurzoru je bud *ukazatel* na dosud neprojity podstrom (s dolni
//! mezi klicu), nebo *polozka* listu. Krok:
//! - oba na stejnem ulozenem ukazateli -> preskocit oba (podstromy maji
//!   stejne klice, vse mensi uz je zpracovane v obou),
//! - ukazatel s mensi (nebo stejnou) dolni mezi nez pozice druheho ->
//!   sestoupit do nej (muze obsahovat mensi klice); pri stejne mezi nejdriv
//!   ten s vyssi urovni, aby se dalsi ukazatele mohly znovu potkat,
//! - dve polozky -> slevani jako u serazenych seznamu.
//!
//! Tvary stromu se mohou lisit (rozdelene/sloucene uzly): pak se sestupuje,
//! dokud se ukazatele znovu nepotkaji, nejhur az na listy - porovnani je
//! vzdy spravne, jen pomalejsi. Rozpracovane uzly v pameti (`Child::Mem`)
//! se neporovnavaji podle identity, vzdy se do nich sestupuje.
//!
//! Cena: uzly na cestach k rozdilnym klicum (plus uzly, kde se tvar lisi),
//! ne velikost stromu.

use alloc::vec::Vec;

use abi::Result;

use crate::btree::check_level;
use crate::device::BlockDevice;
use crate::layout::Key;
use crate::node::{Body, Child};
use crate::store::Store;

/// Uroven zasobniku kurzoru: kopie obsahu uzlu (rekurze potrebuje `Store`
/// znovu, cache se mezitim muze vyprazdnit).
enum Frame {
    Leaf { items: Vec<(Key, Vec<u8>)>, i: usize },
    /// `lb` = dolni mez klicu uzlu (prvni klic potomku se ignoruje - pokryva
    /// vse mensi, proto plati mez rodice), `level` = uroven uzlu.
    Internal { children: Vec<(Key, Child)>, i: usize, lb: Key, level: u8 },
}

enum Pos<'a> {
    Ptr { child: Child, lb: Key, level: u8 },
    Item { key: Key, val: &'a [u8] },
}

impl Pos<'_> {
    fn key(&self) -> Key {
        match self {
            Pos::Ptr { lb, .. } => *lb,
            Pos::Item { key, .. } => *key,
        }
    }
}

struct Cursor {
    stack: Vec<Frame>,
}

impl Cursor {
    fn new<D: BlockDevice>(s: &mut Store<D>, root: &Child) -> Result<Self> {
        let level = check_level(s.node(root)?, None)?;
        // Pomocny vnitrni uzel nad korenem: koren je pak obycejny ukazatel
        // (shodne koreny se preskoci hned).
        let top = Frame::Internal { children: alloc::vec![(Key::MIN, *root)], i: 0, lb: Key::MIN, level: level + 1 };
        Ok(Self { stack: alloc::vec![top] })
    }

    /// Odstrani vycerpane urovne (a posune rodice).
    fn normalize(&mut self) {
        while let Some(top) = self.stack.last() {
            let done = match top {
                Frame::Leaf { items, i } => *i >= items.len(),
                Frame::Internal { children, i, .. } => *i >= children.len(),
            };
            if !done {
                return;
            }
            self.stack.pop();
            self.bump();
        }
    }

    fn bump(&mut self) {
        match self.stack.last_mut() {
            Some(Frame::Leaf { i, .. }) | Some(Frame::Internal { i, .. }) => *i += 1,
            None => {}
        }
    }

    /// Posun za aktualni pozici (polozku nebo cely podstrom).
    fn advance(&mut self) {
        self.bump();
        self.normalize();
    }

    fn pos(&self) -> Option<Pos<'_>> {
        match self.stack.last()? {
            Frame::Leaf { items, i } => {
                let (k, v) = &items[*i];
                Some(Pos::Item { key: *k, val: v })
            }
            Frame::Internal { children, i, lb, level } => {
                let (k, c) = &children[*i];
                let lb = if *i == 0 { *lb } else { (*k).max(*lb) };
                Some(Pos::Ptr { child: *c, lb, level: level - 1 })
            }
        }
    }

    /// Sestoupi do podstromu na aktualni pozici (musi byt ukazatel).
    fn descend<D: BlockDevice>(&mut self, s: &mut Store<D>) -> Result<()> {
        let Some(Pos::Ptr { child, lb, level }) = self.pos() else {
            return Ok(());
        };
        let n = s.node(&child)?;
        check_level(n, Some(level))?;
        let frame = match &n.body {
            Body::Leaf(items) => Frame::Leaf { items: items.clone(), i: 0 },
            Body::Internal(children) => Frame::Internal { children: children.clone(), i: 0, lb, level },
        };
        self.stack.push(frame);
        // Prazdny list (prazdny strom) se hned vycerpa.
        self.normalize();
        Ok(())
    }
}

/// Zpracovani rozdilneho klice: (klic, stara hodnota, nova hodnota).
pub type DiffFn<'a> = dyn FnMut(&Key, Option<&[u8]>, Option<&[u8]>) -> Result<()> + 'a;

/// Zavola `f(klic, stara hodnota, nova hodnota)` pro kazdy klic, ktery je
/// jen v jednom stromu nebo ma v obou ruznou hodnotu, vzestupne podle klice.
pub fn diff<D: BlockDevice>(
    s: &mut Store<D>,
    old: &Child,
    new: &Child,
    f: &mut DiffFn<'_>,
) -> Result<()> {
    let mut a = Cursor::new(s, old)?;
    let mut b = Cursor::new(s, new)?;
    // Co udelat v kroku (rozhodnuti oddelene od vypujcek pozic).
    enum Act {
        SkipBoth,
        DescA,
        DescB,
        DescBoth,
        OnlyA,
        OnlyB,
        Both,
        Done,
    }
    loop {
        let act = match (a.pos(), b.pos()) {
            (None, None) => Act::Done,
            (Some(Pos::Ptr { .. }), None) => Act::DescA,
            (None, Some(Pos::Ptr { .. })) => Act::DescB,
            (Some(Pos::Item { key, val }), None) => {
                f(&key, Some(val), None)?;
                Act::OnlyA
            }
            (None, Some(Pos::Item { key, val })) => {
                f(&key, None, Some(val))?;
                Act::OnlyB
            }
            (Some(pa), Some(pb)) => match (&pa, &pb) {
                (Pos::Ptr { child: Child::Disk(x), .. }, Pos::Ptr { child: Child::Disk(y), .. }) if x == y => Act::SkipBoth,
                (Pos::Ptr { level: la, .. }, Pos::Ptr { level: lb, .. }) => {
                    let (ka, kb) = (pa.key(), pb.key());
                    if ka < kb {
                        Act::DescA
                    } else if kb < ka {
                        Act::DescB
                    } else if la > lb {
                        Act::DescA
                    } else if lb > la {
                        Act::DescB
                    } else {
                        Act::DescBoth
                    }
                }
                (Pos::Ptr { lb, .. }, Pos::Item { key, val }) => {
                    if lb <= key {
                        Act::DescA
                    } else {
                        // Podstrom A zacina az za polozkou B: polozka v A neni.
                        f(key, None, Some(val))?;
                        Act::OnlyB
                    }
                }
                (Pos::Item { key, val }, Pos::Ptr { lb, .. }) => {
                    if lb <= key {
                        Act::DescB
                    } else {
                        f(key, Some(val), None)?;
                        Act::OnlyA
                    }
                }
                (Pos::Item { key: ka, val: va }, Pos::Item { key: kb, val: vb }) => match ka.cmp(kb) {
                    core::cmp::Ordering::Less => {
                        f(ka, Some(va), None)?;
                        Act::OnlyA
                    }
                    core::cmp::Ordering::Greater => {
                        f(kb, None, Some(vb))?;
                        Act::OnlyB
                    }
                    core::cmp::Ordering::Equal => {
                        if va != vb {
                            f(ka, Some(va), Some(vb))?;
                        }
                        Act::Both
                    }
                },
            },
        };
        match act {
            Act::Done => return Ok(()),
            Act::SkipBoth | Act::Both => {
                a.advance();
                b.advance();
            }
            Act::DescA => a.descend(s)?,
            Act::DescB => b.descend(s)?,
            Act::DescBoth => {
                a.descend(s)?;
                b.descend(s)?;
            }
            Act::OnlyA => a.advance(),
            Act::OnlyB => b.advance(),
        }
    }
}
