# libs/ssh

Protokol SSH pro `sshd` (`docs/cs/ssh.md`): převzatý sunset 0.6.0
(`third_party/crates/sunset-0.6.0`, bez I/O, naše změny označené
`MagnumOpus:`) a nad ním `server.rs` (spojení, přihlášení přes trait
`Backend`, kanály, PTY), `client.rs` (klient: `ClientConn`, trait
`ClientBackend`, podpis mimo proces přes `SignKey::AgentEd25519`),
`sftp.rs` (server SFTP v3 nad traitem `SftpFs`), `sftp_client.rs` (pakety
klienta SFTP), `keys.rs` (klíče ed25519 a sk-ssh-ed25519, otisk, ověření
podpisu, `ed25519_sign`, `parse_private` pro soukromý klíč OpenSSH). Feature `proto`
(výchozí) = server; `accounts`/`accountd` berou jen `keys` (bez sunset).

## Invarianty

- Server podpis přihlášení neověřuje sám: `Backend::auth` ho pošle ven
  (accountd). Heslo se nikdy nepřijme.
- Klient soukromý klíč nemá: `ClientBackend::sign` (v systému služba účtů).
  Čtení dat kanálu jde přes `read_channel_either` (`read_channel` po EOF
  nevrátí data, která přišla před ním).
- Před přihlášením jde ze sítě jen do sunset a `server.rs`; každá změna
  parseru = fuzz (`tests_fuzz.rs`, `fuzz/`).
- Nejvýš `MAX_AUTH_TRIES` pokusů a `MAX_CHANNELS` kanálů na spojení;
  SFTP nejvýš `MAX_HANDLES` handlů, cesty jen přes `sftp::resolve`.
- Změny sunset jen se značkou `MagnumOpus:` a záznamem v `THIRD-PARTY.md`.

## Testy

- `cargo test -p ssh` (server proti klientovi sunset, SFTP, klíče, fuzz).
  QEMU: `cargo xtask test --only ssh`.
- `fuzz/` je samostatný workspace pro cargo-fuzz (Linux/WSL, nightly).

## Pasti

- RustCrypto je v binárkách dvakrát (sunset nová řada, TLS stará); čísla
  v `docs/cs/ssh.md`. Nepřidávat další crate ze třetí řady.
- Velikost okna a paketu sunset (256 KiB / 32 KiB) je záplata ve
  vendorovaném crate (feature `alloc`).
