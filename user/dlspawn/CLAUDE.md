# user/dlspawn

Měřicí a testovací spouštěč (DL-4b): statický program s relací libd a lístkem (`spawn::Options::libd`). Na zadání z kanálu spustí N programů se zavaděčem a měří spawn, `libd::child` (`Connect` + `ADOPT`), statický spawn a oběh k libd; povely `hold`/`go` pro test pořadí obnovy (pád libd před `ADOPT`). Řídí ho `user/init/src/tests/dynlink.rs`. Popis: `docs/cs/dynamicke-linkovani.md` (Relace s libd a obnova).

## Pasti

- Testy, které ho používají, musí řídit supervisor (`SUPERVISOR.lock().step`): `ADOPT` a `RECONNECT` obsluhuje init.
