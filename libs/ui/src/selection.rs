//! Spolecna logika vyberu radku (tabulka i seznam), jako na ostatnich
//! systemech: klik / sipka vybere jen radek, Ctrl prepne radek (ostatni
//! zustanou), Shift vybere rozsah od kotvy, Ctrl+Shift rozsah prida
//! k vyberu. Bez vicenasobneho vyberu vzdy jen jeden radek.

use alloc::vec::Vec;

/// Novy vyber po volbe radku `k` (id `id`). `range(a, b)` vrati id radku
/// mezi `a` a `b` vcetne (v libovolnem poradi), `anchor` = radek kotvy.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pick<T: Copy + PartialEq>(cur: &[T], range: &dyn Fn(usize, usize) -> Vec<T>, anchor: Option<usize>, k: usize, id: T, ctrl: bool, shift: bool, multi: bool) -> Vec<T> {
    if !multi {
        return alloc::vec![id];
    }
    if shift && let Some(a) = anchor {
        let mut v = range(a, k);
        if ctrl {
            for &x in cur {
                if !v.contains(&x) {
                    v.push(x);
                }
            }
        }
        return v;
    }
    if ctrl {
        let mut v = cur.to_vec();
        match v.iter().position(|&x| x == id) {
            Some(p) => {
                v.remove(p);
            }
            None => v.push(id),
        }
        return v;
    }
    alloc::vec![id]
}

/// Posune se kotva? Ne pri rozsahu (Shift) s vicenasobnym vyberem, kdyz
/// uz nejaka je.
pub(crate) fn moves_anchor(shift: bool, multi: bool, has_anchor: bool) -> bool {
    !(shift && multi) || !has_anchor
}
