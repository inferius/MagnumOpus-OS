# libs/fs

Souborový systém (`no_std`, `std` jen přes feature pro nástroje). Copy-on-write, Merkleův strom s kontrolními součty, jeden B+strom metadat, extenty s odloženou alokací, space logy, intent log (rychlý `fsync`), snapshoty, datasety a klony, defragmentace, fsck a scrub. Aktuální formát na disku je **v3** (`MAGIC = OSFS\0\0v3`, `src/layout.rs`). Pracuje nad libovolným `BlockDevice`: ve službě `fsd` nad ovladačem disku, v xtask nad souborem, v testech nad pamětí. Popis: `docs/cs/souborovy-system.md`, verze `docs/cs/fs-verze.md`, návrh `docs/cs/navrh-fs-v3.md`.

## Klíčové soubory

- `src/fs/mod.rs` (soubory, adresáře), `fs/commit.rs`, `fs/zil.rs` (intent log), `fs/check.rs`, `fs/scrub.rs`, `fs/snap.rs`, `fs/dataset.rs`, `fs/merge.rs`.
- `src/layout.rs` + `src/layout/` (formát na disku), `store.rs`, `records.rs`, `btree.rs`, `space/` (volné místo), `names.rs` (jména), `perm.rs` (oprávnění), `mode.rs` (mód POSIX jako pohled na granty: zobrazení, zúžení při vytvoření, `chmod`; čistá logika, testy `tests/mode.rs`).
- `src/posix_lock.rs` - zámky POSIX pro fsd (vlastník, dělení a slučování rozsahů, graf čekání pro EDEADLK); čistá logika bez I/O, testy `src/tests/posix_lock.rs`.
- `src/tests/` - model, výpadky proudu, výkon, verze, sdílení.

## Invarianty

- Nic živého se nepřepisuje na místě: commit zapisuje nové uzly od listu ke kořeni, pak metadata volného místa, nakonec superblok do dalšího slotu kruhu. Výpadek kdykoli = stav posledního dokončeného commitu (plus `fsync` přes intent log).
- Každý ukazatel nese kontrolní součet a generaci; při poškození se připojení vrátí až o 3 generace.
- Ovladač připojí jen svou verzi formátu; jiná = `NOT_SUPPORTED`. Menší změny formátu přes příznaky vlastností (compat / ro_compat / incompat), ne tichou změnou layoutu. Kompatibilita se starými verzemi není priorita, správnost a výkon ano.
- Jména se porovnávají po NFC a převodu na malá písmena (case-insensitive), ukládají se tak, jak je uživatel zadal.
- Hodnoty z disku jsou nedůvěryhodný vstup: poškozený disk nesmí způsobit panic ani čtení mimo buffer.
- Chyba zařízení během commitu (i `NO_MEMORY`) nesmí nechat stav v paměti nekonzistentní: `write_tree` vyjme uzly z `dirty` až po úspěšném zápisu a při chybě vrátí alokované bloky, `writeback` vrátí nezapsaná data do bufferu. Další commit to zopakuje (test `tests/commit_errors.rs`: chyba na každém zápisu commitu).
- Logická chyba operace (už existuje, neexistuje, ...) musí přijít před první změnou stavu, jinak `guarded` svazek přepne do stavu chyby (v testech knihovny to rovnou spadne; audit `tests/logical_errors.rs`).
- Knihovna odkazy nenásleduje (`walk` dá `NOT_A_DIRECTORY`/`WRONG_TYPE`); `find_link` vrátí první odkaz na cestě s právy jako `walk` (následuje fsd, `user/fsd/src/links.rs`). `readlink` chce jen viditelnost.
- Mód POSIX se neukládá: `Stat::mode` počítá `with_mode` ze zděděných grantů (`Walked::above`, u handlu `Open::above` z doby `open`); `set_mode` mění jen vlastní popis (`PROTECTED`, zúžení při vytvoření nikdy nerozšíří).
- `Stat` rozlišuje známou, skrytou a nedostupnou hodnotu (masky `known`/`hidden`, bity `avail` = `fsproto::avail`): nula bez bitu v `known` není hodnota. Bez `MANAGE` jsou souhrny adresáře v `hidden` (`limit_stat`), `TREE` patří jen adresáři, hranice datasetu má `known` jen `RIGHTS` (test `tests/avail.rs`).

## Testy

- `cargo test -p fs --features std` (proptest); důkladně `PROPTEST_CASES=150000 cargo test -p fs --features std --release`; výkon `-- --ignored --nocapture perf`.
- Služba v QEMU: `cargo xtask test --only "souborovy system"` (další sady: `historie FS`, `data FS`, `vykon FS`, `obnova fsd`).
- Změna formátu na disku: `src/tests/ondisk.rs` a výpadkové testy (`crash*.rs`) musí zůstat zelené.

## Pasti

- Úvodní komentáře `lib.rs` a `layout.rs` ještě říkají "format v2"; platí v3 (`MAGIC`). Při úpravě je oprav.
- Soubory jsou velké (`fs/mod.rs` ~1200 řádků): při růstu dělit do podmodulů, limit je ~1500.
- Změna chování = upravit `docs/cs/souborovy-system.md` i en a značku stavu.
