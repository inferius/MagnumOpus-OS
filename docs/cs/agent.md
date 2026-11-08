# Agent AI v systému

Agent AI je **součást systému, ne jedné aplikace**: služba `agentd` drží
relace agentů, model je za jedním rozhraním nezávislým na poskytovateli
(dnes Claude API, později lokální model) a nástroje dodávají služby
systému. Agent nikdy nemá víc práv než uživatel, který relaci otevřel,
a **zápis mimo přidělený pracovní prostor neumí ani technicky** - jeho
capability k souborům jsou jen pro čtení. Bezpečnost stojí na capability
a potvrzování uživatelem, **ne na tom, že model poslechne**.

Verze 1 (tato stránka). Kód: `libs/agentproto` (rozhraní modelu,
nástroje, protokoly, pravidla, limity, falešný poskytovatel, kodek Claude
API, šifrování klíče), `libs/agentsh` (pomocník v shellu: ověření návrhu
příkazu parserem shlang), `user/agentd` (služba), `user/init/src/agent.rs`
(brána: spuštění `agentd`, vydání capability relace, systémové nástroje),
testy `user/init/src/tests/agent.rs`.

<figure class="diagram">
<svg viewBox="0 0 760 380" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Součásti vrstvy agentů">
<defs><marker id="ag1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="ag1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="110" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">klient</text><text class="m" x="95" y="66" text-anchor="middle">shell (ai), testy,</text><text class="m" x="95" y="84" text-anchor="middle">později GUI</text><text class="m" x="95" y="106" text-anchor="middle">potvrzuje a provádí</text><text class="m" x="95" y="122" text-anchor="middle">zápisy mimo prostor</text>
<rect class="acc" x="250" y="20" width="250" height="210" rx="6"/><text class="t b" x="375" y="45" text-anchor="middle">agentd</text><text class="m" x="375" y="68" text-anchor="middle">relace: konverzace, kroky</text><text class="m" x="375" y="86" text-anchor="middle">pravidla: volně / potvrdit / ne</text><text class="m" x="375" y="104" text-anchor="middle">pracovní prostor: commity</text><text class="m" x="375" y="122" text-anchor="middle">limity tokenů a útraty</text><text class="m" x="375" y="140" text-anchor="middle">auditní log</text><text class="m" x="375" y="158" text-anchor="middle">cesty, které neopustí PC</text><text class="m" x="375" y="176" text-anchor="middle">šifrované API klíče</text><text class="m" x="375" y="200" text-anchor="middle">trait Provider</text>
<rect class="box" x="570" y="20" width="180" height="60" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">Claude API</text><text class="m" x="660" y="64" text-anchor="middle">TLS + HTTP, jen 443</text>
<rect class="box" x="570" y="95" width="180" height="50" rx="6"/><text class="t b" x="660" y="118" text-anchor="middle">falešný model</text><text class="m" x="660" y="135" text-anchor="middle">testy, bez sítě</text>
<rect class="box" x="570" y="160" width="180" height="50" rx="6"/><text class="t b" x="660" y="183" text-anchor="middle">lokální model</text><text class="m" x="660" y="200" text-anchor="middle">později</text>
<rect class="ok" x="10" y="270" width="230" height="100" rx="6"/><text class="t b" x="125" y="294" text-anchor="middle">capability relace</text><text class="m" x="125" y="315" text-anchor="middle">FS jen čtení (identita uživatele)</text><text class="m" x="125" y="333" text-anchor="middle">FS zápis jen v prostoru</text><text class="m" x="125" y="351" text-anchor="middle">systémové nástroje uživatele</text>
<rect class="box" x="270" y="270" width="230" height="100" rx="6"/><text class="t b" x="385" y="294" text-anchor="middle">init (brána)</text><text class="m" x="385" y="315" text-anchor="middle">spouští agentd s manifestem</text><text class="m" x="385" y="333" text-anchor="middle">razí capability relace</text><text class="m" x="385" y="351" text-anchor="middle">stav služeb, pády, log</text>
<rect class="box" x="530" y="270" width="220" height="100" rx="6"/><text class="t b" x="640" y="294" text-anchor="middle">poskytovatelé nástrojů</text><text class="m" x="640" y="315" text-anchor="middle">služby se registrují samy</text><text class="m" x="640" y="333" text-anchor="middle">schéma JSON, druh účinku</text><text class="m" x="640" y="351" text-anchor="middle">volání nese capability relace</text>
<path class="ln-acc" d="M180 70 L248 70" marker-end="url(#ag1a)"/><text class="m" x="186" y="62">relace</text>
<path class="ln" d="M500 50 L568 50" marker-end="url(#ag1)"/>
<path class="ln" d="M500 120 L568 120" marker-end="url(#ag1)"/>
<path class="ln dash" d="M500 185 L568 185" marker-end="url(#ag1)"/>
<path class="ln" d="M385 268 L385 232" marker-end="url(#ag1)"/><text class="m" x="392" y="256">otevře relaci</text>
<path class="ln-acc" d="M200 268 L290 232" marker-end="url(#ag1a)"/>
<path class="ln" d="M600 268 L480 232" marker-end="url(#ag1)"/>
</svg>
<figcaption>Klient otevře relaci přes bránu v initu; agentd mluví s modelem a nástroji jen přes capability relace.</figcaption>
</figure>

## Pojmy

| Pojem | Význam |
|---|---|
| **relace agenta** | jedna konverzace s jedním uživatelem: zprávy, kroky, pracovní prostor, limity. Patří uživateli, který ji otevřel; jiný uživatel ji nevidí |
| **poskytovatel modelu** | implementace `agentproto::Provider` (Claude API, falešný model, později lokální). Klienti ho nevidí - mluví jen s `agentd` |
| **nástroj** | operace, kterou model smí zavolat (`fs.read`, `ws.diff`, `sys.services`, ...). Má jméno, popis, **schéma vstupu** (JSON Schema) a **druh účinku** |
| **druh účinku** | `read` (čtení), `write` (zápis do FS), `exec` (spuštění), `net` (síť), `settings` (nastavení), `propose` (jen návrh pro uživatele). Podle něj se rozhoduje, zda je potřeba potvrzení |
| **pracovní prostor** | dataset, kde agent zapisuje **bez potvrzování**. Začátek relace = commit, každý krok = commit; relace jde vrátit jedním příkazem |
| **krok** | jedna odpověď modelu se všemi jejími voláními nástrojů. Zapsal-li krok do prostoru, vznikne commit „agent: krok N“ |
| **potvrzení** | dotaz klientovi s náhledem změny (diff). Potvrzenou akci **provede klient svými právy**, `agentd` ji provést nemůže |
| **brána** | část initu (`user/init/src/agent.rs`), která spouští `agentd` a pro každou relaci razí capability s identitou uživatele |

## Rozhraní modelu (agentproto)

Model je za traitem nezávislým na poskytovateli. Klienti posílají jen text
a odpovědi na potvrzení; konverzaci, nástroje a smyčku volání drží `agentd`.
Nového poskytovatele (lokální model, jiná firma) jde přidat **bez změny
klientů** - stačí implementovat trait a přidat jméno do výběru v `agentd`.

```rust
pub trait Provider {
    /// Jméno poskytovatele a modelu (do auditu a přehledu útraty).
    fn name(&self) -> &str;
    fn model(&self) -> &str;
    /// Ceník (mikrodolary za milion tokenů); falešný a lokální model 0.
    fn price(&self) -> Price;
    /// Jedno volání modelu. Události proudu (text, začátek volání
    /// nástroje, ...) jdou do `sink` průběžně; výsledek je celá zpráva.
    fn complete(&mut self, req: &Request, sink: &mut dyn FnMut(&Event)) -> Result<Response, ProviderError>;
}
```

- **Zprávy** (`Message`): role `user` / `assistant`, obsah je seznam bloků:
  `Text`, `ToolUse { id, name, input }`, `ToolResult { id, content,
  is_error }` a `Opaque` (blok poskytovatele, kterému vrstva nerozumí -
  např. myšlenkový blok Claude s podpisem; posílá se zpět **beze změny**).
- **Streamování** (`Event`): `MessageStart`, `TextDelta`, `ToolUseStart`,
  `ToolInputDelta`, `BlockStop`, `MessageDelta { stop, usage }`,
  `MessageStop`. Sběrač (`stream::Accumulator`) z událostí složí zprávu -
  stejně pro SSE Claude i pro falešný model.
- **Důvod konce** (`Stop`): `EndTurn`, `ToolUse`, `MaxTokens`, `Refusal`,
  `Other(text)`. `Refusal` agentd hlásí uživateli a nic dalšího nevolá.
- **Spotřeba** (`Usage`): vstup, výstup, zápis a čtení cache v tokenech.
- **Ceník** (`Price`): mikrodolary za milion tokenů pro vstup, výstup,
  zápis a čtení cache. Cena volání = celočíselný výpočet, žádné `f64`.

### Claude API

První poskytovatel (`agentproto::claude`): `POST https://api.anthropic.com/v1/messages`,
hlavičky `x-api-key`, `anthropic-version: 2026-10-21`, tělo s `model`,
`max_tokens`, `system`, `messages`, `tools` (jméno, popis,
`input_schema`) a `output_config.effort`. Výchozí model `claude-opus-5-5`
(4 / 20 USD za milion tokenů vstupu / výstupu, čtení cache 0,20, zápis
5). Výsledky nástrojů jednoho kroku jdou v **jedné** zprávě `user`.
Kodek je čistá funkce (testy na hostiteli): sestavení těla, čtení odpovědi
i proudu SSE (`message_start`, `content_block_start`,
`content_block_delta` s `text_delta` / `input_json_delta`,
`content_block_stop`, `message_delta`, `message_stop`, `error`).

Doprava jde přes `libs/tls` + `libs/httpc`. Ty zatím vrací celé tělo
najednou, proto verze 1 posílá požadavek bez `stream` a události vyrobí
z hotové odpovědi; kodek SSE je hotový pro chvíli, kdy `httpc` dostane
čtení po částech.

## Nástroje ze služeb

Nástroje popisuje `ToolSpec`: jméno (`skupina.akce`), popis pro model,
**schéma vstupu** (podmnožina JSON Schema: `object`, `properties`,
`required`, typy `string` / `integer` / `boolean` / `array`) a druh
účinku. `agentd` vstup od modelu **ověří proti schématu dřív, než nástroj
zavolá** - neplatný vstup se vrátí modelu jako chyba nástroje.

| Nástroj | Účinek | Kdo ho dává | Popis |
|---|---|---|---|
| `fs.read` | read | agentd | obsah souboru (nejvýš 64 KiB, offset) |
| `fs.list` | read | agentd | výpis složky (jméno, druh, velikost) |
| `fs.stat` | read | agentd | metadata cesty |
| `fs.search` | read | agentd | hledání podle jména a textu pod složkou (omezená hloubka a počet) |
| `fs.write` | write | agentd | zápis souboru (celý obsah) |
| `fs.mkdir` | write | agentd | nová složka |
| `fs.remove` | write | agentd | přesun do koše (nikdy trvalé smazání) |
| `fs.rename` | write | agentd | přejmenování / přesun |
| `ws.diff` | read | agentd | změny v prostoru od začátku relace nebo kroku |
| `ws.steps` | read | agentd | kroky relace (commity) |
| `shell.propose` | propose | agentd | návrh příkazu shlang, ověří ho parser (režim shell) |
| `dev.build`, `dev.test` | exec | později překladač | připravené, dnes vrátí `NOT_SUPPORTED` |
| `sys.services` | read | init | stav služeb ze supervisoru |
| `sys.crashes` | read | init | pády: kód konce, restarty, důvod vzdání |
| `sys.log` | read | init | poslední události supervisoru |
| `sys.net` | read | init | stav sítě (adresa, brána, DNS) |
| `sys.tasks` | read | init + `taskd` | úlohy uživatele relace (název, původ, stav, průběh) |
| `sys.task_log` | read | init + `taskd` | poslední řádky výstupu úlohy |

Úlohy čte brána z `taskd` kanálem klienta s identitou uživatele relace
(jen jeho úlohy, bez řízení). Nastavení se přidá stejně, až jeho služba
poběží:
služba pošle `agentd` zprávu `Register` se seznamem `ToolSpec` (protokol
`agentproto::registry`), podobně jako server MCP. Volání nese
**capability relace** (nové spojení FS se stejnou identitou a připojenými
cestami), takže služba operaci provede s právy relace, ne s právy `agentd`.

### Práva nástrojů přes capability

`agentd` sám nemá k souborům uživatelů **žádná práva** - jeho vlastní
relace FS vidí jen `/system/agent` (stav, audit, klíče). Pro každou relaci
mu brána v initu vydá:

1. **FS jen pro čtení** - `CONNECT_AS(uživatel, skupiny, [/ -> / READ_ONLY])`.
   Čtení vidí přesně to, co uživatel; zápis přes ni FS odmítne.
2. **FS prostoru** - `CONNECT_AS(uživatel, skupiny, [P -> P])` jen pro
   dataset prostoru `P` (cesta je v obou relacích stejná).
3. **Systémové nástroje** - kanál ke zdroji v initu vázaný na tuto relaci.

Kontrola pravidel v `agentd` (`agentproto::policy`) je **první** vrstva;
druhá je, že bez capability operace neprojde ani při chybě v `agentd` nebo
při podvrženém volání.

## Práva a pracovní prostor

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Rozhodnutí o volání nástroje">
<defs><marker id="ag2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="ag2a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="110" width="150" height="70" rx="6"/><text class="t b" x="85" y="138" text-anchor="middle">model chce</text><text class="m" x="85" y="158" text-anchor="middle">tool_use</text>
<rect class="acc" x="200" y="100" width="170" height="90" rx="6"/><text class="t b" x="285" y="128" text-anchor="middle">ověření</text><text class="m" x="285" y="148" text-anchor="middle">schéma, režim,</text><text class="m" x="285" y="166" text-anchor="middle">cesta, limity</text>
<rect class="ok" x="420" y="10" width="330" height="70" rx="6"/><text class="t b" x="585" y="36" text-anchor="middle">volně</text><text class="m" x="585" y="56" text-anchor="middle">čtení (bez zakázaných cest), zápis v prostoru</text><text class="m" x="585" y="72" text-anchor="middle">capability relace, commit kroku</text>
<rect class="box" x="420" y="110" width="330" height="70" rx="6"/><text class="t b" x="585" y="136" text-anchor="middle">potvrdit</text><text class="m" x="585" y="156" text-anchor="middle">zápis mimo prostor, spuštění, síť, nastavení</text><text class="m" x="585" y="172" text-anchor="middle">náhled klientovi; provede klient svými právy</text>
<rect class="err" x="420" y="210" width="330" height="70" rx="6"/><text class="t b" x="585" y="236" text-anchor="middle">odmítnout</text><text class="m" x="585" y="256" text-anchor="middle">neplatný vstup, cesta, která neopustí PC,</text><text class="m" x="585" y="272" text-anchor="middle">režim nástroj nemá, limit, klient bez UI</text>
<path class="ln" d="M160 145 L198 145" marker-end="url(#ag2)"/>
<path class="ln-acc" d="M370 120 L418 50" marker-end="url(#ag2a)"/>
<path class="ln" d="M370 145 L418 145" marker-end="url(#ag2)"/>
<path class="ln" d="M370 170 L418 240" marker-end="url(#ag2)"/>
<text class="m" x="10" y="230">Každé rozhodnutí i výsledek jde</text><text class="m" x="10" y="248">do auditního logu. Výsledek nástroje</text><text class="m" x="10" y="266">se modelu vrátí jako data</text><text class="m" x="10" y="284">(tool_result), nikdy jako pokyn.</text>
</svg>
<figcaption>Tři výsledky pravidel. Potvrzenou akci neprovádí agentd - nemá na ni capability.</figcaption>
</figure>

| Operace | Výchozí režim | Pracovní prostor |
|---|---|---|
| čtení souborů, výpis, hledání | volně (v právech uživatele) | volně |
| zápis, nová složka, koš, přejmenování | **potvrdit** s náhledem | **volně**, commit kroku |
| cesta ze seznamu „neopustí počítač“ | odmítnout | odmítnout |
| spuštění programu (`dev.build`, příkaz) | potvrdit | potvrdit |
| síť, nastavení | potvrdit | potvrdit |
| stav služeb, pády, log | volně (jen čtení) | volně |

**Potvrzení** posílá `agentd` klientovi jako `Confirm { id, action,
preview }`. Akce (`agentproto::action::Action`) je přesný popis
(`FsWrite { path, content }`, `FsMkdir`, `FsTrash`, `FsRename`, `Run`).
Náhled je diff starého a nového obsahu (řádky `-` / `+`). Klient se zeptá
uživatele; při „ano“ akci **provede sám svými právy** a vrátí výsledek,
při „ne“ vrátí `Declined`. Klient bez uživatelského rozhraní (např. úloha
na pozadí) odpoví vždy `Declined`. Jak klient akci provede, je společná
funkce `action::apply` nad traitem `ActionFs`.

### Pracovní prostor

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Kroky relace v pracovním prostoru">
<defs><marker id="ag3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="ag3a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<path class="ln" d="M30 80 L730 80"/>
<rect class="acc" x="20" y="55" width="130" height="50" rx="6"/><text class="t b" x="85" y="78" text-anchor="middle">začátek</text><text class="m" x="85" y="96" text-anchor="middle">commit 0</text>
<rect class="ok" x="200" y="55" width="130" height="50" rx="6"/><text class="t b" x="265" y="78" text-anchor="middle">krok 1</text><text class="m" x="265" y="96" text-anchor="middle">commit</text>
<rect class="ok" x="380" y="55" width="130" height="50" rx="6"/><text class="t b" x="445" y="78" text-anchor="middle">krok 2</text><text class="m" x="445" y="96" text-anchor="middle">commit</text>
<rect class="ok" x="560" y="55" width="130" height="50" rx="6"/><text class="t b" x="625" y="78" text-anchor="middle">krok 3</text><text class="m" x="625" y="96" text-anchor="middle">commit</text>
<path class="ln-acc dash" d="M625 110 C 600 180, 120 180, 85 112" marker-end="url(#ag3a)"/><text class="m" x="250" y="170">vrátit relaci: obsah prostoru jako na začátku (+ commit „vráceno“)</text>
<path class="ln dash" d="M445 50 C 430 15, 285 15, 268 52" marker-end="url(#ag3)"/><text class="m" x="300" y="22">vrátit na krok 1</text>
<text class="m" x="20" y="200">Rozdíl mezi kroky = DIFF mezi snímky commitů. Nic se nemaže natrvalo: přepsané soubory zůstávají ve snímcích.</text>
</svg>
<figcaption>Každý krok agenta v prostoru je commit FS; vrácení obnoví obsah ze snímku začátku nebo zvoleného kroku.</figcaption>
</figure>

- Prostor přidělí uživatel při otevření relace (`ai --prostor=~/projekt`).
  **Musí to být dataset** (subvolume). Relace vidí jen své připojené
  cesty a historie (commity, snímky, rozdíly) je vlastnost datasetu; pro
  obyčejnou složku by brána musela agentovi připojit celý nadřazený
  dataset, tedy dát víc práv, než uživatel přidělil. Nová cesta se na
  přání vytvoří jako nový dataset (`DATASET_CREATE`).
- **Začátek relace**: `COMMIT_VERSION(P, "agent <relace>: začátek")`.
- **Každý krok**, který do prostoru zapsal: `COMMIT_VERSION(P, "agent
  <relace>: krok N: <nástroje>")`. Commit dělá `agentd` po vyřízení všech
  volání kroku.
- **Procházení**: `ws.steps` (seznam kroků s commity), `ws.diff` (rozdíl
  mezi kroky nebo proti začátku). Totéž vidí klient (`Steps`, `StepDiff`).
- **Vrácení** (`Revert { to }`, `to = 0` = celá relace): `agentd` spočítá
  `DIFF(P, snímek cíle, živá verze)` a změny vrátí: přidané soubory do koše,
  změněné a smazané zpět ze snímku, složky podle potřeby. Pak commit
  „agent: vráceno na krok N“. Nic se nemaže natrvalo - vrácený stav jde
  znovu vrátit.

### Auditní log

Každé volání nástroje a každé volání modelu se zapíše do
`/system/agent/audit/<uživatel>.log` (relace FS `agentd`, uživatel ani agent
do něj psát nemohou). Řádek obsahuje: pořadí, čas, relaci, krok, nástroj,
**souhrn vstupu** (cesta, délka a hash obsahu - ne obsah), rozhodnutí
(`allow`, `confirm-yes`, `confirm-no`, `deny`), výsledek (kód chyby)
a u volání modelu poskytovatele, model, tokeny, cenu a **počet bajtů
odeslaných do cloudu**. Klient čte jen záznamy svého uživatele
(`Audit { from, max }`).

## Pomocník v shellu

Příkaz `ai` v shellu otevře relaci v režimu **shell**: nástroje jen pro
čtení a `shell.propose`. Návrh příkazu (`agentsh::validate`) **ověří parser
shlang** v zápisu B i C a převede ho do druhého zápisu; neplatný návrh se
vrátí modelu s hlášením parseru, aby ho opravil. Příkazy, které mažou,
přesouvají nebo přepisují, jsou označené jako riskantní.

```shell-b
ai "které soubory v ~/Dokumenty jsou větší než 100 MB"
```

- **Dotaz v přirozeném jazyce** -> odpověď a navržený příkaz
  (`{ prikaz, zapis, druhy_zapis, riskantni, vysvetleni }`). Příkaz se
  **nespustí sám** - uživatel ho spustí (nebo upraví); to je potvrzení.
- **Vysvětlení chyby**: `ai --vysvetli="..."` pošle poslední chybu a text
  z místní dokumentace chyb (`shlang::explain`).
- **Úloha přes více kroků**: model navrhne plán (seznam ověřených
  příkazů); shell ukáže celý plán jako náhled, kroky spouští uživatel.
- Režim shell **nic nezapisuje** (nemá zápisové nástroje), proto
  nepotřebuje dialog potvrzení.
- Další přepínače: `--rezim=diag|chat|dev`, `--prostor=~/projekt` (režim
  dev s pracovním prostorem), `--kroky`, `--rozdil=N`, `--vrat=N`,
  `--audit`, `--utrata`, `--klic=soubor`. Relaci agenta drží `shsess` pro
  další `ai` v téže relaci shellu (stejný režim a prostor).
- Příkaz `ai` běží uvnitř vyhodnocení řádku a shell tam nemá dialog
  potvrzení. Relace otevřené ze shellu proto potvrzení **neumí**
  (`can_confirm = false`): zápis mimo prostor se odmítne, v prostoru
  projde. Potvrzení s náhledem bude mít GUI klient.

Napojení: init dá `termd` zapisovací konec **brány terminálů**, `termd` ho
duplikuje každé relaci `shsess`. Žádost je `OpenSpec` s přibaleným kanálem
odpovědi; vlákno brány v initu doplní identitu a capability a odpoví
`ok <číslo>` s kanálem klienta. Duplikát brány neumí nic víc než požádat
o relaci s identitou terminálu.

> **Upozornění:** dokud systém nemá přihlášení, běží terminál s identitou
> systému. Relaci FS se systemovou identitou ale vyrazit nejde (FS to
> odmítne), proto brána terminálu otevírá relace agenta jako **výchozí
> uživatel 1000**: agent čte jen to, co smí tento uživatel (méně než
> terminál), a pracovní prostor musí patřit jemu. Po zavedení přihlášení
> dostane brána identitu přihlášeného uživatele.

## Diagnostika

Režim **diag** má jen nástroje pro čtení: `sys.services`, `sys.crashes`,
`sys.log`, `sys.net` a čtení souborů. Agent navrhne opravu textem (nebo
příkazem, který spustí uživatel); sám nic nemění.

## Vývoj v systému

Režim **dev**: pracovní prostor projektu, čtení a úpravy kódu (`fs.*`
volně v prostoru), rozdíly (`ws.diff`), kroky a vrácení. Překlad a testy
jsou připravené nástroje `dev.build` a `dev.test` s účinkem `exec`
(potvrzuje se); dnes vrátí `NOT_SUPPORTED`. Až v systému poběží překladač,
zaregistruje nástroje se stejným jménem a `agentd` je začne nabízet.

## Bezpečnost

> **Závazné riziko:** model běží v cloudu. **Všechno, co agent přečte,
> odchází k poskytovateli** (výsledky nástrojů jsou součástí konverzace).
> Prompt injection v souboru, logu nebo na webu může model navést, aby
> přečetl jiné soubory - v rámci práv uživatele. Proti tomu stojí seznam
> cest, které nikdy neopustí počítač, režimy s menší sadou nástrojů,
> audit a limity útraty; **úplná ochrana čtení to není**. Pro citlivá data
> použijte režim s omezeným čtením (`cteni = "prostor"`) nebo později
> lokální model.

### Prompt injection nesmí rozšířit práva

- Práva určují **capability relace**, které vydala brána podle uživatele.
  Model ani text v souboru je nemůže změnit: `agentd` nemá kanál k bráně,
  přes který by si řekl o víc.
- Zápis mimo prostor jde **jen přes klienta** po potvrzení uživatelem
  s náhledem. Capability `agentd` jsou jen pro čtení - i kdyby pravidla
  v `agentd` selhala, FS zápis odmítne (test „zápis mimo prostor bez
  potvrzení selže“).
- Výsledky nástrojů jdou modelu jako `tool_result` (data), systémový
  pokyn modelu říká, že obsah souborů a logů jsou nedůvěryhodná data.
  To je jen pomoc, **ne ochrana** - na model se nespoléháme.
- Síť má `agentd` jen k `api.anthropic.com:443` (manifest
  `agentd.manifest.toml`, `ask = false`); agent v1 nemá nástroj pro web,
  takže nemůže data poslat jinam než poskytovateli.
- Návrhy příkazů shellu se nespouštějí samy; riskantní jsou označené.

### Co se posílá do cloudu

| Posílá se | Neposílá se |
|---|---|
| systémový pokyn relace a popis nástrojů | API klíč jiným než poskytovateli (jen v hlavičce `x-api-key`) |
| dotazy uživatele | obsah cest ze seznamu níže |
| výsledky nástrojů: obsah přečtených souborů, výpisy, stav služeb | auditní log, stav `agentd`, klíče |
| návrhy a odpovědi modelu (konverzace) | soubory, které agent nečetl |

Audit u každého volání modelu zapíše počet odeslaných bajtů.

### Cesty, které nikdy neopustí počítač

Nástroje pro čtení vrátí `ACCESS_DENIED` („cesta nesmí opustit počítač“)
pro cesty, které odpovídají vzorům (`*` uvnitř jména, `**` libovolná
hloubka):

```text
/system/agent/**
/system/keys/**
**/.ssh/**
**/.gnupg/**
**/*.key
**/*.pem
**/.config/agent/**
**/hesla/**
**/passwords/**
```

Uživatel může seznam rozšířit v nastavení relace (`neopusti`), zúžit ho
nejde. Platí pro všechny poskytovatele (i lokální model, kvůli auditu
a jednotnosti).

### Limity útraty

| Limit | Výchozí | Co se stane |
|---|---|---|
| tokeny výstupu na volání (`max_tokens`) | 16 000 | model skončí s `MaxTokens` |
| volání modelu na jeden dotaz | 24 | krok se zastaví s chybou `OUT_OF_RANGE` |
| útrata relace | 1,00 USD | další volání se odmítne předem (`NO_RESOURCES`) |
| útrata uživatele za den | 5,00 USD | dtto; počítadlo v `/system/agent/spend/<uživatel>` |

Před každým voláním `agentd` spočítá **horní odhad** ceny (odhad vstupu
podle délky textu + `max_tokens` výstupu) a volání nepustí, když by limit
překročil. Po volání přičte skutečnou cenu z `usage`.

### API klíč

- Uživatel zadá klíč příkazem `ai --klic=soubor` (shell klíč přečte
  ze souboru a soubor nabídne smazat; v řádku příkazu by klíč zůstal
  v historii). Klient pošle klíč `agentd` zprávou `SetKey`.
- `agentd` klíč **zašifruje** (ChaCha20-Poly1305) klíčem odvozeným
  HKDF-SHA256 z tajemství stroje a čísla uživatele; přidaná data AEAD =
  verze, uživatel, poskytovatel. Uloží ho do `/system/agent/keys/<uživatel>.<poskytovatel>`.
  Klíč jiného uživatele nejde použít (jiný odvozený klíč, jiná přidaná data).
- Klíč **nikdy** není v čitelném souboru ani v logu: typ `Secret` má
  `Debug` jen `***`, paměť se po použití přepíše nulami, audit klíč
  nezapisuje, klient ho zpět nedostane.

> **Závazné riziko:** tajemství stroje leží na disku v `/system/agent`
> (chráněné právy FS, ne hardwarem). Kdo přečte disk mimo systém (jiný OS,
> ukradený disk; FS zatím nemá šifrování dat), klíč dešifruje. Opravou je
> klíč odvozený z hesla při přihlášení nebo TPM - až budou.

## Protokoly

### Klient - agentd (`agentproto::proto`)

Zpráva = kód `u8` + pole (`libs/wire`); neplatná zpráva = chyba, nikdy
panika.

| Klient -> agentd | Význam |
|---|---|
| `Ask { text }` | dotaz; agentd odpovídá zprávami až po `TurnEnd` nebo `Error` |
| `Confirmed { id, outcome }` | odpověď na `Confirm`: `Done(text)`, `Declined`, `Failed(kód, text)` |
| `Steps`, `StepDiff { step }`, `Revert { to }` | procházení a vrácení prostoru |
| `Audit { from, max }`, `Spend` | auditní záznamy uživatele, útrata a limity |
| `SetKey { provider, key }` | uložit API klíč (zašifrovaně) |

| agentd -> klient | Význam |
|---|---|
| `Text { text }` | text modelu (po částech) |
| `Tool { name, decision, summary }` | oznámení volání nástroje |
| `Confirm { id, action, preview }` | žádost o potvrzení |
| `Proposal { command, style, other, risky, explanation }` | návrh příkazu (režim shell) |
| `TurnEnd { stop, usage, cost, step }` | konec dotazu |
| `Error { code, message }` | chyba (limit, poskytovatel, ...) |
| `Steps`, `Diff`, `Reverted`, `AuditPage`, `Spend`, `KeyStored` | odpovědi |

### Brána (init) - agentd

Bootstrap kanál `agentd`: zpráva `Open { uživatel, režim, prostor,
poskytovatel, model, limity, neopusti, skript }` s handly `[kanál klienta,
FS čtení, FS prostoru?, systémové nástroje?]` a `Provider` s kanálem
poskytovatele nástrojů. Klient dostane druhý konec svého kanálu od brány.

### Poskytovatel nástrojů - agentd (`agentproto::registry`)

`Register { tools }` od služby, pak `Call { id, tool, input }` s handlem
capability relace a odpověď `Result { id, is_error, content }`.

## Testy

Na hostiteli (`cargo test -p agentproto -p agentsh`): JSON, schémata,
pravidla, kodek Claude (tělo, odpověď, SSE, chyby), ceník a limity,
falešný model, sběrač proudu, šifrování klíče (vazba na uživatele),
ověření návrhu příkazu parserem.

V QEMU (`cargo xtask test`, sada „agent AI“) s **falešným poskytovatelem**
(deterministický skript odpovědí a volání nástrojů, žádná síť):

- čtení souboru nástrojem, výsledek dorazí k modelu,
- **zápis mimo prostor bez potvrzení selže** (klient odmítne -> soubor se
  nezmění; přímý zápis přes capability `agentd` -> `ACCESS_DENIED`),
- potvrzený zápis mimo prostor provede klient,
- zápis v prostoru bez potvrzení, commit za každý krok,
- vrácení celé relace a návrat na krok,
- auditní log obsahuje všechna volání a rozhodnutí,
- cesta „neopustí počítač“ se nepřečte,
- limit útraty zastaví další volání,
- návrh příkazu shellu ověří parser,
- systémové nástroje (stav služeb) a registrace cizího poskytovatele.

V CI se nikdy nevolá skutečné API.

### Ruční zkouška se skutečným API

Jen ručně, s klíčem uživatele:

1. `cargo xtask run`, v terminálu uložte klíč do souboru (např. zkopírujte
   přes osfs do `~/anthropic.key`) a spusťte `ai --klic=~/anthropic.key`
   (klíč se zašifruje, soubor smažte).
2. `ai "kolik místa zabírá ~/Dokumenty"` - odpověď a návrh příkazu.
3. `ai --prostor=~/zkouska "založ soubor poznamky.txt s osnovou"` -
   zápis bez dotazu, `ai --kroky`, `ai --vrat` vrátí relaci.
4. Audit: `ai --audit`; útrata: `ai --utrata`.

## Omezení verze 1

- `httpc` vrací celé tělo, proto se odpověď Claude nestreamuje po síti.
- Logy jádra nejdou ze služby číst (chybí syscall); `sys.log` vrací
  události supervisoru. Služba logů je samostatný úkol.
- Nastavení nemá vlastní nástroje, dokud jeho služba není ve větvi `main`.
  Identitu uživatele (`uzivatel-<číslo>` v `taskd`, uživatel 1000 v bráně
  terminálu) převezme agent od služby účtů, až bude.
- Terminál nemá skrytý vstup hesla, proto se klíč zadává souborem.
- Prostor musí být dataset (viz výše).
- Volání poskytovatele z různých relací jdou přes jeden zámek (po jednom).
- U nástroje služby se zápisovým účinkem potvrzení jen **povolí** volání;
  službu pak volá `agentd` s capability relace (klient akci neprovádí).
- Kontrola cest je textová; FS při překladu cesty nenásleduje symbolické
  odkazy, takže odkaz do zakázané cesty se nepřečte.
