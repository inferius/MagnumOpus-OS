//! Prepinani kontextu mezi vlakny jadra.
//!
//! Ulozi se jen registry, ktere podle System V ABI musi zachovat volany
//! (rbx, rbp, r12-r15); ostatni uz ulozil prekladac pred volanim `switch`.
//!
//! Stav FPU/SSE (x87, MMX, XMM0-15, MXCSR) ma kazde vlakno vlastni a `switch`
//! ho uklada a obnovuje vzdy (FXSAVE64/FXRSTOR64, 512 B). Jadro se preklada
//! bez SSE (soft-float), takze stav mezi vstupem do jadra a prepnutim patri
//! porad programu. Nove vlakno zacina s vychozim stavem - data jineho
//! procesu nikdy neuniknou. AVX (XSAVE) zatim neni zapnute: instrukce AVX
//! skonci vyjimkou #UD.

use alloc::boxed::Box;
use core::arch::{asm, naked_asm};

use crate::arch::api::ThreadContext;

/// Oblast pro FXSAVE64 (musi byt zarovnana na 16 B).
#[repr(C, align(16))]
pub struct FpuArea([u8; 512]);

impl FpuArea {
    /// Vychozi stav po FNINIT: FCW = 0x037F (vsechny vyjimky maskovane,
    /// rozsirena presnost), MXCSR = 0x1F80 (maskovane, zaokrouhleni k nejblizsimu).
    fn initial() -> Box<Self> {
        let mut a = Box::new(Self([0; 512]));
        a.0[0..2].copy_from_slice(&0x037Fu16.to_le_bytes());
        a.0[24..28].copy_from_slice(&0x1F80u32.to_le_bytes());
        a
    }
}

/// Ulozeny kontext vlakna: ukazatel zasobniku (zbytek je na zasobniku)
/// a stav FPU/SSE. Poradi poli pouziva `switch` (offset 0 a 8).
#[repr(C)]
pub struct Context {
    rsp: u64,
    fpu: Box<FpuArea>,
}

impl Context {
    /// Ulozeny stav FPU/SSE (obraz FXSAVE, 512 B).
    pub fn fpu_bytes(&self) -> &[u8] {
        &self.fpu.0
    }
}

impl Default for Context {
    fn default() -> Self {
        Self {
            rsp: 0,
            fpu: FpuArea::initial(),
        }
    }
}

impl core::fmt::Debug for Context {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Context")
            .field("rsp", &self.rsp)
            .finish_non_exhaustive()
    }
}

impl ThreadContext for Context {
    /// Kontext noveho vlakna: prvni `switch` na nej skonci skokem do `start`
    /// se zarovnanym zasobnikem.
    ///
    /// # Safety
    ///
    /// `stack_top` musi byt vrchol namapovaneho zasobniku zarovnany na 16 B,
    /// ktery nic jineho nepouziva.
    unsafe fn new(stack_top: u64, start: extern "C" fn() -> !) -> Self {
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
        Self {
            rsp,
            fpu: FpuArea::initial(),
        }
    }
}

/// Zapne FPU a SSE na aktualnim CPU: CR0.MP=1, NE=1, EM=0, TS=0, CR4.OSFXSR
/// (FXSAVE/SSE) a OSXMMEXCPT (vyjimky SSE jako #XM), pak FNINIT.
///
/// # Safety
///
/// Volat jednou pri startu CPU.
pub unsafe fn init_cpu() {
    use super::cpu;
    const CR0_MP: u64 = 1 << 1;
    const CR0_EM: u64 = 1 << 2;
    const CR0_TS: u64 = 1 << 3;
    // NE: vyjimky x87 jako #MF (bez nej starsi zpusob pres FERR#, ktery
    // na realnem hardwaru muze zastavit CPU).
    const CR0_NE: u64 = 1 << 5;
    const CR4_OSFXSR: u64 = 1 << 9;
    const CR4_OSXMMEXCPT: u64 = 1 << 10;
    // SAFETY: FPU a SSE2 ma kazde x86_64 CPU; zapnuti meni jen to, zda
    // instrukce FPU/SSE vyvolaji vyjimku.
    unsafe {
        cpu::write_cr0((cpu::read_cr0() | CR0_MP | CR0_NE) & !(CR0_EM | CR0_TS));
        cpu::write_cr4(cpu::read_cr4() | CR4_OSFXSR | CR4_OSXMMEXCPT);
        asm!("fninit", options(nomem, nostack));
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
        // Stav FPU/SSE (rax smi volany prepsat).
        "mov rax, [rdi + 8]",
        "fxsave64 [rax]",
        "mov rax, [rsi + 8]",
        "fxrstor64 [rax]",
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
