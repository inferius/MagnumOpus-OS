//! Copy-on-write B+strom nad [`Store`].
//!
//! Funkce pracuji se stromem urcenym korenem, ktery dostanou jako parametr
//! (kazdy dataset ma vlastni strom, strom datasetu je dalsi). Kazda zmena
//! zkopiruje uzly na ceste od korene k listu (`Store::cow`) do pameti a
//! upravi predany koren; ulozeny stav zustava nedotceny az do commitu, ktery
//! rozpracovane uzly zapise (od listu ke koreni, aby rodic znal soucty
//! potomku).
//!
//! Listy se pri preplneni deli podle bajtu, vnitrni uzly podle poctu
//! potomku. Pri mazani se odstranuji prazdne uzly a sousedni podplnene
//! listy se slucuji.
//!
//! Kazdy sestup kontroluje uroven: potomek musi mit uroven o 1 nizsi nez
//! rodic. Poskozeny strom (cyklus, prilis hluboky) tak skonci chybou
//! `CORRUPTED`, ne zacyklenim nebo pretecenim zasobniku.

use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::device::BlockDevice;
use crate::layout::{Key, NODE_SIZE};
use crate::node::{Body, Child, MAX_CHILDREN, MAX_VALUE, Node};
use crate::store::Store;

/// Index potomka, jehoz podstrom muze obsahovat `key`.
fn child_index(children: &[(Key, Child)], key: &Key) -> usize {
    children
        .partition_point(|(k, _)| k <= key)
        .saturating_sub(1)
}

/// Uroven uzlu `c` s kontrolou, ze odpovida ocekavane (`None` = koren).
fn level_of<D: BlockDevice>(s: &mut Store<D>, c: &Child, expect: Option<u8>) -> Result<u8> {
    let n = s.node(c)?;
    let ok = match &n.body {
        Body::Leaf(_) => n.level == 0,
        Body::Internal(ch) => n.level > 0 && !ch.is_empty(),
    };
    if !ok || expect.is_some_and(|e| e != n.level) {
        return Err(Error::Corrupted);
    }
    Ok(n.level)
}

/// Hodnota pro `key` ve stromu s korenem `root`.
pub fn get<D: BlockDevice>(s: &mut Store<D>, root: &Child, key: &Key) -> Result<Option<Vec<u8>>> {
    let mut c = *root;
    let mut expect = None;
    loop {
        let level = level_of(s, &c, expect)?;
        match &s.node(&c)?.body {
            Body::Leaf(items) => {
                return Ok(items
                    .binary_search_by(|(k, _)| k.cmp(key))
                    .ok()
                    .map(|i| items[i].1.clone()));
            }
            Body::Internal(children) => c = children[child_index(children, key)].1,
        }
        expect = Some(level - 1);
    }
}

/// Vlozi nebo nahradi polozku ve stromu `root` (koren se zmeni).
pub fn insert<D: BlockDevice>(
    s: &mut Store<D>,
    root: &mut Child,
    key: Key,
    value: Vec<u8>,
) -> Result<()> {
    if value.len() > MAX_VALUE {
        return Err(Error::InvalidArgs);
    }
    let level = level_of(s, root, None)?;
    let (id, split) = insert_rec(s, root, key, value)?;
    *root = Child::Mem(id);
    if let Some((k, right)) = split {
        let new_root = s.new_node(
            level + 1,
            Body::Internal(vec![(Key::MIN, Child::Mem(id)), (k, Child::Mem(right))]),
        );
        *root = Child::Mem(new_root);
    }
    s.mark_changed();
    Ok(())
}

fn insert_rec<D: BlockDevice>(
    s: &mut Store<D>,
    c: &Child,
    key: Key,
    value: Vec<u8>,
) -> Result<(u64, Option<(Key, u64)>)> {
    let id = s.cow(c)?;
    let level = s.dirty_mut(id).level;
    let child = match &s.dirty_mut(id).body {
        Body::Internal(children) => {
            let i = child_index(children, &key);
            Some((i, children[i].1))
        }
        Body::Leaf(_) => None,
    };
    match child {
        None => {
            let Body::Leaf(items) = &mut s.dirty_mut(id).body else {
                unreachable!()
            };
            let at = match items.binary_search_by(|(k, _)| k.cmp(&key)) {
                Ok(i) => {
                    items[i].1 = value;
                    i
                }
                Err(i) => {
                    items.insert(i, (key, value));
                    i
                }
            };
            if Node::leaf_size(items) <= NODE_SIZE {
                return Ok((id, None));
            }
            let right = split_leaf(items, at);
            let right_key = right[0].0;
            let r = s.new_node(0, Body::Leaf(right));
            Ok((id, Some((right_key, r))))
        }
        Some((i, child)) => {
            level_of(s, &child, Some(level - 1))?;
            let (new_child, split) = insert_rec(s, &child, key, value)?;
            let Body::Internal(children) = &mut s.dirty_mut(id).body else {
                unreachable!()
            };
            let appended = split.is_some() && i + 1 == children.len();
            children[i].1 = Child::Mem(new_child);
            if let Some((k, r)) = split {
                children.insert(i + 1, (k, Child::Mem(r)));
            }
            if children.len() <= MAX_CHILDREN {
                return Ok((id, None));
            }
            // Pridavani na konec (sekvencni zapis): levy uzel nechat plny.
            let at = if appended {
                children.len() - 1
            } else {
                children.len() / 2
            };
            let right = children.split_off(at);
            let right_key = right[0].0;
            let r = s.new_node(level, Body::Internal(right));
            Ok((id, Some((right_key, r))))
        }
    }
}

/// Rozdeli preplneny list: vrati pravou cast. Kdyz se pridavalo na konec
/// (`at` = posledni polozka; sekvencni zapis), jde doprava jen nova polozka a
/// levy list zustane plny - jinak by postupne plneni nechavalo listy z
/// poloviny prazdne. Jinak hleda deleni, kde se obe casti vejdou a jsou co
/// nejvyrovnanejsi (existuje vzdy diky limitu `MAX_VALUE`).
fn split_leaf(items: &mut Vec<(Key, Vec<u8>)>, at: usize) -> Vec<(Key, Vec<u8>)> {
    if at + 1 == items.len() && Node::leaf_size(&items[..at]) <= NODE_SIZE {
        return items.split_off(at);
    }
    let total = Node::leaf_size(items);
    let mut best = 1;
    let mut best_diff = usize::MAX;
    for m in 1..items.len() {
        let left = Node::leaf_size(&items[..m]);
        let right = total - left + Node::leaf_size(&[]);
        if left <= NODE_SIZE && right <= NODE_SIZE {
            let diff = left.abs_diff(right);
            if diff < best_diff {
                best = m;
                best_diff = diff;
            }
        }
    }
    items.split_off(best)
}

/// Odstrani polozku ze stromu `root`; vraci, zda existovala.
pub fn remove<D: BlockDevice>(s: &mut Store<D>, root: &mut Child, key: &Key) -> Result<bool> {
    if get(s, root, key)?.is_none() {
        return Ok(false);
    }
    let (id, empty) = remove_rec(s, root, key)?;
    *root = Child::Mem(id);
    if empty && s.dirty_mut(id).level > 0 {
        s.free_node(&Child::Mem(id));
        *root = Child::Mem(s.new_node(0, Body::Leaf(Vec::new())));
    }
    // Koren s jedinym potomkem nahradit potomkem (strom se snizi).
    loop {
        let only = match &s.node(root)?.body {
            Body::Internal(children) if children.len() == 1 => Some(children[0].1),
            _ => None,
        };
        let Some(child) = only else { break };
        let old = *root;
        s.free_node(&old);
        *root = child;
    }
    s.mark_changed();
    Ok(true)
}

fn remove_rec<D: BlockDevice>(s: &mut Store<D>, c: &Child, key: &Key) -> Result<(u64, bool)> {
    let id = s.cow(c)?;
    let level = s.dirty_mut(id).level;
    let child = match &s.dirty_mut(id).body {
        Body::Internal(children) => {
            let i = child_index(children, key);
            Some((i, children[i].1))
        }
        Body::Leaf(_) => None,
    };
    match child {
        None => {
            let Body::Leaf(items) = &mut s.dirty_mut(id).body else {
                unreachable!()
            };
            if let Ok(i) = items.binary_search_by(|(k, _)| k.cmp(key)) {
                items.remove(i);
            }
            let empty = items.is_empty();
            Ok((id, empty))
        }
        Some((i, child)) => {
            level_of(s, &child, Some(level - 1))?;
            let (new_child, child_empty) = remove_rec(s, &child, key)?;
            if child_empty {
                s.free_node(&Child::Mem(new_child));
            }
            let Body::Internal(children) = &mut s.dirty_mut(id).body else {
                unreachable!()
            };
            if child_empty {
                children.remove(i);
            } else {
                children[i].1 = Child::Mem(new_child);
                merge_leaves(s, id, i)?;
            }
            let Body::Internal(children) = &s.dirty_mut(id).body else {
                unreachable!()
            };
            let empty = children.is_empty();
            Ok((id, empty))
        }
    }
}

/// Slouci list `i` (rozpracovany) s pravym sousedem, pokud levy zabira nejvys
/// ctvrtinu uzlu a spolecne nejvys polovinu. Brani tomu, aby po mazani
/// zustaval ridky strom (vic cteni, vic mista).
fn merge_leaves<D: BlockDevice>(s: &mut Store<D>, parent: u64, i: usize) -> Result<()> {
    let Body::Internal(children) = &s.dirty_mut(parent).body else {
        unreachable!()
    };
    let Child::Mem(left_id) = children[i].1 else {
        return Ok(());
    };
    let Some(&(_, right)) = children.get(i + 1) else {
        return Ok(());
    };
    let left_size = match &s.dirty_mut(left_id).body {
        Body::Leaf(items) => Node::leaf_size(items),
        Body::Internal(_) => return Ok(()),
    };
    if left_size > NODE_SIZE / 4 {
        return Ok(());
    }
    let right_items = match &s.node(&right)?.body {
        Body::Leaf(items) => items.clone(),
        Body::Internal(_) => return Ok(()),
    };
    if left_size + Node::leaf_size(&right_items) > NODE_SIZE / 2 {
        return Ok(());
    }
    s.free_node(&right);
    let Body::Leaf(items) = &mut s.dirty_mut(left_id).body else {
        unreachable!()
    };
    items.extend(right_items);
    let Body::Internal(children) = &mut s.dirty_mut(parent).body else {
        unreachable!()
    };
    children.remove(i + 1);
    Ok(())
}

/// Projde polozky s klici v `start..end` (vzestupne) a zavola na ne `f`;
/// `f` vraci `false` pro ukonceni. Vraci `false`, pokud se ukoncilo.
pub fn range<D: BlockDevice>(
    s: &mut Store<D>,
    root: &Child,
    start: &Key,
    end: &Key,
    f: &mut dyn FnMut(&Key, &[u8]) -> Result<bool>,
) -> Result<bool> {
    let level = level_of(s, root, None)?;
    range_rec(s, root, level, start, end, f)
}

fn range_rec<D: BlockDevice>(
    s: &mut Store<D>,
    c: &Child,
    level: u8,
    start: &Key,
    end: &Key,
    f: &mut dyn FnMut(&Key, &[u8]) -> Result<bool>,
) -> Result<bool> {
    let node = s.node(c)?.clone();
    match node.body {
        Body::Leaf(items) => {
            let from = items.partition_point(|(k, _)| k < start);
            for (k, v) in &items[from..] {
                if k >= end {
                    break;
                }
                if !f(k, v)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Body::Internal(children) => {
            let first = child_index(&children, start);
            for (i, (k, child)) in children.iter().enumerate().skip(first) {
                if i > first && k >= end {
                    break;
                }
                level_of(s, child, Some(level - 1))?;
                if !range_rec(s, child, level - 1, start, end, f)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

/// Vsechny polozky v rozsahu (pro male rozsahy: adresar, extenty souboru).
pub fn collect<D: BlockDevice>(
    s: &mut Store<D>,
    root: &Child,
    start: &Key,
    end: &Key,
) -> Result<Vec<(Key, Vec<u8>)>> {
    let mut out = Vec::new();
    range(s, root, start, end, &mut |k, v| {
        out.push((*k, v.to_vec()));
        Ok(true)
    })?;
    Ok(out)
}
