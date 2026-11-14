//! Procesy a vlakna.

use crate::handle::object_type;
use crate::{Handle, Vmo, sys, wait_one};
use abi::{Result, Signals};

#[derive(Debug)]
pub struct Process(Handle);
object_type!(Process);

#[derive(Debug)]
pub struct Thread(Handle);
object_type!(Thread);

impl Process {
    /// Prazdny proces (bez pameti a vlaken) v jobu `job`.
    pub fn create(job: &crate::job::Job, name: &str) -> Result<Self> {
        sys::process_create(job.handle().raw(), name).map(|h| Self(Handle::from_raw(h)))
    }

    /// Namapuje objekt pameti do adresniho prostoru procesu.
    pub fn map(&self, vmo: &Vmo, offset: u64, len: u64, flags: u32, addr: u64) -> Result<u64> {
        sys::vm_map(self.0.raw(), vmo.handle().raw(), offset, len, flags, addr)
    }

    pub fn unmap(&self, addr: u64, len: u64) -> Result<()> {
        sys::vm_unmap(self.0.raw(), addr, len)
    }

    /// Mapovani v rozsahu zmeni na rezervaci (pamet se vrati, adresy
    /// zustanou zabrane); rezervace zustanou.
    pub fn decommit(&self, addr: u64, len: u64) -> Result<()> {
        sys::vm_unmap_flags(self.0.raw(), addr, len, abi::vm::UNMAP_DECOMMIT)
    }

    /// Zarezervuje adresy bez pameti (`abi::vm::MAP_FIXED`,
    /// `abi::vm::RESERVE_GUARD`); mapuje se do ni s
    /// `abi::vm::MAP_IN_RESERVATION`.
    pub fn reserve(&self, addr: u64, len: u64, flags: u32) -> Result<u64> {
        sys::vm_reserve(self.0.raw(), addr, len, flags)
    }

    /// Zmeni prava mapovani v rozsahu (`MAP_READ | MAP_WRITE |
    /// MAP_EXECUTE`, 0 = zadny pristup).
    pub fn protect(&self, addr: u64, len: u64, prot: u32) -> Result<()> {
        sys::vm_protect(self.0.raw(), addr, len, prot)
    }

    /// Oblast (mapovani, rezervace, volne misto) na adrese `addr`.
    pub fn query(&self, addr: u64) -> Result<abi::vm::RegionInfo> {
        sys::vm_query(self.0.raw(), addr)
    }

    /// Spusti proces prvnim vlaknem; `bootstrap` (handle) se presune do
    /// noveho procesu a dostane ho v `rdi`.
    pub fn start(&self, thread: &Thread, entry: u64, stack: u64, bootstrap: Handle) -> Result<()> {
        sys::process_start(
            self.0.raw(),
            thread.0.raw(),
            entry,
            stack,
            bootstrap.into_raw(),
        )
    }

    pub fn kill(&self) -> Result<()> {
        sys::process_kill(self.0.raw())
    }

    /// Pocka na konec procesu a vrati jeho navratovy kod.
    pub fn wait_exit(&self, deadline: Option<u64>) -> Result<i64> {
        wait_one(&self.0, Signals::TERMINATED, deadline)?;
        sys::process_exit_code(self.0.raw())
    }
}

impl Thread {
    pub fn create(process: &Process, name: &str) -> Result<Self> {
        sys::thread_create(process.0.raw(), name).map(|h| Self(Handle::from_raw(h)))
    }

    /// Spusti dalsi vlakno v jiz bezicim procesu.
    pub fn start(&self, entry: u64, stack: u64, arg0: u64, arg1: u64) -> Result<()> {
        sys::thread_start(self.0.raw(), entry, stack, arg0, arg1)
    }

    pub fn wait_exit(&self, deadline: Option<u64>) -> Result<()> {
        wait_one(&self.0, Signals::TERMINATED, deadline).map(|_| ())
    }

    /// Pozastavi vlakno (pravo `MANAGE`): stoji, dokud zije vraceny token.
    /// Na skutecne zastaveni se ceka [`Thread::wait_suspended`].
    pub fn suspend(&self) -> Result<SuspendToken> {
        sys::thread_suspend(self.0.raw()).map(|h| SuspendToken(Handle::from_raw(h)))
    }

    /// Pocka, az pozastavene vlakno opravdu stoji (signal `SUSPENDED`).
    pub fn wait_suspended(&self, deadline: Option<u64>) -> Result<()> {
        wait_one(&self.0, Signals::SUSPENDED, deadline).map(|_| ())
    }

    /// Precte kontext pozastaveneho vlakna (`abi::thread::kind`, struktura
    /// podle architektury; pravo `READ`).
    pub fn read_state(&self, kind: u32, buf: &mut [u8]) -> Result<()> {
        sys::thread_state_read(self.0.raw(), kind, buf)
    }

    /// Zapise kontext pozastaveneho vlakna (pravo `WRITE`); plati od
    /// obnoveni. Neplatny kontext (adresa mimo userspace, rezervovane bity)
    /// = `INVALID_ARGS`.
    pub fn write_state(&self, kind: u32, buf: &[u8]) -> Result<()> {
        sys::thread_state_write(self.0.raw(), kind, buf)
    }
}

/// Token pozastaveni vlakna: zavreni (Drop) vlakno obnovi.
pub struct SuspendToken(Handle);

impl SuspendToken {
    pub fn handle(&self) -> &Handle {
        &self.0
    }
}
