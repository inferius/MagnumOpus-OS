//! Umisteni vlaken na CPU podle tridy a kapacity jadra.
//!
//! - Realtime a Interactive: necinne jadro s nejvyssi kapacitou (vykonne).
//! - Background: na hybridnim systemu necinne usporne jadro; kdyz zadne
//!   neni volne, zustane na svem (vykonne jadro kvuli nemu neprobouzet).
//! - Normal: posledni CPU, je-li necinne (cache), jinak necinne s nejvyssi
//!   kapacitou, jinak posledni.
//! - Kradeni (necinne CPU si bere praci): usporne jadro bere Realtime
//!   a Interactive jen z fronty, kde cekaji aspon dve (vykonna jadra
//!   nestihaji). Na homogennim systemu bez omezeni.

use abi::sched::core_kind;

use crate::class::Class;

/// Pohled na jedno CPU v okamziku rozhodovani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuView {
    pub index: usize,
    pub kind: u8,
    pub capacity: u32,
    pub idle: bool,
    /// Pocet cekajicich vlaken ve fronte.
    pub queued: usize,
    /// Soucet vah trid cekajicich i bezicicho vlakna (`RunQueue::weighted_load`).
    pub load: u64,
}

/// Vybere CPU pro vlakno tridy `class`; `last` = kde bezelo naposled
/// (u noveho vlakna CPU rodice). `hybrid` = system ma vic typu jader.
pub fn select_cpu<I>(class: Class, last: usize, hybrid: bool, cpus: I) -> usize
where
    I: Iterator<Item = CpuView> + Clone,
{
    let last_view = cpus.clone().find(|c| c.index == last);
    let max_cap = cpus.clone().map(|c| c.capacity).max().unwrap_or(0);
    // Necinne CPU s nejvyssim skore; pri shode posledni (cache).
    let best_idle = |score: &dyn Fn(&CpuView) -> Option<u32>| {
        cpus.clone()
            .filter(|c| c.idle)
            .filter_map(|c| score(&c).map(|s| (s, c.index == last, c.index)))
            .max_by_key(|&(s, is_last, i)| (s, is_last, core::cmp::Reverse(i)))
            .map(|(_, _, i)| i)
    };
    match class {
        Class::Realtime | Class::Interactive => {
            if let Some(l) = last_view
                && l.idle
                && l.capacity == max_cap
            {
                return last;
            }
            if let Some(i) = best_idle(&|c| Some(c.capacity)) {
                return i;
            }
            // Vse obsazene: zustat, pokud je posledni vykonne; jinak
            // vykonne jadro s nejkratsi frontou.
            match last_view {
                Some(l) if l.capacity == max_cap || !hybrid => last,
                _ => cpus
                    .clone()
                    .filter(|c| c.capacity == max_cap)
                    .min_by_key(|c| (c.queued, c.index))
                    .map_or(last, |c| c.index),
            }
        }
        Class::Background if hybrid => {
            let efficient = |c: &CpuView| c.kind == core_kind::EFFICIENCY;
            if let Some(l) = last_view
                && l.idle
                && efficient(&l)
            {
                return last;
            }
            if let Some(i) = best_idle(&|c| efficient(c).then_some(1)) {
                return i;
            }
            match last_view {
                Some(l) if efficient(&l) => last,
                _ => cpus
                    .clone()
                    .filter(efficient)
                    .min_by_key(|c| (c.queued, c.index))
                    .map_or(last, |c| c.index),
            }
        }
        Class::Normal | Class::Background => {
            if last_view.is_some_and(|l| l.idle) {
                return last;
            }
            best_idle(&|c| Some(c.capacity)).unwrap_or(last)
        }
    }
}

/// Smi vlakno tridy `class` bezet na `c`? Na hybridnim systemu
/// Background jen na usporna jadra (jsou-li), Interactive a Realtime jen na
/// jadra s nejvyssi kapacitou.
fn allowed(class: Class, c: &CpuView, hybrid: bool, max_cap: u32, has_efficiency: bool) -> bool {
    if !hybrid {
        return true;
    }
    match class {
        Class::Realtime | Class::Interactive => c.capacity == max_cap,
        Class::Background => !has_efficiency || c.kind == core_kind::EFFICIENCY,
        Class::Normal => true,
    }
}

/// Vyvazovani mezi vytizenymi CPU: kam presunout vystridane vlakno tridy
/// `class` z CPU `me`, kde ceka dalsi prace. Zatez = soucet vah trid
/// (Interactive 15, Normal 4, Background 1) vydeleny kapacitou jadra -
/// dve Background vlakna tak nejsou "stejna prace" jako dve Interactive.
/// Presune jen, kdyz cil bude i po presunu mene zatizeny nez `me` pred
/// nim (zadne prehazovani tam a zpet).
pub fn push_target<I>(class: Class, me: usize, hybrid: bool, cpus: I) -> Option<usize>
where
    I: Iterator<Item = CpuView> + Clone,
{
    let mine = cpus.clone().find(|c| c.index == me)?;
    let max_cap = cpus.clone().map(|c| c.capacity).max().unwrap_or(0);
    let has_efficiency = cpus.clone().any(|c| c.kind == core_kind::EFFICIENCY);
    let w = class.load_weight();
    let my_load = mine.load * 1024 / mine.capacity.max(1) as u64;
    cpus.filter(|c| c.index != me && allowed(class, c, hybrid, max_cap, has_efficiency))
        .map(|c| ((c.load + w) * 1024 / c.capacity.max(1) as u64, c.index))
        .filter(|&(after, _)| after < my_load)
        .min()
        .map(|(_, i)| i)
}

/// Smi CPU `thief` vzit cekajici vlakno tridy `class` z fronty, kde ve
/// tride ceka `waiting` vlaken?
pub fn may_steal(thief: &CpuView, class: Class, waiting: usize, hybrid: bool) -> bool {
    if !hybrid || thief.kind != core_kind::EFFICIENCY {
        return true;
    }
    match class {
        Class::Realtime | Class::Interactive => waiting >= 2,
        Class::Normal | Class::Background => true,
    }
}
