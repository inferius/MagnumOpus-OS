//! Strukturalni meze prostredku.
//!
//! Kolik pameti smi proces spotrebovat, neurcuji pevne pocty, ale rozpocet
//! jeho jobu v bajtech (`crate::job`): pamet jadra (objekty, sloty handlu,
//! registrace portu, fronty kanalu, mapovani, zasobniky vlaken) a zvlast
//! uzivatelska pamet. Rodic urci rozpocet potomka, init rozdeli pamet
//! systemu mezi sluzby. Pamet programu navic nikdy nesahne do rezervy
//! jadra (1/32 RAM, nejmene 16 MiB) - pak `NO_MEMORY`.
//!
//! Tady zustavaji jen meze dane formatem dat, ne politikou.

/// Nejvic handlu v tabulce jednoho procesu: handle ma 20 bitu indexu
/// (zbytek je generace slotu, viz `crate::object::Handle`).
pub const PROCESS_MAX_HANDLES: usize = (1 << 20) - 1;
/// Nejvic bajtu dat ve fronte jednoho konce kanalu (rizeni toku, spolu
/// s `syscall::CHANNEL_MAX_QUEUED`; pamet hlida rozpocet jobu odesilatele).
pub const CHANNEL_MAX_QUEUED_BYTES: usize = 4 * 1024 * 1024;
