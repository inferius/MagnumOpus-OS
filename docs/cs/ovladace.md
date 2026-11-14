# Ovladače: příručka autora

Jak v MagnumOpus-OS napsat ovladač rychle, bezpečně a s minimem opakovaného
kódu. Všechny ovladače běží v userspace jako samostatné procesy; jádro jim
dá jen to, co jejich zařízení potřebuje. Sada **driverkit**
(`libs/driverkit`) dělá společnou práci (registry, DMA, přerušení, smyčka
událostí, obnova po pádu) a stejný ovladač jde spustit v systému
i v simulaci hardwaru na hostiteli.

Rychlý start:

```bash
cargo xtask new-driver muj-ovladac --pci 1234:5678
cargo test -p muj-ovladac
cargo xtask test --driver muj-ovladac
```

První příkaz vytvoří kostru (kód, manifest, model zařízení, test, stránku
dokumentace), druhý ji otestuje v simulaci bez QEMU, třetí přeloží systém
a v QEMU spustí jen testy tohoto ovladače.

## Architektura

<figure class="diagram">
<svg viewBox="0 0 780 380" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Architektura ovladačů">
<defs><marker id="dr1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="300" width="760" height="70" rx="6"/>
<text class="t b" x="390" y="324" text-anchor="middle">jádro</text>
<text class="m" x="390" y="343" text-anchor="middle">Resource (capability k hardwaru) · přerušení MSI/MSI-X · DMA paměť · kanály</text>
<text class="m" x="390" y="360" text-anchor="middle">IOMMU: každé zařízení PCI má vlastní doménu, smí jen do paměti svého ovladače</text>
<rect class="acc" x="10" y="20" width="200" height="90" rx="6"/>
<text class="t b" x="110" y="45" text-anchor="middle">init (supervisor)</text>
<text class="m" x="110" y="64" text-anchor="middle">spustí devmgr</text>
<text class="m" x="110" y="80" text-anchor="middle">dostává kanály k ovladačům</text>
<text class="m" x="110" y="96" text-anchor="middle">předá je službám</text>
<rect class="acc" x="290" y="20" width="200" height="90" rx="6"/>
<text class="t b" x="390" y="45" text-anchor="middle">devmgr</text>
<text class="m" x="390" y="64" text-anchor="middle">projde PCI, manifesty</text>
<text class="m" x="390" y="80" text-anchor="middle">omezí Resource, spustí</text>
<text class="m" x="390" y="96" text-anchor="middle">obnoví po pádu</text>
<rect class="ok" x="570" y="20" width="200" height="90" rx="6"/>
<text class="t b" x="670" y="45" text-anchor="middle">služba</text>
<text class="m" x="670" y="64" text-anchor="middle">fsd, inputd, netd ...</text>
<text class="m" x="670" y="80" text-anchor="middle">klient ovladače</text>
<text class="m" x="670" y="96" text-anchor="middle">(protokol přes kanál)</text>
<rect class="ok" x="290" y="170" width="200" height="90" rx="6"/>
<text class="t b" x="390" y="195" text-anchor="middle">ovladač</text>
<text class="m" x="390" y="214" text-anchor="middle">jen své BARy, kvóty</text>
<text class="m" x="390" y="230" text-anchor="middle">přerušení a DMA</text>
<text class="m" x="390" y="246" text-anchor="middle">driverkit</text>
<rect class="box" x="570" y="170" width="200" height="90" rx="6"/>
<text class="t b" x="670" y="195" text-anchor="middle">zařízení</text>
<text class="m" x="670" y="214" text-anchor="middle">registry (BAR)</text>
<text class="m" x="670" y="230" text-anchor="middle">DMA přes IOMMU</text>
<text class="m" x="670" y="246" text-anchor="middle">přerušení MSI</text>
<path class="ln" d="M210 65 L288 65" marker-end="url(#dr1)"/>
<path class="ln" d="M390 110 L390 168" marker-end="url(#dr1)"/>
<text class="m" x="382" y="145" text-anchor="end">Resource, konfigurace PCI, kanál</text>
<path class="ln dash" d="M290 200 L110 200 L110 112" marker-end="url(#dr1)"/>
<text class="m" x="120" y="190">dev &lt;jméno&gt; + kanál</text>
<path class="ln" d="M110 20 L110 8 L670 8 L670 18" marker-end="url(#dr1)"/>
<path class="ln" d="M570 215 L492 215" marker-end="url(#dr1)"/>
<path class="ln" d="M490 230 L568 230" marker-end="url(#dr1)"/>
<path class="ln" d="M570 95 L530 95 L530 185 L492 185" marker-end="url(#dr1)"/>
<text class="m" x="536" y="150">požadavky</text>
</svg>
<figcaption>Kdo co spouští a kdo s kým mluví. Plné šipky: předání práv a zprávy, čárkovaně: hlášení nového ovladače initu.</figcaption>
</figure>

- **Jádro** dává hardware jen přes **Resource**: capability, která
  vyjmenuje fyzické rozsahy (BARy, stránku konfigurace PCI), kvóty přerušení
  a DMA paměti. Kořenový Resource má init a předá ho devmgr.
- **devmgr** (správce zařízení) projde sběrnice PCI, pro každé zařízení
  najde ovladač (vestavěné tabulky v `user/devmgr/src/main.rs`, nebo
  [manifest](#manifest)), vytvoří **omezený Resource** jen pro to zařízení
  (`restrict_device`: s IOMMU i vlastní doména DMA) a spustí proces
  ovladače.
- **Ovladač** dostane: omezený Resource, stránku konfigurace svého
  zařízení a kanál ke klientovi. Nic dalšího v systému nevidí.
- **Služba** (fsd, inputd, netd, init) je klient ovladače. Kanál k ní
  předá init.
- **Obnova**: když ovladač z manifestu spadne, devmgr vypne zařízení bus
  master (zastaví DMA), vytvoří nový Resource a spustí ho znovu; init
  dostane nový kanál ([Životní cyklus](#životní-cyklus-a-obnova-po-pádu)).

## Kdo na čem pracuje a hranice bezpečnosti

| Část | Běží | Smí | Nesmí |
|---|---|---|---|
| jádro | ring 0 / EL1 | vše | |
| devmgr | userspace, kořenový Resource bez `SET_CLOCK` | celou konfiguraci PCI, vytvářet omezené Resource | nastavit hodiny |
| ovladač | userspace, omezený Resource | své BARy, svou stránku konfigurace, `irq_max` přerušení, `dma_max` B DMA paměti | cizí zařízení, jiné BARy, paměť jiných procesů, DMA mimo svou paměť (s IOMMU) |
| zařízení | hardware | DMA do domény svého ovladače (s IOMMU) | s IOMMU nic jiného |
| služba | userspace | protokol přes kanál | hardware |

Rizika, která je třeba znát (a hlásit, pokud se jich ovladač týká):

- **Bez IOMMU** (`--iommu none`, stroj bez VT-d / AMD-Vi / SMMU) smí
  zařízení přes DMA zapsat **kamkoli do paměti**. Chyba v ovladači
  (špatná adresa v deskriptoru) pak může poškodit jádro. Jádro to hlásí
  při startu (`IOMMU neni: ...`, viz [Hlášky](hlasky.md#jádro-iommu)).
  driverkit proti tomu chrání aspoň na straně ovladače: adresu pro
  zařízení dává jen `DmaBuffer::iova_of` s kontrolou mezí.
- **Zařízení může lhát.** Vadné nebo podvržené zařízení (Thunderbolt,
  USB) vrátí nesmyslný index, délku nebo stav. Každé číslo od zařízení je
  vstup zvenku: kruhy driverkitu ho kontrolují (`DescRing::complete_to`
  odmítne hlavu mimo kruh) a ovladač má vracet
  `DriverError::Device`, ne panikařit ani číst mimo buffer.
- **Přerušení MSI bez remapování** (`intremap=off` v QEMU): zařízení
  s DMA může teoreticky poslat zprávu MSI jiného zařízení. Remapování
  přerušení zatím jádro nezapíná.
- **Po pádu ovladače** může zařízení ještě dokončovat DMA. devmgr proto
  vypne bus master dřív, než ovladač spustí znovu, a nový ovladač začíná
  resetem zařízení v `probe`.

## Co se v ovladačích opakuje

Průzkum stávajících ovladačů (`virtio-blk`, `virtio-net`, `virtio-input`,
`virtio-rng`, `virtio-gpu`, `xhci`, `usb-hid`, `ps2`, `fbdisplay`, `nvme`,
`ahci`) ukázal, že velkou část kódu tvoří stále stejné kroky:

| Co | Kde se opakovalo | Co dělá driverkit |
|---|---|---|
| převzetí startovní zprávy (Resource, konfigurace, kanál), zpráva `pci b d f` | každý ovladač vlastní `main` (30-50 řádků) | `driverkit::os::main::<Ovladac>(startup, "jmeno")` |
| mapování BARu (zarovnání na stránku, offset) | `virtio::Window`, xhci, nvme, ahci | `Platform::map_bar`, okno s kontrolou mezí |
| registry jako čísla (`read32(0x98) \| 1 << 2`) | všude | typované registry a pole: `registers!`, chyba místo tichého přepisu |
| MSI-X, nebo MSI, když MSI-X není | virtio, xhci, nvme, ahci | `Platform::enable_irq` vybere sám |
| DMA paměť + fyzická adresa + mapování | každý ovladač | `Platform::alloc_dma` -> `DmaBuffer` s mezemi |
| kruhy deskriptorů (head/tail, fázový bit) | nvme, ahci, xhci, virtqueue | `ring::DescRing`, `ring::PhaseRing` |
| smyčka `wait_many` na přerušení a kanál, potvrzení přerušení | virtio-input, virtio-net, xhci, usb-hid | `Driver` + `run`: přerušení, zpráva, časovač, konec klienta |
| reset zařízení při konci (žádné DMA po konci procesu) | každý ovladač ručně, různě | `Driver::stop` se volá vždy, i po chybě |
| logy s prefixem `jmeno BB:ZZ.F:` | ručně v každé hlášce | `driverkit::log!` |
| restart spadlého ovladače | nebyl (devmgr jen spustil) | devmgr podle manifestu |
| zápis do devmgr (tabulka ovladačů) a xtask (seznam programů) | každý nový ovladač | manifest `<jmeno>.driver.toml`, xtask ho najde sám |

Převedené ovladače: `virtio-rng` (171 -> 147 řádků) a `virtio-input`
(251 -> 203 řádků) používají `driverkit::os` a knihovnu `virtio` dál;
ubylo převzetí startu, smyčky a zastavení zařízení. Ukázkový ovladač `edu`
je celý na driverkitu a testuje se i v simulaci.

## Sada driverkit

`libs/driverkit`, `no_std`. Feature `os` = backend nad jádrem (jen pro
cíl OS), feature `sim` = simulace na hostiteli.

| Modul | Obsah |
|---|---|
| `regs` | `Reg<W>`, `Field<W>`, makro `registers!` |
| `dma` | `DmaBuffer`: čtení a zápis s mezemi, `iova_of` |
| `ring` | `DescRing` (producent ovladač), `PhaseRing` (producent zařízení) |
| `Platform` | co ovladač potřebuje od systému: BAR, DMA, přerušení, čas, kanál, logy, bariéry |
| `Driver`, `run`, `Runner` | životní cyklus a smyčka událostí |
| `manifest` | čtení a kontrola `<jmeno>.driver.toml` |
| `config` | zpráva od devmgr (`pci ... bar ...`) |
| `proto` | zprávy `dk:` společné všem ovladačům, návratové kódy |
| `os` | `OsPlatform` nad jádrem (feature `os`) |
| `sim` | `Sim`, `DeviceModel`, `RegFile` (feature `sim`) |

### Registry

Registry se popíšou jednou podle datasheetu. Konstanta je registr, modul
stejného jména jeho pole (`JMÉNO: bit` nebo `JMÉNO: od..=do`):

```rust
driverkit::registers! {
    /// Identifikace `0xRRrr00ed`: RR hlavni verze, rr vedlejsi.
    pub ID @ 0x00: u32 {
        MAGIC: 0..=7,
        MINOR: 16..=23,
        MAJOR: 24..=31,
    }
    /// Kontrola zivota: precte se zapsana hodnota s prevracenymi bity.
    pub LIVENESS @ 0x04: u32;
    /// Zapis cisla spusti vypocet faktorialu, cteni vrati vysledek.
    pub FACTORIAL @ 0x08: u32;
    pub STATUS @ 0x20: u32 {
        /// Vypocet probiha (jen cteni).
        BUSY: 0,
        /// Po vypoctu vyvolat preruseni.
        IRQ_FACT: 7,
    }
```

Použití: `STATUS::IRQ_FACT.set(&regs)` (read-modify-write jednoho bitu),
`FACTORIAL.write(&regs, n)`, `ID::MAJOR.get(id)`. Zápis hodnoty, která se
do pole nevejde (`Field::write`, `Field::value`), vrátí
`DriverError::FieldOverflow` s názvem registru a pole; sousední bity se
nezmění. Pozor na registry, kde čtení nebo zápis něco spouští (W1C,
fronty): tam patří `Reg::write` s celou hodnotou, ne `modify`.

### DMA a kruhy

`Platform::alloc_dma(jméno, velikost)` vrátí `DmaBuffer`. Čísla v něm jsou
little endian (`read::<u32>`, `write::<u64>`), každý přístup kontroluje
meze. Adresu, kterou ovladač zapíše do registru nebo deskriptoru, dává
**jen** `iova_of(offset, délka)`: kontroluje, že celá oblast leží
v bufferu.

`DescRing` je kruh, kam píše ovladač (e1000/igc, r8169, NVMe SQ, AHCI):
`push` vrací slot, `tail` jde do registru zařízení, `complete_to(hlava)`
vrátí dokončené sloty a odmítne nesmyslnou hlavu od zařízení. `PhaseRing`
je kruh dokončení s fázovým bitem (NVMe CQ, xHCI event ring): `pop`
vrátí novou položku podle fáze. Bariéry: `p.dma_wmb()` mezi deskriptory
a indexem, `p.mmio_wmb()` před zápisem do registru „doorbell“,
`p.dma_rmb()` po přečtení indexu od zařízení.

### Platform a Driver

Ovladač je generický přes `P: Platform`. Stejný kód běží nad jádrem
(`OsPlatform`) i v simulaci (`SimPlatform`):

```rust
impl<P: Platform> Driver<P> for Edu<P> {
    fn probe(p: &P) -> Result<Self> {
        let regs = p.map_bar(0)?;
        let id = ID.read(&regs);
        if ID::MAGIC.get(id) != 0xed {
            return Err(DriverError::Device(format!("neznama identifikace {id:#x}")));
        }
```

| Metoda | Kdy | Co dělat |
|---|---|---|
| `probe` | start procesu | reset zařízení, kontrola identifikace, registry, `enable_irq`, `alloc_dma` |
| `start` | po `probe` | představení klientovi, první požadavky |
| `message` | zpráva od klienta | požadavek; odpověď `p.send` |
| `interrupt` | přerušení (už potvrzené) | přečíst stav, potvrdit v zařízení, dokončit práci |
| `next_timer` / `timer` | termín | periodická práce (virtio-rng: doplnění entropie) |
| `stop` | vždy při konci | zastavit zařízení: žádné DMA ani přerušení po návratu |
| `client_closed` | klient zavřel kanál | `false` (výchozí) = konec; `true` = běžet dál bez klienta (`send` vrací `Sent::NoClient`) |

Ovladač zařízení, jehož služba se po pádu obnovuje přes adaptér
supervisoru (`inputd`, viz [Obnova služeb](obnova-sluzeb.md)), nesmí se
ztrátou služby skončit: vrátí z `client_closed` `true` a na synchronizaci
nové instance odpoví v `message` (tak to dělá `virtio-input` s `ISYN`).

Kód, který potřebuje něco jen z jádra (např. knihovnu `virtio` nad `Ecam`),
implementuje `Driver<OsPlatform>` přímo a použije `p.ecam()`,
`p.resource()`, `p.adopt_irq(irq)` (tak to dělají `virtio-rng`
a `virtio-input`). Simulace pak pro něj není.

### Společné zprávy `dk:`

Zprávy klienta začínající `dk:` vyřídí smyčka sama:

| Zpráva | Odpověď / účinek |
|---|---|
| `dk:ping` | `dk:pong <jméno>` |
| `dk:info` | `dk:info <jméno> <adresa PCI> <vendor>:<device>` |
| `dk:restart` | ovladač zastaví zařízení a skončí kódem 75; devmgr ho hned spustí znovu (nepočítá se jako pád) |
| `dk:crash` | jako skutečný pád (kód 70): ovladač se nezastaví a nic neuvolní (zařízení zastaví jádro, karanténa DMA), devmgr ho obnoví jako po pádu; pro testy obnovy |

## První ovladač krok za krokem

Ukázka je ovladač `edu` (`user/edu`) pro výukové zařízení QEMU `edu`
(PCI `1234:11e8`): registry v BAR0, výpočet faktoriálu s přerušením MSI
a přenos DMA. Je to nejmenší ovladač, který má všechno, co skutečný
hardware. Fakta o zařízení jsou ze specifikace QEMU `docs/specs/edu.rst`
(kód QEMU je pod GPL, převzatá jsou jen fakta o registrech).

### 1. Kostra

```bash
cargo xtask new-driver edu --pci 1234:11e8
```

Vytvoří `user/edu/` (Cargo.toml, manifest, `src/lib.rs` s ovladačem,
`src/main.rs`, `src/model.rs` s modelem zařízení, `src/tests.rs`), stránky
`docs/cs/ovladac-edu.md` a `docs/en/driver-edu.md`, přidá člena workspace
a řádek do `docs/README.md`. Jinde nic měnit netřeba: devmgr i xtask
ovladač najdou podle manifestu.

Logika je v knihovně (`src/lib.rs`), program jen spustí driverkit. Na
hostiteli se program přeloží prázdný, aby šlo spustit `cargo test`:

```rust
#![cfg_attr(target_os = "magnumopus", no_std, no_main)]

#[cfg(target_os = "magnumopus")]
libos::entry!(main);

#[cfg(target_os = "magnumopus")]
fn main(startup: libos::Startup) -> i64 {
    driverkit::os::main::<edu::Edu<_>>(startup, "edu")
}
```

### 2. Manifest

```toml
[match]
pci = ["1234:11e8"]

[resources]
bars = [0]
irq_max = 1
dma_max = "16K"
```

Jen to, co ovladač opravdu potřebuje: edu má registry v BAR0, jedno
přerušení a dva buffery po 4 KiB. Celý formát v kapitole
[Manifest](#manifest).

### 3. Registry a probe

Registry podle specifikace (viz [Registry](#registry)). `probe` namapuje
BAR, ověří identifikaci a kontrolu života, zapne přerušení a alokuje DMA:

```rust
        // Kontrola zivota: karta vrati prevracene bity.
        LIVENESS.write(&regs, 0x1234_5678);
        if LIVENESS.read(&regs) != !0x1234_5678 {
            return Err(DriverError::Device(String::from("karta neprosla kontrolou zivota")));
        }
        let irq = p.enable_irq()?;
        let out = p.alloc_dma("edu-tam", EDU_BUF_SIZE)?;
        let back = p.alloc_dma("edu-zpet", EDU_BUF_SIZE)?;
```

### 4. Požadavek a přerušení

Zpráva od klienta spustí práci, přerušení ji dokončí. Ovladač nečeká
aktivně: mezi zprávou a přerušením smyčka obsluhuje další události.

```rust
        if let Some(n) = text.strip_prefix("fact ") {
            let Ok(n) = n.trim().parse::<u32>() else {
                return reply(p, "chyba fact ocekava cislo");
            };
            STATUS::IRQ_FACT.set(&self.regs);
            FACTORIAL.write(&self.regs, n);
            self.op = Op::Factorial(n);
            return Ok(());
        }
```

```rust
    fn interrupt(&mut self, p: &P, irq: IrqId) -> Result<()> {
        debug_assert_eq!(irq, self.irq);
        let why = IRQ_STATUS.read(&self.regs);
        // Potvrdit driv nez dalsi prace: nove preruseni se neztrati.
        IRQ_ACK.write(&self.regs, why);
        match self.op.clone() {
            Op::Factorial(n) if IRQ_STATUS::FACT.get(why) != 0 => {
                let v = FACTORIAL.read(&self.regs);
                self.op = Op::Idle;
                reply(p, &format!("fact {n} {v}"))
            }
```

### 5. DMA

Adresy pro zařízení jen z `iova_of`; před spuštěním přenosu bariéra, aby
data v paměti byla vidět dřív, než zařízení začne číst:

```rust
    fn start_dma(&self, p: &P, src: u64, dst: u64, count: u64, to_ram: bool) -> Result<()> {
        DMA_SRC.write(&self.regs, src);
        DMA_DST.write(&self.regs, dst);
        DMA_COUNT.write(&self.regs, count);
        let cmd = DMA_CMD::START.value(1)? | DMA_CMD::IRQ.value(1)? | DMA_CMD::TO_RAM.value(to_ram as u64)?;
        // Data v pameti driv, nez zarizeni zacne prenos.
        p.mmio_wmb();
        DMA_CMD.write(&self.regs, cmd);
        Ok(())
    }
```

### 6. Stop

`stop` se volá vždy, i po chybě. Musí zařízení zastavit tak, aby už
nesahalo do DMA paměti ani nevyvolávalo přerušení:

```rust
    fn stop(&mut self, _p: &P) {
        // Zadne dalsi preruseni ani DMA: vypnout preruseni po vypoctu.
        // Rozbehnuty prenos edu nejde zrusit, dobehne do bufferu, ktere
        // patri ovladaci (s IOMMU je po konci procesu zarizeni nedosahne).
        STATUS::IRQ_FACT.clear(&self.regs);
    }
```

### 7. Test v simulaci

Model zařízení (`src/model.rs`) napodobí registry, přerušení a DMA
([Simulace hardwaru](#simulace-hardwaru)). Test spustí ovladač nad
simulací s kvótami z manifestu a projde ho po krocích ve virtuálním čase:

```rust
fn sim() -> Sim {
    let info = DeviceInfo { bus: 0, device: 4, function: 0, vendor: crate::VENDOR, device_id: crate::DEVICE };
    let manifest = driverkit::manifest::parse(include_str!("../edu.driver.toml")).unwrap();
    Sim::new("edu", info, EduModel::new()).with_manifest(&manifest)
}
```

```rust
#[test]
fn dma_roundtrip_stays_in_iommu() {
    let sim = sim();
    let p = sim.platform();
    let mut r = driverkit::Runner::<Edu<_>>::new(&p).ok().unwrap();
    sim.send(b"dma ahoj z DMA");
    for _ in 0..3 {
        assert_eq!(r.step(&p), None); // zprava, DMA tam, DMA zpet
    }
    assert_eq!(sim.sent_text(), ["dma ahoj z DMA"]);
```

`cargo test -p edu` trvá zlomek sekundy; žádné QEMU, žádný build systému.

### 8. Test v QEMU

`cargo xtask test --driver edu`: testy ovladače na hostiteli, build
systému a v QEMU jen sada `ovladace z manifestu` (ping všech ovladačů
z manifestu, faktoriál přes MSI, DMA tam a zpět přes IOMMU, obnova po
`dk:crash` a `dk:restart`). Bez kontroly jader ostatních architektur
a dokumentace, takže je to nejrychlejší smyčka. Úplný běh je
`cargo xtask test` (na `--arch x86_64` i `--arch aarch64`).

xtask přidá do QEMU `-device edu,dma_mask=0xffffffffffffffff`, když ho
QEMU má (`-device help`); jinak testy edu přeskočí s hláškou. Ovladač
z `new-driver` pro hardware, který QEMU neemuluje, test v QEMU jen
přeskočí (`zarizeni v QEMU neni`); ověřuje ho simulace.

### 9. Dokumentace

Stránku ovladače (`docs/cs/ovladac-<jméno>.md` a anglickou) doplnit:
co zařízení dělá, odkud jsou fakta, původ kódu a licence, protokol, testy.
`cargo xtask docs` zkontroluje odkazy.

## Manifest

`user/<jméno>/<jméno>.driver.toml`. xtask ho najde, přeloží program,
přibalí program i manifest do initfs a přidá crate do testů na hostiteli.
devmgr manifesty čte při startu a ovladač spustí pro zařízení, pro která
nemá vestavěný ovladač. Chyba manifestu je chyba buildu.

| Klíč | Význam | Výchozí |
|---|---|---|
| `driver.name` | jméno programu v initfs = jméno adresáře; malá písmena, číslice, `-` | povinné |
| `driver.description` | popis | |
| `driver.license` | licence kódu ovladače | povinné |
| `driver.origin` | `vlastni`, nebo odkud je kód převzatý (projekt, verze, licence) | povinné |
| `match.pci` | `["vvvv:dddd", ...]` (hex) | |
| `match.class` | `["tt.pp.rr", ...]` třída, podtřída, rozhraní (hex) | |
| `resources.bars` | BARy, které ovladač dostane (0-5) | žádné |
| `resources.irq_max` | kolik přerušení smí zapnout (0-64) | 1 |
| `resources.dma_max` | kolik DMA paměti smí alokovat za život (`65536`, `"64K"`, nejvýš 256 MiB) | 0 |
| `service.restart` | po pádu spustit znovu | `true` |
| `service.max_restarts` | nejvýš tolik pádů za běh systému | 5 |

Aspoň jedno `match.*` je povinné. Ovladač dostane jen vyjmenované BARy:
pokus namapovat jiný skončí chybou `ovladac nesmi: mapovat BAR 2 ... ->
pridejte BAR do resources.bars v manifestu ovladace`.

Initu devmgr pošle `dev <jméno>` s kanálem ke klientovi ovladače; init ho
drží v `Devices::drivers` a po obnově ovladače ho vymění.

## Simulace hardwaru

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Ovladač nad jádrem i v simulaci">
<defs><marker id="dr2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="260" y="10" width="240" height="60" rx="6"/>
<text class="t b" x="380" y="35" text-anchor="middle">ovladač: Driver&lt;P: Platform&gt;</text>
<text class="m" x="380" y="54" text-anchor="middle">stejný kód v systému i v testu</text>
<rect class="acc" x="40" y="120" width="300" height="70" rx="6"/>
<text class="t b" x="190" y="144" text-anchor="middle">OsPlatform (feature os)</text>
<text class="m" x="190" y="162" text-anchor="middle">BAR přes Resource, MSI/MSI-X, DMA přes IOMMU</text>
<text class="m" x="190" y="178" text-anchor="middle">wait_many na přerušení a kanál</text>
<rect class="acc" x="420" y="120" width="300" height="70" rx="6"/>
<text class="t b" x="570" y="144" text-anchor="middle">SimPlatform (feature sim)</text>
<text class="m" x="570" y="162" text-anchor="middle">virtuální čas, kvóty z manifestu</text>
<text class="m" x="570" y="178" text-anchor="middle">hlídá DMA jako IOMMU, záznam přístupů</text>
<rect class="box" x="40" y="210" width="300" height="34" rx="6"/>
<text class="t" x="190" y="232" text-anchor="middle">skutečné zařízení / QEMU</text>
<rect class="box" x="420" y="210" width="300" height="34" rx="6"/>
<text class="t" x="570" y="232" text-anchor="middle">DeviceModel (model podle datasheetu)</text>
<path class="ln" d="M330 70 L200 118" marker-end="url(#dr2)"/>
<path class="ln" d="M430 70 L560 118" marker-end="url(#dr2)"/>
<path class="ln" d="M190 190 L190 208" marker-end="url(#dr2)"/>
<path class="ln" d="M570 190 L570 208" marker-end="url(#dr2)"/>
</svg>
<figcaption>Jeden ovladač, dva backendy: jádro a simulace na hostiteli.</figcaption>
</figure>

Simulace (`driverkit::sim`, feature `sim`) je určená hlavně pro hardware,
který QEMU neemuluje (grafika Intel, síťovky Realtek a Intel igc, kodeky
HDA): ovladač jde vyvíjet a testovat podle datasheetu dřív, než je po ruce
skutečný stroj, a test zůstane jako regresní. Co simulace dělá:

- **registry**: přístup do BARu volá `DeviceModel::read` / `write` modelu;
  každý přístup se zapíše do `Sim::trace` (čas, BAR, offset, šířka,
  hodnota),
- **přerušení**: model volá `bus.raise_irq(n)`; přerušení, které ovladač
  nezapnul, se zahodí a zaloguje,
- **DMA**: model čte a zapisuje paměť přes `bus.dma_read` / `dma_write`;
  přístup mimo paměť, kterou ovladač alokoval, se zapíše do `Sim::faults`
  jako `DmaFault` (na stroji by ho zablokovala IOMMU). Mezi buffery je
  mezera jedné stránky, takže přetečení za konec bufferu se pozná,
- **kvóty a práva** podle manifestu (`Sim::with_manifest`): BAR, který
  ovladač nedostal, `irq_max`, `dma_max`,
- **čas** je virtuální: `wait` posouvá čas a volá `DeviceModel::tick`
  (model tam dokončí rozpracovanou práci). Test je deterministický
  a rychlý; když ovladač čeká a nic se neděje, `wait` po `Sim::time_limit`
  (výchozí 10 s virtuálního času) vrátí chybu místo zaseknutí,
- **klient**: `Sim::send`, `Sim::close`, `Sim::sent_text`,
  `Sim::set_client_full` (plná fronta klienta).

Model se píše jen v rozsahu, který ovladač používá. `RegFile` vyřeší
běžnou mechaniku registrů (hodnota po resetu, bity jen pro čtení, W1C):

```rust
            regs: RegFile::new()
                .ro(ID, 0x0100_00ed)
                .rw(LIVENESS, 0)
                .rw(FACTORIAL, 0)
                // BUSY (bit 0) jen cteni.
                .reg(STATUS, 0, 1, 0)
                .ro(IRQ_STATUS, 0)
```

Model má vlastní chování jen tam, kde zápis něco spouští:

```rust
    fn tick(&mut self, bus: &mut Bus) {
        if self.fact_done.is_some_and(|t| bus.now() >= t) {
            self.fact_done = None;
            let n = self.regs.get(FACTORIAL) as u32;
            let v = (1..=n).fold(1u32, |a, k| a.wrapping_mul(k));
            self.regs.set(FACTORIAL, v as u64);
            self.regs.clear_bits(STATUS, 1);
            if self.regs.get(STATUS) & 0x80 != 0 {
                self.raise(bus, 1);
            }
        }
```

Pro vlákna ovladačů (síť, zvuk, nativní zobrazení): model patří do crate
ovladače (`src/model.rs`, jen `#[cfg(test)]`), události zvenku (přijatý
paket, připojený monitor, odpojený kabel) vkládá test přes
`Sim::with_model(|m: &mut Model, bus| ...)`. Model není důkaz, že ovladač
funguje na hardwaru (model může datasheet pochopit stejně špatně jako
ovladač); ověřuje logiku, meze a pořadí kroků. Před sloučením ovladače pro
skutečný hardware patří do popisu, na čem byl vyzkoušený.

## Životní cyklus a obnova po pádu

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Životní cyklus ovladače">
<defs><marker id="dr3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="10" y="20" width="110" height="50" rx="6"/><text class="t b" x="65" y="50" text-anchor="middle">probe</text>
<rect class="acc" x="160" y="20" width="110" height="50" rx="6"/><text class="t b" x="215" y="50" text-anchor="middle">start</text>
<rect class="ok" x="310" y="10" width="200" height="70" rx="6"/><text class="t b" x="410" y="36" text-anchor="middle">události</text><text class="m" x="410" y="55" text-anchor="middle">přerušení · zpráva · časovač</text><text class="m" x="410" y="70" text-anchor="middle">dk:ping, dk:info</text>
<rect class="acc" x="560" y="20" width="110" height="50" rx="6"/><text class="t b" x="615" y="50" text-anchor="middle">stop</text>
<path class="ln" d="M120 45 L158 45" marker-end="url(#dr3)"/>
<path class="ln" d="M270 45 L308 45" marker-end="url(#dr3)"/>
<path class="ln" d="M510 45 L558 45" marker-end="url(#dr3)"/>
<text class="m" x="515" y="35">konec</text>
<rect class="box" x="400" y="130" width="350" height="80" rx="6"/>
<text class="t b" x="575" y="154" text-anchor="middle">devmgr</text>
<text class="m" x="575" y="172" text-anchor="middle">kód 0: konec · 75 (dk:restart): hned znovu</text>
<text class="m" x="575" y="188" text-anchor="middle">jinak pád: vypnout bus master, nový Resource,</text>
<text class="m" x="575" y="204" text-anchor="middle">spustit znovu (nejvýš max_restarts), init: nový kanál</text>
<path class="ln" d="M615 70 L615 128" marker-end="url(#dr3)"/>
<path class="ln dash" d="M400 170 L65 170 L65 72" marker-end="url(#dr3)"/>
<text class="m" x="80" y="160">nový proces: probe začíná resetem zařízení</text>
</svg>
<figcaption>Ovladač od startu po konec a obnova devmgr po pádu.</figcaption>
</figure>

Návratové kódy ovladače na driverkitu (devmgr je vypisuje slovy):

| Kód | Význam | devmgr |
|---|---|---|
| 0 | klient zavřel kanál | konec |
| 2 | špatná konfigurace od devmgr | obnova (pád) |
| 3 | chybí právo nebo kvóta (manifest) | obnova (pád); oprava je v manifestu |
| 4 | chyba zařízení nebo ovladače | obnova (pád) |
| 70 | `dk:crash` | obnova (pád) |
| 75 | `dk:restart` | hned znovu, nepočítá se |
| záporné | jádro ukončilo proces (výjimka CPU, panika, kill) | obnova (pád) |

Rychlý restart bez restartu systému: klient pošle `dk:restart` (např. test
nebo služba po změně nastavení), nebo proces ovladače spadne. Obnova trvá
desetiny sekundy. Pravidlo pro autora: **`probe` nesmí předpokládat žádný
stav z minula**, vždy začíná resetem zařízení; klient po obnově dostane nový
kanál (init: `init: ovladac X obnoven (N. start)`).

## Čitelné chyby

`DriverError` nese, co se stalo, a radu za `->`. Ukázky hlášek:

| Chyba | Hláška |
|---|---|
| chybí BAR v manifestu | `ovladac nesmi: mapovat BAR 2 (spravce zarizeni ho nepredal) -> pridejte BAR do resources.bars v manifestu ovladace` |
| kvóta DMA | `DMA buffer 'edu-zpet' (4096 B) se nevejde do kvoty -> zvyste resources.dma_max v manifestu, nebo pouzijte mensi buffer` |
| kvóta přerušení | `preruseni c. 1 nelze zapnout -> zvyste resources.irq_max v manifestu ovladace` |
| DMA mimo buffer (ovladač) | `pristup mimo DMA buffer 'tx': offset 0x40 + 16 B > velikost 64 B -> chyba v ovladaci (spatny index nebo delka)` |
| DMA mimo IOMMU (simulace) | `DMA mimo pridelenou pamet: zarizeni zapsalo 3 B na 0x100003000 -> ovladac zapsal do zarizeni adresu, kterou nedostal od DmaBuffer::iova_of (IOMMU by pristup zablokovala)` |
| pole registru | `hodnota 0x8 se nevejde do pole CTRL.MODE (3 b)` |
| volání jádra | `mapovani BARu: ACCESS_DENIED (-2): ... -> ovladac na to nema pravo; zkontrolujte resources v manifestu ovladace` |

Na skutečném stroji zablokovaný přístup DMA hlásí jádro:
`IOMMU: zablokovan zapis zarizeni BB:ZZ.F na adrese ...`
([Hlášky](hlasky.md#jádro-iommu)).

## Pravidla zdrojů a licencí

**Nevymýšlet znovu, převzít.** Ovladač pro běžný hardware už někdo napsal
a odladil na tisících strojů; jeho chyby a obejití chyb hardwaru
(errata) jsou cennější než vlastní pokus.

| Původ | Jak převzít |
|---|---|
| malý ovladač pod BSD, MIT, ISC, Apache-2.0 (FreeBSD, OpenBSD, NetBSD, Redox, Haiku) | přeložit do Rustu na driverkit; zachovat copyright a licenci v hlavičce souboru |
| kód pod GPL (většina Linuxu) | **nepřekládat** a nekopírovat; brát jen fakta o hardwaru (registry, pořadí kroků, errata), nejlépe z datasheetu |
| obří ovladač (grafika DRM AMD/NVIDIA, Wi-Fi, ACPICA) | převzít v C jako izolovaný proces přes vrstvu kompatibility (níže); jen s licencí, která to dovoluje |
| datasheet, specifikace (PCI-SIG, NVMe, AHCI, xHCI, virtio, HDA) | volně; uvést zdroj v hlavičce |

Každý převzatý soubor má v hlavičce: odkud je (projekt, verze nebo commit,
cesta), licenci a copyright původních autorů, co se změnilo. Převzatý kód
patří i do `THIRD-PARTY.md` v kořeni repozitáře (projekt, verze, licence,
cesta u nás). V manifestu to říká `driver.origin` a `driver.license`.

## Rust, nebo převzaté C

| Situace | Volba |
|---|---|
| nový nebo malý ovladač (do několika tisíc řádků), zdroj pod BSD/MIT nebo jen datasheet | Rust na driverkitu |
| ovladač, kde je hlavní hodnota v letech ladění (stovky tisíc řádků, firmware, errata): DRM AMD/NVIDIA, Wi-Fi (iwlwifi, ath), ACPICA | převzít C beze změny logiky, izolovaný proces |
| kód pod GPL | ani jedno: jen fakta, ovladač napsat v Rustu |

Převzaté C běží jako **samostatný proces** se stejnými pravidly jako
ovladač v Rustu: omezený Resource, IOMMU, manifest, obnova po pádu.
Chyba v C (přetečení bufferu) tak zůstane uvnitř procesu. Vrstva
kompatibility mapuje služby, které C kód čeká od svého jádra (alokace
paměti, mapování registrů, DMA, přerušení, zámky, časovače, logy), na
stejné prostředky, jaké má `Platform` v driverkitu. Vrstvu zavádí vlákno
ACPI (ACPICA); další převzaté ovladače ji použijí. Překlad C pro cíl OS
a jeho knihovna patří k plánu jazyků (vlastní libc, clang).

## Ladění

- **Logy**: každá hláška ovladače má prefix `jméno BB:ZZ.F:`
  (`driverkit::log!`). Výstup sériové linky je po běhu v
  `target/serial-*.log` ([Sestavení](sestaveni.md#výstup-a-dohledání-chyb)).
  Význam hlášek: [Hlášky](hlasky.md#ovladače-a-správce-zařízení).
- **Živost**: `dk:ping` a `dk:info` ověří, že smyčka ovladače běží
  a ke kterému zařízení patří.
- **Simulace**: `Sim::trace()` je úplný záznam přístupů do registrů,
  `Sim::logs()` hlášky ovladače i simulace, `Sim::faults()` DMA mimo
  paměť. Nejrychlejší cesta k chybě v pořadí kroků.
- **GDB**: `cargo xtask run --gdb` (QEMU čeká na GDB), pak symboly
  programu ovladače z `target/user/<cíl>/debug/<jméno>`
  ([Ladění přes GDB](sestaveni.md#ladění-přes-gdb)).
- **QEMU monitor**: `cargo xtask run -- -monitor stdio` a v něm
  `info pci` (BARy, přerušení), `info mtree` (mapování paměti),
  `info qtree`. Testy používají QMP (`xtask/src/qmp.rs`) pro vkládání
  událostí.
- **Jen jeden ovladač**: `cargo xtask test --driver <jméno>`, nebo jen
  vybrané sady `cargo xtask test --only "ovladace z manifestu"`, nestabilní
  test opakovaně `--repeat 20`.

## Kontrolní seznam před sloučením

- [ ] Manifest: jen potřebné BARy, `irq_max` a `dma_max` s rezervou, ne
      s velkou; `driver.origin` a `driver.license` vyplněné.
- [ ] Původ kódu: hlavička souborů, `THIRD-PARTY.md` u převzatého kódu,
      žádný překlad kódu pod GPL.
- [ ] `probe` začíná resetem zařízení a nepředpokládá stav z minula.
- [ ] `stop` zastaví DMA i přerušení; ověřeno testem (stav modelu po
      `finish`).
- [ ] Všechny adresy pro zařízení z `iova_of`; žádné ruční sčítání IOVA.
- [ ] Každé číslo od zařízení (index, délka, stav) je kontrolované;
      vadné zařízení = `DriverError::Device`, ne panika.
- [ ] Bariéry: `dma_wmb` mezi deskriptorem a indexem, `mmio_wmb` před
      doorbell, `dma_rmb` po indexu od zařízení (aarch64 to potřebuje).
- [ ] Test v simulaci: běžná cesta, chyba zařízení, `Sim::faults()`
      prázdné.
- [ ] `cargo xtask test --driver <jméno>` a celý `cargo xtask test` na
      x86_64 i aarch64.
- [ ] Stránka ovladače cs+en, nové hlášky v [Hlášky](hlasky.md),
      `cargo xtask docs`.
- [ ] Komentáře česky, v kódu jen ASCII interpunkce, soubory do ~1500
      řádků, testy v oddělených souborech.
- [ ] Závažná omezení (bez IOMMU, chybějící reset, errata) řečená
      výslovně v popisu změny i v dokumentaci.
