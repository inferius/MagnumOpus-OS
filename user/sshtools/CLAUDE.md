# user/sshtools

Klient SSH: `ssh`, `scp` a `sftp` v jednom programu se std (chování podle `argv[0]`). V initfs pod každým jménem s vlastním manifestem (`xtask` `STD_ALIASES`). Spouští je shell službou exec initu. Popis: `docs/cs/ssh.md`, kapitola Klient.

## Klíčové soubory

- `src/session.rs` - TCP + `ssh::client::ClientConn`, rozhodnutí klienta (`Back`): `~/.ssh/known_hosts`, identity a podpis přes kanál podpisu (`rt::agent`, `accountproto::agent`), heslo k serveru se skrytým vstupem (ECHO přes `rt::tty`); `Sess::pump` vybírá data kanálu do fronty (bez toho klient nepřijme další vstup a bajty z TCP by se ztratily).
- `src/main.rs` - `ssh` (exec, shell po řádcích), `src/transfer.rs` - `scp` a `sftp` nad `ssh::sftp_client`.

## Invarianty

- Soukromý klíč klienta program nikdy nemá: podpis dělá služba účtů (kanál `SSH_AGENT` jen od initu a jen programu z initfs, `INITFS_ONLY`).
- Heslo účtu program nevidí (systémová výzva „Účty“ jde přímo službě účtů); heslo k cizímu serveru jen do skryté výzvy, nikam se neukládá.
- Změněný klíč serveru = odmítnutí s varováním, nikdy tiché přijetí.
- Přepínače anglicky, hlášení česky; kód 255 = chyba spojení nebo přihlášení.

## Testy

- `cargo test -p ssh` (klient, SFTP klient). QEMU: `cargo xtask test --only ssh` (test `ssh: klient v systemu`, loopback na náš sshd).

## Pasti

- Program se std má dvě kopie `abi` (std a knihovny): chyby převádět na text, ne mezi typy.
- `accountproto` táhne kryptografii CTAP; používá se jen modul `agent` (linker zbytek zahodí, překlad je delší).
