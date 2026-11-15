# libos - knihovna pro programy

`libs/libos` je knihovna, kterou používá každý program v userspace. Je to
**stabilní rozhraní pro aplikace**; syscally pod ní se smí měnit.

## Minimální program

```rust
#![no_std]
#![no_main]

extern crate alloc;

use libos::{Startup, println};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    println!("ahoj");
    0 // návratový kód procesu
}
```

Program potřebuje `build.rs`, který přidá linker skript `user/user.ld`
(viz `user/init/build.rs`), a sestavuje ho `cargo xtask build` (stejný
target jako jádro, ale jiné příznaky překladu). Jméno programu se přidá do
`USER_PROGRAMS` v `xtask/src/main.rs`, aby se dostal do initfs.

## Start procesu

Vstupní bod, heap a panic handler jsou ve feature `rt` (výchozí)
**[implementováno]**. Knihovny závisí na libos s `default-features = false`;
aplikace se `std` mají vlastní runtime (`libs/osrt`, viz
[Rust std](rust-std.md)) a startovní zprávu čtou přes `libos::rt::startup`.

Jádro (u `init`) nebo rodič (u ostatních) spustí první vlákno na symbolu
`_start` z libos s handlem bootstrap kanálu v `rdi`. libos pak:

1. přečte startovní zprávu (`abi::bootstrap`): handly a jejich význam,
2. namapuje heap (začíná na 4 MiB na pevné adrese a roste podle potřeby: když alokace selže, namapuje další paměť hned za konec, nejvýš 64 GiB) a zapne globální alokátor (`alloc::vec::Vec` atd.),
3. zavolá funkci programu z `entry!` se strukturou [`Startup`]:
   - `process` - handle na vlastní proces (mapování paměti, vlákna),
   - `initfs` - obraz initfs (jen `init`),
   - `handles` - další handly od rodiče,
   - `framebuffer` - framebuffer z firmwaru (`init` od jádra, vlastník
     obrazovky od initu; druh `FRAMEBUFFER`, geometrie a převzetí
     `sys::fb_info` / `sys::fb_claim`, viz [Syscally](syscally.md#fb_control)),
   - `acpi` - údaje o ACPI od jádra (jen `init`, druh `ACPI`,
     `abi::acpi::Info`; init je předá službě `acpid`),
   - `bootstrap` - kanál k rodiči pro další komunikaci,
   - `flags` - příznaky startu z argumentu `SELF_PROCESS`
     (`abi::bootstrap::flags`; jádro nastavuje jen initu, např. `TEST_RUN`),
   - `params` - blok parametrů procesu (druh `PARAMS`, viz níže),
4. návratovou hodnotu použije jako návratový kód procesu (`PROCESS_EXIT`).

Panika v programu vypíše zprávu do logu a ukončí proces s kódem -1 000 001.

### Alokátor heapu

- **Malé alokace** (do 4 KiB, zarovnání do 16 B) jdou do 16 velikostních
  tříd (`libs/sizeclass`): každá třída má vlastní seznam volných objektů,
  alokace i uvolnění je O(1). Prázdná třída si vezme kus 4-32 KiB
  a rozdělí ho na objekty. Uvolněný objekt zůstane ve své třídě.
- **Velké alokace** a kusy pro třídy jdou do `linked_list_allocator`
  (first-fit). Jeho seznam volných úseků tak zůstane krátký.
- **Proč:** samotný first-fit prochází při každé alokaci celý seznam
  volných úseků. Po roztříštění heapu (fsd s velkou cache uzlů po testech
  FS) měl seznam tisíce položek: jedno vložení do B-stromu stálo ~10 ms
  a řádek shellu se zápisem 2-4 s. S třídami 0,3 s.
- **Režie:** zaokrouhlení na třídu (nejvýš ~1,5x) a nejvýš jeden
  nevyužitý kus na třídu (dohromady pod 0,3 MiB).

## Typy objektů

Každý typ vlastní handle a zavře ho při zrušení (`Drop`).

| Typ | Účel | Hlavní metody |
|---|---|---|
| `Handle` | obecný handle | `duplicate(práva)`, `into_raw`, `from_raw` |
| `Event` | signál mezi vlákny a procesy | `create`, `signal`, `reset` |
| `DmaMap` | mapování připnuté paměti pro zařízení ([dma.md](dma.md)); `Vmo::create_pinned` | `new`, `at` (nápověda IOVA), `segments`, `device_addr`, `sync`; zavření = odmapování |
| `Fence` | plot: časová osa pro explicitní synchronizaci ([ploty.md](ploty.md)) | `create`, `for_waiter`, `signal`, `fail`, `wait`, `wait_async`, `query`; `fence::wait_any`, `wait_all` |
| `Channel` | zprávy s daty a handly | `create`, `write`, `try_read`, `read(deadline)` |
| `Vmo` | objekt paměti | `create`, `read`, `write` (libovolně dlouhé; jádro přenese 1 MiB na volání, libos dělí) |
| `Process` | proces | `create`, `map`, `unmap`, `start`, `kill`, `wait_exit` |
| `Thread` | vlákno | `create`, `start`, `wait_exit` |
| `Mapping` | namapovaný objekt paměti ve vlastním adresním prostoru (odmapuje se při zrušení) | `new`, `read::<T>`, `write::<T>`, `read_bytes`, `write_bytes` |
| `hw::Resource` | oprávnění k hardwaru (ovladače) | `restrict` (rozsahy paměti, `hw::io_ports`, `hw::irq_lines`), `restrict_device` (s doménou IOMMU), `device_memory`, `dma_memory`, `phys_addr`, `interrupt`, `interrupt_line`, `port_read8`, `port_write8` |
| `hw::Interrupt` | přerušení zařízení (MSI nebo stará linka) | `msi`, `wait(deadline)`, `ack` |

`Mapping::read`/`write` jsou volatile (vhodné i pro registry zařízení) a
přijímají jen celá čísla (`libos::Pod`: `u8`..`u64`, `i8`..`i64`) - z
libovolných bajtů zařízení tak nikdy nevznikne neplatná hodnota (`bool`,
odkaz, výčet). Offset musí být zarovnaný na velikost typu a ležet
v mapování (kontrola i proti přetečení); jinak panika.

```rust
let regs = resource.device_memory(bar_phys, 4096)?;          // jen v rozsahu Resource
let map = Mapping::new(&regs, 0, 4096, MAP_READ | MAP_WRITE)?;
let status: u32 = map.read(0x14);
map.write::<u32>(0x14, status | 1);
```

### Čekání na mnoho klientů: `Waiter`

**[ověřeno v QEMU]** x86_64 i aarch64 (fsd, test „v2: 200 soucasnych
relaci a proud" v sadě `souborovy system`; `WaitSet`: compd 100 klientů
v sadě `gui`, termd 70 relací v sadě `shell`, taskd 100 producentů v sadě
`ulohy`).

`wait_many` (`OBJECT_WAIT_MANY`) bere nejvýš 64 objektů (`WAIT_MANY_MAX`,
pole na zásobníku jádra). **Pravidlo: služba s proměnným počtem klientů
nečeká přes `wait_many` nad seznamem klientů, ale přes port** - jinak
65. klient smyčku zastaví nebo ho služba musí odmítnout. Počet klientů pak
omezuje jen paměť (rozpočet jobu: registrace a paket na portu jsou
strženy jako paměť jádra).

`libos::Waiter` obaluje `Port` a jednorázové registrace
(`OBJECT_WAIT_ASYNC`):

- `key()` - nový jedinečný klíč (nikdy 0, nikdy znovu). Paket zavřeného
  objektu (registrace mohla zůstat) tak nikdy nepatří jinému klientovi;
  neznámý klíč se ignoruje.
- `arm(handle, key, mask)` - jednorázová registrace. Když už signály
  sedí, paket přijde hned (žádné ztracené probuzení mezi obsluhou
  a registrací).
- `wait(deadline)` - čeká na první paket, další připravené vybere bez
  čekání; prázdný výsledek = termín uplynul.

Smyčka: zaregistrovat každý objekt bez registrace -> `wait` -> u klíčů
z paketů zrušit příznak registrace a obsloužit jen ty objekty (vyčíst vše
do `SHOULD_WAIT`) -> znovu. Zavření objektu registraci uklidí jádro
(vrátí rezervaci portu).

**`libos::WaitSet`** - náhrada `wait_many` se stejným rozhraním pro
smyčky, které už skládají pole `(handle, maska)` (compd, termd, accountd,
taskd, inputd, audiod, devmgr, revocd, timed, supervisor initu):
`wait(&items, deadline)` vrátí index první připravené položky a signály
po položkách, termín = `TIMED_OUT`. Registrace drží mezi voláními
(objekt bez události se znovu neregistruje), objekt pozná podle hodnoty
handlu (obsahuje generaci slotu, po zavření a novém otevření je jiná),
objekt, který v poli chybí, zapomene. Port jde sdílet (`port()`) pro
vlastní registrace s klíči pod `FOREIGN_KEYS` (ploty v compd, lístky
supervisoru): takový paket smyčku probudí, s `keep_foreign` si ho volající
vyzvedne `take_foreign`. Port samotný do pole dát nejde (`NOT_SUPPORTED`
jádra) - místo toho sdílet port množiny.

Funkce: `wait_one`, `wait_many` (čekání na signály), `time::now`,
`time::deadline_after`, `time::sleep`, `time::wall`, `time::local` (místní
čas v pásmu z `time::zone`), `exit(kód)`, `println!` (log jádra). Viz
[Reálný čas](cas.md).

Chyby jsou `libos::Error` (= `abi::Error`), popsané v [chyby.md](chyby.md).
Konstanty rozhraní (příznaky mapování, kódy procesů) jsou v `libos::abi`.

## Spuštění programu

```rust
let fs = libos::initfs::Initfs::open(&startup)?;          // jen init
let image = fs.find("echo").ok_or(Error::NotFound)?;
let (process, channel) = libos::spawn("echo", image, Vec::new())?;
channel.write(b"ping:1", Vec::new())?;
let reply = channel.read(None)?;
let code = process.wait_exit(None)?;
```

`spawn` vytvoří prázdný proces ve stejném jobu jako volající, namapuje do
něj segmenty ELF (s právy podle segmentů, W^X) a zásobník 256 KiB, pošle
startovní zprávu a spustí první vlákno. Vrátí proces a konec kanálu
k potomkovi. Paměť programu (segmenty, zásobník) jde na účet jobu potomka.

`spawn_in(&job, ...)` spustí program v jiném jobu, například v podřízeném
s limity:

```rust
use libos::job::{Job, pool};
let job = Job::create(&libos::job::current()?)?;
job.set_limit(pool::USER, 64 << 20)?;
let (process, channel) = libos::spawn_in(&job, "echo", image, Vec::new())?;
```

Potomek dostane svůj job ve startovní zprávě bez práva `WRITE`, takže si
limity nezmění (viz [Joby](syscally.md#joby-rozpočty-prostředků)).

## Blok parametrů procesu

Argumenty, proměnné prostředí a počáteční pracovní adresář předává rodič
objektem paměti jen pro čtení ve startovní zprávě (`kind::PARAMS`,
formát `abi::params`) **[ověřeno v QEMU]** (test `std: hello-std:
argumenty, vlakna, TLS, soubory`). Objekt paměti místo dat zprávy:
velikost neomezuje limit zprávy kanálu. Standardní handly jdou ve stejné
zprávě jako `kind::USER` se štítky z `abi::params::tag` (`STDIN`,
`STDOUT`, `STDERR`, `FS`, `NET`, `LAUNCH` = kanál ke spouštěcí službě
launchd, `AUDIO` a `AUDIO_LEASE` = relace zvukové služby s právy aplikace
a její lístek, [Zvuk](zvuk.md#relace-aplikací-a-nahrávání), `TTY` = řídicí
kanál terminálu, protokol `ttyproto`, [termios](cstd.md#termios)). Štítky, které
runtime std sám nepoužije, si nechá pro knihovny
(`osrt::start::take_tagged`). Formát v2 má pole příznaků (`abi::params::flags`:
`STDOUT_NULL`, `STDERR_NULL` = výstup zahodit, `Stdio::null`). Formát v3
přidal zděděné fd (`Params::fds`, `abi::params::Fd`: číslo, druh
`fd_kind`, stav, data; nejvýš `MAX_FDS` = 48) - handly položek jdou se
štítkem `FD` v pořadí položek ([posix_spawn](cstd.md#procesy-posix_spawn-a-waitpid)).
**[ověřeno v QEMU]** (krok `inherit` testu `ctest`, sada `cstd`).

```rust
use libos::params::Params;
use libos::abi::params::tag;
let params = Params {
    cwd: b"/".to_vec(),
    args: vec![b"hello-std".to_vec(), b"alfa".to_vec()],
    env: vec![b"HELLO=svet".to_vec()],
    ..Default::default()
};
let (out, out_child) = Channel::create()?;
let handles = vec![
    params.start_handle()?,
    StartHandle::tagged(tag::STDOUT, out_child.into_handle()),
];
let (process, _) = libos::spawn("hello-std", image, handles)?;
```

`stdout`, `stderr` a `stdin` jsou kanály s protokolem proudu bajtů:
každá zpráva = kus bajtů, zavření kanálu = konec proudu.

## Zámek nad futexem

`libos::sync::Mutex` (API jako `spin::Mutex`: `lock` vrací strážce bez
chyby) čeká v jádře přes `FUTEX` (`WAIT`), takže čekající vlákno nebere CPU
držiteli zámku. Krátce se nejdřív točí (100 pokusů). Používají ho
`fsproto` a `svcclient` **[ověřeno v QEMU]** (sady `souborovy system`,
`obnova sluzeb`, `shell`).

## Velikost stránky

`libos::vm::page_size()` vrátí velikost stránky vlastního procesu (zeptá
se jádra jednou, `VM_PAGE_SIZE`), `libos::vm::page_size_of(&proces)` jiného
procesu. Není to konstanta: v ABI je jen mez `MIN_PAGE_SIZE` (4 KiB)
a `MAX_PAGE_SIZE` (64 KiB). Zarovnání pro mapování a rozsahy prostředků se
řídí touto hodnotou:

```rust
let page = libos::vm::page_size();
let len = data_len.next_multiple_of(page);
```

## Bariéry pro ovladače

`libos::barrier` řadí přístupy do paměti sdílené se zařízením (DMA) a do
registrů zařízení. `core::sync::atomic::fence` na to nestačí: řadí jen
přístupy mezi procesory a na ARM zápis do registru zařízení může předběhnout
dřívější zápisy do paměti.

| funkce | kdy | x86_64 | aarch64 |
|---|---|---|---|
| `dma_wmb()` | data a deskriptory v DMA paměti před zápisem indexu | `fence(SeqCst)` | `dmb oshst` |
| `dma_rmb()` | přečtený index od zařízení před čtením jeho dat | `fence(SeqCst)` | `dmb oshld` |
| `mmio_wmb()` | všechny zápisy do paměti před zápisem do registru („doorbell“) | `fence(SeqCst)` | `dsb st` |

```rust
write_descriptors();
libos::barrier::dma_wmb();  // deskriptory dřív než index
write_avail_index();
libos::barrier::mmio_wmb(); // index dřív než upozornění zařízení
notify.write::<u16>(offset, queue_index);
```

Takhle to dělá `libs/virtio` (`Virtqueue::submit`, `take_used`,
`Device::notify`).

## Nízká úroveň

`libos::sys` obsahuje přímá volání syscallů (jedna funkce na syscall, viz
[syscally.md](syscally.md)). Je veřejné kvůli testům rozhraní; programy mají
používat typy výše.
