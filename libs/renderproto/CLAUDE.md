# libs/renderproto

Protokol třídy render (`no_std`): klient 3D <-> ovladač GPU (`Request`/`Reply`) a poskytovatel render uzlů (`node`: ovladač <-> gpud, init <-> gpud). Nahradil `gpuproto`. Popis: `docs/cs/render.md`.

## Invarianty

- `SUBMIT` nese ploty v handlech zprávy: nejdřív `wait.len()` k čekání, pak `signal.len()` k signalizaci; v datech jen hodnoty. Ovladač odpoví `SUBMITTED` hned, dokončení hlásí jen ploty jádra.
- Každý příkaz s ploty skončí: splněním, `FENCE_FAILED` (chybný vstupní plot, chyba zařízení) nebo po `HANG_TIMEOUT` (10 s, konstanta třídy). Nový ovladač třídy musí totéž dodržet.
- Data specifická pro ovladač (proud Venusu, uAPI NVK) jdou uvnitř obálky; obálka se kvůli ovladači nemění.
- `IMPORT_BUFFER` nese `bufdesc`; ovladač ho ověří proti velikosti VMO (`BufDesc::validate`).

## Testy

- `cargo test -p renderproto` (`src/tests.rs`: kódování, limity, poskytovatel).
