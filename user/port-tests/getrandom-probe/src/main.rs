//! getrandom-probe: overi backend `getrandom` pro MagnumOpus (syscall
//! `RANDOM_GET`). Vypise `ok <jmeno>` za kazdou kontrolu a nakonec
//! `getrandom-probe: hotovo`; chyba = `FAIL ...` na stderr a kod 1.

fn check() -> Result<(), String> {
    let mut a = [0u8; 64];
    let mut b = [0u8; 64];
    getrandom::fill(&mut a).map_err(|e| format!("fill: {e}"))?;
    getrandom::fill(&mut b).map_err(|e| format!("fill: {e}"))?;
    if a == [0u8; 64] || a == b {
        return Err(format!("nenahodna data: {a:02x?} {b:02x?}"));
    }
    println!("ok fill");
    // Vetsi nez jeden syscall (4096 B): po kusech.
    let mut big = vec![0u8; 10_000];
    getrandom::fill(&mut big).map_err(|e| format!("fill 10000: {e}"))?;
    let zeros = big.iter().filter(|b| **b == 0).count();
    // Ocekavano ~39 nul; 200 by znamenalo nevyplnenou cast.
    if zeros > 200 {
        return Err(format!("10000 B ma {zeros} nul (cast nevyplnena?)"));
    }
    println!("ok fill_big");
    let (x, y) = (getrandom::u64().map_err(|e| e.to_string())?, getrandom::u64().map_err(|e| e.to_string())?);
    if x == y {
        return Err(format!("u64 dvakrat stejne: {x}"));
    }
    println!("ok u64");
    Ok(())
}

fn main() {
    if let Err(e) = check() {
        eprintln!("FAIL getrandom: {e}");
        std::process::exit(1);
    }
    println!("getrandom-probe: hotovo");
}
