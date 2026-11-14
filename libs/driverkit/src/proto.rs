//! Spolecne zpravy, kterym rozumi kazdy ovladac na driverkitu.
//!
//! Zpravy klienta zacinajici `dk:` zpracuje smycka ([`crate::run`]) sama,
//! ovladac je nedostane:
//!
//! | zprava       | odpoved / ucinek                                              |
//! |--------------|---------------------------------------------------------------|
//! | `dk:ping`    | `dk:pong <jmeno>` (ovladac zije, smycka udalosti bezi)         |
//! | `dk:info`    | `dk:info <jmeno> <adresa PCI> <vendor>:<device>`               |
//! | `dk:restart` | ovladac zastavi zarizeni a skonci kodem [`EXIT_RESTART`]; spravce zarizeni ho hned spusti znovu (nepocita se jako pad) |
//! | `dk:crash`   | jako pad ovladace (kod [`EXIT_CRASH`]): bez zastaveni a uvolneni prostredku (zarizeni zastavi jadro), spravce ho obnovi jako po padu; pro testy obnovy |

/// Navratovy kod "restart na zadost" (spravce zarizeni spusti ovladac hned
/// znovu, nepocita ho do limitu padu).
pub const EXIT_RESTART: i64 = 75;
/// Navratovy kod simulovaneho padu (`dk:crash`).
pub const EXIT_CRASH: i64 = 70;

pub const PING: &[u8] = b"dk:ping";
pub const PONG: &str = "dk:pong";
pub const INFO: &[u8] = b"dk:info";
pub const RESTART: &[u8] = b"dk:restart";
pub const CRASH: &[u8] = b"dk:crash";

/// Patri zprava smycce driverkitu?
pub fn is_reserved(msg: &[u8]) -> bool {
    msg.starts_with(b"dk:")
}

/// Popis navratoveho kodu ovladace na driverkitu (pro logy spravce
/// zarizeni); kody jadra popise `abi::process::describe_exit`.
pub fn describe_exit(code: i64) -> &'static str {
    match code {
        0 => "klient zavrel kanal",
        1 | 2 => "spatna konfigurace od spravce zarizeni",
        3 => "chybi pravo nebo kvota (manifest resources)",
        4 => "chyba zarizeni nebo ovladace",
        EXIT_CRASH => "simulovany pad (dk:crash)",
        EXIT_RESTART => "restart na zadost (dk:restart)",
        c => abi::process::describe_exit(c),
    }
}
