//! Prepinani kontextu mezi vlakny jadra.
//!
//! Ulozi se jen registry, ktere podle System V ABI musi zachovat volany
//! (rbx, rbp, r12-r15); ostatni uz ulozil prekladac pred volanim `switch`.
//! Jadro se preklada bez SSE (soft-float), takze FPU stav zatim neresime.

use core::arch::naked_asm;

/// Ulozeny kontext vlakna: jen ukazatel zasobniku, zbytek je na zasobniku.
#[repr(C)]
#[derive(Debug, Default)]
pub struct Context {
    rsp: u64,
}

impl Context {
    /// Kontext noveho vlakna: prvni `switch` na nej skonci skokem do `start`
    /// se zarovnanym zasobnikem.
    ///
    /// # Safety
    ///
    /// `stack_top` musi byt vrchol namapovaneho zasobniku zarovnany na 16 B,
    /// ktery nic jineho nepouziva.
    pub unsafe fn new(stack_top: u64, start: extern "sysv64" fn() -> !) -> Self {
        let frame = [
            0u64,                  // r15
            0,                     // r14
            0,                     // r13
            0,                     // r12
            0,                     // rbx
            0,                     // rbp (0 = konec retezu ramcu)
            start as usize as u64, // navratova adresa pro `ret` v `switch`
            0,                     // falesna navratova adresa `start` (zarovnani)
        ];
        let rsp = stack_top - (frame.len() * 8) as u64;
        // SAFETY: zapis 64 B pod vrchol zasobniku, ktery vlastnime.
        unsafe { core::ptr::copy_nonoverlapping(frame.as_ptr(), rsp as *mut u64, frame.len()) };
        Self { rsp }
    }
}

/// Ulozi aktualni kontext do `from` a prepne na `to`.
///
/// # Safety
///
/// `to` musi byt kontext vytvoreny `Context::new` nebo ulozeny touto funkci,
/// jehoz vlakno prave nebezi na zadnem CPU. Volat se zakazanymi prerusenimi.
#[unsafe(naked)]
pub unsafe extern "sysv64" fn switch(from: *mut Context, to: *const Context) {
    naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, [rsi]",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}
