//! Plot (fence): casova osa `u64` pro explicitni synchronizaci mezi
//! procesy (GPU, video, kompozitor). Popis: `docs/cs/ploty.md`.
//!
//! ```ignore
//! let fence = Fence::create(0)?;            // producent (pravo SIGNAL)
//! let shared = fence.for_waiter()?;          // spotrebitel dostane jen WAIT
//! // ... `shared` poslat kanalem ...
//! fence.signal(1)?;                          // spotrebitel: shared.wait(1, Some(termin))
//! ```
//!
//! Na plot cizi sluzby se ceka vzdy s terminem: cykly cekani mezi ploty
//! jadro nepozna. Po padu producenta (zanikl posledni handle s pravem
//! `SIGNAL`) skonci cekani na nedosazenou hodnotu `FENCE_ABANDONED`.

pub use abi::fence::{Info, WaitItem};
use abi::fence::{signal, wait as wait_flags};
use abi::syscall::DEADLINE_INFINITE;
use abi::{Error, Result, Rights};

use crate::handle::object_type;
use crate::{Handle, Port, sys};

#[derive(Debug)]
pub struct Fence(Handle);
object_type!(Fence);

/// Chyba `wait_any`/`wait_all`: kod a index polozky, ktera cekani ukoncila
/// (`None` = termin nebo preruseni).
pub type WaitError = (Error, Option<usize>);

impl Fence {
    /// Novy plot s hodnotou `initial` (handle s pravem `SIGNAL`).
    pub fn create(initial: u64) -> Result<Self> {
        sys::fence_create(initial).map(|h| Self(Handle::from_raw(h)))
    }

    /// Duplikat pro spotrebitele: jen cekani a dotaz (bez `SIGNAL`).
    pub fn for_waiter(&self) -> Result<Self> {
        self.0.duplicate(Some(Rights::BASIC)).map(Self)
    }

    /// Posune hodnotu na `value` (>= dosazene). Pravo `SIGNAL`.
    pub fn signal(&self, value: u64) -> Result<()> {
        sys::fence_signal(self.0.raw(), value, 0)
    }

    /// Trvala chyba od bodu `value` (> dosazene hodnoty). Pravo `SIGNAL`.
    pub fn fail(&self, value: u64) -> Result<()> {
        sys::fence_signal(self.0.raw(), value, signal::FAILED)
    }

    /// Dosazena hodnota a chyba.
    pub fn query(&self) -> Result<Info> {
        sys::fence_query(self.0.raw())
    }

    /// Ceka, az hodnota dosahne `value`; `deadline` v ns od startu.
    pub fn wait(&self, value: u64, deadline: Option<u64>) -> Result<()> {
        wait_any(&[(self, value)], deadline).map(|_| ()).map_err(|(e, _)| e)
    }

    /// Az bude `value` rozhodnuta (dosazena nebo chyba), port dostane paket
    /// s `key`; vysledek pak zjisti [`Self::query`].
    pub fn wait_async(&self, value: u64, port: &Port, key: u64) -> Result<()> {
        sys::fence_wait_async(self.0.raw(), value, port.handle().raw(), key)
    }
}

fn wait_with(list: &[(&Fence, u64)], flags: u64, deadline: Option<u64>) -> core::result::Result<usize, WaitError> {
    if list.is_empty() || list.len() > abi::syscall::WAIT_MANY_MAX {
        return Err((Error::InvalidArgs, None));
    }
    // Nejvys WAIT_MANY_MAX polozek bez alokace.
    let mut buf = [WaitItem::default(); abi::syscall::WAIT_MANY_MAX];
    for (slot, (f, v)) in buf.iter_mut().zip(list) {
        *slot = WaitItem::new(f.handle().raw(), *v);
    }
    sys::fence_wait(&buf[..list.len()], flags, deadline.unwrap_or(DEADLINE_INFINITE))
}

/// Ceka na prvni rozhodnutou polozku; vrati jeji index. Chyba plotu
/// (`FENCE_FAILED`, `FENCE_ABANDONED`) cekani ukonci s indexem polozky.
pub fn wait_any(list: &[(&Fence, u64)], deadline: Option<u64>) -> core::result::Result<usize, WaitError> {
    wait_with(list, 0, deadline)
}

/// Ceka na vsechny polozky (prvni chyba plotu cekani ukonci).
pub fn wait_all(list: &[(&Fence, u64)], deadline: Option<u64>) -> core::result::Result<(), WaitError> {
    wait_with(list, wait_flags::ALL, deadline).map(|_| ())
}
