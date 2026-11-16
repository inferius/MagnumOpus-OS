# Vzdálený přístup (SSH)

Přihlášení k počítači po síti klientem SSH (OpenSSH, Windows `ssh.exe`,
PuTTY): shell v terminálu, jednotlivé příkazy (`ssh pc ls`), přenos
souborů `scp` a `sftp`. Server je ve výchozím stavu **vypnutý**;
zapíná ho správce pro celý počítač a každý účet si vzdálený přístup
povoluje sám.

| Část | Kde | Co dělá |
|---|---|---|
| protokol | `libs/ssh` | převzatý sunset 0.6.0 (bez I/O), `server.rs` (spojení, kanály, PTY), `sftp.rs` (SFTP v3), `keys.rs` (klíče, otisk, ověření podpisu) |
| služba | `user/sshd` (`sshd`) | jen naslouchá na portu 22, pro každé spojení spustí proces `sshd-conn` |
| spojení | `user/sshd` (`sshd-conn`) | šifrování, přihlášení, kanály; terminál přes `termd`, soubory přes relaci FS účtu |
| klíče a ověření | `libs/accounts/src/remote.rs`, `user/accountd/src/remote.rs` | klíče SSH v záznamu účtu, ověření podpisu, zpomalení pokusů |
| relace | `user/init/src/remote.rs`, `user/init/src/sshd.rs` | vzdálená relace účtu, jednorázový lístek, zapnutí služby, hostitelský klíč |
| příkazy | `libs/shlang/src/eval/builtins/remote.rs` | `sshd`, `ssh-key`, `remote-access` |
| lišta | `libs/comp` (`remote_label`) | pilulka „Vzdáleně: uživatel@adresa“ |
| testy | `libs/ssh/src/tests*.rs`, `user/init/src/tests/ssh.rs`, `xtask/src/sshtest.rs` | hostitel: server proti klientovi sunset, fuzz; QEMU: klient OpenSSH z hostitele |

> ⚠ **Rizika a omezení v1 (čtěte před zapnutím):**
> - **Přihlášení jen klíčem.** Heslo samotné server nikdy nepřijme;
>   heslo s druhým faktorem (TOTP) je **[navrženo]**. Kdo ztratí
>   soukromý klíč, musí klíč odebrat u počítače (`ssh-key remove`).
> - **Hostitelský klíč drží proces spojení.** `sshd-conn` dostane seed
>   hostitelského klíče, aby mohl podepsat výměnu klíčů. Chyba
>   v parseru (běží před přihlášením) by ho mohla vyzradit. Proces je
>   izolovaný (vlastní job, 24 MiB, bez sítě ven, bez FS do přihlášení),
>   ale podpis přes samostatnou službu je dluh **[navrženo]**.
> - **Kryptografie RustCrypto dvakrát.** sunset používá ed25519-dalek 3,
>   curve25519-dalek 5, sha2 0.11 a signature 3, TLS starší řady. Obě
>   verze jsou v binárkách současně: `accountd` (release, načítaná
>   velikost) narostl z 685 852 na 807 839 B (+122 KB, +18 %). Sjednocení
>   čeká na rustls s novou řadou RustCrypto; zatím dluh.
> - **Parser fuzzovaný jen bez libFuzzeru.** Každý `cargo test` pošle
>   serveru tisíce náhodných a zmutovaných vstupů (`tests_fuzz.rs`);
>   cíle pro cargo-fuzz (`libs/ssh/fuzz`) jsou připravené, ale zatím
>   nespuštěné (potřebují Linux nebo WSL s nightly).
> - **Firewall v profilu „veřejná síť“ příchozí spojení zahodí.** SSH
>   funguje jen v profilu „domácí“ nebo s výjimkou `allow_in`
>   ([Firewall](firewall.md#profily-sítí)).

## Architektura

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Součásti vzdáleného přístupu">
<defs><marker id="sh1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="sh1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="140" height="70" rx="6"/><text class="t b" x="80" y="48" text-anchor="middle">klient SSH</text><text class="m" x="80" y="68" text-anchor="middle">ssh, scp, sftp</text>
<rect class="box" x="200" y="20" width="150" height="70" rx="6"/><text class="t b" x="275" y="45" text-anchor="middle">sshd</text><text class="m" x="275" y="64" text-anchor="middle">port 22, jen naslouchá</text><text class="m" x="275" y="80" text-anchor="middle">max 8 spojení</text>
<rect class="acc" x="200" y="130" width="150" height="90" rx="6"/><text class="t b" x="275" y="155" text-anchor="middle">sshd-conn</text><text class="m" x="275" y="174" text-anchor="middle">proces na spojení</text><text class="m" x="275" y="192" text-anchor="middle">job 24 MiB</text><text class="m" x="275" y="208" text-anchor="middle">šifrování, kanály</text>
<rect class="box" x="420" y="20" width="160" height="80" rx="6"/><text class="t b" x="500" y="45" text-anchor="middle">accountd</text><text class="m" x="500" y="64" text-anchor="middle">klíče účtu, podpis</text><text class="m" x="500" y="82" text-anchor="middle">zpomalení pokusů</text>
<rect class="box" x="620" y="20" width="130" height="80" rx="6"/><text class="t b" x="685" y="45" text-anchor="middle">init</text><text class="m" x="685" y="64" text-anchor="middle">vzdálená relace</text><text class="m" x="685" y="82" text-anchor="middle">lístek</text>
<rect class="ok" x="420" y="150" width="160" height="70" rx="6"/><text class="t b" x="500" y="175" text-anchor="middle">termd + shsess</text><text class="m" x="500" y="195" text-anchor="middle">shell relace účtu</text>
<rect class="ok" x="620" y="150" width="130" height="70" rx="6"/><text class="t b" x="685" y="175" text-anchor="middle">fsd</text><text class="m" x="685" y="195" text-anchor="middle">relace FS účtu</text>
<path class="ln" d="M150 55 L198 55" marker-end="url(#sh1)"/>
<path class="ln" d="M275 90 L275 128" marker-end="url(#sh1)"/><text class="m" x="282" y="114">spustí</text>
<path class="ln-acc" d="M350 150 L418 80" marker-end="url(#sh1a)"/><text class="m" x="356" y="112">podpis</text>
<path class="ln" d="M580 60 L618 60" marker-end="url(#sh1)"/>
<path class="ln" d="M350 190 L418 190" marker-end="url(#sh1)"/>
<path class="ln" d="M350 210 L618 210" marker-end="url(#sh1)"/>
<text class="m" x="10" y="260">accountd ověří podpis a požádá init o relaci; init vrátí jednorázový lístek.</text>
<text class="m" x="10" y="280">sshd si relaci lístkem vyzvedne a předá procesu spojení jen kanál okna termd</text>
<text class="m" x="10" y="300">a relaci FS účtu. Bez kompozitoru, schránky, zvuku a bez ELEVATE.</text>
</svg>
<figcaption>Rozdělení oprávnění: kdo naslouchá, kdo mluví s klientem a kdo rozhoduje o přihlášení.</figcaption>
</figure>

- **`sshd`** jen naslouchá (síťová relace: `listen` na 22, žádná spojení
  ven) a každé přijaté spojení dá novému procesu `sshd-conn` ve vlastním
  jobu (24 MiB). Nejvýš 8 spojení; nepřihlášené spojení se zavře po
  120 s **[implementováno]**.
- **`sshd-conn`** mluví s klientem. Podpis přihlášení neověřuje sám:
  pošle ho přes `sshd` službě `accountd`, která má klíče účtu
  **[ověřeno v QEMU]**.
- **`accountd`** ověří podpis klíčem z účtu, zkontroluje, že účet má
  vzdálený přístup povolený, a odmítne opakované použití stejného
  identifikátoru relace SSH. Pak požádá init o vzdálenou relaci.
- **init** založí relaci (vlastní job, relace FS s identitou účtu)
  a vrátí jednorázový náhodný lístek; `sshd` si jím relaci vyzvedne.
  Šifrovaný domov, který není odemčený, vrátí `Locked` - nejdřív je
  potřeba přihlášení u počítače.

### Vzdálená relace

Vzdálená relace je **oddělená** od místních (`init/src/remote.rs`):
nemá plochu, kompozitor, schránku ani zvuk, nevidí místní obrazovku
a nepočítá se jako „aktivní relace“ pro aplikace z lišty. Úkony správy
(`ELEVATE`) z ní nejdou - nemá kanál pro potvrzení u počítače. Pro v1
je to záměr: správa systému na dálku potřebuje vlastní návrh potvrzení
**[navrženo]**.

Kanály:

| Kanál | Co | Stav |
|---|---|---|
| `shell` s PTY | shell v relaci termd/shsess; velikost okna, Ctrl+C, popředí a pozadí jako v okně terminálu | **[ověřeno v QEMU]** |
| `exec` | jeden příkaz shellu, výstup a návratový kód | **[ověřeno v QEMU]** |
| subsystém `sftp` | SFTP v3 nad relací FS účtu (i `scp`, který v OpenSSH 9 používá SFTP) | **[ověřeno v QEMU]** |
| přesměrování portů, X11, agent | neimplementováno, server je odmítne | - |

## Zapnutí

1. Správce zapne službu: `sshd enable` (úkon správy, potvrzení
   `ELEVATE`). Dialog ukáže otisk hostitelského klíče - ten si
   poznamenejte a porovnejte při prvním připojení klienta.
2. Uživatel si povolí přístup pro svůj účet: `remote-access on`
   (zeptá se na heslo).
3. Uživatel přidá veřejný klíč: `ssh-key add ~/id_ed25519.pub` nebo
   `ssh-key add -` (klíč na vstupu, třeba z flashky). Otisk se ukáže
   při přidání.

| Příkaz | Co dělá |
|---|---|
| `sshd` / `sshd status` | stav služby, otisk hostitelského klíče, počet spojení |
| `sshd enable` / `sshd disable` | zapne nebo vypne službu (úkon správy); stav přežije restart |
| `ssh-key list` | klíče účtu: číslo, typ, otisk, komentář |
| `ssh-key add <soubor\|->` | přidá klíč (heslo); soukromý klíč odmítne |
| `ssh-key remove <číslo>` | odebere klíč |
| `remote-access` / `remote-access on\|off` | stav nebo povolení vzdáleného přístupu účtu (zapnutí chce heslo) |

Hostitelský klíč (ed25519) vznikne při prvním startu z CSPRNG
a je v `/system/config/ssh` (jen system) **[ověřeno v QEMU]**.

Podporované klíče: `ssh-ed25519` a `sk-ssh-ed25519@openssh.com` (FIDO
klíč u klienta, server kontroluje příznak přítomnosti uživatele).
RSA a ECDSA zatím ne **[implementováno]**.

## Ochrana proti hádání

- Zpomalení po neúspěšných pokusech zvlášť podle **adresy** a podle
  **jména účtu**, s narůstající prodlevou (tabulka
  v [Účty](ucty.md); stejná funkce jako u místního přihlášení, ale
  vlastní počítadla). Platí delší z obou prodlev **[implementováno]**
  (test `cargo test -p accounts remote`).
- Počítadla jsou jen v paměti `accountd` (nejvýš 4096 adres a jmen,
  nejstarší vypadne).
- Nejvýš 6 pokusů o přihlášení v jednom spojení, pak se spojení zavře.
- Důvod odmítnutí klient nezjistí (neexistující účet, cizí klíč
  i vypnutý přístup vypadají stejně).

## Audit

Do logu (`logd`) jde každé přihlášení i odmítnutí: kdy, odkud
(adresa), účet, otisk klíče; konec relace s délkou. Obsah relace
(příkazy, data) se nezapisuje **[implementováno]**.

Dokud je připojená nějaká vzdálená relace, ukazuje lišta pilulku
**„Vzdáleně: uživatel@adresa“** (u více relací „+N“) v barvě
upozornění **[implementováno]** (test `cargo test -p comp remote`).

## Připojení z Windows (QEMU)

```bash
cargo xtask run --ssh
```

přesměruje port 2222 hostitele na port 22 hosta. V hostu:
`sshd enable`, `remote-access on`, `ssh-key add -` (vložit obsah
`id_ed25519.pub`). Pak na Windows:

```bash
ssh -p 2222 uzivatel@127.0.0.1
```

```bash
scp -P 2222 soubor.txt uzivatel@127.0.0.1:
```

Klíč vytvoří `ssh-keygen -t ed25519` (OpenSSH je ve Windows 10/11
součástí systému). Firewall hosta musí být v profilu „domácí“.

## Klient

Příkazy `ssh`, `scp` a `sftp` (program `user/sshtools`, jeden program
podle jména, každý s vlastním manifestem) spouští shell službou exec
initu ([Návrh shellu](navrh-shell.md)) **[ověřeno v QEMU]** (test `ssh:
klient v systemu ...`, klient proti našemu sshd přes loopback).

| Příkaz | Co dělá |
|---|---|
| `ssh [--port N] [uživatel@]server [příkaz...]` | příkaz na serveru (výstup, kód); bez příkazu shell serveru po řádcích (surový terminál zatím ne) |
| `scp [--port N] soubor server:[cesta]` / `scp server:cesta [soubor]` | jeden soubor tam nebo zpět, jen přes SFTP (jako OpenSSH 9, starý protokol scp ne) |
| `sftp [--port N] [uživatel@]server` | příkazy ze vstupu: `ls`, `cd`, `pwd`, `get`, `put`, `rm`, `mkdir`, `quit` |
| `ssh-keygen [komentář]` | nový klíč klienta přímo ve službě účtů; vypíše veřejný klíč pro `authorized_keys` serveru |
| `ssh-key identities` / `import <soubor\|->` / `forget <číslo>` | klíče klienta: výpis, převzetí soukromého klíče OpenSSH (jen nešifrovaný ed25519), odebrání |

**Klíče drží služba účtů** **[ověřeno v QEMU]**:

- Soukromý klíč klienta (seed ed25519) vznikne nebo se naimportuje do
  služby účtů a je obalený heslem účtu (`accounts::client`). Žádný program
  ho nedostane. `ssh` má od initu jen **kanál podpisu**
  (`accountproto::agent`) a služba podepíše jen zprávu přihlášení SSH
  (RFC 4252) s blobem právě toho klíče.
- Kanál podpisu dostane jen systémový program z initfs (`ssh`, `scp`,
  `sftp`): init ho spouští s příznakem launchd `INITFS_ONLY`, takže
  program z `/system/bin` ani aplikace z disku si jméno nevezme, a launchd
  roli `Agent` bez tohoto příznaku odmítne (test ověří obojí).
- **Odemčení:** přihlášení heslem u počítače klíče relace rovnou odemkne
  (jako ssh-agent). Jinak (relace přes SSH, klíč přidaný později) se
  služba zeptá **systémovou výzvou** se štítkem **Účty**: zobrazí ji
  systémová část shellu na žádost služby účtů a heslo jde přímo službě,
  program `ssh` heslo účtu nikdy nevidí. Odznak výzvy program svým
  výstupem nenakreslí.
- **Potvrzení klíče pro nový server** je druhá systémová výzva (při
  prvním použití klíče k serveru); služba si server zapamatuje.
- Zamčení obrazovky klíče nezahodí, zahodí je až konec relace. Volba
  „zamknout klíče se zámkem obrazovky“ v nastavení účtu **[navrženo]**.
- Každý podpis, odemčení a odmítnutí jde do logu (logd).
- Změna hesla klíče přebalí; nové heslo od správy nebo obnovovací klíč je
  rozbalit neumí, klíče klienta pak zaniknou (bez starého hesla nejdou
  otevřít). Odemčení klíčem FIDO **[navrženo]** (dnes jen heslem).

**Ověření serveru:** `~/.ssh/known_hosts` ve formátu OpenSSH. Neznámý
server = dotaz s otiskem ve vstupu programu (`yes` uloží), změněný klíč
= spojení odmítnuto s varováním **[ověřeno v QEMU]**.

**Heslo k cizímu serveru** (jen když ho server nabízí) jde do skryté
výzvy programu: `ssh` vypne ECHO přes řídicí kanál terminálu (`ttyproto`)
a shell výzvu zamaskuje, do bloku ani logu nejde **[implementováno]**.

Omezení v1: shell serveru bez příkazu běží po řádcích (surový terminál
pro programy **[navrženo]**), šifrovaný soukromý klíč OpenSSH nejde
importovat (bcrypt-pbkdf), spolupráce klienta se serverem OpenSSH není
automaticky ověřená (náš sshd přes loopback; OpenSSH server ve WSL je
otázka pro uživatele).

## Testy

| Test | Co ověřuje |
|---|---|
| `cargo test -p ssh` | server proti klientovi sunset (exec, PTY, cizí klíč, heslo odmítnuto, velký přenos), SFTP, klíče (vektory OpenSSH 9.7), fuzz |
| `cargo test -p accounts remote` | klíče v účtu, ověření podpisu, zpomalení, opakovaný identifikátor relace |
| `cargo test -p ssh` (klient) | klient proti našemu serveru (exec, výstup, kód, EOF, cizí klíč serveru, špatný klíč), klient SFTP, soukromý klíč OpenSSH, fuzz klienta |
| `cargo test -p accounts client` | klíče klienta: vytvoření, import, odemčení, změna hesla, reset, potvrzené servery |
| `cargo xtask test --only ssh` (klient) | `ssh`, `scp`, `sftp` v systému proti našemu sshd: known_hosts, výzva Účty, scp tam a zpět, sftp, podvrh v `/system/bin`, změněný klíč serveru, role Agent bez `INITFS_ONLY` **[ověřeno v QEMU]** |
| `cargo xtask test --only ssh` | klient OpenSSH z hostitele: přihlášení klíčem, příkaz a kód, PTY, cizí klíč a heslo odmítnuty, scp tam a zpět (70 KB), sftp, zápis a čtení souboru shellem, vypnutý server odmítne spojení **[ověřeno v QEMU]** |

Odchylka od zadání: test z hostitele používá klienta OpenSSH (systémový
`ssh.exe`), ne knihovnu russh - russh by do xtask přinesl tokio
a ring/aws-lc, OpenSSH ověří skutečnou spolupráci.

## Plán

- interaktivní `ssh` se surovým terminálem pro programy **[navrženo]**
  (`ping`, `nc` a `resolve` jsou hotové, [Síť](sit.md#síťové-příkazy-shellu)),
- heslo s druhým faktorem (TOTP přes accountd) **[navrženo]**,
- podpis výměny klíčů v samostatné službě (hostitelský klíč mimo proces
  spojení) **[navrženo]**,
- potvrzení úkonů správy ze vzdálené relace **[navrženo]**,
- běh cílů cargo-fuzz s pokrytím (Linux/WSL) **[navrženo]**.
