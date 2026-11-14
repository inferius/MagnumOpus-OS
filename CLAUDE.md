# MagnumOpus-OS: deník pro agenty

Capability mikrojádro v Rustu pro x86_64 a aarch64 (milník 4+, GUI a terminál
rozpracované). Tenhle soubor je rozcestník: krátký, jen ověřená fakta a odkazy.
Podrobnosti jsou v `docs/cs/` (zdroj pravdy), ne tady. Každá důležitá složka
má vlastní `CLAUDE.md` (co dělá, invarianty, testy, pasti) - přečti ten
ve složce, kde pracuješ.

## Mapa systému

- `loader/` - UEFI loader: načte `kernel.elf` + `initfs.img`, postaví tabulky stránek, skočí do jádra s `BootInfo` (`libs/boot-info`).
- `kernel/` - mikrojádro: `mm`, `sched`, `object` (handly+práva), `syscall`, `arch/{api,x86_64,aarch64}`, self-testy v `tests/`.
- `libs/` - ~70 crate. `no_std` knihovny, většina testovatelná na hostiteli:
  - rozhraní: `abi` (jádro<->userspace), `boot-info`, `libos` (API pro programy), `osrt` (std), `*proto` (protokoly služeb), `svcclient`
  - souborový systém: `fs`, `fsproto`, `osfs-proto`, `initfs`
  - GUI: `comp` (jádro kompozitoru), `ui`, `gfx`, `font`, `theme`, `guiproto`, `vkcomp`
  - shell a terminál: `shlang`, `term`, `agentsh`; účty: `accounts`; ovladače: `driverkit`, `pci`, `virtio`, `nic`, ...
- `user/` - procesy userspace: `init` (první proces, supervisor, testy userspace), služby (`fsd`, `netd`, `compd`, `termd`, `accountd`, `devmgr`, `inputd`, `logd`, `timed`, `launchd`, ...), ovladače (`virtio-*`, `nvme`, `ahci`, `xhci`, `e1000e`, `hda`, ...), Soubory (`gui-files` + `files-copy`), ukázky (`echo`, `gui-demo`).
- `xtask/` - `cargo xtask`: build, obraz disku, QEMU, testy, docs, USB, VMware.
- `design/` - klikací HTML prototypy GUI a `tokens.toml` (zdroj pravdy pro vzhled).
- `docs/` - `cs/` a `en/` (vývojářská kniha), `prirucka/` (uživatelská), `README.md` (rozcestník, tabulka stránek).
- `tools/` - `osfs` (FS na hostiteli, WinFsp), `wsl`; `third_party/` - převzatý kód (`THIRD-PARTY.md`).

Vrstvy: aplikace -> libos -> služby (userspace) -> jádro -> `arch`. Přehled a důvody: `docs/cs/architektura.md`, `docs/cs/jadro.md`.

## Pravidla projektu

- Odpovědi, komentáře i dokumentace česky (angličtina jen v `docs/en/`). Komentáře v `.rs` jsou česky **bez diakritiky**; `.md` s diakritikou.
- Kód (i komentáře) jen ASCII: `->` místo šipek, `-` místo en/em-dash, rovné uvozovky, `...` místo `…`. Výjimka: testy, které se znaků týkají (kódování, unicode).
- Soubor max ~1500 řádků; dělit do podmodulů hned, jak to dává smysl. Testy v oddělených souborech (`tests.rs`, `tests/`, `*_tests.rs`).
- `unsafe` jen kde je nutné a vždy s komentářem `// SAFETY:`. Kód specifický pro architekturu jen v `kernel/src/arch/<arch>/`.
- Konce řádků LF, UTF-8 bez BOM; čtení tolerantní k CRLF, žádné tiché převody.
- Veřejné API (abi, libos, protokoly) je hotové až s ověřeným chováním na x86_64 **i** aarch64. Rozhraní se nesmí jen "hodit" na x86.
- Kód pod GPL smí být převzat jen do složky `gpl/` (GPL-2.0-only) a jen jako samostatný proces ovladače mluvící přes veřejný protokol; nikdy do jádra, `libs/`, služeb ani aplikací (`gpl/README.md`).
- Aplikace nikdy nedostane capability k ovladači (registry, DMA); mluví se službou (`docs/cs/rozhrani-zarizeni.md`).
- Dokumentace cs+en, se SVG diagramy a značkami stavu **[navrženo]** / **[implementováno]** / **[ověřeno v QEMU]** / **[ověřeno na HW]** (pravidla: `docs/cs/dokumentace.md`). Změníš kód -> změň značku ve stejném commitu.
- Rozumné rozhodnutí = správné, ne nejjednodušší; kompatibilita se starými verzemi FS není priorita. Drahé úspory místa (komprese) jsou ve výchozím stavu vypnuté.
- Testy cílené: host testy dotčeného crate + `cargo xtask test --only <sada>`, jedna architektura. **Plnou sadu a obě architektury pouští koordinátor** při dávce slučování.
- Commity končí řádkem `Co-Authored-By: <model> <noreply@anthropic.com>` (aktuální model dané relace). Do `main` neslučuj, pracuj ve své větvi.
- Při nejistotě (chybí detail, víc postupů, "asi/nějak takto" v zadání) se zeptej s variantami A/B/C dřív, než napíšeš kód. Návrhy a vložené analýzy jsou podklad, ne zadání - nesouhlas řekni.
- Závažná omezení (např. DMA bez IOMMU) řekni výslovně hned při návrhu, ne jen v komentáři.

## Nejčastější příkazy

Úplný přehled: `PRIKAZY.md`, `cargo xtask help`, `docs/cs/sestaveni.md`. Výchozí `x86_64`, `debug`.

| Co | Příkaz |
|---|---|
| spustit v QEMU | `cargo xtask run --release` (`--arch aarch64`, `--headless`, `--accel whpx`) |
| jen sestavit / obraz | `cargo xtask build` / `cargo xtask image` |
| jen kontrola překladu jádra pro architekturu | `cargo xtask check --arch aarch64` |
| host test jedné knihovny | `cargo test -p <crate>` (fs: `--features std`; driverkit: `--features sim`) |
| vybrané sady userspace testů | `cargo xtask test --only gui,sit` (jména sad: `SUITES` v `user/init/src/tests/mod.rs`) |
| testy jednoho ovladače | `cargo xtask test --driver <jmeno>` |
| dokumentace + kontrola odkazů a značek | `cargo xtask docs` |
| kostra ovladače | `cargo xtask new-driver <jmeno> --pci vendor:device` |

Log sériové linky testu: `target/serial-test-<arch>-<pid>.log`. Běžící `cargo xtask run` zamyká `xtask.exe` - jiný `cargo xtask` se nepřeloží.

## Dokumentace

- `docs/README.md` - tabulka všech stránek (cs i en). Čeština `docs/cs/*.md`, angličtina `docs/en/*.md` (jména anglicky, např. `syscally.md` -> `syscalls.md`).
- Každá věc má jednu vlastnickou stránku; jinde jen odkaz. Čísla (syscally, práva, limity) jsou v `docs/cs/syscally.md` a u kódu.
- Stav milníku: `docs/cs/architektura.md` (kapitola Aktuální odchylky od návrhu).

## Čísla syscallů

Zdroj: `libs/abi/src/syscall.rs`, dnes `COUNT = 57` (čísla 0..56, bez děr). Nový syscall dostane číslo `COUNT` a v jednom commitu:

1. konstanta + zvýšit `COUNT` (`libs/abi/src/syscall.rs`),
2. jméno na správný index `NAMES` a větev v `kernel/src/syscall/mod.rs`,
3. jméno do seznamu v `libs/abi/src/tests.rs` (`every_syscall_is_documented`),
4. nadpis `### \`JMENO\`` v `docs/cs/syscally.md` **i** `docs/en/syscalls.md` (test ověří oba),
5. obálka v `libs/libos/src/sys.rs` (aplikace volají libos, ne čísla).

Existující čísla se nepřečíslovávají. Dvě větve se stejným číslem = konflikt při slučování, řeší koordinátor. Nový kód chyby: další volné číslo v `libs/abi/src/error.rs` + `docs/cs/chyby.md` i `docs/en/errors.md`, jinak selže test.

## Kontrolní seznam před sloučením

Plný text: `docs/cs/dokumentace.md` (kapitola Kontrola před sloučením). Zkráceně: značky stavu sedí s kódem, vlastnická stránka aktuální, cs i en říkají totéž, nová tvrzení mají test, čísla z měření podmínky, `cargo xtask docs` projde, **`CLAUDE.md` dotčené složky odpovídá změnám** (invarianty, klíčové soubory).
