# user/notifyd

Služba oznámení (`docs/cs/oznameni.md`). Logika je v `libs/notifycore` (host testy), protokol v `libs/notifyproto`; tady jen kanály, čas a smyčka přes `libos::WaitSet`. Bootstrap kanál = řídicí kanál initu (`sender <uid> <system> <app_id> <jméno>`, `viewer <uid>`, `status`), klienty obsluhuje `user/init/src/notify.rs`.

## Invarianty

- Identitu odesílatele určuje jen init (řídicí příkaz); aplikace ji nemůže změnit. Klíče oznámení jsou v prostoru identity (app_id + uid).
- Prohlížeč vidí oznámení svého `uid` a systémová (`uid` 0). `Urgent` smí jen systémová identita.
- Služba nikdy nečeká na klienta: plná fronta prohlížeče = zpráva zahozena (`ShouldWait`), mrtvý kanál pryč na konci kola.
- Stav jen v paměti (pád = ztráta oznámení).

## Testy

- `cargo test -p notifycore -p notifyproto`; v QEMU `cargo xtask test --only oznameni` (x86_64 i aarch64).
