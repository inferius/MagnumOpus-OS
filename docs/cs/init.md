# Init a supervisor

`init` je první proces v userspace. Spustí systémové služby a pak zůstane
jako **supervisor**: předává zařízení připojená za běhu a hlídá služby.
V testovacím běhu místo toho spustí testy userspace a skončí s jejich
výsledkem. Jen v testovacím běhu zapne `fsd` ladicí režim (simulace
výpadku proudu pro testy FS).

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Init a supervisor">
<defs><marker id="in9" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="150" height="56" rx="6"/><text class="t b" x="85" y="43" text-anchor="middle">jádro</text><text class="m" x="85" y="62" text-anchor="middle">spustí init</text>
<rect class="acc" x="210" y="20" width="200" height="90" rx="6"/><text class="t b" x="310" y="45" text-anchor="middle">init: start</text><text class="m" x="310" y="64" text-anchor="middle">timed, devmgr (ovladače),</text><text class="m" x="310" y="80" text-anchor="middle">fsd, inputd</text><text class="m" x="310" y="96" text-anchor="middle">settled</text>
<path class="ln" d="M160 48 L208 48" marker-end="url(#in9)"/>
<rect class="ok" x="470" y="20" width="280" height="90" rx="6"/><text class="t b" x="610" y="45" text-anchor="middle">supervisor (běžný běh)</text><text class="m" x="610" y="64" text-anchor="middle">zařízení za běhu -> inputd</text><text class="m" x="610" y="80" text-anchor="middle">konec služby -> log, restart</text><text class="m" x="610" y="96" text-anchor="middle">nikdy nekončí</text>
<rect class="box" x="470" y="140" width="280" height="70" rx="6"/><text class="t b" x="610" y="165" text-anchor="middle">testy (TEST_RUN)</text><text class="m" x="610" y="184" text-anchor="middle">init/src/tests, supervisor řídí testy</text><text class="m" x="610" y="200" text-anchor="middle">konec = výsledek, jádro ukončí QEMU</text>
<path class="ln" d="M410 55 L468 55" marker-end="url(#in9)"/>
<path class="ln" d="M410 90 L468 165" marker-end="url(#in9)"/>
<text class="m" x="10" y="250">Příznak TEST_RUN dá jádro s feature qemu-test (cargo xtask test) do argumentu SELF_PROCESS ve startovní zprávě.</text>
<text class="m" x="10" y="270">cargo xtask run a skutečný stroj: bez příznaku, init testy nespouští a běží jako supervisor.</text>
</svg>
<figcaption>Init po startu služeb: supervisor, nebo v testovacím běhu testy.</figcaption>
</figure>

## Start

1. `timed` (služba času, jediná s právem nastavovat hodiny),
2. `devmgr`: najde zařízení a spustí ovladače. Hlásí disky (`block`)
   a vstupní zařízení (`input`) a pak `done`, až jsou hotová i zařízení
   USB při startu,
3. `fsd` nad diskem se souborovým systémem; init připraví úložiště
   kořenových certifikátů `/system/config/certs` (viz
   [TLS a HTTP](tls-http.md#úložiště-kořenových-certifikátů)),
4. `inputd`: init mu předá vstupní zařízení, svůj kanál fokusu a kanál
   `secure` a pošle `settled` (zařízení připojená teď jsou důvěryhodná),
5. `netd` nad první síťovou kartou (s kanálem pro `dnsd`), `fwd` (firewall,
   konfigurace na FS) a `dnsd` (šifrované DNS a DNSSEC, viz
   [Síť](sit.md#šifrované-dns-a-dnssec-dnsd)),
   oba v jobech podle svých manifestů (viz [Síť](sit.md),
   [Firewall](firewall.md)), potom `ntpd` (synchronizace času, viz
   [Reálný čas](cas.md#synchronizace-podle-sítě-ntpd)) a `revocd`
   (odvolání certifikátů, viz [TLS a HTTP](tls-http.md#odvolání-certifikátů)),
6. všechny tyto procesy předá supervisoru k hlídání,
7. v běžném běhu GUI (`compd`, okna `gui-term` a `gui-demo`), bez GUI
   záložní konzoli (viz [Terminál](#terminál)).

Konfiguraci `fsd` (první zpráva řídicího kanálu) posílá init vždy:
`debug` v testovacím běhu, jinak `normal`. fsd na ni před připojením
disku čeká.

## Supervisor

Smyčka čeká na kanál správce zařízení a na konec hlídaných procesů:

| Událost | Co supervisor udělá |
|---|---|
| `devmgr` pošle `input` s kanálem | předá ho službě vstupu (`device`). Takhle se do systému dostanou zařízení USB připojená za běhu |
| `devmgr` pošle `block` | zatím jen zaloguje (disky za běhu přijdou s více disky v `fsd`) |
| `devmgr` zavře kanál | zaloguje, hotplug přestane fungovat |
| skončila služba s restartem (`timed`) | zaloguje kód a spustí ji znovu po 100 ms, 200 ms, 400 ms ... (nejvýš 5 s). Na potvrzení nové služby nečeká blokující: její kanál je další položka smyčky. Když do 5 s neodpoví (nebo odpoví chybou), ukončí ji a pokus se počítá jako další pád. Víc než 5 restartů za 60 s = vzdá to a služba zůstane zastavená |
| okno terminálu pošle `connect`, `termd` skončí | spustí `termd`, obslouží registr relací (viz [Terminál](#terminál)) |
| skončil `compd` | zaloguje a spustí záložní konzoli |
| skončila služba s adaptérem v registru | jako u `timed`, navíc v pořadí závislostí: služby s `Dep::restart` na spadlé službě ukončí a spustí po ní, služby s `Dep::notify` dostanou oznámení; klienti se připojí přes lístky (viz [Obnova služeb](obnova-sluzeb.md)) |
| skončila jen hlídaná služba (`devmgr`, `fsd`, `inputd`, `netd`, `fwd`, dokud je nezapojí jejich vlákna) | zaloguje kód. Obnova zatím není (viz níže) |

Log při pádu služby:

```text
init: sluzba timed skoncila: ukoncen jinym procesem (PROCESS_KILL)
init: sluzba timed znovu spustena
...
init: sluzba timed spadla 6x za 60 s, vzdavam to
```

Kód je v `user/init/src/supervisor/` (`mod.rs` smyčka, `lifecycle.rs` restart a závislosti, `leases.rs` lístky, `service.rs` adaptér). Jedno kolo je
`Supervisor::step(startup, deadline)`. Vrací, co se stalo
(`DeviceAdded`, `Exited`, `Restarted`, `GaveUp`, `Stopped`, `Notified`, `Reconnected`, `DevmgrGone`), takže ho
testy řídí stejně jako běžný běh, jen s termínem. Novou službu
supervisoru předá `watch(jméno, proces, restart)`, kde `restart` je
`Restart { start, ready }` (`None` = jen hlídat). `start` spustí nový
proces a vrátí `Started { process, ready }`: handle procesu a případně
kanál, na kterém služba potvrdí start. Až je kanál čitelný, supervisor
zavolá `ready` (přečte potvrzení) a teprve pak hlásí `Restarted`:

```rust
let mut sup = supervisor::SUPERVISOR.lock();
sup.watch("timed", &t.process, Some(system::TIME_RESTART));
sup.watch("fsd", &fsd_process, None);
sup.run(&startup); // nikdy se nevrátí
```

## Terminál

V běžném běhu je terminál okno `gui-term` (GUI) a bez GUI záložní konzole
`console` nad framebufferem z firmware. Obě kreslí interaktivní terminál
`term::ui`; shell běží ve službě relací `termd` a relace ve vlastních
procesech `shsess` (viz [Shell](navrh-shell.md)). Kód je
v `user/init/src/terminal.rs`.

```text
gui-term ──connect──▶ init (supervisor) ──spustí──▶ termd ──▶ shsess (relace)
    ▲                      │ registr relací okna          │
    └──── kanál k termd ───┘ (náhradní konce relací) ◀────┘
```

- Okno dostane od initu **kanál připojení** (druhý handle startovní
  zprávy, za kanálem okna). Na zprávu `connect` init spustí `termd`
  (vlastní relace fsd, obraz initfs, kanál registru) a odpoví `ok`
  s kanálem k němu, nebo `err <KÓD>`.
- **Pád termd:** okno uvidí zavřený kanál a požádá znovu. Nový `termd` si
  z registru převezme relace (`shsess` běží dál), takže se okno připojí ke
  stejné relaci i s historií bloků. Každé okno má vlastní registr, nový
  termd jednoho okna tak nepřevezme relace jiného. Nejvýš 10 startů termd
  za 60 s na okno, pak init odmítne (`err NO_RESOURCES`) a okno to zkouší
  s rostoucí prodlevou (nejvýš 5 s).
- **Záložní konzole:** když GUI nenaběhne (bez virtio-gpu) nebo kompozitor
  spadne (supervisor uvidí konec `compd`), init spustí `console`, předá jí
  framebuffer z firmware (`display`) a fokus od `inputd` (`focus`) a ta se
  připojí stejně jako okno.
- **Úvodní nápověda:** nová relace začne blokem s nápovědou (`help`, pár
  příkladů, klávesy).

⚠️ **Omezení:** po pádu kompozitoru zůstanou okna aplikací pryč (restart
`compd` zatím není). Jestli je framebuffer z firmware po převzetí obrazu
ovladačem virtio-gpu ještě vidět, záleží na zařízení (QEMU q35 s VGA ano).

## Obnova `inputd`, `fsd`, `netd`, `compd` a `devmgr`

Společný protokol (registr služeb se závislostmi, lístky, držák konců
ovladačů, pravidla pro knihovny klientů) popisuje stránka
[Obnova služeb](obnova-sluzeb.md). Registr (`register(Spec { name, deps,
service }, proces)`) a lístky jsou v supervisoru hotové; adaptéry
jednotlivých služeb zapojují jejich vlákna. Do té doby se tyto služby jen
hlídají přes `watch(jméno, proces, None)`.

## Testovací běh

Jádro s feature `qemu-test` (`cargo xtask test`) nastaví ve startovní
zprávě initu příznak `abi::bootstrap::flags::TEST_RUN` (argument položky
`SELF_PROCESS`; v libos `Startup::flags`). Init pak:

- spustí testy userspace (`user/init/src/tests`) a skončí s kódem 0 nebo 1,
- jádro výsledek předá QEMU (isa-debug-exit).

Testy, které potřebují supervisor (hotplug USB, restart služby), ho řídí
přes `Supervisor::step`. Bez příznaku (`cargo xtask run`, skutečný stroj)
init testy nespouští.

Testy sady `supervisor`:

| Test | Co ověří |
|---|---|
| pád služby času: restart a odpověď | `timed` ukončený přes `PROCESS_KILL`: supervisor zaznamená konec, spustí ho znovu a nová služba odpovídá |
| opakované pády: supervisor to vzdá | po 5 restartech za minutu další pád = `GaveUp`, služba neběží; `revive` ji spustí znovu |
| restart bez potvrzení: supervisor službu ukončí | vlastní supervisor (`Supervisor::for_tests`, lhůta 300 ms) se službou, která start nikdy nepotvrdí: supervisor na ni čeká bez blokování, po lhůtě ji ukončí (`PROCESS_KILL`) a nehlásí `Restarted` |
