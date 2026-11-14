//! Prepinani kontextu mezi vlakny jadra.
//!
//! Ulozi se jen registry, ktere podle System V ABI musi zachovat volany
//! (rbx, rbp, r12-r15); ostatni uz ulozil prekladac pred volanim `switch`.
//!
//! Stav FPU/SIMD ma kazde vlakno vlastni a `switch` ho uklada a obnovuje
//! vzdy: s XSAVE (CPUID.1:ECX.XSAVE) obraz XSAVE ve standardnim formatu
//! pro casti zapnute v XCR0 (x87, SSE, AVX, AVX-512 podle CPU, viz
//! [`XCR0_WANTED`]), jinak obraz FXSAVE (512 B). Velikost oblasti je stejna
//! pro vsechna vlakna a zjisti ji `init_cpu` na bootovacim CPU (CPUID 0xD).
//! Jadro se preklada bez SSE (soft-float), takze stav mezi vstupem do jadra
//! a prepnutim patri porad programu. Nove vlakno zacina s vychozim stavem -
//! data jineho procesu nikdy neuniknou.

use core::alloc::Layout;
use core::arch::{asm, naked_asm};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};

use crate::arch::api::ThreadContext;

/// Casti stavu, ktere jadro v XCR0 zapne, kdyz je CPU umi: x87 (0),
/// SSE (1), AVX (2), AVX-512 opmask (5), ZMM_Hi256 (6), Hi16_ZMM (7).
/// MPX (3, 4) je zastarale, AMX (17, 18; 8 KiB na vlakno) jen na vyzadani
/// procesu - zatim ne. Bity 0 a 1 musi byt vzdy.
pub const XCR0_WANTED: u64 = 0b1110_0111;

/// Jak se uklada stav FPU (zjisti `init_cpu` na BSP): 0 = FXSAVE,
/// 1 = XSAVE, 2 = XSAVEOPT.
static FP_MODE: AtomicU8 = AtomicU8::new(0);
/// Zapnute casti (XCR0); 0 = bez XSAVE.
static XCR0: AtomicU64 = AtomicU64::new(0);
/// Velikost oblasti stavu FPU (B).
static FP_LEN: AtomicUsize = AtomicUsize::new(512);

/// Velikost ulozeneho stavu FPU vlakna (B): obraz XSAVE nebo FXSAVE.
pub fn fp_len() -> usize {
    FP_LEN.load(Ordering::Relaxed)
}

/// Zapnute casti XSAVE (XCR0); 0 = CPU XSAVE nema, stav je obraz FXSAVE.
pub fn xcr0() -> u64 {
    XCR0.load(Ordering::Relaxed)
}

/// Oblast stavu FPU zarovnana na 64 B (XSAVE to vyzaduje) o velikosti
/// [`fp_len`].
pub struct FpuArea {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: oblast vlastni jen kontext vlakna; pristup z jineho CPU jen se
// zamky planovace (vlakno nebezi).
unsafe impl Send for FpuArea {}

impl FpuArea {
    /// Vychozi stav: FCW = 0x037F (vsechny vyjimky maskovane, rozsirena
    /// presnost), MXCSR = 0x1F80 (maskovane, zaokrouhleni k nejblizsimu);
    /// hlavicka XSAVE nulova (XSTATE_BV = 0: ostatni casti ve vychozim
    /// stavu, XRSTOR je vynuluje).
    pub fn initial() -> Self {
        let mut a = Self::zeroed(fp_len());
        let b = a.bytes_mut();
        b[0..2].copy_from_slice(&0x037Fu16.to_le_bytes());
        b[24..28].copy_from_slice(&0x1F80u32.to_le_bytes());
        a
    }

    /// Nulova oblast dane velikosti (zarovnana na 64 B).
    pub fn zeroed(len: usize) -> Self {
        let layout = Layout::from_size_align(len, 64).expect("velikost oblasti FPU");
        // SAFETY: nenulova velikost (nejmene 512 B).
        let p = unsafe { alloc::alloc::alloc_zeroed(layout) };
        let ptr = NonNull::new(p).unwrap_or_else(|| alloc::alloc::handle_alloc_error(layout));
        Self { ptr, len }
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `len` B alokovanych v `zeroed`, vlastnime je.
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: viz `bytes`, vylucny pristup pres `&mut`.
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }

    /// Ulozi stav FPU tohoto CPU do oblasti.
    ///
    /// # Safety
    ///
    /// Vola vlakno, kteremu registry FPU patri (jadro je nepouziva).
    pub unsafe fn save(&mut self) {
        let p = self.ptr.as_ptr();
        // SAFETY: oblast ma `fp_len` B a je zarovnana na 64 B; instrukce
        // jen ctou registry FPU. eax:edx = vsechny casti zapnute v XCR0.
        unsafe {
            if xcr0() != 0 {
                asm!("xsave64 [{}]", in(reg) p, in("eax") u32::MAX, in("edx") u32::MAX, options(nostack));
            } else {
                asm!("fxsave64 [{}]", in(reg) p, options(nostack));
            }
        }
    }

    /// Nahraje stav FPU z oblasti do registru tohoto CPU.
    ///
    /// # Safety
    ///
    /// Obsah prosel kontrolou (`ustate::check_fp`) nebo ho ulozilo `save`;
    /// vola vlakno, ktere se vraci do userspace.
    pub unsafe fn load(&self) {
        let p = self.ptr.as_ptr();
        // SAFETY: predano volajicim (rezervovane bity nulove, jinak #GP).
        unsafe {
            if xcr0() != 0 {
                asm!("xrstor64 [{}]", in(reg) p, in("eax") u32::MAX, in("edx") u32::MAX, options(nostack, readonly));
            } else {
                asm!("fxrstor64 [{}]", in(reg) p, options(nostack, readonly));
            }
        }
    }
}

impl Drop for FpuArea {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.len, 64).expect("velikost oblasti FPU");
        // SAFETY: alokovano v `zeroed` se stejnym layoutem.
        unsafe { alloc::alloc::dealloc(self.ptr.as_ptr(), layout) };
    }
}

/// Ulozeny kontext vlakna: ukazatel zasobniku (zbytek je na zasobniku)
/// a stav FPU. Poradi poli pouziva `switch` (offset 0 = rsp, 8 = ukazatel
/// oblasti FPU).
#[repr(C)]
pub struct Context {
    rsp: u64,
    fpu: FpuArea,
}

impl Context {
    /// Ulozeny stav FPU (obraz XSAVE nebo FXSAVE, [`fp_len`] B).
    pub fn fpu_bytes(&self) -> &[u8] {
        self.fpu.bytes()
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

/// Zapne FPU, SSE a XSAVE na aktualnim CPU: CR0.MP=1, NE=1, EM=0, TS=0,
/// CR4.OSFXSR (FXSAVE/SSE), OSXMMEXCPT (vyjimky SSE jako #XM), s XSAVE
/// i CR4.OSXSAVE a XCR0 = [`XCR0_WANTED`] & co CPU umi; pak FNINIT.
/// Bootovaci CPU (`bsp`) zjisti format a velikost oblasti stavu; ostatni
/// CPU pouziji stejne XCR0 (symetricky system).
///
/// # Safety
///
/// Volat jednou pri startu CPU, na BSP pred vytvorenim prvniho kontextu.
pub unsafe fn init_cpu(bsp: bool) {
    use super::cpu;
    const CR0_MP: u64 = 1 << 1;
    const CR0_EM: u64 = 1 << 2;
    const CR0_TS: u64 = 1 << 3;
    // NE: vyjimky x87 jako #MF (bez nej starsi zpusob pres FERR#, ktery
    // na realnem hardwaru muze zastavit CPU).
    const CR0_NE: u64 = 1 << 5;
    const CR4_OSFXSR: u64 = 1 << 9;
    const CR4_OSXMMEXCPT: u64 = 1 << 10;
    const CR4_OSXSAVE: u64 = 1 << 18;
    let xsave = cpu::cpuid(1, 0).ecx & (1 << 26) != 0;
    // SAFETY: FPU a SSE2 ma kazde x86_64 CPU; zapnuti meni jen to, zda
    // instrukce FPU/SSE vyvolaji vyjimku. XSETBV jen s bity, ktere CPU
    // hlasi v CPUID 0xD (jinak #GP), a vzdy s x87 a SSE.
    unsafe {
        cpu::write_cr0((cpu::read_cr0() | CR0_MP | CR0_NE) & !(CR0_EM | CR0_TS));
        let mut cr4 = cpu::read_cr4() | CR4_OSFXSR | CR4_OSXMMEXCPT;
        if xsave {
            cr4 |= CR4_OSXSAVE;
        }
        cpu::write_cr4(cr4);
        if xsave {
            let mask = if bsp {
                let d = cpu::cpuid(0xD, 0);
                ((u64::from(d.eax) | (u64::from(d.edx) << 32)) & XCR0_WANTED) | 0b11
            } else {
                xcr0()
            };
            asm!("xsetbv", in("ecx") 0u32, in("eax") mask as u32, in("edx") (mask >> 32) as u32, options(nomem, nostack));
            if bsp {
                // EBX = velikost obrazu pro prave zapnute XCR0.
                let len = cpu::cpuid(0xD, 0).ebx as usize;
                let opt = cpu::cpuid(0xD, 1).eax & 1 != 0;
                XCR0.store(mask, Ordering::Relaxed);
                FP_LEN.store(len.max(576), Ordering::Relaxed);
                FP_MODE.store(if opt { 2 } else { 1 }, Ordering::Relaxed);
            }
        }
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
        // Stav FPU (rax, rcx, rdx a r8 smi volany prepsat). XSAVE(OPT)
        // a XRSTOR s eax:edx = vsechny casti XCR0.
        "mov rcx, [rdi + 8]",
        "mov r8, [rsi + 8]",
        "movzx eax, byte ptr [rip + {mode}]",
        "test eax, eax",
        "jz 3f",
        "cmp eax, 2",
        "mov eax, -1",
        "mov edx, -1",
        "jne 2f",
        "xsaveopt64 [rcx]",
        "jmp 4f",
        "2:",
        "xsave64 [rcx]",
        "4:",
        "xrstor64 [r8]",
        "jmp 5f",
        "3:",
        "fxsave64 [rcx]",
        "fxrstor64 [r8]",
        "5:",
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
        mode = sym FP_MODE,
    )
}
