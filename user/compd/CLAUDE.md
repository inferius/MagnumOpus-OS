# user/compd

Kompozitor (milník 6c) jako služba. Logika je v knihovně `libs/comp` (host testy); tady jsou jen kanály, objekty paměti a smyčka událostí: čekání na všechny kanály s termínem příštího ticku, vstup vždy první, pak zprávy klientů, nakonec tik (skládání do VMO snímku a `PRESENT` ovladači displeje). Řídicí příkazy od initu popisuje `guiproto::control`. Popis: `docs/cs/gui.md`.

## Klíčové soubory

- `src/main.rs` - smyčka; `screen.rs` (displej, režimy), `gpu.rs` (GPU backend přes `libs/vkcomp`), `peers.rs` (klienti), `video.rs` (buffery s popisem `bufdesc`, převod YUV, ploty commitu), `login.rs` (přihlášení), `snapshot.rs` (obnova po pádu), `splash.rs`, `status.rs`, `tasks.rs` (odběr `taskd` pro panel Úlohy, `sources tasks`), `notify.rs` (prohlížeče oznámení `notifyd` pro každou relaci, příkaz `notify <uid>`).

## Invarianty

- Smyčka čeká přes `libos::WaitSet` (počet klientů bez limitu `wait_many`); plot získání se registruje na port množiny (`Peers::set_port`, duplikát) s klíčem klienta pod `libos::FOREIGN_KEYS` - paket jen budí smyčku, port se z `Peers` nečte.

- Priorita vstupu před skládáním; na displeji je vždy nejvýš jeden snímek.
- Obsahu bufferů aplikací se nevěří (kontroly rozměrů, čtení omezené na VMO).
- Pád `compd` nesmí ztratit okna: stav se obnoví ze snímku (`docs/cs/gui.md#obnova-po-pádu-kompozitoru`); init spustí záložní konzoli.
- Aplikace ani kompozitor nesahají na HW displeje; jde to přes ovladač displeje (protokol `guiproto::display`).
- Render uzel (3D) kompozitor sám neotevírá: dostane ho s příkazem `backend gpu` (init přes `gpud`). Při změně velikosti obrazovky znovu použije stejné spojení Venus (`GpuBackend::with`).
- Vzhled jen z tokenů (`libs/theme`).
- Plot získání (`SYNC_POINTS`): commit s nesplněným plotem se podrží i se všemi dalšími zprávami klienta (kanál se nečte, v čekání smyčky jen `PEER_CLOSED`), nejdéle `video::ACQUIRE_TIMEOUT`; chyba nebo termín = commit se použije a klient dostane `ERROR`. Plot uvolnění: YUV hned po převodu, BGRA až po `RELEASED`. Popis `bufdesc` se vždy ověří a VMO se mapuje přesně v potřebné velikosti.

## Testy

- Logika: `cargo test -p comp`. Služba v QEMU: `cargo xtask test --only gui` (podle potřeby `displej`, `vulkan`). Crate `user/compd` nemá host testy.

## Pasti

- Nová logika patří do `libs/comp`, ne sem; tady jen lepidlo, jinak ji nelze testovat na hostiteli.
- Software skládání stojí CPU; změny v horké cestě měř (podmínky měření: `docs/cs/dokumentace.md`).
