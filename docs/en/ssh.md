# Remote access (SSH)

Logging in to the computer over the network with an SSH client (OpenSSH,
Windows `ssh.exe`, PuTTY): a shell in a terminal, single commands
(`ssh pc ls`), file transfer with `scp` and `sftp`. The server is **off**
by default; an administrator turns it on for the whole computer and each
account enables remote access for itself.

| Part | Where | What it does |
|---|---|---|
| protocol | `libs/ssh` | vendored sunset 0.6.0 (sans-IO), `server.rs` (connection, channels, PTY), `sftp.rs` (SFTP v3), `keys.rs` (keys, fingerprint, signature check) |
| service | `user/sshd` (`sshd`) | only listens on port 22, starts an `sshd-conn` process per connection |
| connection | `user/sshd` (`sshd-conn`) | encryption, login, channels; terminal via `termd`, files via the account's FS session |
| keys and verification | `libs/accounts/src/remote.rs`, `user/accountd/src/remote.rs` | SSH keys in the account record, signature check, attempt throttling |
| session | `user/init/src/remote.rs`, `user/init/src/sshd.rs` | the account's remote session, one-time ticket, enabling the service, host key |
| commands | `libs/shlang/src/eval/builtins/remote.rs` | `sshd`, `ssh-key`, `remote-access` |
| panel | `libs/comp` (`remote_label`) | "Vzdáleně: user@address" pill |
| tests | `libs/ssh/src/tests*.rs`, `user/init/src/tests/ssh.rs`, `xtask/src/sshtest.rs` | host: server against the sunset client, fuzzing; QEMU: OpenSSH client from the host |

> ⚠ **Risks and v1 limitations (read before enabling):**
> - **Key login only.** The server never accepts a password alone;
>   password with a second factor (TOTP) is **[designed]**. Whoever loses
>   the private key has to remove it at the computer (`ssh-key remove`).
> - **The connection process holds the host key.** `sshd-conn` gets the
>   host key seed so it can sign the key exchange. A bug in the parser
>   (which runs before login) could leak it. The process is isolated
>   (own job, 24 MiB, no outbound network, no FS before login), but
>   signing through a separate service is debt **[designed]**.
> - **RustCrypto twice.** sunset uses ed25519-dalek 3, curve25519-dalek 5,
>   sha2 0.11 and signature 3, TLS the older series. Both versions are in
>   the binaries at once: `accountd` (release, loadable size) grew from
>   685 852 to 807 839 B (+122 KB, +18 %). Unifying waits for rustls on
>   the new RustCrypto series; debt for now.
> - **Parser fuzzed only without libFuzzer.** Every `cargo test` sends the
>   server thousands of random and mutated inputs (`tests_fuzz.rs`);
>   cargo-fuzz targets (`libs/ssh/fuzz`) are ready but not run yet (they
>   need Linux or WSL with nightly).
> - **The firewall in the "public network" profile drops inbound
>   connections.** SSH works only in the "home" profile or with an
>   `allow_in` exception ([Firewall](firewall.md#network-profiles)).

## Architecture

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Parts of remote access">
<defs><marker id="sh1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="sh1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="140" height="70" rx="6"/><text class="t b" x="80" y="48" text-anchor="middle">SSH client</text><text class="m" x="80" y="68" text-anchor="middle">ssh, scp, sftp</text>
<rect class="box" x="200" y="20" width="150" height="70" rx="6"/><text class="t b" x="275" y="45" text-anchor="middle">sshd</text><text class="m" x="275" y="64" text-anchor="middle">port 22, listens only</text><text class="m" x="275" y="80" text-anchor="middle">max 8 connections</text>
<rect class="acc" x="200" y="130" width="150" height="90" rx="6"/><text class="t b" x="275" y="155" text-anchor="middle">sshd-conn</text><text class="m" x="275" y="174" text-anchor="middle">process per connection</text><text class="m" x="275" y="192" text-anchor="middle">job 24 MiB</text><text class="m" x="275" y="208" text-anchor="middle">encryption, channels</text>
<rect class="box" x="420" y="20" width="160" height="80" rx="6"/><text class="t b" x="500" y="45" text-anchor="middle">accountd</text><text class="m" x="500" y="64" text-anchor="middle">account keys, signature</text><text class="m" x="500" y="82" text-anchor="middle">attempt throttling</text>
<rect class="box" x="620" y="20" width="130" height="80" rx="6"/><text class="t b" x="685" y="45" text-anchor="middle">init</text><text class="m" x="685" y="64" text-anchor="middle">remote session</text><text class="m" x="685" y="82" text-anchor="middle">ticket</text>
<rect class="ok" x="420" y="150" width="160" height="70" rx="6"/><text class="t b" x="500" y="175" text-anchor="middle">termd + shsess</text><text class="m" x="500" y="195" text-anchor="middle">account's shell session</text>
<rect class="ok" x="620" y="150" width="130" height="70" rx="6"/><text class="t b" x="685" y="175" text-anchor="middle">fsd</text><text class="m" x="685" y="195" text-anchor="middle">account's FS session</text>
<path class="ln" d="M150 55 L198 55" marker-end="url(#sh1)"/>
<path class="ln" d="M275 90 L275 128" marker-end="url(#sh1)"/><text class="m" x="282" y="114">starts</text>
<path class="ln-acc" d="M350 150 L418 80" marker-end="url(#sh1a)"/><text class="m" x="356" y="112">signature</text>
<path class="ln" d="M580 60 L618 60" marker-end="url(#sh1)"/>
<path class="ln" d="M350 190 L418 190" marker-end="url(#sh1)"/>
<path class="ln" d="M350 210 L618 210" marker-end="url(#sh1)"/>
<text class="m" x="10" y="260">accountd checks the signature and asks init for a session; init returns a one-time ticket.</text>
<text class="m" x="10" y="280">sshd claims the session with the ticket and gives the connection process only a termd</text>
<text class="m" x="10" y="300">window channel and the account's FS session. No compositor, clipboard, audio or ELEVATE.</text>
</svg>
<figcaption>Privilege separation: who listens, who talks to the client and who decides about login.</figcaption>
</figure>

- **`sshd`** only listens (network session: `listen` on 22, no outbound
  connections) and hands each accepted connection to a new `sshd-conn`
  process in its own job (24 MiB). At most 8 connections; a connection
  that has not logged in is closed after 120 s **[implemented]**.
- **`sshd-conn`** talks to the client. It does not check the login
  signature itself: it sends it through `sshd` to `accountd`, which holds
  the account's keys **[verified in QEMU]**.
- **`accountd`** checks the signature with a key from the account, checks
  that the account has remote access enabled, and refuses reuse of the
  same SSH session identifier. Then it asks init for a remote session.
- **init** creates the session (own job, FS session with the account's
  identity) and returns a random one-time ticket; `sshd` claims the
  session with it. An encrypted home that is not unlocked returns
  `Locked` - a login at the computer is needed first.

### Remote session

A remote session is **separate** from local ones (`init/src/remote.rs`):
it has no desktop, compositor, clipboard or audio, cannot see the local
screen and does not count as the "active session" for apps from the
panel. Administrative actions (`ELEVATE`) are not possible from it - it
has no channel for confirmation at the computer. For v1 this is
intentional: remote administration needs its own confirmation design
**[designed]**.

Channels:

| Channel | What | Status |
|---|---|---|
| `shell` with PTY | shell in a termd/shsess session; window size, Ctrl+C, foreground and background as in the terminal window | **[verified in QEMU]** |
| `exec` | a single shell command, output and exit code | **[verified in QEMU]** |
| `sftp` subsystem | SFTP v3 over the account's FS session (also `scp`, which OpenSSH 9 runs over SFTP) | **[verified in QEMU]** |
| port forwarding, X11, agent | not implemented, the server refuses them | - |

## Enabling

1. An administrator turns the service on: `sshd enable` (administrative
   action, `ELEVATE` confirmation). The dialog shows the host key
   fingerprint - note it and compare it on the client's first
   connection.
2. A user enables access for their account: `remote-access on` (asks for
   the password).
3. The user adds a public key: `ssh-key add ~/id_ed25519.pub` or
   `ssh-key add -` (key on input, e.g. from a USB stick). The fingerprint
   is shown when adding.

| Command | What it does |
|---|---|
| `sshd` / `sshd status` | service state, host key fingerprint, number of connections |
| `sshd enable` / `sshd disable` | turns the service on or off (administrative action); the state survives a restart |
| `ssh-key list` | account keys: number, type, fingerprint, comment |
| `ssh-key add <file\|->` | adds a key (password); refuses a private key |
| `ssh-key remove <number>` | removes a key |
| `remote-access` / `remote-access on\|off` | state or enabling of the account's remote access (enabling asks for the password) |

The host key (ed25519) is created at first start from the CSPRNG and
lives in `/system/config/ssh` (system only) **[verified in QEMU]**.

Supported keys: `ssh-ed25519` and `sk-ssh-ed25519@openssh.com` (FIDO key
at the client; the server checks the user-presence flag). RSA and ECDSA
not yet **[implemented]**.

## Protection against guessing

- Throttling after failed attempts separately per **address** and per
  **account name**, with a growing delay (table in
  [Accounts](accounts.md); the same function as local login, but separate
  counters). The longer of the two delays applies **[implemented]** (test
  `cargo test -p accounts remote`).
- The counters live only in `accountd` memory (at most 4096 addresses and
  names, the oldest drops out).
- At most 6 login attempts per connection, then the connection closes.
- The client does not learn the reason for a refusal (unknown account,
  foreign key and disabled access look the same).

## Audit

Every login and refusal goes to the log (`logd`): when, from where
(address), account, key fingerprint; the end of a session with its
length. Session content (commands, data) is not recorded
**[implemented]**.

While any remote session is connected, the panel shows a
**"Vzdáleně: user@address"** pill (with several sessions "+N") in the
warning colour **[implemented]** (test `cargo test -p comp remote`).

## Connecting from Windows (QEMU)

```bash
cargo xtask run --ssh
```

forwards host port 2222 to guest port 22. In the guest: `sshd enable`,
`remote-access on`, `ssh-key add -` (paste the content of
`id_ed25519.pub`). Then on Windows:

```bash
ssh -p 2222 user@127.0.0.1
```

```bash
scp -P 2222 file.txt user@127.0.0.1:
```

`ssh-keygen -t ed25519` creates the key (OpenSSH ships with Windows
10/11). The guest firewall must be in the "home" profile.

## Client

The `ssh`, `scp` and `sftp` commands (program `user/sshtools`, one
program by name, each with its own manifest) are started by the shell
through init's exec service ([Shell design](shell-design.md))
**[verified in QEMU]** (test `ssh: klient v systemu ...`, the client
against our sshd over loopback).

| Command | What it does |
|---|---|
| `ssh [--port N] [user@]server [command...]` | a command on the server (output, exit code); without a command the server's shell line by line (no raw terminal yet) |
| `scp [--port N] file server:[path]` / `scp server:path [file]` | one file there or back, over SFTP only (like OpenSSH 9, not the old scp protocol) |
| `sftp [--port N] [user@]server` | commands from input: `ls`, `cd`, `pwd`, `get`, `put`, `rm`, `mkdir`, `quit` |
| `ssh-keygen [comment]` | a new client key right in the account service; prints the public key for the server's `authorized_keys` |
| `ssh-key identities` / `import <file\|->` / `forget <number>` | client keys: list, taking over an OpenSSH private key (unencrypted ed25519 only), removal |

**The account service holds the keys** **[verified in QEMU]**:

- The client private key (ed25519 seed) is created in or imported into
  the account service and wrapped by the account password
  (`accounts::client`). No program gets it. `ssh` has from init only a
  **signing channel** (`accountproto::agent`) and the service signs only
  an SSH login message (RFC 4252) carrying the blob of exactly that key.
- Only a system program from initfs (`ssh`, `scp`, `sftp`) gets the
  signing channel: init starts it with the launchd flag `INITFS_ONLY`, so a
  program in `/system/bin` or an app on disk cannot take the name, and
  launchd refuses the `Agent` role without that flag (the test checks
  both).
- **Unlocking:** a password login at the computer unlocks the session's
  keys at once (like ssh-agent). Otherwise (a session over SSH, a key
  added later) the service asks with a **system prompt** labelled
  **Účty**: the system part of the shell shows it on request of the
  account service and the password goes straight to the service; the
  `ssh` program never sees the account password. A program cannot draw
  the prompt's badge with its own output.
- **Confirming the key for a new server** is a second system prompt (on
  the key's first use with a server); the service remembers the server.
- Locking the screen does not drop the keys, only the end of the session
  does. An account setting "lock keys with the screen lock"
  **[designed]**.
- Every signature, unlock and refusal goes to the log (logd).
- A password change rewraps the keys; a new password from the
  administration or the recovery key cannot unwrap them, the client keys
  are then lost (they cannot be opened without the old password).
  Unlocking with a FIDO key **[designed]** (password only today).

**Server verification:** `~/.ssh/known_hosts` in the OpenSSH format. An
unknown server = a question with the fingerprint on the program's input
(`yes` stores it), a changed key = connection refused with a warning
**[verified in QEMU]**.

**A password for a foreign server** (only when the server offers it) goes
to the program's hidden prompt: `ssh` turns ECHO off through the terminal
control channel (`ttyproto`) and the shell masks the prompt; it goes
neither to the block nor to the log **[implemented]**.

v1 limitations: the server's shell without a command runs line by line
(raw terminal for programs **[designed]**), an encrypted OpenSSH private
key cannot be imported (bcrypt-pbkdf), the client's interoperability with
an OpenSSH server is not tested automatically (our sshd over loopback; an
OpenSSH server in WSL is a question for the user).

## Tests

| Test | What it verifies |
|---|---|
| `cargo test -p ssh` | server against the sunset client (exec, PTY, foreign key, password refused, large transfer), SFTP, keys (OpenSSH 9.7 vectors), fuzzing |
| `cargo test -p accounts remote` | keys in the account, signature check, throttling, repeated session identifier |
| `cargo test -p ssh` (client) | the client against our server (exec, output, exit code, EOF, foreign server key, wrong key), the SFTP client, OpenSSH private key, client fuzzing |
| `cargo test -p accounts client` | client keys: creation, import, unlocking, password change, reset, approved servers |
| `cargo xtask test --only ssh` (client) | `ssh`, `scp`, `sftp` in the system against our sshd: known_hosts, the Účty prompt, scp there and back, sftp, an impostor in `/system/bin`, a changed server key, the Agent role without `INITFS_ONLY` **[verified in QEMU]** |
| `cargo xtask test --only ssh` | OpenSSH client from the host: key login, command and exit code, PTY, foreign key and password refused, scp there and back (70 KB), sftp, writing and reading a file with the shell, disabled server refuses the connection **[verified in QEMU]** |

Deviation from the brief: the host test uses the OpenSSH client (system
`ssh.exe`), not the russh library - russh would bring tokio and
ring/aws-lc into xtask, OpenSSH checks real interoperability.

## Plan

- interactive `ssh` with a raw terminal for programs **[designed]** (`ping`,
  `nc` and `resolve` are done, [Network](network.md#network-commands-of-the-shell)),
- password with a second factor (TOTP via accountd) **[designed]**,
- key exchange signing in a separate service (host key outside the
  connection process) **[designed]**,
- confirming administrative actions from a remote session **[designed]**,
- running the cargo-fuzz targets with coverage (Linux/WSL) **[designed]**.
