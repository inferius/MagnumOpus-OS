//! Spusteni programu (ELF) v novem procesu.
//!
//! Jadro o formatu programu nic nevi: libos vytvori prazdny proces,
//! namapuje do nej segmenty programu a zasobnik a spusti prvni vlakno se
//! startovni zpravou (`abi::bootstrap`).

use alloc::vec::Vec;

use abi::bootstrap::{self, Entry, Header};
use abi::vm::{MAP_EXECUTE, MAP_FIXED, MAP_READ, MAP_WRITE, PAGE_SIZE};
use abi::{Error, Result};

use crate::{Channel, Handle, Process, Thread, Vmo};

/// Velikost zasobniku hlavniho vlakna noveho procesu.
pub const STACK_SIZE: u64 = 256 * 1024;

/// Handle predavany novemu procesu ve startovni zprave.
pub struct StartHandle {
    pub kind: u32,
    pub arg: u32,
    pub handle: Handle,
}

impl StartHandle {
    /// Handle podle dohody s programem (`kind::USER`).
    pub fn user(handle: Handle) -> Self {
        Self {
            kind: bootstrap::kind::USER,
            arg: 0,
            handle,
        }
    }

    pub fn new(kind: u32, arg: u32, handle: Handle) -> Self {
        Self { kind, arg, handle }
    }
}

/// Spusti program `image` (ELF) jako proces `name` a preda mu handly
/// `handles` ve startovni zprave (vlastni proces doplni sam). Vraci proces
/// a konec bootstrap kanalu rodice (pro dalsi komunikaci s potomkem).
pub fn spawn(name: &str, image: &[u8], handles: Vec<StartHandle>) -> Result<(Process, Channel)> {
    let elf = elf::ElfFile::parse(image).map_err(|_| Error::Corrupted)?;
    let process = Process::create(name)?;

    for seg in elf.load_segments() {
        let seg = seg.map_err(|_| Error::Corrupted)?;
        let start = seg.vaddr & !(PAGE_SIZE - 1);
        let end = (seg.vaddr + seg.mem_size).next_multiple_of(PAGE_SIZE);
        let vmo = Vmo::create(end - start)?;
        vmo.write(seg.vaddr - start, elf.segment_data(&seg))?;
        let mut flags = MAP_READ | MAP_FIXED;
        if seg.flags & elf::PF_W != 0 {
            flags |= MAP_WRITE;
        }
        if seg.flags & elf::PF_X != 0 {
            flags |= MAP_EXECUTE;
        }
        process.map(&vmo, 0, end - start, flags, start)?;
    }

    let stack = Vmo::create(STACK_SIZE)?;
    let stack_top = process.map(&stack, 0, STACK_SIZE, MAP_READ | MAP_WRITE, 0)? + STACK_SIZE;

    let (parent_end, child_end) = Channel::create()?;
    let mut data = Vec::new();
    let header = Header {
        magic: bootstrap::MAGIC,
        version: bootstrap::VERSION,
        count: 1 + handles.len() as u32,
        _reserved: 0,
    };
    data.extend_from_slice(&header.to_bytes());
    data.extend_from_slice(
        &Entry {
            kind: bootstrap::kind::SELF_PROCESS,
            arg: 0,
        }
        .to_bytes(),
    );
    let mut all = Vec::with_capacity(1 + handles.len());
    all.push(process.handle().duplicate(None)?);
    for h in handles {
        data.extend_from_slice(
            &Entry {
                kind: h.kind,
                arg: h.arg,
            }
            .to_bytes(),
        );
        all.push(h.handle);
    }
    parent_end.write(&data, all)?;

    let thread = Thread::create(&process, name)?;
    process.start(&thread, elf.entry, stack_top, child_end.into_handle())?;
    Ok((process, parent_end))
}
