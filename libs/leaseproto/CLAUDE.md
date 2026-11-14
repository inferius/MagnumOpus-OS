# libs/leaseproto

Protokol lístku klient <-> supervisor (`RECONNECT`, `NARROW`, `STATUS`, `ADOPT`) a limity `ADOPT`. Samostatný crate, protože ho potřebuje i libos (obnova relace s libd), na kterém `svcclient` závisí; `svcclient::proto` je tento crate. Popis: `docs/cs/obnova-sluzeb.md` (kapitola Protokol lístku).

## Invarianty

- Dekódování nedůvěryhodné zprávy vrátí `INVALID_ARGS`, nikdy paniku; čísla operací se nepřečíslovávají.
- `ADOPT` nemá odpověď: odmítnutí se ukáže až klientovi dítěte (zavřený lístek).
- `no_std`, závislosti jen `abi` a `wire` (sestavuje se i jako závislost std přes libos).

## Testy

- `cargo test -p svcclient` (testy protokolu a omezovače `AdoptLimit`).
