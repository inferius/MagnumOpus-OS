//! Udalosti a kopie snimku: spolecny modul s `libs/mowin` (okna programu
//! Windows), tady jen jmena pro SDL. `mosdl_event` ma stejne rozlozeni
//! jako `mowin_event`; pole `window` je v `mosdl.h` `reserved` (vzdy 0,
//! SDL ma jedno okno).

pub use mowin::event::{MoEvent as MosdlEvent, copy_frame, from_input, kind};
