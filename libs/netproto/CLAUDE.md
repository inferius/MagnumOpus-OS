# libs/netproto

Protokol sítě: zprávy mezi programy a službou `netd` (relace a sockety), mezi `netd` a ovladačem karty (`link`), firewall (`fw`: netd <-> fwd <-> agent), oprávnění (`policy`, `auth`), formáty DNS, NTP a NTS. Feature `client` = klientská knihovna nad libos (`Net`, `TcpStream`, ...). Popis: `docs/cs/sit.md` (kapitoly API pro programy a Protokol netproto).

## Klíčové soubory

- `src/msg.rs` - hlavička v2: dotaz `op, id, deadline`, odpověď `id, status`; `CANCEL`.
- `src/client.rs` - `Net`, `TcpStream`, `TcpListener`, `UdpSocket`, `Deadline`; `src/client/mux.rs` - sdílený kanál (směrování odpovědí podle čísla, futex) a `CancelToken`.
- `src/lib.rs` - kódy operací (`op`), `Info`, `Buffers`, `DIRECTION_QUEUE`; `src/auth.rs` - relace, vrstvy, rozpočet (`Quota`: sockety, bajty, souběžné dotazy).
- `src/link.rs` - ovladač karty <-> netd (jiný protokol, bez hlavičky v2).
- `ready` (v `lib.rs`) - bity připravenosti socketu na eventu (`USER_0..3`); `Readiness` v `client.rs` = základ backendu mio.

## Invarianty

- Každá zpráva na kanálu relace a socketu má hlavičku v2; starý formát se nečte (převádějí se všichni klienti najednou).
- Číslo dotazu je jedinečné mezi rozpracovanými na kanálu a nikdy 0; termín je absolutní čas `libos::time::now` v ns, `u64::MAX` = bez limitu.
- Klientské typy jsou `Send + Sync` a metody berou `&self`; jedno vlákno nesmí kvůli `Mux` platit syscall navíc.
- Pozdní odpověď (dotaz vypršel nebo byl zrušen) se zahodí i s handly; kanál se kvůli ní nezavírá.
- Kanál fwd <-> agent má vlastní formát odpovědi (`fw::agent_reply`, status <= 0 odliší odpověď od zprávy fwd).

## Testy

- Host: `cargo test -p netproto --features std,nts` (hlavička, limit dotazů relace, formáty). `Mux` potřebuje kanály a futex jádra: testuje ho QEMU, `cargo xtask test --only "sit,sit soubeh,firewall"`.

## Pasti

- Změna hlavičky nebo kódů operací zasahuje `netd`, `compd` (ručně skládaný `INFO` v `status.rs`), osrt (`std::net`) a všechny programy s `Net`.
- `into_channel` vrací `BAD_STATE`, když socket drží klon nebo na něm běží dotaz.
