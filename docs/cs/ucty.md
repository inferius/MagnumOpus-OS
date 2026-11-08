# Uživatelské účty

Služba **`accountd`** spravuje účty lidí na počítači: kdo existuje, jak se
přihlašuje (heslo, PIN, bezpečnostní klíč, vývojové automatické
přihlášení) a jaký má domov. Po úspěšném ověření požádá **init** o relaci;
init razí identitu pro `fsd` (`CONNECT_AS`) a stejně kanály terminálu, GUI
a úloh. **Klient si identitu nikdy nevymyslí** - dostane jen kanály, které
mu init vyrazil.

Nikde není `root` ani účet správce. Uživatel 0 (systém) je jen identita
řídicí relace initu; přihlásit se za něj nejde.

Kód: protokol a CTAP2 `libs/accountproto`, logika (testovaná na
hostiteli) `libs/accounts`, služba `user/accountd`, start a správce
sezení v initu `user/init/src/accounts.rs` a `user/init/src/session.rs`,
testy v QEMU `user/init/src/tests/accounts.rs`.
Obrazovky přihlášení a nastavení (`design/prihlaseni.html`,
`design/nastaveni.html`) kreslí jiné vlákno; tato stránka popisuje službu
a rozhraní, které obrazovky použijí.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sluzba uctu a jeji okoli">
<defs><marker id="uc1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="uc1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">přihlašovací obrazovka</text><text class="m" x="110" y="64" text-anchor="middle">systémová vrstva GUI</text><text class="m" x="110" y="80" text-anchor="middle">kanál login</text>
<rect class="box" x="10" y="120" width="200" height="70" rx="6"/><text class="t b" x="110" y="145" text-anchor="middle">nastavení účtu</text><text class="m" x="110" y="164" text-anchor="middle">v relaci uživatele</text><text class="m" x="110" y="180" text-anchor="middle">kanál self</text>
<rect class="acc" x="290" y="20" width="200" height="170" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">accountd</text><text class="m" x="390" y="68" text-anchor="middle">účty, ověření</text><text class="m" x="390" y="86" text-anchor="middle">Argon2id, PIN, FIDO2</text><text class="m" x="390" y="104" text-anchor="middle">zpomalení po chybách</text><text class="m" x="390" y="122" text-anchor="middle">obaly klíče domova</text><text class="m" x="390" y="140" text-anchor="middle">identita FS: účet 902</text><text class="m" x="390" y="158" text-anchor="middle">vidí jen /system/accounts</text>
<rect class="ok" x="570" y="20" width="180" height="90" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">init</text><text class="m" x="660" y="64" text-anchor="middle">správce sezení</text><text class="m" x="660" y="80" text-anchor="middle">řídicí relace fsd</text><text class="m" x="660" y="96" text-anchor="middle">CONNECT_AS, domovy</text>
<rect class="box" x="570" y="140" width="180" height="50" rx="6"/><text class="t b" x="660" y="162" text-anchor="middle">fsd</text><text class="m" x="660" y="180" text-anchor="middle">oprávnění F1d</text>
<rect class="box" x="290" y="230" width="200" height="60" rx="6"/><text class="t b" x="390" y="254" text-anchor="middle">bezpečnostní klíč</text><text class="m" x="390" y="273" text-anchor="middle">usb-hid, CTAPHID</text>
<rect class="box" x="570" y="230" width="180" height="60" rx="6"/><text class="t b" x="660" y="254" text-anchor="middle">relace uživatele</text><text class="m" x="660" y="273" text-anchor="middle">FS, terminál, GUI, úlohy</text>
<path class="ln" d="M210 55 L288 55" marker-end="url(#uc1)"/>
<path class="ln" d="M210 155 L288 155" marker-end="url(#uc1)"/>
<path class="ln-acc" d="M490 55 L568 55" marker-end="url(#uc1a)"/><text class="m" x="529" y="47" text-anchor="middle">relace</text>
<path class="ln" d="M660 110 L660 138" marker-end="url(#uc1)"/>
<path class="ln-acc" d="M700 110 L700 125 L745 125 L745 228" marker-end="url(#uc1a)"/>
<path class="ln" d="M390 190 L390 228" marker-end="url(#uc1)"/>
<path class="ln" d="M490 170 L568 170" marker-end="url(#uc1)"/><text class="m" x="529" y="162" text-anchor="middle">DB</text>
<text class="m" x="10" y="318">Identitu razí jen init. accountd rozhoduje, kdo se přihlásil; nikdy sám nevidí cizí domov.</text>
</svg>
<figcaption>accountd ověří, init razí relaci, fsd vynucuje oprávnění.</figcaption>
</figure>

## Pojmy

| Pojem | Význam |
|---|---|
| **účet** | člověk na tomto počítači: číslo uživatele, jméno pro přihlášení, zobrazované jméno, skupiny, metody ověření, domov |
| **číslo uživatele** (`uid`) | identita pro FS a ostatní služby (`u64`) |
| **metoda ověření** | heslo, PIN, bezpečnostní klíč nebo vývojové automatické přihlášení |
| **relace** (sezení) | to, co init vyrazí po přihlášení: job s procesy uživatele a kanály s jeho identitou |
| **domov** | vlastní dataset `/home/<jméno>` patřící uživateli |
| **role správy** | „smí spravovat tento počítač“ (viz [Správa](#správa-počítače)) |
| **klíč zařízení** | tajemství vázané na počítač (dnes soubor, později TPM), viz [PIN](#pin-vázaný-na-zařízení) |
| **klíč domova** | náhodný klíč šifrování domova, zabalený zvlášť pro každou metodu |
| **obnovovací klíč** | náhodný klíč, který uživatel dostane jednou při zapnutí šifrování; otevře domov i bez metody |

## Čísla identit

| Rozsah | Kdo | Poznámka |
|---|---|---|
| 0 | systém | jen řídicí relace fsd (init); přihlásit se nejde, `CONNECT_AS` 0 fsd odmítne |
| 1-899 | rezerva | |
| 900-999 | servisní účty služeb | `fwd` 900, `revocd` 901, **`accountd` 902** |
| 1000-999 999 | lidé | přiděluje accountd vzestupně, číslo smazaného účtu se **nepoužije znovu** (staré granty na FS by jinak platily pro nového člověka) |
| 1 000 000+ | skupiny | `uzivatele` (1 000 000) = všichni lidé; další skupiny zakládá správa |
| `u64::MAX` | `EVERYONE` | v grantech FS „každý kromě systému“ |

Skupiny a uživatelé sdílejí prostor čísel grantů FS, proto mají oddělené
rozsahy.

## Databáze účtů

- **Vlastní dataset** `/system/accounts` (vytvoří init při prvním startu).
  Vlastník je servisní účet `accountd` (902), popis oprávnění `PROTECTED`
  bez grantů: kromě systému ho nikdo nevidí (`NOT_FOUND`). Dataset má
  `NOEXEC`.
- accountd dostane relaci FS jen s připojením `/db -> /system/accounts`.
  Nic jiného na FS nevidí.
- **Jeden soubor na účet** `/db/users/<uid>`, binárně (`libs/wire`):
  verze formátu, záznam, na konci BLAKE2s-256 obsahu (poškozený soubor
  accountd pozná a účet hlásí jako poškozený, ostatní účty fungují).
  Zápis přes dočasný soubor a `rename_replace` - po výpadku je vždy celý
  starý, nebo celý nový záznam.
- `/db/meta`: další volné číslo uživatele a skupiny, verze DB.
- `/db/device.key`: klíč zařízení (32 B), dokud není TPM.

| Pole záznamu | Typ | Poznámka |
|---|---|---|
| `uid` | u64 | |
| `name` | text | jméno pro přihlášení: `a-z`, `0-9`, `-`, `_`, 1-32 znaků, začíná písmenem; jedinečné; je i jménem domova |
| `display` | text | zobrazované jméno (UTF-8, do 64 znaků) |
| `groups` | u64[] | vždy obsahuje `uzivatele` |
| `flags` | u32 | role správy, účet vypnutý, vývojový účet |
| `phrase` | text | bezpečnostní fráze (do 64 znaků), ukáže ji systémový dialog a přihlášení |
| `password` | volitelné | parametry Argon2id, sůl, ověřovací hodnota |
| `pin` | volitelné | parametry, sůl, ověřovací hodnota, počet chyb |
| `keys` | seznam | bezpečnostní klíče: id přihlašovacích údajů, veřejný klíč COSE, počítadlo podpisů, název |
| `failures` | u32 + i64 | neúspěšné pokusy po sobě a čas posledního |
| `home` | text | `/home/<name>` |
| `crypt` | volitelné | šifrování domova: obaly klíče domova (viz [Šifrování](#šifrování-domova)) |
| `created`, `last_login` | i64 | reálný čas |

Tajemství v DB **nejsou čitelná zpět**: hesla a PIN jen jako výstup
Argon2id, klíč domova jen zabalený.

## Domov

- Dataset `/home/<jméno>` založí init na žádost accountd
  (`CREATE_HOME`): vlastník = `uid`, popis oprávnění `PROTECTED` bez
  grantů (ostatní uživatelé domov ani nevidí: `NOT_FOUND`), `NOEXEC`
  podle profilu (standardní a přísný: zapnuto - data nejsou programy).
- `/home` patří systému a má grant `TRAVERSE` pro skupinu `uzivatele`
  (projít, ne vypsat): kdo nezná jméno, nedozví se, jaké domovy existují.
- Smazání účtu: domov se **nemaže hned**, přesune se do koše správy
  (`/home/.smazane/<uid>`, `PROTECTED` bez grantů - vidí ho jen systém;
  prohlížení pro roli správy a mazání po 30 dnech přijdou se správou
  účtů). Omylem smazaný účet tak nepřijde o data.
- Data aplikací leží v `/home/<jméno>/apps/<id>/` (viz
  [Architektura](architektura.md)).

## Ověření

Všechny čtyři metody jsou v první verzi. Uživatel si u svého účtu zvolí,
které používá; heslo je povinné u každého účtu kromě vývojového (je to
záchranná metoda, když PIN nebo klíč nejde použít). Heslo má **aspoň 8
znaků**; pravidla typu „velké písmeno a číslice“ se nevynucují (délka je
hlavní síla hesla, NIST SP 800-63B).

### Heslo (Argon2id)

Ověřovací hodnota = Argon2id (RFC 9106, verze 0x13) z hesla v UTF-8
(normalizace NFC) a náhodné soli 16 B, výstup **64 B**: prvních 32 B je
ověřovací hodnota v DB, druhých 32 B je klíč k obalu klíče domova (jeden
běh Argon2id na obojí; z ověřovací hodnoty klíč obalu odvodit nejde).
Parametry se ukládají u každé hodnoty, takže jdou zvyšovat: po úspěšném
přihlášení se hodnota se slabšími parametry přepočítá.

| Profil | Paměť `m` | Průchody `t` | Vlákna `p` | Čas (změřeno) | Kdy |
|---|---|---|---|---|---|
| `qemu` | 8 MiB | 2 | 1 | hostitel 5 ms (nativně); celé přihlášení v QEMU TCG (x86_64 i aarch64, vč. dvou zápisů DB s `fsync`) 0,1-0,3 s | `cargo xtask run`, `cargo xtask test` |
| `hw` (výchozí) | 64 MiB | 3 | 1 | hostitel 70 ms (Ryzen/Core 2024, nativně) | skutečné PC |
| `hw` po kalibraci (plán) | 64 MiB | 3-10 | 1 | cíl 250-500 ms | instalace změří a zvýší `t` |

Srovnání: OWASP doporučuje nejméně 19 MiB a `t=2`, RFC 9106 druhou
volbu 64 MiB a `t=3`. Paměť je hlavní ochrana proti hádání na grafických
kartách, proto se drží 64 MiB a na pomalém stroji se raději ubere čas.
⚠️ Při přihlášení si accountd na chvíli vezme 64 MiB paměti; souběžná
ověření proto běží **jedno po druhém** (frontou), aby se paměť nezdvojila.

Implementace: crate `argon2` (RustCrypto), žádná vlastní kryptografie.
V ladicím buildu se `argon2`, `blake2` a `accounts` překládají
s optimalizací (`Cargo.toml`): bez ní trvalo přihlášení v QEMU ~2 s.

### PIN vázaný na zařízení

PIN (4-12 číslic) je krátký: z ukradené DB by se uhodl za vteřiny. Proto
ověřovací hodnota PINu **závisí na klíči zařízení**, který z počítače
nejde odnést:

```text
p = Argon2id(PIN, sůl)                        // stejné parametry jako heslo
ověřovací hodnota = HMAC-SHA256(klíč zařízení, "pin-verify" || uid || p)
```

Klíč zařízení schovává rozhraní **`DeviceKey`** (trait v `libs/accounts`):

| Operace | Dnes (`FileKey`) | Později (`TpmKey`, TPM 2.0) |
|---|---|---|
| `mac(label, data)` | HMAC-SHA256 s klíčem z `/db/device.key` | HMAC uvnitř TPM (klíč zapečetěný k PCR), klíč TPM nikdy neopustí |
| `hardware()` | `false` | `true` |
| čítač proti hádání | počítadlo v DB (zpomalení níže) | navíc čítač v TPM (dictionary attack lockout) |

Po **5 špatných PINech** se PIN zablokuje, dokud se uživatel jednou
nepřihlásí heslem nebo klíčem.

⚠️ **Riziko (bez TPM):** `FileKey` leží na stejném disku jako DB. Kdo
odnese disk, odnese obojí a PIN uhodne offline za vteřiny. Bez TPM PIN
chrání **jen proti hádání u klávesnice** (zpomalení, blokace), ne proti
krádeži disku - a u šifrovaného domova je pak obal klíče z PINu stejně
slabý. PIN je i tak **povolený pro všechno** (přihlášení, zamčená
obrazovka, odemčení šifrovaného domova; rozhodnutí uživatele), ale
nastavení při zapnutí PINu na počítači bez TPM **výslovně varuje**
(`DeviceKey::hardware() == false`, ve výpisu a v `WHOAMI` příznak
`PIN_WEAK`). S TPM (dnes běžná součást PC) se nevaruje: klíč PIN neopustí
čip a čítač proti hádání vede TPM.

### Bezpečnostní klíč (FIDO2, CTAP2 přes USB HID)

Klíč (YubiKey, SoloKey, Nitrokey, ...) se hlásí jako zařízení HID
s usage page `0xF1D0`. Ovladač `usb-hid` takové rozhraní nepředá vstupu,
ale **přepne do přenosu CTAPHID**: hlášení po 64 B předává po kanálu
a init ho dá accountd (`key` + handle; jedna zpráva = jedno hlášení).
devmgr vytvoří kanál pro každé rozhraní HID; ovladač zařízení, které
klíč není, ho zavře a accountd zavřené kanály zahodí (použije poslední
připojený klíč). Nad ním `accountd` mluví CTAP2
(`libs/accountproto::ctap`: CTAPHID rámce, CBOR, zprávy).

| Krok | CTAP2 | Co accountd ověří |
|---|---|---|
| informace | `authenticatorGetInfo` | verze `FIDO_2_0`/`FIDO_2_1`, podpora `clientPin`, `hmac-secret`, protokoly PIN 1 a 2 |
| PIN klíče | `authenticatorClientPIN`: `getKeyAgreement`, `setPIN`, `changePIN`, `getPinToken` / `getPinUvAuthTokenUsingPinWithPermissions`, `getRetries` | sdílené tajemství ECDH P-256 (protokol 1: SHA-256, protokol 2: HKDF-SHA256), PIN jen zašifrovaný AES-256-CBC, token PIN |
| registrace | `authenticatorMakeCredential` (rp `magnumopus.local`, uživatel = uid, ES256 a EdDSA, `rk=false`, `hmac-secret: true`, `pinUvAuthParam`) | atestace `none` nebo `packed` (vlastní podpis), příznaky UP a UV, uloží id přihlašovacích údajů a veřejný klíč COSE |
| přihlášení | `authenticatorGetAssertion` (allowList = klíče účtu, náhodná výzva 32 B, `pinUvAuthParam`, `hmac-secret` se solí účtu) | podpis nad `authData` + `SHA-256(výzva)` veřejným klíčem (p256 / ed25519 z RustCrypto), `rpIdHash`, příznaky **UP** (dotek) a **UV** (PIN klíče), počítadlo podpisů roste; výstup `hmac-secret` rozbalí klíč domova |

- Plné CTAP2 v první verzi: **klíč + dotek + PIN klíče** (UV). PIN klíče
  ověřuje klíč sám (8 pokusů, pak se klíč zablokuje - vlastnost klíče),
  accountd ho jen zašifrovaný předá. Klíč bez nastaveného PINu: nastavení
  nabídne PIN klíče nastavit (`setPIN`); bez PINu jde klíč použít jen
  jako „klíč + dotek“ a nastavení varuje (kdo klíč ukradne a zná jméno
  účtu, přihlásí se).
- **`hmac-secret`**: klíč z tajemství přihlašovacích údajů a soli účtu
  (32 B, v DB) spočítá HMAC; z něj accountd odvodí obal klíče domova.
  Bez klíče v ruce obal nikdo nerozbalí.
- Počítadlo podpisů, které neroste (a není 0), znamená možný klon klíče:
  přihlášení se odmítne a uživatel dostane upozornění.
- Testy: simulovaný klíč na hostiteli (`accountproto::ctap::soft`)
  s p256; QEMU skutečný FIDO2 klíč neemuluje (`u2f-emulated` umí jen
  starší U2F).

### Automatické přihlášení (jen vývoj)

`cargo xtask run` přidá do initfs soubor `accounts-dev` a accountd pak
založí (pokud chybí) účet `vyvojar` (první volné číslo, příznak vývojový
účet)
a hned ho přihlásí. **Dvojí pojistka:** kód automatického přihlášení je
v accountd jen v ladicím buildu (`cfg(debug_assertions)`) a soubor přidá
jen `xtask run` - `xtask image` (obraz pro uživatele) ani `--release` ho
nemají. Vývojový účet nemá heslo, nemá roli správy a ve výpisu účtů je
označený.

### Ochrany

**Zpomalení po neúspěšných pokusech** (počítá se na účet, všechny metody
dohromady, PIN navíc svých 5 pokusů):

| Neúspěšných pokusů po sobě | Další pokus nejdřív za |
|---|---|
| 1-2 | hned |
| 3 | 30 s |
| 4 | 1 min |
| 5 | 2 min |
| 6 | 5 min |
| 7 a víc | 15 min |

- **Trvalé zamčení účtu se nepoužívá:** kdokoli u klávesnice by tak
  majiteli účet zablokoval. Po 7 pokusech je to 4 pokusy za hodinu, tedy
  heslo z 8 náhodných znaků se neuhodne.
- Počítadlo se **zvýší a uloží před ověřením** a vynuluje až po úspěchu:
  pád nebo vypnutí uprostřed pokusu ho nevynuluje.
- Odpověď na neúspěch je u všech metod stejná (`DENIED` + čekání), pro
  neexistující účet se spočítá Argon2id naprázdno (stejná doba odpovědi).
- **Porovnání v konstantním čase** (`subtle::ConstantTimeEq`) u všech
  ověřovacích hodnot.
- **Žádná tajemství v logu:** hesla, PINy, výzvy a klíče jsou v typu
  `Secret` (vynuluje se při zahození, `zeroize`; `Debug` vypíše `***`).
  Log obsahuje jen jméno účtu, metodu a výsledek.
- Zpráva s heslem po zpracování: buffer kanálu accountd vynuluje.

## Relace

<figure class="diagram">
<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Prubeh prihlaseni">
<defs><marker id="uc2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="t b" x="90" y="24" text-anchor="middle">přihlášení (login)</text><text class="t b" x="290" y="24" text-anchor="middle">accountd</text><text class="t b" x="490" y="24" text-anchor="middle">init</text><text class="t b" x="680" y="24" text-anchor="middle">fsd</text>
<path class="ln dash" d="M90 32 L90 350"/><path class="ln dash" d="M290 32 L290 350"/><path class="ln dash" d="M490 32 L490 350"/><path class="ln dash" d="M680 32 L680 350"/>
<path class="ln" d="M90 60 L288 60" marker-end="url(#uc2)"/><text class="m" x="190" y="54" text-anchor="middle">AUTH alice, heslo</text>
<rect class="acc" x="230" y="70" width="120" height="44" rx="4"/><text class="m" x="290" y="88" text-anchor="middle">zpomalení? Argon2id,</text><text class="m" x="290" y="104" text-anchor="middle">porovnání v konst. čase</text>
<path class="ln" d="M290 130 L488 130" marker-end="url(#uc2)"/><text class="m" x="390" y="124" text-anchor="middle">OPEN_SESSION uid, skupiny, domov</text>
<path class="ln" d="M490 160 L678 160" marker-end="url(#uc2)"/><text class="m" x="585" y="154" text-anchor="middle">CONNECT_AS 1000, / -> /</text>
<path class="ln" d="M680 185 L492 185" marker-end="url(#uc2)"/><text class="m" x="585" y="179" text-anchor="middle">relace + kanál</text>
<rect class="ok" x="420" y="195" width="140" height="44" rx="4"/><text class="m" x="490" y="213" text-anchor="middle">job relace, terminál,</text><text class="m" x="490" y="229" text-anchor="middle">GUI, úlohy (identita)</text>
<path class="ln" d="M490 255 L292 255" marker-end="url(#uc2)"/><text class="m" x="390" y="249" text-anchor="middle">číslo relace</text>
<path class="ln" d="M290 280 L92 280" marker-end="url(#uc2)"/><text class="m" x="190" y="274" text-anchor="middle">OK relace (bez kanálů)</text>
<text class="m" x="10" y="318">Kanály s identitou drží init a dává je jen procesům v jobu relace. Přihlašovací obrazovka dostane</text>
<text class="m" x="10" y="336">jen číslo relace (přepne na plochu), ne kanály - nemůže se vydávat za uživatele.</text>
</svg>
<figcaption>Přihlášení: accountd rozhodne, init razí identitu.</figcaption>
</figure>

- accountd má k initu soukromý kanál (`accountproto::ToInit`). Po úspěšném
  ověření pošle `OPEN_SESSION { uid, groups, home }`. Init (správce sezení,
  `user/init/src/session.rs`):
  1. ověří, že jde o člověka (uid 1000-999 999, skupiny od
     1 000 000) - relaci systému ani servisního účtu nevyrazí, ani když
     o ni accountd požádá,
  2. vyrazí relaci FS `CONNECT_AS(uid, groups, profil.user_mounts())`,
  3. uloží si kanál self relace (dostane ho nastavení v relaci).
  - *Další krok* (s plochou a taskd): **job relace** se všemi procesy
    uživatele (termd, plocha, aplikace) s touto identitou, aplikace
    s užšími relacemi podle manifestu (`app_mounts`), kanály taskd a GUI
    se stejnou identitou a recept relace v lístku (obnova po pádu fsd).
    Terminál dnes dostává relaci FS initu (systém) - přepnutí na relaci
    přihlášeného uživatele patří ke stejnému kroku.
- **Odhlášení** (`LOGOUT`): init zahodí relaci FS a kanál self (s jobem
  relace ukončí i procesy uživatele - teprve to zavře i kanály, které si
  procesy relace samy otevřely přes `CONNECT`). Šifrovaný
  domov se zamkne (`UNLOAD_KEY`).
- **Zamčení obrazovky** (`LOCK`, nebo Ctrl+Alt+Delete -> Zamknout):
  procesy běží dál, kompozitor ukáže zamykací obrazovku v systémové vrstvě
  a vstup jde jen do ní. Odemknout jde **jen stejný účet** (`UNLOCK`
  s ověřením) nebo odhlášení. Při zamčení se klíč domova z paměti
  nemaže (procesy s domovem pracují dál).
- Více současných relací (rychlé přepnutí uživatele) protokol umí (číslo
  relace); obrazovka přepnutí přijde s plochou.
- Pád accountd: relace běží dál (drží je init). Supervisor accountd
  zatím jen hlídá; restart s převzetím relací (`LIST_SESSIONS`) je další
  krok. Pád initu = konec systému.

## Správa počítače

> **Schváleno uživatelem (2026-11-08):** potvrzení úkonu osobou s rolí;
> pětiminutový režim správy jen jako **dobrovolná volba** v dialogu.
> Zvažované varianty: A (jen potvrzení na úkon), B (A + režim správy
> automaticky), C (oddělený účet správce - dvě hesla, méně přívětivé).

Cíl: běžný uživatel nikdy neřeší „administrátora“, a přesto aplikace ani
malware v relaci nemůže měnit celý počítač.

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Potvrzeni ukonu pro cely pocitac">
<defs><marker id="uc3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="170" height="70" rx="6"/><text class="t b" x="95" y="55" text-anchor="middle">aplikace / nastavení</text><text class="m" x="95" y="74" text-anchor="middle">chce: přidat účet</text><text class="m" x="95" y="90" text-anchor="middle">„bob“</text>
<rect class="acc" x="250" y="30" width="170" height="70" rx="6"/><text class="t b" x="335" y="55" text-anchor="middle">accountd</text><text class="m" x="335" y="74" text-anchor="middle">ELEVATE úkon +</text><text class="m" x="335" y="90" text-anchor="middle">parametry, žadatel</text>
<rect class="warn" x="490" y="20" width="260" height="128" rx="6"/><text class="t b" x="620" y="44" text-anchor="middle">systémový dialog</text><text class="m" x="620" y="64" text-anchor="middle">„Nastavení chce přidat účet bob“</text><text class="m" x="620" y="82" text-anchor="middle">bezpečnostní fráze, kdo potvrzuje</text><text class="m" x="620" y="100" text-anchor="middle">ověření: heslo / PIN / klíč</text><text class="m" x="620" y="118" text-anchor="middle">osoby s rolí správy</text><text class="m" x="620" y="136" text-anchor="middle">[ ] 5 minut nepotvrzovat (volba)</text>
<path class="ln" d="M180 65 L248 65" marker-end="url(#uc3)"/>
<path class="ln" d="M420 65 L488 65" marker-end="url(#uc3)"/>
<path class="ln" d="M620 148 L620 170 L337 170 L337 102" marker-end="url(#uc3)"/><text class="m" x="480" y="164" text-anchor="middle">potvrzeno + ověřeno</text>
<rect class="ok" x="10" y="190" width="420" height="44" rx="6"/><text class="m" x="220" y="210" text-anchor="middle">capability jen na tento úkon s těmito parametry,</text><text class="m" x="220" y="226" text-anchor="middle">platí 60 s a jen jednou (nebo režim správy 5 min)</text>
<path class="ln" d="M300 172 L250 188" marker-end="url(#uc3)"/>
</svg>
<figcaption>Úkon pro celý počítač potvrdí člověk s rolí v dialogu, který aplikace nepodvrhne.</figcaption>
</figure>

- **Žádný root ani admin účet.** Uživatel 0 zůstává jen identitou řídicí
  relace initu.
- **Pro sebe** bez potvrzení: instalace aplikace pro sebe podle manifestu,
  změna vlastního hesla, PINu, klíče, fráze, šifrování domova.
- **Pro celý počítač** (účty jiných, ovladače, aktualizace systému, disky,
  globální firewall, čas systému): žadatel pošle `ELEVATE { úkon,
  parametry }`. accountd požádá kompozitor o **systémový dialog**
  (capability `dialog` má jen accountd a zprostředkovatel oprávnění):
  ukáže úkon se skutečnými parametry, žadatele (aplikace z manifestu)
  a bezpečnostní frázi. Potvrdí ho **osoba s rolí** stejnou metodou jako
  při přihlášení. Má-li roli sám přihlášený, potvrdí sebe; jinak vybere
  jiný účet s rolí („přes rameno“).
- Výsledek je **capability jen na ten úkon s těmi parametry**, platí 60 s
  a jen jednou. Služba úkonu (accountd, devmgr, fwd, ...) ji ověří
  u accountd (`CHECK_GRANT`) a spotřebuje.
- **Režim správy (volba):** v dialogu je zaškrtávátko „Další úkony správy
  z této relace 5 minut nepotvrzovat“, **výchozí vypnuto**. Zaškrtne ho
  jen člověk sám (aplikace ho předvolit nemůže). Po dobu režimu dostane
  relace na `ELEVATE` capability bez dialogu; lišta ukazuje odpočet
  a tlačítko „Ukončit režim správy“. Režim končí i zamčením obrazovky
  a odhlášením. ⚠️ Riziko: proces v relaci (i malware) může v těch
  5 minutách provést libovolný úkon správy bez dalšího dialogu.
  `ELEVATE` z umělého zdroje (agent AI) režim správy **nepoužije**,
  vždy ukáže dialog.
- První účet z instalace má roli; může ji dát jiným a odebrat (ne
  poslední). Do auditu (`/db/audit`) se zapíše každý úkon: kdo žádal,
  kdo potvrdil, co, zda v režimu správy.

| Úkon (`ELEVATE`) | Parametry |
|---|---|
| 1 `ACCOUNT_CREATE` | jméno, zobrazované jméno, role |
| 2 `ACCOUNT_DELETE` | uid |
| 3 `ACCOUNT_RESET_PASSWORD` | uid (šifrovaný domov se tím neotevře) |
| 4 `ROLE_SET` | uid, zapnout |
| 5-15 | ovladače, aktualizace, disky, firewall, čas - doplní jejich služby |

Proč tak: potvrzuje se konkrétní úkon, který člověk vidí (stejný princip
jako [API akcí](gui.md#api-akcí)); kdo dělá delší správu, si dialogy na
chvíli vypne sám a ví o tom.

## Šifrování domova

Volba uživatele u jeho účtu (výchozí: vypnuto, rozhodne se při založení
účtu nebo kdykoli v nastavení). Samotné šifrování datasetu je **samostatný
krok FS**; accountd připraví klíče a rozhraní.

<figure class="diagram">
<svg viewBox="0 0 760 280" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Obaly klice domova">
<defs><marker id="uc4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="40" rx="6"/><text class="m" x="110" y="45" text-anchor="middle">heslo -> Argon2id (32 B)</text>
<rect class="box" x="10" y="75" width="200" height="40" rx="6"/><text class="m" x="110" y="100" text-anchor="middle">PIN -> klíč zařízení / TPM</text>
<rect class="box" x="10" y="130" width="200" height="40" rx="6"/><text class="m" x="110" y="155" text-anchor="middle">klíč FIDO2 -> hmac-secret</text>
<rect class="warn" x="10" y="185" width="200" height="40" rx="6"/><text class="m" x="110" y="210" text-anchor="middle">obnovovací klíč (160 bitů)</text>
<rect class="acc" x="290" y="20" width="190" height="205" rx="6"/><text class="t b" x="385" y="45" text-anchor="middle">obaly v DB</text><text class="m" x="385" y="70" text-anchor="middle">KEK = HKDF-SHA256(</text><text class="m" x="385" y="86" text-anchor="middle">tajemství metody, uid)</text><text class="m" x="385" y="112" text-anchor="middle">obal = ChaCha20-Poly1305(</text><text class="m" x="385" y="128" text-anchor="middle">KEK, klíč domova,</text><text class="m" x="385" y="144" text-anchor="middle">AAD = uid, metoda)</text><text class="m" x="385" y="175" text-anchor="middle">jeden obal na metodu</text>
<rect class="ok" x="560" y="80" width="190" height="80" rx="6"/><text class="t b" x="655" y="105" text-anchor="middle">klíč domova</text><text class="m" x="655" y="124" text-anchor="middle">256 bitů, náhodný</text><text class="m" x="655" y="140" text-anchor="middle">-> fsd LOAD_KEY</text>
<path class="ln" d="M210 40 L288 60" marker-end="url(#uc4)"/><path class="ln" d="M210 95 L288 100" marker-end="url(#uc4)"/><path class="ln" d="M210 150 L288 140" marker-end="url(#uc4)"/><path class="ln" d="M210 205 L288 180" marker-end="url(#uc4)"/>
<path class="ln" d="M480 120 L558 120" marker-end="url(#uc4)"/><text class="m" x="519" y="112" text-anchor="middle">rozbalí</text>
<text class="m" x="10" y="260">Změna hesla přebalí jen jeden obal, data domova se nepřešifrovávají.</text>
</svg>
<figcaption>Klíč domova je zabalený zvlášť pro každou metodu a pro obnovovací klíč.</figcaption>
</figure>

- Při zapnutí accountd vygeneruje **klíč domova** (256 bitů, systémový
  CSPRNG) a **obnovovací klíč** (160 bitů), který uživatel uvidí **jen
  jednou** jako 7 skupin po 5 znacích (32 znaků Crockford base32 + 3
  kontrolní, např. `7KQ2M-...`; velikost písmen, mezery a záměna O/0,
  I/L/1 nevadí, překlep se pozná) a potvrdí, že si ho uložil (opíše 2
  skupiny).
- Obal pro každou metodu: heslo (druhá polovina výstupu Argon2id),
  bezpečnostní klíč (výstup `hmac-secret`), PIN (`DeviceKey::mac` nad
  druhou polovinou výstupu Argon2id z PINu; bez TPM s varováním, viz
  [PIN](#pin-vázaný-na-zařízení)), obnovovací klíč. Při zapnutí vzniknou
  obaly pro heslo a obnovovací klíč; PIN a bezpečnostní klíče dostanou
  obal při dalším nastavení (nastavení je po zapnutí nabídne znovu).
  Metoda bez obalu odemkne jen domov, který už je odemčený jinou relací.
- Změna hesla = nový obal. Zapomenuté heslo = přihlášení obnovovacím
  klíčem a nastavení nového hesla.
- **Správa nevidí cizí data:** role správy může účet smazat nebo mu
  nastavit nové heslo, ale u šifrovaného domova tím **nezíská** klíč
  domova (nové heslo dostane obal až po odemčení obnovovacím klíčem
  majitele).

⚠️ **Riziko ztráty dat:** kdo zapomene heslo, ztratí klíč a **nemá
obnovovací klíč, přijde o data domova natrvalo**. Nikdo (ani správa, ani
výrobce) je neobnoví - to je smysl šifrování. Obrazovka zapnutí to musí
říct výslovně a obnovovací klíč nechat opsat; nastavení upozorní, když
účet s šifrováním má jen jednu metodu.

**Co je potřeba od FS** (návrh rozhraní pro krok „šifrování datasetu“):

| Operace (fsproto) | Kdo | Význam |
|---|---|---|
| `create_dataset(path, enc)` | řídicí relace (init) | dataset se šifrováním; FS si sám vygeneruje klíč dat (DEK) a uloží ho zabalený klíčem z `LOAD_KEY` |
| `LOAD_KEY { dataset, key }` | řídicí relace | odemkne dataset klíčem domova (FS z něj odvodí klíč obalu DEK); špatný klíč = `ACCESS_DENIED` |
| `UNLOAD_KEY { dataset }` | řídicí relace | zamkne; otevřené handly vrátí `LOCKED`, klíče FS vynuluje |
| `KEY_STATUS { dataset }` | kdokoli s viditelností | `none` / `locked` / `unlocked` |
| zamčený dataset | - | jména i obsah nečitelné, `stat` kořene vrátí `LOCKED`; snapshoty a replikace jdou dál nad šifrovanými bloky |

Klíč domova jde do FS přes init (accountd řídicí relaci nemá): accountd
pošle initu `UNLOCK_HOME { uid, key }` v kanálu, init ho předá fsd
a buffer vynuluje. Než FS šifrování umí, vrací accountd na zapnutí
`NOT_SUPPORTED` a obaly se nevytvářejí (rozhraní a testy obalů ano).

## Protokol `accountproto`

Zpráva = kód operace `u32` + pole (`libs/wire`, little-endian), jako
ostatní služby. Neplatná zpráva = `INVALID_ARGS`, nikdy panika. Tajemství
(heslo, PIN) se v protokolu posílají v poli `secret` a obě strany je po
použití vynulují. Kanály jsou různé capability:

| Kanál | Kdo ho má | Co smí |
|---|---|---|
| `login` | přihlašovací obrazovka (systémová vrstva), testy initu | výpis účtů pro výběr, ověření, odemčení |
| `self` | relace uživatele (nastavení) | vlastní účet: metody, fráze, šifrování, zamčení, odhlášení, žádost o úkon správy |
| `setup` | instalátor, jen dokud neexistuje žádný účet | založení prvního účtu s rolí |
| `init` | init (soukromý) | relace, domovy, klíče domovů |

| Op | Kanál | Jméno | Pole | Odpověď |
|---|---|---|---|---|
| 1 | login | `LIST` | - | účty: uid, jméno, zobrazované, metody (bity), příznaky |
| 2 | login | `PHRASE` | uid | fráze (ukáže se po výběru účtu) |
| 3 | login | `AUTH` | uid, metoda u8, secret | `OK` relace u64 / `DENIED` čekání ms, zbývá PIN / `LOCKED_OUT` ms |
| 4 | login | `UNLOCK` | relace, metoda, secret | jako `AUTH` |
| 10 | self | `WHOAMI` | - | záznam účtu bez tajemství |
| 11 | self | `SET_PASSWORD` | staré secret, nové secret | - |
| 12 | self | `SET_PIN` | heslo secret, PIN secret (prázdný = smazat) | - |
| 13 | self | `ENROLL_KEY` | heslo secret, PIN klíče secret, název | id klíče |
| 14 | self | `REMOVE_KEY` | heslo secret, id | - |
| 15 | self | `SET_PHRASE` | text | - |
| 16 | self | `ENCRYPT_HOME` | heslo secret | obnovovací klíč (jen jednou) / `NOT_SUPPORTED` |
| 17 | self | `LOCK` | - | - |
| 18 | self | `LOGOUT` | - | - |
| 19 | self | `ELEVATE` | úkon u32, parametry | capability (viz [Správa](#správa-počítače)); do dialogu kompozitoru `NOT_SUPPORTED` |
| 20 | self | `SET_KEY_PIN` | PIN secret | - (klíč, který PIN ještě nemá) |
| 30 | setup | `CREATE_FIRST` | jméno, zobrazované, heslo secret | uid |
| 50 | init | `OPEN_SESSION` (accountd -> init, + handle kanálu self) | uid, skupiny, domov | relace u64 |
| 51 | init | `CLOSE_SESSION` | relace | - |
| 52, 53 | init | `LOCK_SESSION`, `UNLOCK_SESSION` | relace | - |
| 54, 55 | init | `CREATE_HOME`, `DELETE_HOME` | uid, jméno | - |
| 56 | init | `LIST_SESSIONS` | - | relace: číslo, uid, zamčená |
| 57 | init | `UNLOCK_HOME` | uid, klíč domova secret | - (dnes `NOT_SUPPORTED`) |
| 60 | init | `SESSION_CLOSED` (init -> accountd, událost) | relace | - |

Metody (`u8`): 1 heslo, 2 PIN, 3 bezpečnostní klíč (secret = PIN
klíče, prázdný = jen dotek), 4 vývojové přihlášení, 5 obnovovací klíč.
Odpověď na kanálu init je stav `i32` (0 nebo záporný kód chyby);
událost initu začíná kladným kódem operace, takže je accountd rozliší.
Úkony správy (`ELEVATE`) viz
[Správa počítače](#správa-počítače).

Klient (`accountproto::client`, funkce `client`):

```rust
use accountproto::client::{Login, SelfService, session_of};
use accountproto::{AuthReply, Method, Secret};

let login = Login(channel);
for a in login.list()? {
    println!("{} ({})", a.display, a.name);
}
match login.auth(1000, Method::Password, Secret::from("heslo"))? {
    AuthReply::Ok { session } => println!("relace {session}"),
    AuthReply::Denied { retry_after_ms, .. } => println!("další pokus za {} s", retry_after_ms / 1000),
    AuthReply::LockedOut { retry_after_ms } => println!("počkej {} s", retry_after_ms / 1000),
}
```

## Agent AI a identita

Pracovní prostor agenta (vlákno agentů, `user/agentd`) dnes běží jako
uživatel 1000 napevno. S účty: agent dostane relaci **přihlášeného
uživatele** zúženou na pracovní prostor (`NARROW` lístku), identita = uid
uživatele, v auditu a přehledu úloh označená jako agent. Vlastní číslo
uživatele agent nedostane (jeho soubory jsou soubory uživatele). Úkony
správy agent nikdy nepotvrdí sám - `ELEVATE` z umělého zdroje vždy ukáže
dialog člověku. Domluva přes koordinátora.

## Testy

**Na hostiteli** (`cargo xtask test` spouští i testy knihoven):

- `accountproto` (22 testů): kódování všech zpráv tam a zpět, každé
  zkrácení je chyba, neplatné zprávy, CBOR (vektory RFC 8949, kanonické
  pořadí klíčů CTAP2, odmítnutí neurčitých délek, tagů, duplicitních
  klíčů a hlubokého vnoření), rozdělení a složení CTAPHID, protokoly PIN 1
  a 2, CTAP2 nad simulovaným klíčem (registrace a přihlášení s PINem klíče
  i bez něj, `hmac-secret`, blokace po 8 špatných PINech, klíč s PINem
  vyžaduje token, bez doteku, neznámý klíč, podpis jiným klíčem,
  `excludeList`, EdDSA, vše i přes CTAPHID).
- `accounts` (22 testů): Argon2id podle vektoru RFC 9106 (kap. 5.3)
  a vektoru referenční implementace, obaly (špatné tajemství, jiný
  uživatel, poškozený obal), obnovovací klíč (formát, volný zápis,
  poznání překlepu), formát záznamu a poškozené soubory, tabulka
  zpomalení, počítadlo uložené před ověřením (selhání zápisu = žádné
  ověření), heslo, normalizace NFC, PIN s klíčem zařízení (jiný klíč
  zařízení neprojde), blokace PINu a odblokování heslem, změna hesla,
  přepočet slabších parametrů, vývojové přihlášení jen s povolením,
  bezpečnostní klíč s PINem i bez něj, klon klíče (počítadlo neroste),
  šifrovaný domov rozbalený každou metodou i obnovovacím klíčem,
  poškozený záznam neshodí ostatní, čísla smazaných účtů se nepoužijí
  znovu.

**V QEMU** (sada `ucty`, `user/init/src/tests/accounts.rs`, x86_64
i aarch64):

| Test | Ověří |
|---|---|
| accountd běží | `status`, dataset DB patří accountd |
| založení prvního účtu | `CREATE_FIRST` (krátké heslo odmítnuto, druhý „první“ účet odmítnut), role, domov `/home/alice` je dataset s vlastníkem uid, `PROTECTED` bez grantů, `NOEXEC` |
| přihlášení heslem | `AUTH` -> relace, FS relace zapisuje do domova, `WHOAMI`, fráze, `ELEVATE` a `ENCRYPT_HOME` `NOT_SUPPORTED`, odhlášení |
| přihlášení PINem | `SET_PIN` (špatné heslo odmítnuto), příznak `PIN_WEAK`, špatný PIN se zbývajícími pokusy, přihlášení PINem |
| relace vidí jen svůj domov | cizí domov i soubory v něm `NOT_FOUND`, `/home` nejde vypsat, DB účtů `NOT_FOUND` |
| init nevyrazí systémovou identitu | `OPEN_SESSION` pro 0, 902, 999 a skupinu 0 odmítnut, domov pro systém ne |
| zamčení a odhlášení | zamčení, odemčení špatným a správným heslem, po odhlášení relace i její FS zmizí |
| špatné heslo a zpomalení | neznámý účet, 3. chyba = 30 s, v čekání neprojde ani správné heslo |

## Rizika a omezení

- ⚠️ **PIN bez TPM** chrání jen proti hádání u klávesnice, ne proti
  krádeži disku; obal klíče domova z PINu je bez TPM stejně slabý. Je
  povolený s varováním (rozhodnutí uživatele, viz
  [PIN](#pin-vázaný-na-zařízení)).
- ⚠️ **Režim správy** (volba, 5 minut): úkony správy z relace bez dialogu
  - i pro malware v relaci. Výchozí vypnuto.
- ⚠️ **Šifrování domova bez obnovovacího klíče** = možná trvalá ztráta
  dat (viz [Šifrování](#šifrování-domova)). Dokud FS šifrování neumí,
  domov **šifrovaný není** a disk jde přečíst jinde (`osfs` běží jako
  systém).
- ⚠️ Klíč zařízení (`/db/device.key`) i DB chrání jen oprávnění FS:
  proti procesům v systému ano, proti spuštění jiného systému z USB ne.
- Bezpečnostní klíč **bez PINu klíče**: kdo ukradne klíč a zná jméno
  účtu, přihlásí se. Nastavení proto PIN klíče nabídne a bez něj varuje.
- Přihlašovací obrazovka a dialog správy jsou jen tak bezpečné jako
  systémová vrstva kompozitoru (aplikace do ní nekreslí, vstup jde jen do
  ní) a bezpečná zkratka `inputd`.
- Dokud není job relace, odhlášení zavře kanály relace u initu, ale ne
  relace FS, které si proces sám otevřel přes `CONNECT`.

## Stav zavedení

| Část | Stav |
|---|---|
| návrh (tato stránka) | hotovo |
| model správy | schváleno; role, první účet s rolí a ochrana poslední role hotové (`libs/accounts`), `ELEVATE` s dialogem po přihlašovací obrazovce (vlákno GUI) |
| `libs/accountproto` (zprávy, klient, CTAP2: CBOR, CTAPHID, COSE, protokoly PIN 1 a 2, simulovaný klíč) | hotovo, testy na hostiteli |
| `libs/accounts` (záznamy, Argon2id, PIN s klíčem zařízení, zpomalení, klíče, obaly, obnovovací klíč) | hotovo, testy na hostiteli |
| `user/accountd` (DB na FS, kanály login/self/setup, vývojové přihlášení) | hotovo, testy v QEMU |
| správce sezení v initu (domovy, relace FS, zamčení, odhlášení) | hotovo; job relace s procesy uživatele a přepnutí terminálu na identitu uživatele = další krok |
| přenos CTAPHID v `usb-hid` (usage page `0xF1D0`), devmgr a init (`fido` -> `key`) | hotovo; QEMU klíč FIDO2 neemuluje, na skutečném klíči zatím neověřeno |
| šifrování domova | obaly a obnovovací klíč hotové; šifrování datasetu = krok FS (do té doby `NOT_SUPPORTED`) |
| přihlašovací obrazovka, nastavení účtu | vlákno GUI |
