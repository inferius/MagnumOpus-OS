# Historie souborového systému (fáze 2)

Fáze 2 formátu v3 („verze“, viz [návrh FS v3](navrh-fs-v3.md#pořadí-implementace))
přidává k datasetům, snapshotům a klonům z fáze 1 **historii**: žurnál
změn, koš, vracení operací, verze souborů, automatické snapshoty,
verzované commity s logickým ID, větve a merge, „zkusit změny bokem“,
rozšířené atributy a expiraci souborů. Formát a obecné algoritmy popisuje
[souborovy-system.md](souborovy-system.md); změny formátu shrnuje
[Verze formátu](fs-verze.md#fáze-2-historie-ro_compat_history).

> **Změna proti v3 (fáze 1):** nová kapitola. Svazek s historií má bit
> `RO_COMPAT_HISTORY`; nové položky stromu 13-23, pevný objekt koše
> `TRASH_OID` = 2, záznamy intent logu 14-18, chyba `JOURNAL_GAP`,
> operace protokolu 54-86.
> **Proč:** historie musí platit pro každý zápis a přežít pád, takže patří
> do FS; rozhodování (kdy dělat snapshot, kdy vysypat koš) je ve službě
> podle nastavení datasetu.

Kde je co (hranice z návrhu - invariant ve FS, nastavení jako data,
rozhodnutí ve službě):

| Co | Vrstva | Kód |
|---|---|---|
| žurnál změn, pořadová čísla, zveřejnění po potvrzení | invariant (FS) | `libs/fs/src/fs/journal.rs` |
| koš (přesun, obnova, vysypání) | invariant (FS) | `libs/fs/src/fs/trash.rs` |
| rozšířené atributy, index expirace | invariant (FS) | `libs/fs/src/fs/xattr.rs` |
| verze souborů, vrácení operací, automatické snapshoty | mechanismus (FS) | `libs/fs/src/fs/history.rs` |
| commity, logické ID, větve | invariant (FS) | `libs/fs/src/fs/commit.rs` |
| merge stromů, „zkusit bokem“ | mechanismus (FS) | `libs/fs/src/fs/merge.rs`, `libs/fs/src/merge3.rs` |
| retence žurnálu a koše, pravidla snapshotů | nastavení (data datasetu) | `records::Settings` |
| kdy spustit krok snapshotů, expirace, úklid koše | rozhodnutí (služba) | `user/fsd` (každou minutu) |
| záznamy na disku | formát | `libs/fs/src/records.rs` |

## Rychlý příklad

```rust
// Žurnál: indexer naváže přesně tam, kde pohled skončil.
let v = fs.view("/home/alice")?;                       // dočasný snapshot + pozice
index_all(&mut fs, "/home/alice", &v.snapshot)?;       // projít pohled
fs.release_view("/home/alice", &v.snapshot)?;
let mut next = v.seq + 1;
loop {
    let page = match fs.journal_read("/home/alice", next, 256) {
        Ok(p) => p,
        Err(Error::JournalGap) => break,               // retence: znovu od VIEW
        Err(e) => return Err(e),
    };
    for (seq, e) in &page.entries {
        apply_to_index(e);
        next = seq + 1;
    }
}

// Koš a vrácení operace.
let id = fs.trash("/home/alice/zprava.docx")?;          // nic se nekopíruje
fs.trash_restore("/home/alice", id, None)?;             // zpět na původní místo
fs.undo("/home/alice", seq_prejmenovani)?;              // přesun zpět

// Verze souboru a obnova.
for v in fs.file_versions("/home/alice/zprava.docx")? {
    println!("{} {} B", v.snapshot, v.size);
}
fs.restore_version("/home/alice/zprava.docx", "auto-2026-11-07-140000",
                   RestoreMode::Copy("/home/alice/zprava (verze 14:00).docx"))?;

// Commity, větev a merge.
fs.commit_version("/projekt", "Funkční přihlášení")?;
fs.branch("/projekt", "refaktor", "/projekt-refaktor")?;
/* ... změny v /projekt-refaktor ... */
fs.commit_version("/projekt-refaktor", "Refaktor")?;
let r = fs.merge("/projekt", "/projekt-refaktor")?;    // trojcestně
assert!(r.conflicts.is_empty());

// Zkusit změny bokem (agent dostane capability jen ke klonu).
fs.aside_begin("/projekt", "/projekt-zkouska")?;
/* ... agent pracuje v /projekt-zkouska ... */
let zmeny = fs.diff_clone("/projekt-zkouska")?;
fs.aside_apply("/projekt-zkouska", Some(&["/src".into()]))?;   // jen vybrané
fs.aside_discard("/projekt-zkouska")?;
```

Přes službu fsd totéž dělá `FsClient` (`fsproto::hist`), viz
[Protokol](#protokol-operace-54-92).

## Žurnál změn

<figure class="diagram">
<svg viewBox="0 0 760 190" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Zurnal zmen">
<defs><marker id="hj1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="warn" x="20" y="40" width="80" height="40" rx="6"/><text class="t" x="60" y="65" text-anchor="middle">zahozeno</text>
<rect class="box" x="110" y="40" width="70" height="40" rx="6"/><text class="t mono" x="145" y="65" text-anchor="middle">first</text>
<rect class="box" x="190" y="40" width="70" height="40" rx="6"/><text class="t mono" x="225" y="65" text-anchor="middle">...</text>
<rect class="ok" x="270" y="40" width="90" height="40" rx="6"/><text class="t mono" x="315" y="65" text-anchor="middle">VIEW.seq</text>
<rect class="box" x="370" y="40" width="70" height="40" rx="6"/><text class="t mono" x="405" y="65" text-anchor="middle">...</text>
<rect class="acc" x="450" y="40" width="100" height="40" rx="6"/><text class="t mono" x="500" y="65" text-anchor="middle">published</text>
<rect class="box" x="560" y="40" width="170" height="40" rx="6"/><text class="t" x="645" y="58" text-anchor="middle">nepotvrzené</text><text class="m" x="645" y="74" text-anchor="middle">čtenář je nevidí</text>
<path class="ln" d="M315 85 L315 120" marker-end="url(#hj1)"/>
<text class="m" x="315" y="138" text-anchor="middle">pohled obsahuje vše do VIEW.seq</text>
<path class="ln" d="M500 85 L500 120" marker-end="url(#hj1)"/>
<text class="m" x="500" y="138" text-anchor="middle">commit nebo fsync posune hranici</text>
<text class="m" x="60" y="105" text-anchor="middle">čtení = JOURNAL_GAP</text>
<text class="m" x="20" y="175">Pořadová čísla od 1, nikdy se neopakují pro nikoho, kdo je viděl.</text>
</svg>
<figcaption>Žurnál datasetu: retence zahazuje nejstarší, čtenář vidí jen potvrzené záznamy.</figcaption>
</figure>

Každý dataset má vlastní žurnál - uspořádaný proud záznamů:

| Pole | Význam |
|---|---|
| pořadové číslo | u64 od 1, offset položky `JOURNAL` |
| operace | `CREATE`, `REMOVE`, `RENAME`, `CONTENT`, `ATTR`, `TRASH`, `RESTORE`, `SNAPSHOT` |
| druh, oid, generace | identita objektu (oid je v datasetu stabilní) |
| rodič, jméno / nový rodič, nové jméno | kde objekt byl a kam šel |
| velikost, doplněk | nová velikost; u `ATTR` co se změnilo (časy, oprávnění, vlastník, atributy), u koše číslo záznamu, u snapshotu jeho číslo |
| čas, kdo, aplikace | čas operace, uživatel; aplikace zatím 0 (vyhrazeno) |

Pravidla:

- **Záznam vzniká atomicky se změnou** - je to položka stromu datasetu,
  uloží ji tentýž commit. Snapshot zachytí i žurnál až do svého vzniku
  (`SnapshotInfo::journal_seq`).
- **Zveřejní se až po potvrzení:** čtenář dostane jen záznamy do
  `published` (poslední commit nebo `fsync`). Změna potvrzená `fsync` se po
  pádu přehraje z intent logu a její záznamy vzniknou znovu **se stejnými
  čísly** - stejné pořadí, čas i uživatel (záznam logu `Actor`).
  Nepotvrzené změny po pádu zmizí i se svými čísly, ty ale nikdo neviděl.
- **Zápisy obsahu se slučují:** jeden `CONTENT` na objekt za commit nebo
  `fsync` (číslo dostane na konci dávky). Databáze nebo log se tisíci zápisy
  za sekundu tak žurnál nezahltí; čtenář stejně čte aktuální obsah.
- **Retence** (nastavení datasetu): výchozí 1 milion záznamů nebo 30 dní,
  co nastane dřív. Při commitu se zahodí nejvýš 4096 nejstarších (commit
  nesmí trvat neomezeně); v datasetu, který se přestal měnit, uplatní
  stáří krok služby `journal_tick` (každou minutu). Číslo starší než nejstarší uložené =
  **`JOURNAL_GAP`**, nikdy tiché vynechání.
- **Nový výchozí stav i s pozicí:** `view(dataset)` udělá dočasný snapshot
  (druh `VIEW`, ve výpisu snapshotů není) a vrátí číslo posledního
  záznamu, který obsahuje. Klient projde pohled a naváže odběrem od
  `seq + 1` - nic nepřeskočí, ani přejmenování adresáře během výpisu.
  Pohled uvolní `release_view`; pohledy z doby před pádem se smažou při
  připojení.
- **Oprávnění:** čtení žurnálu a pohled vyžadují `MANAGE` ke kořeni
  datasetu (žurnál ukazuje jména všech objektů).
- **Nevede se:** zápis do sirotka (smazaného otevřeného souboru) - do
  jmenného prostoru už nepatří.

### Sledování změn (`JOURNAL_WATCH`)

Klient pošle číslo, od kterého chce záznamy, a časový limit (nejvýš 60 s).
Když záznamy jsou, odpověď přijde hned; jinak ji služba odloží a odpoví po
dalším potvrzení (automatický commit každých 5 s nebo `fsync` klienta),
nebo po limitu prázdnou stránkou. Ostatní relace čekání neblokuje.

⚠️ Relace s čekajícím `WATCH` nezpracuje další požadavek, dokud nedostane
odpověď (protokol je synchronní). Pošle-li klient přesto další požadavek,
služba nejdřív odpoví na čekání (i prázdně) - pořadí odpovědí zůstane.
Pro sledování je proto vhodná vlastní relace (`CONNECT`).

## Koš

<figure class="diagram">
<svg viewBox="0 0 760 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Kos">
<defs><marker id="hk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="20" y="30" width="200" height="50" rx="6"/><text class="t" x="120" y="52" text-anchor="middle">/dokumenty/zprava.docx</text><text class="m" x="120" y="70" text-anchor="middle">oid 812</text>
<path class="ln" d="M220 55 L330 55" marker-end="url(#hk1)"/><text class="m" x="275" y="45" text-anchor="middle">trash</text>
<rect class="warn" x="332" y="20" width="200" height="70" rx="6"/><text class="t" x="432" y="42" text-anchor="middle">TRASH_OID (oid 2)</text><text class="m" x="432" y="60" text-anchor="middle">položka "17" -> oid 812</text><text class="m" x="432" y="76" text-anchor="middle">rodič 0, mimo kořen</text>
<rect class="acc" x="560" y="20" width="180" height="70" rx="6"/><text class="t" x="650" y="42" text-anchor="middle">záznam TRASH 17</text><text class="m" x="650" y="60" text-anchor="middle">původní rodič, jméno</text><text class="m" x="650" y="76" text-anchor="middle">čas, kdo, velikost</text>
<text class="m" x="20" y="125">Nic se nekopíruje: objekt se jen přesune. Otevřené handly fungují dál.</text>
<text class="m" x="20" y="147">Obnova = přesun zpět; vysypání = smazání podstromu (jeden záznam REMOVE).</text>
</svg>
<figcaption>Koš je skrytý adresář datasetu s pevným oid; záznam pamatuje, odkud objekt přišel.</figcaption>
</figure>

- **`trash(path)`** přesune soubor nebo celou složku do koše jejího datasetu
  (právo `DELETE` jako `remove`). Běžné `remove` zůstává trvalé - kompilátory
  a dočasné soubory koš nezaplní. Dataset do koše nejde (`WRONG_TYPE`).
- **`trash_list(dataset)`** - od nejnovějšího; každý vidí jen své
  záznamy a záznamy objektů, které vlastní, správce (`MANAGE`) vše.
- **`trash_restore(dataset, id, dest)`** - na původní místo (`dest = None`)
  nebo jinam ve stejném datasetu. Jméno obsazené = `ALREADY_EXISTS`,
  původní složka zmizela = `NOT_FOUND`.
- **`trash_purge(dataset, id)`** - trvalé smazání podstromu; před první
  změnou ověří, že v něm není dataset (operace vše, nebo nic).
- **`trash_tick(now)`** (služba, každou minutu) vysype záznamy starší než
  `trash_max_age` (výchozí 30 dní) a při volném místě pod 10 % i nejstarší
  další, nejvýš 64 za krok.
- Koš má vlastní souhrn podstromu - do velikosti kořene se nepočítá.
- Všechny operace koše jdou přes intent log (`Trash`, `Restore`, `Purge`),
  takže `fsync` je zachová i po pádu.

## Rozšířené atributy a expirace

Atributy jsou obecná data uložená atomicky s objektem (položka `XATTR`,
offset = hash jména; víc atributů se stejným hashem sdílí hodnotu). FS je
jen ukládá a nevykládá - nastavení služeb a aplikací patří sem, ne do bitů
formátu.

| Jmenný prostor | Kdo mění | Příklad |
|---|---|---|
| `user.*` | aplikace a uživatel (`READ` čte, `WRITE` mění) | `user.barva`, `user.stitek` |
| `sys.*` | jen systém nebo vlastní API FS | `sys.expires` |

Limity: jméno do 255 B, hodnota do 4 KiB, počet atributů je v záznamu
objektu (fsck ho kontroluje).

**Expirace** (`set_expires(path, Some(čas))`, právo `WRITE`): po datu
přesune `expire_tick` (služba, každou minutu) objekt do koše a atribut
smaže. Aby krok nemusel procházet strom, má dataset **index podle času**
(položka `EXPIRY`: klíč = sekundy, hodnota = seznam (oid, generace));
atribut a index se mění vždy spolu a fsck kontroluje shodu. Objekt
obnovený z koše už expiraci nemá.

## Verze souboru a vrácení operací

- **`file_versions(path)`** - verze souboru ve snapshotech jeho datasetu,
  od nejstarší. Objekt se ve snapshotu hledá **podle oid**, takže
  přejmenování nebo přesun mezitím nevadí. Každá verze (pořadové číslo
  změn obsahu `Revision::version`) jednou, s nejstarším snapshotem, který ji
  obsahuje.
- **`restore_version(path, snapshot, mode)`** - `Copy(cíl)` = kopie vedle
  originálu; `Replace` = obnovená kopie dostane jméno originálu a aktuální
  verze jde **do koše** (nic se neztratí; obnovený soubor má nové oid).
- **`undo(dataset, seq)`** vrátí operaci zápisu žurnálu:

| Operace | Vrácení |
|---|---|
| `RENAME` | přesun zpět (objekt musí být tam, kam šel; jinak `CONFLICT`) |
| `TRASH` | obnova z koše na původní místo |
| `CREATE`, `RESTORE` | objekt do koše |
| `REMOVE` | kopie z posledního snapshotu před operací na původní místo |
| `CONTENT` | obsah z posledního snapshotu před operací; přepsaná verze jde do koše |
| `ATTR`, `SNAPSHOT` | `NOT_SUPPORTED` |

  „Snapshot před operací“ = nejnovější snapshot, jehož `journal_seq` je
  menší než číslo operace a který objekt obsahuje; bez něj `NOT_FOUND`.
  Proto má domov uživatele mít automatické snapshoty.

## Automatické snapshoty

Pravidla jsou nastavení datasetu (`Settings::rules`, nejvýš 8): „drž
nejnovější snapshot v každém z posledních `keep` intervalů délky
`interval`“ (interval nejméně 60 s). Například:

```rust
fs.set_settings("/home/alice", &Settings {
    rules: vec![
        AutoRule { interval: 300, keep: 12 },        // 5 minut, 1 hodina
        AutoRule { interval: 3600, keep: 24 },       // hodinové, 1 den
        AutoRule { interval: 86_400, keep: 30 },     // denní, 30 dní
        AutoRule { interval: 7 * 86_400, keep: 26 }, // týdenní, půl roku
    ],
    ..Settings::default()
})?;
```

`auto_snapshot_tick(now)` (služba, každou minutu) je deterministický krok:

1. nový snapshot `auto-RRRR-MM-DD-HHMMSS` (druh `AUTO`), když uplynul
   nejkratší interval **a dataset se změnil** (žurnál má od posledního
   automatického snapshotu jiný záznam než `SNAPSHOT`, nebo nepotvrzené
   změny),
2. prořezání: smaže automatické snapshoty, které nedrží žádné pravidlo
   (nejnovější zůstane vždy). Ruční snapshoty a commity nemaže; snapshot
   s klonem se jen skryje.

Výchozí: **bez pravidel** (nastaví je instalace domova nebo nastavení
systému). Pravidla pro „bod před útokem“ (5 minut / 1 hodina) navrhuje
[ochrana proti ransomwaru](navrh-fs-v3.md#ochrana-proti-ransomwaru-detekce-chování).

## Commity a logické ID

<figure class="diagram">
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Commity a vetve">
<defs><marker id="hc1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<circle class="acc" cx="60" cy="60" r="18"/><text class="t" x="60" y="65" text-anchor="middle">C1</text>
<circle class="acc" cx="200" cy="60" r="18"/><text class="t" x="200" y="65" text-anchor="middle">C2</text>
<circle class="box" cx="200" cy="140" r="18"/><text class="t" x="200" y="145" text-anchor="middle">B1</text>
<circle class="ok" cx="360" cy="60" r="18"/><text class="t" x="360" y="65" text-anchor="middle">M</text>
<path class="ln" d="M182 60 L80 60" marker-end="url(#hc1)"/>
<path class="ln" d="M184 130 L76 70" marker-end="url(#hc1)"/>
<path class="ln" d="M342 60 L220 60" marker-end="url(#hc1)"/>
<path class="ln" d="M344 70 L216 132" marker-end="url(#hc1)"/>
<rect class="box" x="430" y="30" width="310" height="44" rx="6"/><text class="t" x="585" y="50" text-anchor="middle">/projekt: HEAD -> M</text><text class="m" x="585" y="66" text-anchor="middle">commity = snapshoty druhu COMMIT</text>
<rect class="warn" x="430" y="118" width="310" height="44" rx="6"/><text class="t" x="585" y="138" text-anchor="middle">/projekt-refaktor = klon C1 (větev)</text><text class="m" x="585" y="154" text-anchor="middle">vlastní dataset, commity B1...</text>
<text class="m" x="20" y="190">Šipky vedou k rodičům; M má dva (merge). Základ merge = společný předek C1.</text>
</svg>
<figcaption>Historie je DAG commitů; větev je zapisovatelný klon, merge je trojcestný.</figcaption>
</figure>

**Commit** (`commit_version(dataset, zpráva)`, právo `MANAGE`) = snapshot
druhu `COMMIT` + autor, zpráva (do 4 KiB), čas, rodiče (poslední commit
datasetu a případně druhý rodič rozpracovaného merge) a **logické ID**.
Jméno snapshotu je `c-` + prvních 12 hex znaků ID. `head(dataset)`
a `log(dataset, max)` čtou historii (log = všichni předci od nejnovějšího;
commit, jehož snapshot už neexistuje, větev historie ukončí).

**Logické ID** = BLAKE3 z jednoznačné reprezentace logického stromu:

```text
soubor  = H("f" | velikost | H(obsah) | metadata)
odkaz   = H("l" | cíl | metadata)
složka  = H("d" | počet | (jméno, druh, hash potomka)* seřazeno podle bajtů jména | metadata)
metadata = mtime, btime, vlastník, oprávnění (kanonicky, bez čísla popisu), rozšířené atributy
commit  = H("osfs-commit-v1" | strom | rodiče | autor | čas | zpráva)
```

Nezávisí na rozmístění bloků, oid, generacích, ctime ani atime - stejný
strom na jiném svazku má stejné ID (ověřuje test
`logical_id_is_layout_independent`). Vnořený dataset se započítá jen jménem
(má vlastní historii).

**Cena:** běžné snapshoty a commity každých 5 s ID nepočítají, jen
verzovaný commit. Hash obsahu souboru se pamatuje (položka `LHASH`: verze
obsahu, generace, hash) a znovu se počítá jen po změně; složky se projdou
celé (cena podle počtu objektů, ne dat). Průchod je bez rekurze (služba má
malý zásobník).

## Větve a merge

- **`branch(dataset, jméno, cesta)`** - zapisovatelný klon posledního
  commitu na `cesta` (vlastní dataset, okamžitý); zdrojový dataset si větev
  pamatuje (položka `BRANCH`). Dataset musí mít commit (`BAD_STATE`).
  `branches`, `delete_branch` (smaže i klon).
- **`merge(cíl, zdroj)`** - sloučí poslední commit zdroje do živé verze
  cíle, trojcestně proti **společnému předkovi** v DAG commitů:

| Situace (předek B, naše O, jejich T) | Výsledek |
|---|---|
| změna jen v T | převezme se |
| změna jen v O | zůstane |
| stejná změna v O i T | jednou |
| textový soubor změněný v O i T | sloučení po řádcích (diff3); při konfliktu značky `<<<<<<< nase` / `=======` / `>>>>>>> větev` |
| binární soubor změněný v O i T | naše zůstane, jejich vedle jako `jméno (konflikt větev).přípona` |
| smazáno v T, beze změny v O | do koše |
| smazáno v T, změněno v O | konflikt, naše zůstane |
| smazáno v O, změněno v T | konflikt, jejich verze se obnoví |
| přesun v T | přesun (i do nových složek); oba jinam = konflikt, naše místo |
| nové jméno na obou stranách | stejný obsah = nic, jinak druhé s „(konflikt …)“; nová složka na obou stranách se sloučí |
| složka smazaná v T, v O v ní něco zbylo | konflikt, zůstane |

  Objekty se párují podle **oid a generace**, ale jen ty, které existovaly
  v B (klon pokračuje se stejnými oid). Nové objekty na obou stranách mohou
  mít stejné oid, a přesto jsou to různé objekty - párují se podle cesty.
  Text = platné UTF-8 bez NUL do 4 MiB.

  Bez konfliktu se hned udělá **commit sloučení** se dvěma rodiči. S
  konflikty zůstanou změny v živé verzi a `HEAD` si pamatuje druhého
  rodiče; příští `commit_version` (po vyřešení) ho dostane. `merge_abort`
  rozpracovaný merge zruší (změny zůstanou).

**Postup:** nejdřív se celý plán spočítá **bez jediné změny** (indexy tří
stromů, simulace jmen v cílových složkách, kontrola cyklů u přesunů,
sloučení textů, odhad místa), pak se provede: koš, nové objekty, přesuny
přes pomocnou složku (výměny jmen nevadí), obsah; nakonec commit. Chyba
před první změnou nic neohrozí; chyba uprostřed (např. vadný blok) přepne
svazek do stavu chyby jako každá operace „vše, nebo nic“ - po novém
připojení platí stav před merge.

## Zkusit změny bokem

```rust
fs.aside_begin("/projekt", "/projekt-zkouska")?;   // snapshot ASIDE + klon
// aplikace (agent, instalátor) dostane capability jen ke klonu
let zmeny = fs.diff_clone("/projekt-zkouska")?;     // co přibylo, zmizelo, změnilo se
let r = fs.aside_apply("/projekt-zkouska", Some(&["/src".into()]))?;
for c in &r.conflicts { println!("nepřevzato: {}", c.path); }
fs.aside_discard("/projekt-zkouska")?;              // klon i snapshot jedním krokem
```

- `aside_apply(klon, výběr)` převezme vše (`None`) nebo jen vybrané cesty
  (uvnitř datasetu, včetně podstromu). Je to stejný trojcestný merge
  (předek = snapshot `ASIDE`, naše = originál, jejich = živý klon), ale
  **konflikt se nepřevezme vůbec** - jen se ohlásí; průzkumník pak nabídne
  sloučení se značkami nebo uložení vedle. Převzetí je atomické (commit).
- Převzetí jde opakovat (např. nejdřív `/src`, pak zbytek).
- Smazané soubory jdou do koše originálu.
- **Vnořené datasety jsou v klonu hranice.** Klon jde udělat i z datasetu
  s vnořenými datasety (i z kořene `/`) a smí ležet uvnitř originálu
  (`/tmp/.aside-1`). Položka vnořeného datasetu je v klonu vidět (výpis
  i `stat` ji ukážou jako `Dataset`, `stat` s právy 0), ale vstup do ní,
  zápis, přejmenování i smazání vrátí `NOT_SUPPORTED` - druhá
  zapisovatelná cesta k témuž datasetu nikdy nevznikne. Platná je jen
  položka, na kterou ukazuje zpětný odkaz datasetu (`DSLINK`); kopie
  v klonu (i položka datasetu, který mezitím zanikl) je hranice.
  `diff_clone` hranice nehlásí a `aside_apply` vnořené datasety originálu
  nemění: jejich jména jsou obsazená (nový soubor stejného jména =
  konflikt) a složka s datasetem se nesmaže (konflikt `DirNotEmpty`).
  Dva současné klony téhož datasetu (dvě relace shellu) tak fungují
  nezávisle - druhý vidí první jen jako hranici. Stejně se chovají
  `clone_snapshot` a `branch`; klon navíc nedědí starší snapshoty
  originálu.

### Výkon řádku shellu (undo bez `VIEW`)

> **Změna proti dřívějšku (výkon FS):** řádek shellu se zápisem stál
> v QEMU 2-4 s, po plné sadě testů víc. **Proč:** každý commit poslal
> ovladači disku 5-15 zpráv a na odpověď každé čekal (cesta tam a zpět
> fsd ↔ `virtio-blk` asi 3-5 ms, samotné zařízení 0,4 ms); řádek dělal
> 7-8 commitů. **Co se změnilo:** (1) zápisy jdou ovladači bez čekání na
> odpověď (`BlockClient::write_queued`, odpovědi se vyberou před dalším
> čtením nebo `flush`), (2) dočasné snapshoty (`VIEW`, začátek „bokem“)
> stojí jeden commit místo dvou, `ASIDE_BEGIN` celkem dva místo tří,
> (3) undo shellu může místo `VIEW` použít snapshot `ASIDE_BEGIN`
> (`KEEP_VIEW`), (4) commit prochází jen datasety se změnami. Formát na
> disku beze změny.

| Krok (QEMU, x86_64) | Dřív | Teď |
|---|---|---|
| commit s malou změnou | 88 ms | 23-50 ms |
| `ASIDE_BEGIN` | 264 ms | 60-90 ms |
| `ASIDE_APPLY` | 248 ms | 70-130 ms |
| `VIEW` | 2 commity | ~5 ms (jen commit stavu, často prázdný) |
| celý řádek (medián) | 666 ms (čerstvý disk), 2-4 s po testech | 160-280 ms (po sadě testů s alokátorem po velikostních třídách: x86_64 174 ms, aarch64 313 ms) |

```
ASIDE_BEGIN ──> snapshot aside-N (stav před řádkem) ──> klon /tmp/.aside/...
zápisy v klonu, ASIDE_CHANGES (z paměti, bez commitu)
ASIDE_APPLY + KEEP_VIEW ──> převzetí; aside-N se změní na pohled (VIEW)
undo čte z aside-N ── VIEW_RELEASE ho uvolní (ASIDE_DISCARD ho nesmaže)
```

- Pohled ze snapshotu `ASIDE_BEGIN` je pro undo rovnocenný pohledu těsně
  před převzetím: cesty, které se převzaly bez konfliktu, měl originál
  v obou okamžicích stejné (jinak by šly do konfliktu).
- `VIEW_RELEASE` a `ASIDE_DISCARD` jdou v libovolném pořadí; uvolněný
  pohled s živým klonem se skryje a zanikne s klonem.
- Po pádu se pohledy při připojení mažou jako dřív (undo po restartu
  `fsd` nejde).
- Záznam dočasného snapshotu uloží až další commit. Pád mezi tím = snapshot
  nevznikl (stav na disku je konzistentní, dočasné snapshoty se po pádu
  stejně mažou). `ASIDE_BEGIN` uloží záznam snapshotu i klon jedním
  commitem: po pádu jsou oba, nebo nic. Obsazený cíl klonu se pozná
  předem (`ALREADY_EXISTS`, dřív se po něm svazek zablokoval).
- **Riziko:** chyba zápisu na disk se ohlásí až u další operace s diskem
  (čtení, `flush`), ne u zápisu samotného. Commit vždy volá `flush` před
  zápisem superbloku, takže neúspěšný zápis commit zastaví jako dřív;
  čtení po chybném zápisu ale vrátí chybu toho zápisu.
- Hlídají to testy: `libs/fs/src/tests/perf_shell.rs` (počet zápisů
  a `flush` na řádek nezávisí na počtu načtených datasetů, 2 proti 300)
  a QEMU sada `vykon FS` (`user/init/src/tests/fs_perf.rs`, po všech
  testech FS: commit pod 80 ms, řádek pod 500 ms).

## Na disku

| Druh | Klíč | Hodnota |
|---|---|---|
| 13 `JOURNAL` | (META, 13, číslo) | záznam žurnálu: operace, druh, čas, kdo, aplikace, oid, generace, rodiče, velikost, doplněk, jména |
| 14 `JSTATE` | (META, 14, 0) | další číslo, nejstarší uložené |
| 15 `SNAPMETA` | (META, 15, snapshot) | druh (ruční, automatický, commit, pohled, bokem), čas, pozice žurnálu, logické ID, autor, rodiče (ID + dataset + snapshot), zpráva |
| 16 `SETTINGS` | (META, 16, 0) | retence žurnálu (počet, stáří), stáří koše, pravidla snapshotů |
| 17 `TRASH` | (META, 17, 0) | počítadlo dalšího čísla koše |
| 17 `TRASH` | (META, 17, číslo) | oid, generace, původní rodič, čas, druh, kdo, velikost, jméno |
| 18 `XATTR` | (oid, 18, hash jména) | seznam (jméno, hodnota) |
| 19 `EXPIRY` | (META, 19, sekundy ^ 2^63) | seznam (oid, generace) |
| 20 `LHASH` | (oid, 20, 0) | verze obsahu, generace, BLAKE3 obsahu |
| 21 `BRANCH` | (META, 21, hash jména) | seznam (dataset, jméno) |
| 22 `HEAD` | (META, 22, 0) | poslední commit (ID, dataset, snapshot), pro který dataset platí, rozpracovaný merge |
| 23 `DSLINK` | (META, 23, 0) ve stromu datasetu samotného | kde je dataset připojený: dataset rodiče, složka, jméno (cesta datasetu bez průchodu stromem; mění se při vytvoření, klonu a přesunu) |

Pevný objekt **`TRASH_OID` = 2**: adresář koše (rodič 0). Klon zdědí
`HEAD` původu ve svém stromu - záznam má pole „pro dataset“, a když nesedí,
je posledním commitem klonu jeho původní snapshot.

**Příznak `RO_COMPAT_HISTORY`** (ro_compat bit 1): nové svazky ho mají od
formátování, starší svazek ho dostane při prvním zapisovatelném připojení.
Starší ovladač smí svazek jen číst - při zápisu by nevedl žurnál ani
indexy.

**Intent log:** nové záznamy 14 `Actor` (kdo provedl následující operace -
žurnál po přehrání vyjde stejně), 15 `Trash`, 16 `Restore`, 17 `Purge`,
18 `SetXattr`. Operace commitů, větví, merge a nastavení hned commitují.

**fsck** navíc kontroluje: souvislou řadu žurnálu `first..next` a jeho
stav, metadata jen u existujících snapshotů, záznam koše ↔ položka
v adresáři koše a počítadlo, počet atributů v záznamu objektu, shodu
indexu expirace s atributy (oid + generace), paměť hashe jen u souborů,
čitelnost nastavení, větví a `HEAD`.

## Protokol (operace 54-92)

Modul `fsproto::hist`; cesty jsou cesty relace, práva kontroluje FS podle
identity relace. Čas na drátě = sekundy i64 + nanosekundy u32.

| op | Název | Parametry | Výsledek |
|---|---|---|---|
| 54 | `JOURNAL_READ` | dataset, od u64, nejvýš u32 | nejstarší u64, poslední u64, počet u32 + záznamy |
| 55 | `JOURNAL_WATCH` | dataset, od, nejvýš, limit ms u32 | jako 54; čeká na záznam nebo limit |
| 56 | `VIEW` | dataset | snapshot, id u64, poslední záznam u64 |
| 57 | `VIEW_RELEASE` | dataset, snapshot | - |
| 58 | `TRASH` | cesta | číslo záznamu u64 |
| 59 | `TRASH_LIST` | dataset | počet + (id, druh, oid, čas, kdo, velikost, jméno, původní složka) |
| 60 | `TRASH_RESTORE` | dataset, id, cíl (prázdný = původní) | cesta |
| 61 | `TRASH_PURGE` | dataset, id | - |
| 62 | `UNDO` | dataset, číslo záznamu | druh u8 (1 přesun zpět, 2 z koše, 3 do koše, 4 ze snapshotu), cesta, snapshot, číslo v koši |
| 63 | `FILE_VERSIONS` | cesta | počet + (snapshot, id, čas, velikost, mtime, verze) |
| 64 | `RESTORE_VERSION` | cesta, snapshot, nahradit u8, cíl | cesta |
| 65 / 66 | `SETTINGS` / `SET_SETTINGS` | dataset [, nastavení] | nastavení: 3x u64, počet + (interval u64, kolik u32) |
| 67 | `COMMIT_VERSION` | dataset, zpráva | commit |
| 68 | `LOG` | dataset, nejvýš | počet + commity |
| 69 | `BRANCH` | dataset, jméno, cesta | dataset větve u64 |
| 70 | `BRANCHES` | dataset | počet + (jméno, dataset, cesta) |
| 71 | `DELETE_BRANCH` | dataset, jméno | - |
| 72 | `MERGE` | cíl, zdroj | zpráva o sloučení |
| 73 | `MERGE_ABORT` | cíl | - |
| 74 | `ASIDE_BEGIN` | dataset, cesta klonu | dataset klonu u64, snapshot původu str, žurnál seq u64 (jako `VIEW`) |
| 75 | `ASIDE_APPLY` | klon, vše u8, počet + cesty, [příznaky u8: 1 = `KEEP_VIEW`] | zpráva o sloučení; za konflikty celkový počet konfliktů u32 (seznam může být zkrácený na 1024). S `KEEP_VIEW` zůstane snapshot původu jako pohled (uvolní `VIEW_RELEASE`) |
| 76 | `ASIDE_DISCARD` | klon | - (snapshot původu smaže, jen když není pohledem) |
| 77 | `ASIDE_CHANGES` | klon, [přeskočit u32] | stránka: celkem u32, počet u32 + (druh u8, cesta, stará cesta, oid, složka u8) |
| 78-81 | `XATTR_GET` / `SET` / `REMOVE` / `LIST` | cesta, jméno [, hodnota = zbytek zprávy] | hodnota / - / - / jména |
| 82 / 83 | `SET_EXPIRES` / `EXPIRES` | cesta [, má u8, čas] | má u8, čas |
| 84 | `HEAD` | dataset | má u8 + commit |
| 85 | `DATASET_CREATE` | cesta | číslo datasetu u64 (nová složka s vlastní historií; právo `CREATE` v rodiči) |
| 86 | `DATASET_DELETE` | cesta | - (i se snapshoty; `MANAGE` + `DELETE`; ne s vnořenými datasety a klony) |
| 87 | `LIST_STAT_PAGE` | cesta, kurzor, nejvýš u32 | výpis s metadaty, popis v [protokolu fsproto](souborovy-system.md#protokol-fsproto) (modul `fsproto::op`) |
| 88 | `DIFF` | dataset, od snapshot, do u8 (0 = živá verze, 1 = následuje jméno snapshotu), [přeskočit u32] | stránka jako `ASIDE_CHANGES`; `MANAGE` ke kořeni datasetu |
| 89 | `DATASET_OF` | cesta | cesta kořene datasetu, ve kterém cesta leží (`/` = kořenový); práva jako `STAT` |
| 90 | `SNAP_STAT_OF` | dataset, snapshot, cesta uvnitř datasetu | `Stat` |
| 91 | `SNAP_LIST_OF` | dataset, snapshot, cesta uvnitř datasetu, kurzor, nejvýš u32 | jako `LIST_PAGE` |
| 92 | `SNAP_READ_OF` | dataset, snapshot, cesta uvnitř datasetu, offset u64, délka u32 | data (kratší na konci souboru) |
| 93 | `DATASETS` | - | počet u32 + (cesta kořene, číslo datasetu u64, `Stat` kořene, původ u8: 0 není klon, 1 klon + cesta původu, 2 původ skrytý, 3 původ neexistuje); modul `fsproto::datasets` |

Commit na drátě: ID (32 B), snapshot, dataset u64, id snapshotu u64, čas,
autor u64, zpráva, počet u32 + ID rodičů. Zpráva o sloučení: převzaté
cesty, konflikty (cesta, druh u8: 1 obsah, 2 naše změněno / jejich
smazáno, 3 naše smazáno / jejich změněno, 4 přidáno na obou stranách,
5 přesunuto na obou stranách, 6 chybí složka, 7 složka není prázdná,
8 cyklus; kam se uložila jejich verze) a volitelně commit sloučení.
Seznamy mají nejvýš 4096 položek.

**Rozdíl verzí a čtení snapshotu datasetu (88-92)** - pro shell (`diff`,
náhled změn, čtení z pohledu):

- **Cesty v `Change`** (`DIFF` i `ASIDE_CHANGES`) jsou **uvnitř datasetu**
  (od jeho kořene, např. `/a.txt` v datasetu `/projekt`), ne cesty relace.
  Absolutní cestu dostane klient spojením s cestou datasetu. Seřazené podle
  cesty; druh 1 přidáno, 2 smazáno, 3 změněno, 4 přesunuto, 5 přesunuto
  a změněno.
- **`DIFF`** porovná snapshot `od` se snapshotem `do`, nebo se živou verzí
  (i neuložené změny). Snapshot může být i dočasný snapshot pohledu
  (`VIEW`).
- **Stránky změn** (`DIFF`, `ASIDE_CHANGES`): odpověď nese celkový počet
  změn a jednu stránku (nejvýš 4096 položek a 60 KiB). Klient pošle stejný
  požadavek s `přeskočit` = počet už přijatých, dokud nemá všechny
  (`FsClient::diff`, `aside_changes` to dělají samy). Seznam se nikdy
  tiše nezkrátí. Každá stránka rozdíl spočítá znovu; cena je úměrná
  změnám, ne velikosti stromu, takže to stojí málo.
- **Konflikty v `ASIDE_APPLY`/`MERGE`**: seznam se do odpovědi vejde
  zkrácený (nejvýš 1024), ale celkový počet jde vždy. Klient tak pozná,
  že nezná všechny konflikty (shell pak převzetí nedovolí vrátit).
- **`DATASET_OF`**: služba projde předky cesty od kořene (`STAT` každého)
  a vrátí prvního se stejným datasetem. Cesta, kterou relace nevidí,
  i kořen datasetu mimo připojené cesty relace = `NOT_FOUND` (jako `STAT`).
- **`SNAP_*_OF`** čtou snapshot libovolného datasetu (i pohled z `VIEW`);
  `SNAP_STAT`, `SNAP_LIST_PAGE` a `SNAP_READ` umí jen kořenový dataset
  a jen relace, která vidí celý FS. Dataset je cesta relace (stačí
  připojení jen pro čtení), cesta ve snapshotu je uvnitř datasetu. Práva
  kontroluje FS jako u čtení (průchod k datasetu, ACL ve snapshotu).

- **`DATASETS`** (seznam datasetů pro postranní panel správce souborů
  a shell): jen datasety, k jejichž kořeni relace projde a které leží
  v jejích připojených cestách; cesty jsou cesty relace, seřazené.
  Kořenový dataset `/` je v seznamu, když relace vidí celý kořen svazku.
  Hodnoty drží tři stavy jako `Stat`: souhrn kořene (`tree_*`) je platný
  jen s bitem `avail::TREE` v `known`, bez práva `MANAGE` je v `hidden`.
  Původ klonu je známý (cesta), **skrytý** (relace původní dataset nevidí
  nebo leží mimo připojené cesty - existuje, ale klient ho nesmí znát),
  nebo **neexistuje** (původní dataset už není; dnes FS smazání původu
  s živým klonem odmítne `BAD_STATE`, stav je pojistka). Cena: fsd hledá
  cesty datasetů průchodem adresářů živé verze (úměrné počtu adresářů
  svazku), proto se volá při startu a po změně, ne ve smyčce. Nejvýš 1024
  položek. Test: `historie FS` - „seznam datasetu (DATASETS)“.

Klient: `diff(dataset, od, do: Option)`, `dataset_of(cesta)`, `datasets()`,
`snapshot_stat_of`, `snapshot_list_of`, `snapshot_read_of` a
`snapshot_read_file_of` (celý soubor).

```rust
let v = fs.view("/projekt")?;
// ... změny ...
for c in fs.diff("/projekt", &v.snapshot, None)? {
    // c.path je uvnitř datasetu: "/projekt" + c.path
}
let stary = fs.snapshot_read_file_of("/projekt", &v.snapshot, "/a.txt")?;
fs.release_view("/projekt", &v.snapshot)?;
```

**Služba fsd** každou minutu (jako systém) volá `auto_snapshot_tick`,
`expire_tick`, `trash_tick` a `journal_tick` - na svazku jen pro čtení ne.

**Capability u „bokem“:** `ASIDE_APPLY` a `ASIDE_DISCARD` mění originál,
proto je služba dovolí jen relaci, která má originál připojený se zápisem.
Aplikace s capability jen ke klonu originál nezmění, ani když má identitu
jeho vlastníka. `ASIDE_BEGIN` vyžaduje zápis do datasetu (vytvoří v něm
snapshot).

## Rizika a omezení

- ⚠️ **Vrácení obsahu a smazání potřebuje snapshot před operací.** Bez
  automatických snapshotů vrátí `undo` u `REMOVE`/`CONTENT` jen
  `NOT_FOUND` - žurnál pamatuje, *co* se stalo, ne starý obsah.
- ⚠️ **Retence žurnálu je kompromis:** dlouho nepřipojený odběratel dostane
  `JOURNAL_GAP` a musí projít dataset znovu (`VIEW`). Velký limit stojí
  místo (~100-200 B na záznam).
- ⚠️ **Logický hash prochází všechny složky** při každém verzovaném
  commitu (obsah souborů jen po změně). U datasetu s miliony objektů trvá
  commit úměrně počtu objektů. Přírůstkový hash ve složkách je možná
  pozdější optimalizace.
- **Rozdíl a sloučení stojí úměrně změně, ne velikosti datasetu.**
  `diff`, `diff_clone`, `aside_apply` i `merge` porovnají stromy současně
  a sdílené podstromy (stejný ukazatel uzlu - klon a jeho výchozí snapshot
  sdílejí všechno nezměněné) přeskočí; změněné objekty pak dohledají
  bodovými dotazy (záznam, jméno v rodičovské složce, cesta přes předky).
  Navíc se čtou výpisy složek, ve kterých změny leží (velká složka se
  změněným souborem = průchod jejích položek). Příklad z testu: 5000
  souborů, změna 2 - `diff_clone` 15 čtení uzlů z disku místo 154,
  `aside_apply` 15 místo 154. Předpoklad: `Inode::parent` odpovídá položce
  adresáře (hlídá `check`).
- ⚠️ **Klon datasetu s vnořenými datasety** do nich nevidí: vnořené
  datasety jsou v klonu jen hranice (`NOT_SUPPORTED`), změny v nich
  „bokem“ vyzkoušet nejde - na to je klon vnořeného datasetu samotného.
- **Obnova verze a merge kopírují data** (okamžité kopie přes sdílené bloky
  přijdou s fází 3).
- **Merge** páruje nové objekty podle cesty; společný předek je první
  nalezený v DAG (u křížových merge nemusí být nejlepší). Konflikty
  oprávnění a atributů se neslučují - u nových objektů platí výchozí
  vlastník volajícího.
- **`WATCH` čeká na potvrzení** - latence až 5 s (interval commitu), pokud
  klient nevolá `fsync`.
- **Aplikace v záznamu žurnálu** je zatím 0 (identita aplikací v relaci
  přijde se správcem sezení).

## Testy

| Test | Co ověřuje |
|---|---|
| `tests/journal.rs` | záznamy operací, slučování zápisů, zveřejnění po potvrzení, **stejná čísla a uživatelé po pádu a přehrání logu**, retence a `JOURNAL_GAP`, pohled s pozicí a úklid po pádu, práva, datasety a snapshoty v žurnálu |
| `tests/trash.rs` | koš: přesun bez kopie, obnova (i do jiné cesty), vysypání a uvolnění místa, otevřený handle, přes intent log po pádu, úklid podle stáří, viditelnost a práva; atributy a jmenné prostory; expirace (index, zrušení, pád) |
| `tests/history.rs` | verze souboru přes přejmenování, obnova kopií i nahrazením, `undo` všech druhů, automatické snapshoty a prořezání, nastavení; diff3 (i property test) |
| `tests/commits.rs` | commity a log, **logické ID nezávislé na rozmístění bloků**, větev a čistý merge, konflikty textu a binárních souborů, „bokem“ s výběrem a konfliktem, nové složky a výměny jmen |
| `tests/aside_nested.rs` | „bokem“ nad `/` s vnořeným datasetem: hranice (výpis, `stat`, zápis, přejmenování, smazání), změny a převzetí bez hranic, dva současné klony uvnitř `/` (převzít jeden, zahodit druhý a obráceně), složka s datasetem, `clone_snapshot` a větev, kontrola oprávnění |
| init `tests/fs_hist.rs` | v systému přes fsd: žurnál a `JOURNAL_WATCH` s limitem, pohled, koš, commit, větev a merge, „bokem“, atributy a expirace, fsck |
