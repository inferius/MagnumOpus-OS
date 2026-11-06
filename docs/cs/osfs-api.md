# Rozšířené API souborového systému pro aplikace ve Windows

Na jednotce připojené přes `osfs mount` (viz [osfs.md](osfs.md)) jde
pracovat běžně přes Win32 - otevřít, číst, zapisovat, přejmenovat, mazat.
Tohle API dodává navíc to, co Win32 neumí a co náš FS ví:

| Co | K čemu |
|---|---|
| **okamžité velikosti adresářů** (`tree_bytes`, `tree_files`, `tree_dirs`) | sloupec Velikost u složek bez procházení podstromu |
| **identita objektu** (`oid` + `generation`) | sledovat soubor i po přejmenování a přesunu |
| **jména skrytá ve Windows** (`CON.txt`, `a.`, jména s U+F0xx, odkazy) | zobrazit je, přečíst, přejmenovat na platné jméno |
| **cíle symbolických odkazů** | zobrazit odkaz a kam míří |
| **snapshoty** | seznam, vytvoření, procházení a čtení starých verzí |
| **stav svazku** | generace, volné místo, jen pro čtení, obnova z logu |

Zdroj: protokol a klient `libs/osfs-proto`, server `tools/osfs/src/api.rs`
(volaný z ovladače `tools/osfs/src/mount/ops.rs`, funkce `control`).

## Jak to funguje

```text
 aplikace (MagnumOpus Explorer, osfs info, ...)
     |  Client::open("Q:\")  ->  CreateFileW(koren jednotky)
     |  DeviceIoControl(IOCTL_READ / IOCTL_WRITE, pozadavek -> odpoved)
     v
 jadro Windows -> WinFsp -> osfs.exe: callback Control
     |  api::handle: dekodovani, kontrola kodu, vykonani pod zamkem Fs
     v
 libs/fs (Fs)
```

- **Kanál:** `DeviceIoControl` na handlu kořene jednotky. Stejné kódy bude
  obsluhovat i budoucí nativní ovladač jádra, takže aplikace nepozná,
  který ovladač běží, a nebude potřeba nic měnit.
- **Rozpoznání svazku:** `GetVolumeInformationW` vrací název systému
  souborů `MagnumOpusFS` (`osfs_proto::FS_NAME`, `Client::is_osfs`).
- **Dva kódy:** `IOCTL_READ` (operace jen pro čtení, handle stačí otevřít
  pro čtení) a `IOCTL_WRITE` (přejmenování, snapshot, commit - Windows ho
  pustí jen na handle otevřený i pro zápis). Server navíc odmítne zápisovou
  operaci poslanou kódem pro čtení (`ACCESS_DENIED`).
- **Zprávy:** malé binární zprávy s hlavičkou `MOFS`, verzí a kódem
  operace; čísla little-endian, řetězec = délka u16 + UTF-8
  (`libs/osfs-proto/src/msg.rs`). Dekódování nikdy nepanikaří - vstup
  přichází od cizího procesu (a v nativním ovladači do jádra).
- **Limit 4 KiB:** WinFsp propustí `DeviceIoControl` jen do 4 KiB v každém
  směru (`FSP_FSCTL_DEVICECONTROL_SIZEMAX`). Proto výpis i seznam snapshotů
  jdou po stránkách a čtení po 4080 B (`MAX_READ`). Server svůj limit hlásí
  v `Hello` (`max_message`), nativní ovladač může mít víc.

## Operace

| Operace | Kód | Vstup | Výstup |
|---|---|---|---|
| `Hello` | čtení | - | verze, schopnosti (`feature::*`), jméno serveru, `max_message` |
| `VolumeInfo` | čtení | - | jmenovka, bloky, volné, generace, algoritmus součtů, jen pro čtení, obnova |
| `Stat` | čtení | `at`, cesta | `EntryInfo` (uložené jméno, i velikost písmen) |
| `List` | čtení | `at`, cesta, kurzor | stránka `EntryInfo` + kurzor další stránky |
| `Read` | čtení | `at`, cesta, offset, délka | data (nejvýš `MAX_READ`) |
| `Snapshots` | čtení | `after` (id) | stránka snapshotů + `next` |
| `Rename` | zápis | z, na | generace |
| `SnapshotCreate` | zápis | jméno | id snapshotu (uloží = commit) |
| `Commit` | zápis | - | generace uloženého stavu |

- **`at`** = jméno snapshotu, prázdné = živý stav.
- **Cesta** je buď `PathArg::Fs("/a/CON.txt")` - jména tak, jak jsou
  v našem FS (i skrytá), nebo `PathArg::Windows("\a\b")` - tak, jak ji
  vidí Windows (přeloží se stejně jako při otevření souboru;
  `PathArg::from_windows_full("Q:\a\b")`).
- **`EntryInfo`:** `name` (náš FS), `windows_name` (prázdné = skryté),
  `hidden` (proč), `kind`, `oid`, `generation`, `size`, časy `mtime`,
  `ctime`, `btime` (ns od 1970), `tree_*`, `link_target`, `truncated`.
  `windows_name` popisuje jen samotné jméno: položka ve skrytém adresáři
  je z Windows nedosažitelná, i když její vlastní jméno je platné.
- **Smazání snapshotu** API záměrně nenabízí: snapshoty chrání proti
  ransomwaru a jakýkoli program uživatele by je jinak mohl smazat.
  Přijde s oprávněními (návrh v3).

### Chyby

Odpověď nese `status`: 0 = v pořádku, jinak kód z `osfs_proto::error`
(stejná čísla jako `abi::Error` v MagnumOpus OS, hlídá test). Klient je
převádí na `io::Error` s čitelným jménem, např. `osfs: NOT_FOUND (-8)`.

## Příklad (Rust)

```rust
use osfs_proto::{Hidden, PathArg, client::Client};
use std::path::Path;

let root = Path::new("Q:\\");
if Client::is_osfs(root) {
    let mut c = Client::open(root)?;
    let v = c.volume_info()?;
    println!("{}: volno {} B", v.label, v.free_blocks * v.block_size as u64);

    // Výpis složky i se skrytými jmény a okamžitými velikostmi.
    for e in c.list_all("", &PathArg::from_windows_full("Q:\\Fotky"))? {
        let size = if e.kind == osfs_proto::Kind::Dir { e.tree_bytes } else { e.size };
        let note = if e.hidden == Hidden::Visible { "" } else { " (ve Windows skryté)" };
        println!("{} {size} B{note}", e.name);
    }

    // Stará verze souboru ze snapshotu.
    let old = c.read("vcera", &PathArg::Fs("/Fotky/popis.txt".into()), 0, 1 << 20)?;

    // Skryté jméno přejmenovat na platné (pak ho uvidí i Windows).
    c.rename(&PathArg::Fs("/CON.txt".into()), &PathArg::Windows("\\CON_.txt".into()))?;
}
# Ok::<(), std::io::Error>(())
```

Totéž z příkazové řádky (ukázka a ruční test API):

```text
> osfs info Q:
server: osfs 0.1.0 (WinFsp) (protokol 1, schopnosti 0x7)
svazek 'Zkouska': 16384 bloku po 4096 B, volnych 13498, generace 17

> osfs ls Q: /
file           11  CON.txt   [ve Windows skryte: rezervovane jmeno]
dir      10846218  slozka2
file           11  x:y?.txt

> osfs snapshot Q: pred-upravou
snapshot 'pred-upravou' vytvoren (id 1)

> osfs cat Q: /verze.txt --at pred-upravou
puvodni
```

## Použití v jiném projektu

`libs/osfs-proto` je samostatný crate bez závislostí (edice 2021, Rust
1.85+, `no_std` + `alloc`; feature `client` přidá klienta pro Windows).
Projekt bez přístupu k tomuto repozitáři (např. s CI na GitHubu) si ho
může zkopírovat k sobě; protokol je verzovaný (`VERSION`, `Hello`), takže
kopie zůstane kompatibilní, dokud se verze nezmění.

## Výkon (orientačně, WinFsp, ladicí build)

| Operace | Čas |
|---|---|
| výpis 3000 položek (`list_all`) | 0,12 s |
| čtení 10 MiB (`read`, po 4 KiB) | 0,25 s |

Pro běžné čtení a zápis souborů je rychlejší obyčejné Win32 přes jednotku;
`Read` přes API je hlavně pro skrytá jména a snapshoty.

## Omezení a rizika

- Kurzor výpisu je složené jméno poslední položky (přežije změny adresáře
  mezi stránkami). U jmen delších než 1 KiB se použije pořadí ve snímku
  výpisu, které platí jen se stejným handlem (`Client` ho drží).
- Položka, která se nevejde do 4 KiB ani sama (jméno ~4 KiB + cíl odkazu),
  přijde zkrácená s `truncated = true`.
- Přejmenování přes API proběhne za zády mezipaměti Windows: Průzkumník
  může do 1 s ukazovat staré jméno (`FileInfoTimeout`).
- Cíl odkazu ve snapshotu API `fs` zatím neumí přečíst (`link_target`
  prázdný).
- `Client` není `Sync`: každé vlákno si otevře vlastní (otevření je levné).
