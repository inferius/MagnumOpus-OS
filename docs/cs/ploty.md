# Ploty

Plot (fence) je objekt jádra pro **explicitní synchronizaci** mezi procesy:
producent (ovladač GPU, dekodér videa) posouvá hodnotu, spotřebitel
(kompozitor, aplikace) čeká, až hodnota nastane. Je to krok 6a-1 základů
pro grafiku a video ([gpu-amd-nvidia.md](gpu-amd-nvidia.md#vlivy-na-základy),
bod 1). Čísla a argumenty syscallů jsou v [syscally.md](syscally.md#ploty),
kódy chyb v [chyby.md](chyby.md).

## Stav

| Část | Co | Stav |
|---|---|---|
| 6a-1 | objekt plotu: časová osa `u64`, trvalá chyba, opuštění po zániku posledního producenta, `FENCE_WAIT` (jedna / všechny položky, termín), registrace do portu, `SIGNALED` pro `OBJECT_WAIT_*`; obálky `libos::Fence`, `libos::fence::{wait_any, wait_all}` | **[ověřeno v QEMU]** x86_64 i aarch64 (sada jádra `objekty`, testy `plot: ...`; sada userspace `ploty`) |
| 6a-3 | ploty ve zprávách `renderproto` (Venus: plot virtio -> plot jádra, `ring_idx > 0`) | **[navrženo]** |
| 6a-4 | ploty získání a uvolnění v okenním protokolu (`SYNC_POINTS`, [gui.md](gui.md#buffery-s-popisem-a-explicitní-synchronizace-video)) | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `gui`, test `compd: video NV12 a I420 ...`) |
| LinuxKPI | `dma_fence`, `sync_file`, `drm_syncobj` převzatého C jako tento objekt | **[navrženo]** |

## Model

<figure>
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Stavy plotu">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.bad{fill:#fbecec;stroke:#a04040;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}.bad{fill:#331f1f;stroke:#d08f8f}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="pl1" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="acc" x="250" y="20" width="260" height="64" rx="6"/><text class="t b" x="380" y="44" text-anchor="middle">běží: hodnota v</text><text class="m" x="380" y="64" text-anchor="middle">FENCE_SIGNAL(w &gt;= v) -&gt; v = w</text>
<rect class="bad" x="20" y="140" width="300" height="64" rx="6"/><text class="t b" x="170" y="164" text-anchor="middle">chyba (FENCE_FAILED)</text><text class="m" x="170" y="184" text-anchor="middle">FENCE_SIGNAL(f &gt; v, FAILED); dál nic</text>
<rect class="bad" x="440" y="140" width="300" height="64" rx="6"/><text class="t b" x="590" y="164" text-anchor="middle">opuštěný (FENCE_ABANDONED)</text><text class="m" x="590" y="184" text-anchor="middle">zanikl poslední handle s právem SIGNAL</text>
<path class="ln" d="M330,84 L200,138" marker-end="url(#pl1)"/>
<path class="ln" d="M430,84 L560,138" marker-end="url(#pl1)"/>
<path class="ln" d="M510,40 C580,10 580,80 512,62" marker-end="url(#pl1)"/>
<text class="m" x="20" y="226">Čekání na t: t &lt;= v uspěje vždy; t &gt; v po chybě nebo opuštění skončí chybou plotu.</text>
</svg>
<figcaption>Stavy plotu. Dosažená hodnota zůstává platná i po chybě a opuštění.</figcaption>
</figure>

- **Časová osa**: hodnota `u64` roste monotónně (posun na stejnou hodnotu
  nic nedělá, zpět = `INVALID_ARGS`). Binární plot je osa s cílem 1;
  `FENCE_CREATE` bere počáteční hodnotu.
- **Práva**: `FENCE_SIGNAL` smí jen handle s právem `SIGNAL` (producent).
  Spotřebitel dostane duplikát jen s `BASIC` (`Fence::for_waiter`), čeká
  a ptá se na stav. Právo `SIGNAL` nejde přidat.
- **Trvalá chyba**: `FENCE_SIGNAL(f, FAILED)` označí bod `f` (větší než
  dosažená hodnota) za neúspěšný. Dosažená hodnota se nemění, každé čekání
  na nedosaženou hodnotu skončí `FENCE_FAILED` a další posun je
  `BAD_STATE`. `FENCE_QUERY` vrátí `failed_at = f`.
- **Opuštění**: každý handle s právem `SIGNAL` drží v jádře vlastní obálku
  producenta (`FenceSignaler`, dosadí ji tabulka handlů). Obálka žije
  i v nepřečtené zprávě kanálu, takže předání producenta jinému procesu plot
  neopustí. Když zanikne poslední (zavření, konec nebo pád procesu,
  zahozená zpráva), plot přejde do stavu opuštěný: čekání na nedosaženou
  hodnotu skončí `FENCE_ABANDONED`. Chybu opuštění nepřepíše. Obdoba
  `dma_fence`, který po pádu ovladače signalizuje jádro.
- **Signál `SIGNALED`** (pro `OBJECT_WAIT_*` a `OBJECT_WAIT_ASYNC`) platí,
  když je hodnota aspoň 1, nebo plot skončil chybou či byl opuštěn.
  Plot založený s hodnotou >= 1 je `SIGNALED` hned.

## Čekání

- `FENCE_WAIT` čeká na seznam `(plot, hodnota)` (nejvýš 64): stačí jedna
  položka, nebo všechny (`fence::wait::ALL`). Položky se kontrolují
  v pořadí; chyba plotu čekání ukončí a jádro zapíše index rozhodující
  položky. Kontrola a registrace čekajícího jsou pod zámkem plotu, takže
  posun mezi kontrolou a usnutím probuzení neztratí (self-test jádra posílá
  signál v každém místě, `plot: signal od sebe sama v kazdem miste cekani`).
- `FENCE_WAIT_ASYNC` je jednorázová registrace do portu (jako
  `OBJECT_WAIT_ASYNC`, stejná rezervace místa a rozpočet jobu portu). Paket
  přijde, až je hodnota **rozhodnutá** (dosažená, chyba nebo opuštění);
  výsledek zjistí `FENCE_QUERY`. Kompozitor a ovladače tak mají jednu
  smyčku nad portem pro kanály i ploty.
- **Cykly čekání mezi ploty jádro nepozná** (A čeká na plot B a naopak).
  Proto služby (compd, ovladače) čekají na plot cizího procesu **vždy
  s termínem** a po vypršení s ním zacházejí jako s chybou (požadavek
  koordinátora pro 6a-3 a 6a-4).

```rust
use libos::fence::{self, Fence};

let fence = Fence::create(0)?;                  // producent (SIGNAL)
let consumer = fence.for_waiter()?;             // jen čekání a dotaz
channel.write(b"frame", vec![consumer.into_handle()])?;
// ... práce GPU ...
fence.signal(1)?;                               // nebo fence.fail(1)

// spotřebitel (jiný proces):
match fence.wait(1, Some(time::deadline_after(Duration::from_millis(100)))) {
    Ok(()) => { /* buffer je hotový */ }
    Err(Error::FenceFailed | Error::FenceAbandoned) => { /* zahodit snímek */ }
    Err(e) => return Err(e),                    // TIMED_OUT, ...
}
```

## Testy

- Host: `cargo test -p abi` (`fence_tests.rs`: stavový automat
  `abi::fence::Timeline`, kódování `WaitItem` a `Info`).
- Jádro (sada `objekty`): signál v každém místě mezi kontrolou a usnutím
  (jedna i všechny položky), souběh s usínáním z jiného CPU (300 kol),
  trvalá chyba a index položky, opuštění po zániku poslední obálky
  producenta, úklid registrací.
- Userspace (sada `ploty`, `user/init/src/tests/fences.rs`, protistrana
  `echo`): producent v jiném procesu (osa ve `FENCE_WAIT` i v portu),
  spotřebitel v jiném procesu, `FENCE_FAILED`, opuštění po `kill` (port
  i čekání), producent ve zprávě kanálu, všechny položky a index chyby,
  práva a neplatné argumenty.
