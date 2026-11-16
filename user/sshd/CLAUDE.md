# user/sshd

Server SSH (`docs/cs/ssh.md`). Dva programy: `sshd` (služba, jen naslouchá
na 22, žádné spojení ven) a `sshd-conn` (proces na spojení ve vlastním jobu,
24 MiB). Spouští ho init jen když je služba zapnutá (`user/init/src/sshd.rs`).

## Klíčové soubory

- `src/main.rs` - naslouchání, limit spojení (`MAX_CONNS`), čas na
  přihlášení (`LOGIN_GRACE`), předávání žádostí accountd a vyzvednutí
  relace lístkem u initu.
- `src/conn.rs` - spojení: `libs/ssh` server, terminál přes okno `termd`,
  exec, SFTP nad relací FS účtu (`AccountFs`).
- `src/proto.rs` - zprávy `sshd` <-> `sshd-conn` (`Req::Probe/Auth/Claim`).

## Invarianty

- `sshd-conn` nedostane klíče účtů ani relaci FS, dokud accountd
  nepotvrdí podpis a init nevydá relaci; pak jen kanál okna termd a relaci
  FS účtu (žádný kompozitor, `ELEVATE`).
- Limit paměti v manifestu pokrývá všechna spojení
  (`MAX_CONNS` x `CONN_MEMORY` + vlastní).

## Testy

- `cargo xtask test --only ssh` (klient OpenSSH z hostitele,
  `xtask/src/sshtest.rs`). Firewall hosta musí být v profilu domácí.

## Pasti

- Hostitelský klíč drží `sshd-conn` (podpis výměny klíčů); přesun do
  samostatné služby je dluh.
- Zápisy shellu ne-systémového uživatele (odklad `/tmp/.aside`) zatím
  selhávají ACCESS_DENIED, test SSH se zápisům přes shell vyhýbá.
