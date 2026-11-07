# Obnova služeb po pádu

Když spadne systémová služba (`fsd`, `inputd`, `netd`, `compd`, ...),
supervisor initu ji spustí znovu a **klienti se znovu připojí sami**: knihovna
klienta ohlásí „služba se restartuje“, počká na novou instanci a obnoví
relaci se **stejnou identitou**. Nový kanál ke službě předává **jen
supervisor**. Rozpracovaný požadavek se zopakuje, jen když je to bezpečné;
jinak klient dostane jasnou chybu. **Nikdy nevisí.**

Stránka je společný protokol pro všechny služby. Kód: supervisor
`user/init/src/supervisor/`, knihovna klienta `libs/svcclient`, testovací
služba `user/svctest`, testy `user/init/src/tests/recovery.rs`.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Obnova služby po pádu">
<defs><marker id="ob1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="ob1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="200" height="110" rx="6"/><text class="t b" x="110" y="55" text-anchor="middle">klient (aplikace)</text><text class="m" x="110" y="76" text-anchor="middle">svcclient::Conn</text><text class="m" x="110" y="94" text-anchor="middle">kanál relace + lístek</text><text class="m" x="110" y="112" text-anchor="middle">generace instance</text>
<rect class="err" x="300" y="20" width="170" height="60" rx="6"/><text class="t b" x="385" y="45" text-anchor="middle">služba gen 1</text><text class="m" x="385" y="64" text-anchor="middle">spadla</text>
<rect class="ok" x="300" y="110" width="170" height="60" rx="6"/><text class="t b" x="385" y="135" text-anchor="middle">služba gen 2</text><text class="m" x="385" y="154" text-anchor="middle">nová instance</text>
<rect class="acc" x="560" y="30" width="190" height="200" rx="6"/><text class="t b" x="655" y="55" text-anchor="middle">supervisor (init)</text><text class="m" x="655" y="78" text-anchor="middle">registr služeb</text><text class="m" x="655" y="96" text-anchor="middle">závislosti, pořadí</text><text class="m" x="655" y="114" text-anchor="middle">backoff, limit pokusů</text><text class="m" x="655" y="132" text-anchor="middle">recepty relací (lístky)</text><text class="m" x="655" y="150" text-anchor="middle">držák konců ovladačů</text><text class="m" x="655" y="168" text-anchor="middle">generace instancí</text><text class="m" x="655" y="186" text-anchor="middle">stav pro přehled úloh</text>
<path class="ln dash" d="M210 60 L298 50" marker-end="url(#ob1)"/><text class="m" x="225" y="44">1. PEER_CLOSED</text>
<path class="ln-acc" d="M210 125 C 380 230, 520 230, 558 200" marker-end="url(#ob1a)"/><text class="m" x="250" y="250">2. RECONNECT na lístku (relace byla z gen 1)</text>
<path class="ln" d="M558 140 L472 140" marker-end="url(#ob1)"/><text class="m" x="480" y="128">3. spustí,</text><text class="m" x="480" y="160">vytvoří relaci</text>
<path class="ln-acc" d="M300 160 L212 120" marker-end="url(#ob1a)"/><text class="m" x="218" y="170">4. nový kanál, gen 2</text>
<rect class="box" x="10" y="270" width="740" height="50" rx="6"/><text class="m" x="20" y="290">5. knihovna zopakuje rozpracovaný požadavek, jen když je idempotentní; jinak SERVICE_RESTARTED.</text><text class="m" x="20" y="308">Služba do 10 s nenaběhne nebo ji supervisor vzdá: SERVICE_UNAVAILABLE. Klient nikdy nevisí.</text>
</svg>
<figcaption>Pád služby: klient požádá supervisor o novou relaci přes svůj lístek.</figcaption>
</figure>

## Pojmy

| Pojem | Význam |
|---|---|
| **instance** | jeden běžící proces služby |
| **generace** | pořadové číslo instance (`u64`, první start = 1, každý start +1). Klient podle ní pozná, že mluví s novou instancí |
| **recept** | bajty, ze kterých supervisor umí na nové instanci vytvořit **stejnou relaci**: u `fsd` uživatel, skupiny a připojené cesty, u `netd` pravidla sítě, u `compd` identita okna. Formát patří službě, supervisor ho jen uloží |
| **lístek** (lease) | kanál klienta k supervisoru, svázaný s jedním receptem. Kdo lístek drží, smí požádat o novou relaci **přesně podle receptu** (nikdy širší). Lístek jde zúžit a předat dál |
| **adaptér** | kód služby v initu (`Service`): spustí instanci, potvrdí start, z receptu vytvoří relaci, zúží recept |
| **držák konců** | supervisor si drží duplikát konce kanálu k ovladači, který předal službě. Ovladač tak pád služby nepozná a nová instance dostane stejný konec |

## Registr služeb v supervisoru

Každá služba je v registru jako **specifikace**:

```rust
sup.register(Spec {
    name: "netd",
    deps: &[Dep::restart("devmgr")],
    service: Some(Box::new(NetdService::new(...))),
}, Some(&process));
```

- `name`: jméno v logu, v přehledu úloh a v lístcích,
- `deps`: na kom služba závisí a co se s ní stane, když závislost spadne:
  - `Dep::restart(x)`: s pádem `x` se služba ukončí a spustí se znovu
    **až po** novém startu `x` (pořadí podle závislostí). Pro služby, které
    drží kanál přímo k instanci `x` bez lístku (`fwd` a `dnsd` ke `netd`),
  - `Dep::notify(x)`: služba běží dál; po novém startu `x` supervisor zavolá
    její `dependency_restarted(x, generace)` (např. `compd` dostane nový
    fokus vstupu po restartu `inputd`),
- `service`: adaptér (`trait Service`):

```rust
pub trait Service {
    /// Spustí novou instanci (neblokuje). ctx: generace, restart?
    fn start(&mut self, ctx: &StartCtx) -> Result<Started>;
    /// Kanál potvrzení je čitelný: Ok(true) = běží, Ok(false) = ještě ne.
    fn ready(&mut self) -> Result<bool> { Ok(true) }
    /// Relace podle receptu na běžící instanci.
    fn connect(&mut self, recipe: &[u8]) -> Result<Channel>;
    /// Zúžený recept; nikdy nesmí dát víc práv než `parent`.
    fn narrow(&self, parent: &[u8], narrowing: &[u8]) -> Result<Vec<u8>>;
    /// Závislost s Dep::notify běží v nové generaci.
    fn dependency_restarted(&mut self, dep: &str, generation: u64) {}
    /// Instance skončila: zahodit její kanály.
    fn stopped(&mut self) {}
}
```

Služba bez adaptéru (`watch(jméno, proces, None)`) se jen hlídá: pád se zaloguje
a objeví v přehledu úloh, restart není.

### Stavy služby

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Stavy služby v supervisoru">
<defs><marker id="ob2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="70" width="120" height="50" rx="6"/><text class="t b" x="70" y="100" text-anchor="middle">Running</text>
<rect class="warn" x="200" y="70" width="120" height="50" rx="6"/><text class="t b" x="260" y="100" text-anchor="middle">Backoff</text>
<rect class="box" x="390" y="10" width="150" height="50" rx="6"/><text class="t b" x="465" y="40" text-anchor="middle">WaitingDeps</text>
<rect class="acc" x="390" y="130" width="150" height="50" rx="6"/><text class="t b" x="465" y="160" text-anchor="middle">Starting</text>
<rect class="err" x="610" y="70" width="140" height="50" rx="6"/><text class="t b" x="680" y="100" text-anchor="middle">GaveUp</text>
<path class="ln" d="M130 95 L198 95" marker-end="url(#ob2)"/><text class="m" x="138" y="88">pád</text>
<path class="ln" d="M320 85 L388 40" marker-end="url(#ob2)"/><text class="m" x="300" y="55">prodleva, deps</text>
<path class="ln" d="M465 60 L465 128" marker-end="url(#ob2)"/><text class="m" x="472" y="100">deps běží</text>
<path class="ln" d="M390 160 C 200 200, 80 170, 70 122" marker-end="url(#ob2)"/><text class="m" x="150" y="200">ready</text>
<path class="ln" d="M540 155 L640 122" marker-end="url(#ob2)"/><text class="m" x="560" y="150">6. pád za 60 s</text>
<path class="ln" d="M320 100 L608 100" marker-end="url(#ob2)"/><text class="m" x="420" y="92">limit pokusů</text>
</svg>
<figcaption>Stavy služby. Start bez potvrzení do 5 s = pád.</figcaption>
</figure>

| Stav | Význam | Lístek `RECONNECT` |
|---|---|---|
| `Running` | instance běží a potvrdila start | novější generace hned, jinak viz níže |
| `Backoff` | po pádu čeká na prodlevu (100 ms, 200 ms, ... nejvýš 5 s) | čeká |
| `WaitingDeps` | čeká, až poběží závislosti (`Dep::restart`) | čeká |
| `Starting` | nová instance ještě nepotvrdila start (lhůta 5 s) | čeká |
| `GaveUp` | víc než 5 restartů za 60 s, nebo závislost to vzdala | hned `SERVICE_UNAVAILABLE` |

Pád závislosti se službě s `Dep::restart` nepočítá do jejího limitu pokusů
(nespadla sama). `revive(jméno)` dá vzdané službě novou šanci (ruční zásah,
testy) a s ní i službám, které na ní závisí.

### Pořadí při pádu `devmgr`

`devmgr` je kořen: ostatní služby drží konce kanálů k jeho ovladačům.
Pád `devmgr` (cílový stav, viz [Stav zavedení](#stav-zavedení)):

1. supervisor ukončí ovladače, které `devmgr` spustil (`devmgr` mu posílá
   duplikát handlu každého procesu ovladače zprávou `driver`),
2. spustí `devmgr` znovu; nové ovladače nahlásí zařízení jako při startu,
3. služby s `Dep::restart("devmgr")` (`fsd`, `inputd`, `netd`, `compd`)
   se spustí znovu nad novými konci, v pořadí závislostí (`fsd` před
   `netd`, `netd` před `fwd` a `dnsd`),
4. klienti všech těchto služeb se připojí znovu přes lístky.

## Protokol lístku

Lístek je kanál; zprávy jsou kódované přes `libs/wire` (little-endian), jako
ostatní protokoly. Na jednom lístku je najednou nejvýš jeden požadavek.

**Požadavek:** `u32 op`, `u32 id` (číslo požadavku; odpověď ho vrátí), dál
podle operace:

| `op` | Jméno | Pole | Odpověď (handly) |
|---|---|---|---|
| 1 | `RECONNECT` | `u32 čekat_ms` | kanál nové relace |
| 2 | `NARROW` | `bytes zúžení` (formát služby) | kanál zúžené relace, nový lístek |
| 3 | `STATUS` | - | - |

**Odpověď:** `u32 op`, `u32 id`, `i32 stav` (0 = ok, jinak kód chyby),
`u64 generace`, `u8 stav_služby` (0 `Running`, 1 `Restarting`, 2 `GaveUp`,
3 `Stopped`).

Pravidla `RECONNECT`:

- supervisor si u lístku pamatuje generaci, na které relaci vydal, a odpoví
  relací na instanci **novější**. Klient, který pád viděl dřív než
  supervisor, tak nedostane relaci na umírající instanci. Klient generaci
  znát nemusí (dozví se ji z odpovědi),
- když služba neběží, odpověď počká na novou instanci, nejdéle
  `čekat_ms` (horní mez 10 min). Pak `SERVICE_UNAVAILABLE`,
- když to supervisor vzdá (`GaveUp`), odpoví hned `SERVICE_UNAVAILABLE`,
- **úmyslné zavření**: když instance, na které byla relace vydána, pořád
  běží i po 200 ms, služba relaci zavřela sama (odebrání práv, odhlášení). Odpověď je
  `PEER_CLOSED`, nová relace se nevydá a **lístek zanikne**: odebrání práv
  nejde obejít dalším pokusem, `NARROW` ani pozdějším restartem služby,
- odpověď s jiným `id`, než klient čeká, je pozdní odpověď na požadavek,
  kterému vypršel čas. Úspěšnou pozdní odpověď `RECONNECT` klient
  **převezme** (je to relace, kterou supervisor vydal naposledy); ostatní
  zahodí. Klient čeká na odpověď nejdéle `čekat_ms` + 5 s.

Pravidla `NARROW`:

- adaptér služby z receptu lístku a zúžení spočítá nový recept. Zúžení
  smí práva jen **odebrat** (podmnožina připojených cest, užší pravidla sítě),
  jinak `ACCESS_DENIED`,
- supervisor vytvoří zúženou relaci hned (služba musí běžet, jinak
  `SERVICE_UNAVAILABLE`) a vrátí ji s novým lístkem. Ten se předá dítěti
  spolu s relací,
- relace, kterou si klient zúžil **přímo u služby** (`Net::restrict`,
  `connect_as` z jiné než řídicí relace), lístek nemá a restart
  nepřežije: dítě dostane `SERVICE_UNAVAILABLE` a musí si říct rodiči.
  Kdyby ji knihovna obnovila přes rodičovský lístek, dítě by dostalo
  **vyšší** práva.

Lístek zanikne, když klient zavře svůj konec. Recept zmizí s ním.

## Předání nové instance

- adaptér dostane `StartCtx { generation, restart }`. Služba, která po
  restartu potřebuje jiné chování než po startu systému, se to dozví ve
  své startovní zprávě (např. `fsd` šetří sirotky, viz níže),
- **konce kanálů k ovladačům** drží supervisor (držák konců): služba dostane
  duplikát. Když spadne, konec u ovladače zůstane otevřený a ovladač nic
  nepozná. Nová instance dostane další duplikát téhož konce a **musí nejdřív
  zahodit staré zprávy** (pozdní odpovědi na požadavky staré instance): pošle
  ovladači synchronizační požadavek a čte, dokud nedostane jeho odpověď.
  Synchronizaci si definuje protokol ovladače (`blockproto`,
  `inputproto::device`, `netproto::link`, `guiproto::display`),
- **lístek ve startovní zprávě** jde hned za relaci, ke které patří:
  `[relace 1, lístek 1, relace 2, lístek 2, ..., další handly programu]`.
  Okno aplikace: `[okno, lístek okna, handly aplikace...]` (`gui-tasks`:
  kanál na `taskd` je třetí). Relace bez obnovy dostane na místo lístku
  kanál bez druhého konce: knihovna pak hlásí `SERVICE_UNAVAILABLE`,
- **ovladače při ztrátě služby nekončí**: zavřený kanál ke službě (když
  supervisor konec nedrží) i zavřený kanál k `devmgr` jen zalogují a čekají,
  až je supervisor ukončí, nebo až dostanou nový kanál.

## Knihovna klienta (`libs/svcclient`)

`Conn` drží kanál relace, lístek, generaci a stav. Knihovny služeb
(`FsClient`, klient `netproto`, `inputproto`, `guiproto`) volají služby přes
ni:

```rust
let conn = Conn::new("fsd", relace, Some(lístek));
conn.set_wait(Duration::from_secs(30));   // dávková úloha čeká déle
conn.on_event(|služba, e| println!("{služba}: {e:?}"));
let odpověď = conn.call(Kind::Idempotent, &požadavek, Vec::new())?;
```

### Pravidla pro knihovny klientů

1. **Každá operace má druh** (`Kind`):
   - `Idempotent`: opakování nic nezmění (čtení, `stat`, výpis, `info`,
     dotaz DNS, stav). Po pádu ji knihovna **zopakuje sama** na nové
     instanci,
   - `Effect`: má vedlejší účinek (zápis, přejmenování, smazání,
     podmíněné uložení, zámek, odeslání dat, akce okna). Po pádu vrátí
     **`SERVICE_RESTARTED`** a neopakuje: nevíme, jestli ji stará instance
     provedla. Rozhodne aplikace (zápis podle offsetu jde zopakovat,
     podmíněné uložení znovu ověří revizi),
   - požadavek s **handly** je vždy `Effect` (handly se odesláním předaly
     staré instanci a zanikly s ní).
2. **Čekání na novou instanci**: výchozí 10 s (`DEFAULT_WAIT`), klient smí
   nastavit vlastní (`set_wait`). Když supervisor službu vzdá, čekání skončí
   hned. Pak `SERVICE_UNAVAILABLE`; další volání zkusí obnovu znovu.
3. **Nikdy nevisí**: ztráta kanálu (`PEER_CLOSED` při zápisu i čtení) vždy
   vede na obnovu nebo chybu. Čekání na odpověď lístku má termín.
4. **Události pro UI**: `Restarting` (zobrazit „služba se restartuje“),
   `Restored { generation }`, `Unavailable(chyba)`.
5. **Proudy a push kanály** (okno `compd`, fokus `inputd`, socket `netd`):
   knihovna čeká na svém kanálu; při `PEER_CLOSED` zavolá
   `conn.recover()` a pak stav obnoví sama (okno pošle znovu strom vrstev).
6. **Odvozené relace pro děti** jen přes `conn.narrow(zúžení)` (lístek
   s receptem). Nikdy nepředávat rodičovský lístek dítěti s užší relací.
7. **Bez lístku** (`Conn::new(jméno, kanál, None)`) knihovna funguje jako
   dřív: ztráta kanálu = `SERVICE_UNAVAILABLE`.

## Co přežije pád jednotlivých služeb

| Služba | Co se obnoví | Co vrátí chybu | Vlastník zapojení |
|---|---|---|---|
| `fsd` | relace (identita, připojené cesty) z receptu; otevřené handly podle trvalého handlu (dataset, oid, generace, práva) - knihovna je otevře znovu (`REOPEN`), práva se ověří znovu. Po restartu (ne po startu systému) `fsd` 30 s neuklidí sirotky (smazané, ale otevřené soubory), aby je klienti stihli znovu otevřít | zámky, rozepsané podmíněné uložení, zápis bez potvrzení: `SERVICE_RESTARTED`, klient zopakuje | vlákno FS |
| `inputd` | ovladače (držák konců + synchronizace), fokus a kanál `secure` předá supervisor znovu (`compd` přes `Dep::notify`), rozložení klávesnice | stisknuté klávesy se považují za uvolněné | vlákno vstupu |
| `netd` | relace programů (pravidla sítě) z receptu, adresy (DHCP znovu), DNS, firewall (`fwd` a `dnsd` s `Dep::restart`) | sockety: `CONNECTION_RESET` (spojení ztraceno), naslouchající sockety je třeba otevřít znovu | vlákno sítě |
| `compd` | okna: aplikace se připojí přes lístek (recept = identita z manifestu) a pošle znovu strom vrstev; displej z držáku konců; fokus od `inputd` | rozpracovaná transakce okna | vlákno GUI |
| `devmgr` | ovladače a všechny služby nad nimi (viz pořadí výše) | jako u jednotlivých služeb | koordinátor obnovy (init) |
| `termd`, `taskd` | relace terminálu, seznam úloh (odběr od posledního čísla) | běžící příkaz shellu ve spadlé relaci | vlákno shellu |

## Přehled úloh

Supervisor je producent přehledu úloh (`taskproto`): každá služba je
úloha původu `Service` se stavem z tabulky výše, generací, počtem restartů
a posledním kódem konce. Vzdaná služba je v přehledu jako chyba s důvodem
(„spadla 6x za 60 s“). Do napojení `taskd` (vlákno shellu) je stav
v `Supervisor::status()`.

## Testy „zabij službu“

Ke každé službě patří test v QEMU: zabít ji uprostřed práce a ověřit, že
klient pokračuje, nebo dostane jasnou chybu - nikdy nevisí. Kostra je
v `user/init/src/tests/recovery_kit.rs`, testy protokolu
v `user/init/src/tests/recovery.rs` (sada `obnova sluzeb`) s testovací
službou `user/svctest`:

| Test | Co ověří |
|---|---|
| idempotentní požadavek přežije pád | klient je uprostřed požadavku, službu zabijeme; knihovna ho zopakuje na generaci 2, identita stejná |
| požadavek s účinkem vrátí chybu | totéž s `Effect`: `SERVICE_RESTARTED`, další volání funguje |
| zúžený lístek zůstane zúžený | dítě se zúženou relací se po pádu připojí znovu se zúženou identitou, ne s rodičovskou |
| vzdaná služba | klient dostane `SERVICE_UNAVAILABLE` hned, nečeká celou lhůtu |
| pomalý restart | lhůta klienta 300 ms, služba startuje déle: `SERVICE_UNAVAILABLE` do lhůty |
| úmyslné zavření | služba relaci zavře sama: `PEER_CLOSED`, nová relace se nevydá |
| závislosti | pád služby ukončí a po jejím startu znovu spustí služby s `Dep::restart`; `Dep::notify` dostane `dependency_restarted` |

Test pro vlastní službu: klient běží v **jiném procesu** než supervisor
(supervisor v testu řídí init přes `step`, klient v initu by čekal sám na
sebe). Kostra spustí `svctest` v roli klienta s kanálem relace a lístkem
a posílá mu příkazy:

```rust
let mut h = Harness::new(startup);          // vlastní supervisor testu
h.add("svctest", &[], "normal")?;           // adaptér + start, generace 1
let c = h.client("svctest", "alice")?;      // klient (jiný proces) s lístkem
h.send(c, "call idem hold")?;               // požadavek, služba ho drží
h.wait_held("svctest")?;
h.kill("svctest")?;                          // pád uprostřed práce
let (odpověď, ms) = h.reply(c)?;            // supervisor běží, klient nevisí
check_eq!(odpověď, String::from("ok gen=2 who=alice"), "zopakováno");
```

Služba s vlastním adaptérem: `h.add_service(jméno, závislosti,
Box::new(MůjAdaptér))`, klient je vlastní testovací program služby
(stejné příkazy `call`, `narrow`, `wait`, `gen` jako `svctest client`).

## Stav zavedení

| Část | Stav |
|---|---|
| protokol lístku, `svcclient::Conn`, pravidla opakování | hotovo, testy na hostiteli i v QEMU |
| registr, závislosti (`Dep::restart`, `Dep::notify`), generace, backoff, vzdání, `revive`, `status()` | hotovo, testy `obnova sluzeb` |
| držák konců (`Kept`) | hotovo v supervisoru; používají ho adaptéry služeb |
| `fsd`, `inputd`, `netd` + `fwd`/`dnsd`, `compd`, `termd`/`taskd` | zapojí vlákna služeb podle této stránky |
| restart `devmgr` (zpráva `driver`, ukončení ovladačů) | další krok po zapojení služeb: bez nich by restart `devmgr` jen odpojil zařízení |
| přehled úloh (`taskproto` producent) | po spuštění `taskd` (vlákno shellu) |

## Rizika a omezení

- **Zaseknutá služba se nepozná.** Supervisor reaguje na konec procesu,
  ne na službu, která neodpovídá. Knihovna čeká na odpověď bez termínu
  jako dřív; hlídání živosti (watchdog) je samostatný krok.
- **Kdo pád neviděl, má stará data.** Zápis přijatý starou instancí
  `fsd`, ale nepotvrzený `FSYNC`, se po pádu ztratí (stejně jako při výpadku
  proudu). Knihovna to nehlásí; aplikace, které na tom záleží, volají
  `fsync`.
- **Odebrání práv službou** zruší lístek, až se klient pokusí znovu připojit
  (`PEER_CLOSED`). Kdyby služba spadla dřív, než se o to klient pokusí,
  lístek přežije; trvalé odebrání proto musí služba ohlásit supervisoru
  (zrušení lístku na řídicím kanálu) - zatím není.
- **Háček událostí** (`Conn::on_event`) se volá se zamčeným spojením a nesmí
  volat stejný `Conn`. Volání jednoho `Conn` z více vláken se řadí za sebe
  (spin zámek drží kanál po celé volání).
- **Lístek je kanál navíc** na každou obnovitelnou relaci (paměť jádra).
- **Supervisor je jednovláknový**: dlouhý `start` nebo `connect` adaptéru
  zdrží všechny. Adaptéry nesmějí blokovat; čekání patří do `ready`.
- **Pád initu** obnovit nejde (je kořenem identit). Init musí zůstat malý.
- **Ovladače spuštěné spadlým `devmgr`** ukončí supervisor jen ty, o kterých
  mu `devmgr` stihl poslat `driver`. Jádro zatím neumí ukončit celý job.
