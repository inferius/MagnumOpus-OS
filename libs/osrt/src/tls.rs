//! Bloky TLS vlaken (nativni ELF TLS, model local-exec).
//!
//! Prekladac adresuje promenne `#[thread_local]` relativne k registru
//! ukazatele vlakna (TP): x86_64 `FS_BASE`, aarch64 `TPIDR_EL0`. Jadro TP
//! uklada pri prepnuti a nastavuje ho `THREAD_SET_POINTER`. Kazde vlakno
//! dostane vlastni blok: sablona z programu (`.tdata` + `.tbss`, linker
//! skript `user/user-std.ld` je dava do jedne sekce, takze cela sablona je
//! v souboru) a kolem ni TCB podle ABI architektury:
//!
//! ```text
//! x86_64 (varianta II):  [ data TLS | TCB: ukazatel sam na sebe ]
//!                                     ^ TP
//! aarch64 (varianta I):  [ TCB 16 B | data TLS ]
//!                        ^ TP
//! ```
//!
//! Posun promennych spocital linker z `p_memsz` a `p_align` segmentu
//! PT_TLS; tady se musi pouzit stejna velikost a zarovnani.

use alloc::alloc::{Layout, alloc_zeroed, dealloc};
use core::ptr;

unsafe extern "C" {
    /// Zacatek sablony TLS (symboly z `user/user-std.ld`).
    static __tls_template_start: u8;
    /// Velikost sablony (`p_memsz`); hodnota = adresa symbolu.
    static __tls_template_size: u8;
    /// Zarovnani segmentu PT_TLS (`p_align`); hodnota = adresa symbolu.
    static __tls_align: u8;
}

/// Sablona TLS programu: (adresa, velikost, zarovnani).
fn template() -> (*const u8, usize, usize) {
    // Symboly definuje linker skript; pouzivaji se jen jejich adresy.
    (
        &raw const __tls_template_start,
        (&raw const __tls_template_size) as usize,
        ((&raw const __tls_align) as usize).max(1),
    )
}

/// Velikost TCB (x86_64: ukazatel sam na sebe + rezerva; aarch64: 16 B
/// vyhrazenych ABI).
const TCB_SIZE: usize = 16;

/// Blok TLS jednoho vlakna.
pub struct Block {
    base: *mut u8,
    layout: Layout,
    tp: u64,
}

// SAFETY: blok je jen pamet; patri vlaknu, ktere ho pouziva, a uvolni ho
// ten, kdo vlakno pripojil (`join`).
unsafe impl Send for Block {}

impl Block {
    /// Novy blok se zkopirovanou sablonou; `None` = malo pameti.
    pub fn new() -> Option<Self> {
        // Posuny musi presne odpovidat linkeru (lld): zarovnani `p_align`
        // segmentu, ne vetsi. Blok jako celek se zarovna aspon na 16 B.
        let (start, size, p_align) = template();
        let (data_off, tp_off, total) = if cfg!(target_arch = "x86_64") {
            // Data konci na TP: TP - zarovnane(p_memsz, p_align); TCB za TP.
            let data_len = size.next_multiple_of(p_align);
            (0, data_len, data_len + TCB_SIZE)
        } else {
            // TCB 16 B na TP, data od TP + zarovnane(16, p_align).
            let data_off = TCB_SIZE.next_multiple_of(p_align);
            (data_off, 0, data_off + size)
        };
        let layout = Layout::from_size_align(total.max(1), p_align.max(TCB_SIZE)).ok()?;
        // SAFETY: nenulova velikost.
        let base = unsafe { alloc_zeroed(layout) };
        if base.is_null() {
            return None;
        }
        // SAFETY: `base..base+total` je nase pamet; sablona ma `size` bajtu
        // a vejde se od `data_off` (spocitano vyse).
        unsafe {
            ptr::copy_nonoverlapping(start, base.add(data_off), size);
        }
        let tp = base as u64 + tp_off as u64;
        if cfg!(target_arch = "x86_64") {
            // `mov rax, fs:0` musi vratit TP (adresa promennych pres TP).
            // SAFETY: TCB lezi v bloku za daty.
            unsafe { (tp as *mut u64).write(tp) };
        }
        Some(Self { base, layout, tp })
    }

    /// Hodnota registru ukazatele vlakna pro tento blok.
    pub fn thread_pointer(&self) -> u64 {
        self.tp
    }

    /// Nastavi blok aktualnimu vlaknu.
    pub fn activate(&self) {
        if libos::sys::thread_set_pointer(self.tp).is_err() {
            crate::debug_log("osrt: nelze nastavit ukazatel vlakna (TLS)");
            crate::abort();
        }
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        // SAFETY: `base` vratila `alloc_zeroed` s timto layoutem.
        unsafe { dealloc(self.base, self.layout) };
    }
}

/// TLS hlavniho vlakna (blok zije do konce procesu).
pub(crate) fn init_main_thread() {
    let Some(block) = Block::new() else {
        crate::debug_log("osrt: malo pameti pro TLS hlavniho vlakna");
        crate::abort();
    };
    block.activate();
    core::mem::forget(block);
}
