//! Limity prostredku na proces.
//!
//! Chrani jadro (a ostatni procesy) pred programem, ktery by jinak mohl
//! spotrebovat pamet jadra bez omezeni. Pamet pro objekty pameti a vlakna
//! navic nikdy nesahne do rezervy jadra (1/32 RAM, nejmene 16 MiB) - pak
//! vraci `NO_MEMORY`. Prekroceni limitu poctu vraci `NO_RESOURCES`, plna
//! fronta kanalu `SHOULD_WAIT`.
//!
//! TODO: kvoty pro skupiny procesu (job) nastavitelne rodicem.

/// Nejvic handlu v tabulce jednoho procesu.
pub const PROCESS_MAX_HANDLES: usize = 65_536;
/// Nejvic vlaken jednoho procesu (zivych i ukoncenych, na ktera jeste
/// nekdo drzi handle).
pub const PROCESS_MAX_THREADS: usize = 1024;
/// Nejvic mapovani v adresnim prostoru jednoho procesu.
pub const PROCESS_MAX_MAPPINGS: usize = 65_536;
/// Nejvic bajtu dat ve fronte jednoho konce kanalu (krome limitu
/// `CHANNEL_MAX_QUEUED` zprav).
pub const CHANNEL_MAX_QUEUED_BYTES: usize = 4 * 1024 * 1024;
