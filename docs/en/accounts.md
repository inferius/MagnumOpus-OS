# User accounts

The **`accountd`** service manages the accounts of the people using the
computer: who exists, how they sign in (password, PIN, security key,
development auto-login) and what their home is. After a successful
authentication it asks **init** for a session; init mints the identity for
`fsd` (`CONNECT_AS`) and likewise the terminal, GUI and task channels.
**A client never makes up an identity** - it only gets the channels init
minted for it.

There is no `root` and no administrator account anywhere. User 0 (the
system) is only the identity of init's control session; nobody can sign in
as it.

Code: protocol and CTAP2 `libs/accountproto`, logic (tested on the host)
`libs/accounts`, the service `user/accountd`, start-up and the session
manager in init `user/init/src/accounts.rs` and `user/init/src/session.rs`,
QEMU tests `user/init/src/tests/accounts.rs`. The sign-in and settings
screens (`design/prihlaseni.html`, `design/nastaveni.html`) are drawn by
another thread; this page describes the service and the interface those
screens use.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="The account service and its surroundings">
<defs><marker id="ue1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="ue1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="70" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">sign-in screen</text><text class="m" x="110" y="64" text-anchor="middle">GUI system layer</text><text class="m" x="110" y="80" text-anchor="middle">login channel</text>
<rect class="box" x="10" y="120" width="200" height="70" rx="6"/><text class="t b" x="110" y="145" text-anchor="middle">account settings</text><text class="m" x="110" y="164" text-anchor="middle">in the user session</text><text class="m" x="110" y="180" text-anchor="middle">self channel</text>
<rect class="acc" x="290" y="20" width="200" height="170" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">accountd</text><text class="m" x="390" y="68" text-anchor="middle">accounts, authentication</text><text class="m" x="390" y="86" text-anchor="middle">Argon2id, PIN, FIDO2</text><text class="m" x="390" y="104" text-anchor="middle">slow-down after failures</text><text class="m" x="390" y="122" text-anchor="middle">home key wraps</text><text class="m" x="390" y="140" text-anchor="middle">FS identity: account 902</text><text class="m" x="390" y="158" text-anchor="middle">sees only /system/accounts</text>
<rect class="ok" x="570" y="20" width="180" height="90" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">init</text><text class="m" x="660" y="64" text-anchor="middle">session manager</text><text class="m" x="660" y="80" text-anchor="middle">fsd control session</text><text class="m" x="660" y="96" text-anchor="middle">CONNECT_AS, homes</text>
<rect class="box" x="570" y="140" width="180" height="50" rx="6"/><text class="t b" x="660" y="162" text-anchor="middle">fsd</text><text class="m" x="660" y="180" text-anchor="middle">F1d permissions</text>
<rect class="box" x="290" y="230" width="200" height="60" rx="6"/><text class="t b" x="390" y="254" text-anchor="middle">security key</text><text class="m" x="390" y="273" text-anchor="middle">usb-hid, CTAPHID</text>
<rect class="box" x="570" y="230" width="180" height="60" rx="6"/><text class="t b" x="660" y="254" text-anchor="middle">user session</text><text class="m" x="660" y="273" text-anchor="middle">FS, terminal, GUI, tasks</text>
<path class="ln" d="M210 55 L288 55" marker-end="url(#ue1)"/>
<path class="ln" d="M210 155 L288 155" marker-end="url(#ue1)"/>
<path class="ln-acc" d="M490 55 L568 55" marker-end="url(#ue1a)"/><text class="m" x="529" y="47" text-anchor="middle">session</text>
<path class="ln" d="M660 110 L660 138" marker-end="url(#ue1)"/>
<path class="ln-acc" d="M700 110 L700 125 L745 125 L745 228" marker-end="url(#ue1a)"/>
<path class="ln" d="M390 190 L390 228" marker-end="url(#ue1)"/>
<path class="ln" d="M490 170 L568 170" marker-end="url(#ue1)"/><text class="m" x="529" y="162" text-anchor="middle">DB</text>
<text class="m" x="10" y="318">Only init mints identities. accountd decides who signed in; it never sees anyone's home itself.</text>
</svg>
<figcaption>accountd authenticates, init mints the session, fsd enforces permissions.</figcaption>
</figure>

## Terms

| Term | Meaning |
|---|---|
| **account** | a person on this computer: user number, sign-in name, display name, groups, authentication methods, home |
| **user number** (`uid`) | identity for the FS and other services (`u64`) |
| **authentication method** | password, PIN, security key or development auto-login |
| **session** | what init mints after sign-in: a job with the user's processes and channels carrying their identity |
| **home** | the user's own dataset `/home/<name>` |
| **administration role** | "may administer this computer" (see [Administration](#administering-the-computer)) |
| **device key** | a secret bound to the computer (a file today, TPM later), see [PIN](#device-bound-pin) |
| **home key** | random home encryption key, wrapped separately for every method |
| **recovery key** | random key the user gets once when turning encryption on; opens the home without any method |

## Identity numbers

| Range | Who | Note |
|---|---|---|
| 0 | system | only init's fsd control session; nobody can sign in, init never mints a session for it |
| 1-899 | reserved | |
| 900-999 | service accounts | `fwd` 900, `revocd` 901, **`accountd` 902** |
| 1000-999 999 | people | assigned by accountd in ascending order; a deleted account's number is **never reused** (old FS grants would otherwise apply to a new person) |
| 1 000 000+ | groups | `uzivatele` (1 000 000) = all people; further groups are created by administration |
| `u64::MAX` | `EVERYONE` | in FS grants "everyone except the system" |

Groups and users share the FS grant number space, so they have separate
ranges.

## Account database

- **Own dataset** `/system/accounts` (created by init on first boot). Its
  owner is the `accountd` service account (902), permission description
  `PROTECTED` without grants: nobody but the system sees it
  (`NOT_FOUND`). The dataset has `NOEXEC`.
- accountd gets an FS session with the single mount
  `/db -> /system/accounts`. It sees nothing else on the FS.
- **One file per account** `/db/users/<uid>`, binary (`libs/wire`):
  format version, the record, and at the end BLAKE2s-256 of the content
  (accountd recognises a damaged file and reports that account as damaged;
  the other accounts keep working). Writes go through a temporary file and
  rename (`save_if`) followed by `fsync` - after a power cut the record is
  always entirely old or entirely new.
- `/db/meta`: next free user and group number, DB version.
- `/db/device.key`: the device key (32 B) until there is a TPM. It is
  created on first use (PIN), not at service start: the start does not
  wait for a seeded random generator (a machine without a hardware entropy
  source, e.g. the live USB on aarch64, may not have one right away and
  init would give up on the account service after 20 s).

| Record field | Type | Note |
|---|---|---|
| `uid` | u64 | |
| `name` | text | sign-in name: `a-z`, `0-9`, `-`, `_`, 1-32 characters, starts with a letter; unique; also the home name |
| `display` | text | display name (UTF-8, up to 64 characters) |
| `groups` | u64[] | always contains `uzivatele` |
| `flags` | u32 | administration role, account disabled, development account |
| `phrase` | text | security phrase (up to 64 characters), shown by system dialogs and the sign-in screen |
| `password` | optional | Argon2id parameters, salt, verifier |
| `pin` | optional | parameters, salt, verifier, failure count |
| `keys` | list | security keys: credential id, COSE public key, signature counter, name, has a key PIN, supports hmac-secret |
| `failures` | u32 + i64 | consecutive failed attempts and the time of the last one |
| `home` | text | `/home/<name>` |
| `crypt` | optional | home encryption: home key wraps (see [Encryption](#home-encryption)) |
| `created`, `last_login` | i64 | wall-clock time |

Secrets in the DB **cannot be read back**: passwords and PINs only as
Argon2id output, the home key only wrapped.

## Home

- init creates the dataset `/home/<name>` when accountd asks
  (`CREATE_HOME`): owner = `uid`, permission description `PROTECTED`
  without grants (other users cannot even see the home: `NOT_FOUND`),
  `NOEXEC` according to the profile (standard and strict: on - data are
  not programs).
- `/home` belongs to the system and has a `LIST | TRAVERSE` grant
  (the directory only, not inherited) for the `uzivatele` group. A listing
  shows only entries the caller has some right to (`Fs::visible`), so a
  user sees exactly their own home in `/home`: other homes and the
  administration bin are `PROTECTED` without grants, hence invisible
  (`NOT_FOUND`), and the names of other accounts are not revealed. init
  adds the grant on every start (`accounts::layout`, idempotent), so
  installations where `/home` had only `TRAVERSE` get it too (the file
  manager showed an empty folder there).
- Deleting an account does **not delete the home right away**; it moves to
  the administration bin (`/home/.smazane/<uid>`, `PROTECTED` without
  grants - only the system sees it; browsing for the administration role
  and deletion after 30 days come with account administration).
- Application data live in `/home/<name>/apps/<id>/` (see
  [Architecture](architecture.md)).

## Authentication

All four methods are in the first version. The user chooses which ones to
use on their account; a password is mandatory for every account except the
development one (it is the fallback when the PIN or key cannot be used).
A password has **at least 8 characters**; rules like "an upper-case letter
and a digit" are not enforced (length is what makes a password strong,
NIST SP 800-63B).

### Password (Argon2id)

The verifier is Argon2id (RFC 9106, version 0x13) of the UTF-8 password
(NFC normalised) and a random 16 B salt, output **64 B**: the first 32 B
are the verifier stored in the DB, the second 32 B are the home key wrap
material (one Argon2id run for both; the wrap material cannot be derived
from the verifier). Parameters are stored with every value so they can be
raised: after a successful sign-in a value with weaker parameters is
recomputed.

| Profile | Memory `m` | Passes `t` | Lanes `p` | Time (measured) | When |
|---|---|---|---|---|---|
| `qemu` | 8 MiB | 2 | 1 | host 5 ms (release, `52d2ebe`); a whole sign-in in QEMU TCG (debug, `52d2ebe`, x86_64 and aarch64, incl. two DB writes with `fsync`) 0.1-0.3 s | `cargo xtask run`, `cargo xtask test` |
| `hw` (default) | 64 MiB | 3 | 1 | host 70 ms (release, `52d2ebe`, 2024 PC) | real PC |
| `hw` after calibration (plan) | 64 MiB | 3-10 | 1 | target a quarter to half a second | installation measures and raises `t` |

For comparison: OWASP recommends at least 19 MiB and `t=2`, RFC 9106's
second choice is 64 MiB and `t=3`. Memory is the main protection against
GPU guessing, so it stays at 64 MiB and a slow machine rather loses time.
⚠️ While verifying, accountd briefly takes 64 MiB of memory; concurrent
verifications therefore run **one after another** so the memory is never
doubled.

Implementation: the `argon2` crate (RustCrypto), no home-grown
cryptography. In the debug build `argon2`, `blake2` and `accounts` are compiled
with optimisation (`Cargo.toml`): without it a sign-in took ~2 s in QEMU
(debug, `52d2ebe`, TCG).

### Device-bound PIN

A PIN (4-12 digits) is short: from a stolen DB it would be guessed in
seconds. That is why the PIN verifier **depends on the device key**,
which cannot be carried away from the computer:

```text
p = Argon2id(PIN, salt)                        // same parameters as the password
verifier = HMAC-SHA256(device key, "pin-verify" || uid || p)
```

The device key sits behind the **`DeviceKey`** interface (a trait in
`libs/accounts`):

| Operation | Today (`FileKey`) | Later (`TpmKey`, TPM 2.0) |
|---|---|---|
| `mac(label, data)` | HMAC-SHA256 with the key from `/db/device.key` | HMAC inside the TPM (key sealed to PCRs), the key never leaves the TPM |
| `hardware()` | `false` | `true` |
| guessing counter | counter in the DB (slow-down below) | additionally the TPM counter (dictionary attack lockout) |

After **5 wrong PINs** the PIN is blocked until the user signs in once
with the password or a key.

⚠️ **Risk (without TPM):** `FileKey` lies on the same disk as the DB.
Whoever takes the disk takes both and guesses the PIN offline in seconds.
Without a TPM the PIN protects **only against guessing at the keyboard**
(slow-down, blocking), not against disk theft - and for an encrypted home
the PIN wrap is equally weak. The PIN is still **allowed for everything**
(sign-in, lock screen, unlocking an encrypted home; the user's decision),
but settings **explicitly warn** when the PIN is turned on on a computer
without a TPM (`DeviceKey::hardware() == false`, flag `PIN_WEAK` in the
list and in `WHOAMI`). With a TPM (a standard PC component today) there is
no warning: the key never leaves the chip and the TPM keeps the guessing
counter.

### Security key (FIDO2, CTAP2 over USB HID)

A key (YubiKey, SoloKey, Nitrokey, ...) presents itself as an HID device
with usage page `0xF1D0`. The `usb-hid` driver does not hand such an
interface to input but **switches to the CTAPHID transport**: it forwards
64 B reports over a channel which init gives to accountd (`key` + handle;
one message = one report). devmgr creates the
channel for every HID interface; the driver of a device that is not a key
closes it and accountd drops closed channels (it uses the most recently
attached key). On top of it `accountd` speaks CTAP2
(`libs/accountproto::ctap`: CTAPHID frames, CBOR, messages).

| Step | CTAP2 | What accountd verifies |
|---|---|---|
| information | `authenticatorGetInfo` | versions `FIDO_2_0`/`FIDO_2_1`, support for `clientPin`, `hmac-secret`, PIN protocols 1 and 2 |
| key PIN | `authenticatorClientPIN`: `getKeyAgreement`, `setPIN`, `changePIN`, `getPinToken` / `getPinUvAuthTokenUsingPinWithPermissions`, `getRetries` | shared secret from ECDH P-256 (protocol 1: SHA-256, protocol 2: HKDF-SHA256), the PIN only encrypted with AES-256-CBC, PIN token |
| registration | `authenticatorMakeCredential` (rp `magnumopus.local`, user = uid, ES256 and EdDSA, `rk=false`, `hmac-secret: true`, `pinUvAuthParam`) | attestation `none` or `packed` (self), UP and UV flags, stores the credential id and COSE public key |
| sign-in | `authenticatorGetAssertion` (allowList = the account's keys, random 32 B challenge, `pinUvAuthParam`, `hmac-secret` with the account salt) | signature over `authData` + `SHA-256(challenge)` with the public key (p256 / ed25519 from RustCrypto), `rpIdHash`, flags **UP** (touch) and **UV** (key PIN), the signature counter grows; the `hmac-secret` output unwraps the home key |

- Full CTAP2 in the first version: **key + touch + key PIN** (UV). The key
  PIN is verified by the key itself (8 attempts, then the key blocks - a
  property of the key); accountd only passes it encrypted. A key without a
  PIN: settings offer to set one (`setPIN`); without a PIN the key works
  only as "key + touch" and settings warn (whoever steals the key and knows
  the account name can sign in).
- **`hmac-secret`**: the key computes an HMAC from the credential secret and
  the account salt (32 B, in the DB); accountd derives the home key wrap
  from it. Nobody unwraps it without the key in hand.
- A signature counter that does not grow (and is not 0) means a possible
  cloned key: the sign-in is refused.
- Tests: a simulated key on the host (`accountproto::ctap::soft`) with p256
  and Ed25519; QEMU does not emulate a real FIDO2 key (`u2f-emulated` only
  does the older U2F).

### Automatic sign-in (development only)

`cargo xtask run`, `cargo xtask usb` and `cargo xtask vmware` (the live
USB and the VM disk made from it are a development profile, the user's decision of 2026-11-13) add the file `accounts-dev` to
initfs and accountd then creates (if missing) the account `vyvojar` (first
free number, development account flag) and signs it in right away, in
both debug and release builds. **Safeguard:** only these commands add the
file; neither the image for users (`xtask image`) nor the tests (`xtask
test`, except the live USB check) have it. **Why only the file and not also a
debug-build condition** (previously `cfg(debug_assertions)`): whoever can
change the initfs on the ESP can just as well replace accountd itself
there, so the second safeguard did not really add security; it only broke
`xtask run --release` (the usual way to start). With Secure Boot the
safeguard will be the image signature **[designed]**. The
`zive-usb` test checks both: the stick boots into a terminal and an image
without the file ends on the sign-in screen. The development account has
no password, no administration role and is marked in the account list.

### Protections

**Slow-down after failed attempts** (counted per account across all
methods; the PIN additionally has its own 5 attempts):

| Consecutive failed attempts | Next attempt not before |
|---|---|
| 1-2 | immediately |
| 3 | 30 s |
| 4 | 1 min |
| 5 | 2 min |
| 6 | 5 min |
| 7 or more | 15 min |

- **No permanent account lockout:** anybody at the keyboard could lock the
  owner out. After 7 attempts it is 4 attempts per hour, so a password of
  8 random characters will not be guessed.
- The counter is **increased and stored before verification** and reset
  only after success: a crash or power-off in the middle of an attempt does
  not reset it. If the store fails, verification does not run at all.
- The failure reply is the same for all methods (`DENIED` + wait); for a
  non-existent account Argon2id is computed anyway (same response time).
- **Constant-time comparison** (`subtle::ConstantTimeEq`) of all
  verifiers.
- **No secrets in the log:** passwords, PINs, challenges and keys are in
  the `Secret` type (zeroed when dropped, `zeroize`; `Debug` prints
  `***`). The log contains only the account name, method and result.
- A message with a password is zeroed by accountd after processing.

## Sessions

<figure class="diagram">
<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sign-in sequence">
<defs><marker id="ue2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="t b" x="90" y="24" text-anchor="middle">sign-in (login)</text><text class="t b" x="290" y="24" text-anchor="middle">accountd</text><text class="t b" x="490" y="24" text-anchor="middle">init</text><text class="t b" x="680" y="24" text-anchor="middle">fsd</text>
<path class="ln dash" d="M90 32 L90 350"/><path class="ln dash" d="M290 32 L290 350"/><path class="ln dash" d="M490 32 L490 350"/><path class="ln dash" d="M680 32 L680 350"/>
<path class="ln" d="M90 60 L288 60" marker-end="url(#ue2)"/><text class="m" x="190" y="54" text-anchor="middle">AUTH alice, password</text>
<rect class="acc" x="230" y="70" width="120" height="44" rx="4"/><text class="m" x="290" y="88" text-anchor="middle">slow-down? Argon2id,</text><text class="m" x="290" y="104" text-anchor="middle">constant-time compare</text>
<path class="ln" d="M290 130 L488 130" marker-end="url(#ue2)"/><text class="m" x="390" y="124" text-anchor="middle">OPEN_SESSION uid, groups, home</text>
<path class="ln" d="M490 160 L678 160" marker-end="url(#ue2)"/><text class="m" x="585" y="154" text-anchor="middle">CONNECT_AS 1000, / -> /</text>
<path class="ln" d="M680 185 L492 185" marker-end="url(#ue2)"/><text class="m" x="585" y="179" text-anchor="middle">session + channel</text>
<rect class="ok" x="420" y="195" width="140" height="44" rx="4"/><text class="m" x="490" y="213" text-anchor="middle">session job, terminal,</text><text class="m" x="490" y="229" text-anchor="middle">GUI, tasks (identity)</text>
<path class="ln" d="M490 255 L292 255" marker-end="url(#ue2)"/><text class="m" x="390" y="249" text-anchor="middle">session number</text>
<path class="ln" d="M290 280 L92 280" marker-end="url(#ue2)"/><text class="m" x="190" y="274" text-anchor="middle">OK session (no channels)</text>
<text class="m" x="10" y="318">init keeps the identity channels and gives them only to processes in the session job. The sign-in screen</text>
<text class="m" x="10" y="336">gets only the session number (it switches to the desktop), not the channels - it cannot pose as the user.</text>
</svg>
<figcaption>Sign-in: accountd decides, init mints the identity.</figcaption>
</figure>

- accountd has a private channel to init (`accountproto::ToInit`). After a
  successful authentication it sends `OPEN_SESSION { uid, groups, home }`.
  init (session manager, `user/init/src/session.rs`):
  1. checks it is a person (uid 1000-999 999, groups from 1 000 000) - it
     never mints a session for the system or a service account, even if
     accountd asks,
  2. mints an FS session `CONNECT_AS(uid, groups, profile.user_mounts())`,
  3. keeps the session's self channel (account settings in the session
     get it),
  4. creates the **session job** and sends an "opened" event to the
     supervisor loop (`session::EVENTS`). **[implemented]**
- **Desktop only after sign-in** **[implemented]** (`user/init/src/desktop.rs`):
  init starts the desktop applications (terminal, task manager, demo) only
  for a signed-in session, in its job. The session's terminal window gets
  a termd with the **signed-in user's FS session** (previously init's
  session = the system), and termd and its `shsess` run in the session
  job. The bar launches applications into the active (most recent
  unlocked) session; without a sign-in it launches nothing. The compositor
  gets `session open|lock|unlock|close` (hides or shows the sign-in
  screen).
- **Basic FS rights of people:** the `uzivatele` group may list the root
  and `/home` (it sees only entries it has a right to - its own home in
  `/home`) and create in `/tmp` and `/tmp/.aside` (shell clones); the grant applies to
  the directory only and is not inherited, so other people's files in
  `/tmp` stay invisible.
- The session recipe in a lease (recovery after an fsd crash),
  applications with narrower sessions by manifest (`app_mounts`) and a
  taskd channel with the session identity **[designed]**.
- **Sign-out** (`LOGOUT`) **[implemented]**: init drops the FS session
  and the self channel and with the [`JOB_KILL`](syscalls.md#job_kill)
  syscall ends **all** the session's processes (also `shsess` from termd
  and applications started by applications) and closes the job, so nothing
  new can start in it; the compositor shows the sign-in screen. An
  encrypted home is locked (`UNLOAD_KEY`). Test `ucty: udalosti a job
  relace` (x86_64 and aarch64).
- **Screen lock** **[implemented]** (`LOCK` from the session, or
  Ctrl+Alt+Delete -> Lock = `LOGIN_LOCK` on the login channel): processes
  keep running, the compositor shows the lock screen and input goes only
  there. **Only the same account** can unlock (`UNLOCK` with
  authentication), or sign out. Locking does not erase the home key from
  memory (processes keep working with the home).

### Sign-in and lock screen

**[implemented]** Following `design/prihlaseni.html`. The compositor
draws it (`libs/comp/src/greeter.rs`) over the whole screen: the windows
underneath are not drawn at all (not in the GPU path either), nor is the
bar. It takes **physical input only** (synthetic input, e.g. from an
agent, is dropped) and ignores input for the first 0.3 seconds after it
appears.

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sign-in screen and its channels">
<defs><marker id="ue5" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="160" height="60" rx="6"/><text class="t b" x="90" y="45" text-anchor="middle">keyboard, mouse</text><text class="m" x="90" y="64" text-anchor="middle">physical input only</text>
<rect class="acc" x="240" y="20" width="220" height="110" rx="6"/><text class="t b" x="350" y="45" text-anchor="middle">compd: screen</text><text class="m" x="350" y="66" text-anchor="middle">account, method,</text><text class="m" x="350" y="82" text-anchor="middle">phrase, slow-down countdown</text><text class="m" x="350" y="98" text-anchor="middle">windows below not drawn</text><text class="m" x="350" y="114" text-anchor="middle">asynchronous (frame never waits)</text>
<rect class="box" x="560" y="20" width="190" height="60" rx="6"/><text class="t b" x="655" y="45" text-anchor="middle">accountd</text><text class="m" x="655" y="64" text-anchor="middle">login channel</text>
<rect class="ok" x="560" y="150" width="190" height="70" rx="6"/><text class="t b" x="655" y="175" text-anchor="middle">init</text><text class="m" x="655" y="194" text-anchor="middle">session, job, desktop</text><text class="m" x="655" y="210" text-anchor="middle">session open|lock|close</text>
<path class="ln" d="M170 50 L238 50" marker-end="url(#ue5)"/>
<path class="ln" d="M460 45 L558 45" marker-end="url(#ue5)"/><text class="m" x="509" y="38" text-anchor="middle">AUTH, UNLOCK</text>
<path class="ln" d="M655 80 L655 148" marker-end="url(#ue5)"/><text class="m" x="700" y="118">OPEN_SESSION</text>
<path class="ln" d="M558 190 L350 190 L350 132" marker-end="url(#ue5)"/><text class="m" x="455" y="183" text-anchor="middle">session ... (hide, lock)</text>
</svg>
<figcaption>The screen only collects secrets; accountd decides about the sign-in and init about what is shown.</figcaption>
</figure>

- The compositor gets a **`login`** channel to the account service from
  init (`guiproto::control::LOGIN`) and the session state (`session open
  <id> <uid> <name>`, `session lock|unlock|close <id>`). The screen is
  shown when no session is unlocked: the sign-in without a session, the
  lock screen (clock, date, only the session's account) with a locked one.
- Methods by account: **PIN** (default when the account has one; digits
  only), **password**, **security key** (field = key PIN; "Touch the
  security key"), **forgotten password** = recovery key. Tab switches the
  method, arrows the account, Enter authenticates, Esc clears the field.
- The account's security phrase appears only after the account is picked
  (a fake screen does not know it). Failure messages: "Wrong password",
  remaining PIN attempts, blocked PIN, slow-down countdown ("Next attempt
  in 30 s").
- compd sends the requests **asynchronously** (`user/compd/src/login.rs`):
  neither Argon2id nor a key touch delays a frame. The password is zeroed
  in the message after encoding.
- **Secure fields** **[implemented]** (sign-in, lock, a dialog with a
  password or PIN): the input trace records only "input into a secure
  field", never a character or the length; the diagnostic snapshot states
  only the screen state and covers the field grey in the image (not even
  the number of dots). Secrets in compd are overwritten with zeros after
  sending (screen field, dialog password, the encoded message and one
  queued on a channel) and never go to logd. Test
  `cargo test -p comp tajemstvi_neni_v_trasovani_ani_snimku`.
- **The lock screen reveals nothing:** instead of the desktop it draws
  only the wallpaper (no windows or their content), no notifications or
  their count, only the clock, date and the session's account.
- Host tests: `cargo test -p comp greeter` (PIN sign-in, synthetic input,
  account and method choice, slow-down, lock and unlock only by the same
  account, windows below are not drawn, sign-out); snapshots
  `target/comp-greeter.png` and `target/comp-greeter-zamceno.png`.
- **No account** **[implemented]**: when the computer has no account (the
  image for users before installation), the screen shows no form but the
  message "There is no account on this computer yet. Accounts are created
  in the installer." and the **Shut down** and **Restart** buttons
  (confirmed by a system dialog). The avatar comes only from an account
  (the initial of its name), never from the heading. Until the account
  list arrives only the background is shown.
- **The installer creates the first account** (`design/instalace.html`,
  `setup` channel) **[designed]**, not the sign-in screen (the user's
  decision of 2026-11-13).
- **Method chips** are shown only for an account with several methods
  (password and PIN, key, recovery key); with a single method they would
  be confusing.
- Tests: `cargo test -p comp bez_uctu_zprava_a_vypnuti_misto_formulare`,
  `cip_metody_jen_pri_vic_metodach`; snapshot `target/comp-greeter-bez-uctu.png`.
- The protocol supports several simultaneous sessions (fast user
  switching) by session number; the switching screen comes with the
  desktop.
- accountd crash: sessions keep running (init holds them). The supervisor
  only watches accountd for now; a restart that takes over the sessions
  (`LIST_SESSIONS`) is a next step. An init crash = the end of the system.

## Administering the computer

> **Approved by the user (2026-11-08):** an action is confirmed by a person
> with the role; the five-minute administration mode only as a
> **voluntary option** in the dialog. Variants considered: A (confirmation
> per action only), B (A + administration mode automatically), C (a
> separate administrator account - two passwords, less friendly).

Goal: an ordinary user never deals with an "administrator", and still no
application or malware in the session can change the whole computer.

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Confirming an action for the whole computer">
<defs><marker id="ue3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="170" height="70" rx="6"/><text class="t b" x="95" y="55" text-anchor="middle">application / settings</text><text class="m" x="95" y="74" text-anchor="middle">wants: add account</text><text class="m" x="95" y="90" text-anchor="middle">"bob"</text>
<rect class="acc" x="250" y="30" width="170" height="70" rx="6"/><text class="t b" x="335" y="55" text-anchor="middle">accountd</text><text class="m" x="335" y="74" text-anchor="middle">ELEVATE action +</text><text class="m" x="335" y="90" text-anchor="middle">parameters, requester</text>
<rect class="warn" x="490" y="20" width="260" height="128" rx="6"/><text class="t b" x="620" y="44" text-anchor="middle">system dialog</text><text class="m" x="620" y="64" text-anchor="middle">"Settings wants to add account bob"</text><text class="m" x="620" y="82" text-anchor="middle">security phrase, who confirms</text><text class="m" x="620" y="100" text-anchor="middle">authentication: password / PIN / key</text><text class="m" x="620" y="118" text-anchor="middle">people with the role</text><text class="m" x="620" y="136" text-anchor="middle">[ ] don't ask for 5 minutes (option)</text>
<path class="ln" d="M180 65 L248 65" marker-end="url(#ue3)"/>
<path class="ln" d="M420 65 L488 65" marker-end="url(#ue3)"/>
<path class="ln" d="M620 148 L620 170 L337 170 L337 102" marker-end="url(#ue3)"/><text class="m" x="480" y="164" text-anchor="middle">confirmed + authenticated</text>
<rect class="ok" x="10" y="190" width="420" height="44" rx="6"/><text class="m" x="220" y="210" text-anchor="middle">capability only for this action with these parameters,</text><text class="m" x="220" y="226" text-anchor="middle">valid 60 s, once (or administration mode 5 min)</text>
<path class="ln" d="M300 172 L250 188" marker-end="url(#ue3)"/>
</svg>
<figcaption>An action for the whole computer is confirmed by a person with the role, in a dialog applications cannot fake.</figcaption>
</figure>

- **No root and no admin account.** User 0 stays only the identity of
  init's control session.
- **For oneself** without confirmation: installing an application for
  oneself according to its manifest, changing one's own password, PIN,
  key, phrase, home encryption.
- **For the whole computer** (other people's accounts, drivers, system
  updates, disks, the global firewall, system time): the requester sends
  `ELEVATE { action, parameters }`. accountd asks the compositor for a
  **system dialog** (only accountd and the permission broker have the
  `dialog` capability): it shows the action with its real parameters, the
  requester (application from its manifest) and the security phrase. A
  **person with the role** confirms it with the same method as at sign-in.
  If the signed-in user has the role, they confirm themselves; otherwise
  they pick another account with the role ("over the shoulder").
- The result is a **capability only for that action with those
  parameters**, valid for 60 s and once. The service performing the action
  (accountd, devmgr, fwd, ...) checks it with accountd (`CHECK_GRANT`) and
  consumes it.
- **Administration mode (option):** the dialog has a check box "Don't ask
  again for administration actions from this session for 5 minutes",
  **off by default**. Only the person can tick it (an application cannot
  preset it). While the mode lasts, the session gets a capability for
  `ELEVATE` without a dialog; the bar shows a countdown and an "End
  administration mode" button. The mode also ends with a screen lock and
  sign-out. ⚠️ Risk: a process in the session (malware too) can perform
  any administration action without another dialog during those 5 minutes.
  `ELEVATE` from an automated source (an AI agent) **never uses** the mode,
  it always shows the dialog.
- The first account from installation has the role; it can give it to
  others and take it away (not the last one). Every action is written to
  the audit (`/db/audit`): who asked, who confirmed, what, whether in
  administration mode.

**Rollout** **[implemented]** (`user/accountd/src/admin.rs`, dialog
`DialogKind::Elevate` in `libs/comp`): the compositor gives the account
service a `dialog` channel (init passes it on to a new instance after a
compositor crash too). `ELEVATE` from a session sends the confirming
person's phrase and a dialog titled "<application> wants to change the
system" with the action's real parameters and a line "Confirms: <name>
(may administer this computer)"; the field accepts a password or a PIN (a
PIN when it is digits only and the account has one). Verification works
like sign-in (slow-down, counter). accountd performs account actions
(1-4) right away and replies with the result (the uid for
`ACCOUNT_CREATE`); other services' actions (5-15) get a **grant**: a 16 B
token, 60 s, once, only for the same parameters (compared byte by byte),
which the service checks and consumes with `CHECK_GRANT` on the
**`grants`** channel. Cancel = `CANCELED`, wrong password =
`ACCESS_DENIED`, no compositor (tests, console) = `NOT_SUPPORTED`. A
signed-in account cannot be deleted (`BAD_STATE`).

First-version limits: the signed-in person confirms when they have the
role, otherwise the **first** account with the role (choosing among
several accounts in the dialog **[designed]**); the requester shows as
"Application of user <name>" until the self channel goes to Settings with
its own identity from the manifest **[designed]**.

| Action (`ELEVATE`) | Parameters |
|---|---|
| 1 `ACCOUNT_CREATE` | name, display name, role |
| 2 `ACCOUNT_DELETE` | uid |
| 3 `ACCOUNT_RESET_PASSWORD` | uid (does not open an encrypted home) |
| 4 `ROLE_SET` | uid, on |
| 5-15 | drivers, updates, disks, firewall, time - added by their services |

Why: what is confirmed is a concrete action the person sees (the same
principle as the [action API](gui.md#action-api)); whoever does longer
administration turns the dialogs off for a while themselves, knowingly.

**A computer managed by an organization** (company, school): enrolling the
computer, policies and endpoint monitoring are described in
[Organization management](organization-management.md) **[designed]**,
including the right to
[install applications](organization-management.md#the-right-to-install-applications)
(Freely / From the catalog / None; the administration role may set it for
another account, e.g. [parental controls](parental-controls.md)). A personal computer never has
these features.

## Home encryption

The user's choice on their account (default: off, decided when creating
the account or any time in settings). Encrypting the dataset itself is a
**separate FS step**; accountd prepares the keys and the interface.

<figure class="diagram">
<svg viewBox="0 0 760 280" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Home key wraps">
<defs><marker id="ue4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="40" rx="6"/><text class="m" x="110" y="45" text-anchor="middle">password -> Argon2id (32 B)</text>
<rect class="box" x="10" y="75" width="200" height="40" rx="6"/><text class="m" x="110" y="100" text-anchor="middle">PIN -> device key / TPM</text>
<rect class="box" x="10" y="130" width="200" height="40" rx="6"/><text class="m" x="110" y="155" text-anchor="middle">FIDO2 key -> hmac-secret</text>
<rect class="warn" x="10" y="185" width="200" height="40" rx="6"/><text class="m" x="110" y="210" text-anchor="middle">recovery key (160 bits)</text>
<rect class="acc" x="290" y="20" width="190" height="205" rx="6"/><text class="t b" x="385" y="45" text-anchor="middle">wraps in the DB</text><text class="m" x="385" y="70" text-anchor="middle">KEK = HKDF-SHA256(</text><text class="m" x="385" y="86" text-anchor="middle">method secret, uid)</text><text class="m" x="385" y="112" text-anchor="middle">wrap = ChaCha20-Poly1305(</text><text class="m" x="385" y="128" text-anchor="middle">KEK, home key,</text><text class="m" x="385" y="144" text-anchor="middle">AAD = uid, method)</text><text class="m" x="385" y="175" text-anchor="middle">one wrap per method</text>
<rect class="ok" x="560" y="80" width="190" height="80" rx="6"/><text class="t b" x="655" y="105" text-anchor="middle">home key</text><text class="m" x="655" y="124" text-anchor="middle">256 bits, random</text><text class="m" x="655" y="140" text-anchor="middle">-> fsd LOAD_KEY</text>
<path class="ln" d="M210 40 L288 60" marker-end="url(#ue4)"/><path class="ln" d="M210 95 L288 100" marker-end="url(#ue4)"/><path class="ln" d="M210 150 L288 140" marker-end="url(#ue4)"/><path class="ln" d="M210 205 L288 180" marker-end="url(#ue4)"/>
<path class="ln" d="M480 120 L558 120" marker-end="url(#ue4)"/><text class="m" x="519" y="112" text-anchor="middle">unwraps</text>
<text class="m" x="10" y="260">A password change rewraps one wrap only; home data are not re-encrypted.</text>
</svg>
<figcaption>The home key is wrapped separately for every method and for the recovery key.</figcaption>
</figure>

- When turned on, accountd generates the **home key** (256 bits, system
  CSPRNG) and the **recovery key** (160 bits), which the user sees **only
  once** as 7 groups of 5 characters (32 Crockford base32 characters + 3
  check characters, e.g. `7KQ2M-...`; case, spaces and the O/0, I/L/1
  mix-ups do not matter, a typo is detected) and confirms having saved it
  (types 2 groups back).
- A wrap per method: password (second half of the Argon2id output),
  security key (the `hmac-secret` output), PIN (`DeviceKey::mac` over the
  second half of the PIN's Argon2id output; with a warning without a TPM,
  see [PIN](#device-bound-pin)), recovery key. Turning encryption on
  creates the password and recovery wraps; the PIN and security keys get a
  wrap the next time they are set (settings offer it again after turning
  encryption on). A method without a wrap only opens a home that another
  session has already unlocked.
- Password change = a new wrap. Forgotten password = sign in with the
  recovery key and set a new password.
- **Administration does not see other people's data:** the administration
  role can delete an account or set a new password for it, but for an
  encrypted home it **does not obtain** the home key (the new password
  gets a wrap only after the owner unlocks with their recovery key).

**Company backup of the home key:** on a computer enrolled in organization
management the home key can get one more wrap with the organization's
public key (the private part outside the computer); see
[Organization management](organization-management.md#company-backup-of-the-home-key-escrow)
**[designed]**.

⚠️ **Risk of data loss:** whoever forgets the password, loses the key and
**has no recovery key, loses the home data for good**. Nobody (neither
administration nor the vendor) can recover them - that is the point of
encryption. The turn-on screen must say so explicitly and have the
recovery key written down; settings warn when an encrypted account has
only one method.

Home encryption builds on the **FS encrypted vaults** (phase 4, see
[Encrypted vaults](fs-v3-design.md#encrypted-vaults)): the home becomes a
vault and accountd supplies its key. Only the interface exists so far.

**What the FS needs to provide** (proposed interface for the "dataset
encryption" step):

| Operation (fsproto) | Who | Meaning |
|---|---|---|
| `create_dataset(path, enc)` | control session (init) | dataset with encryption; the FS generates its own data key (DEK) and stores it wrapped by the key from `LOAD_KEY` |
| `LOAD_KEY { dataset, key }` | control session | unlocks the dataset with the home key (the FS derives the DEK wrap key from it); wrong key = `ACCESS_DENIED` |
| `UNLOAD_KEY { dataset }` | control session | locks; open handles return `LOCKED`, the FS zeroes its keys |
| `KEY_STATUS { dataset }` | anyone with visibility | `none` / `locked` / `unlocked` |
| locked dataset | - | names and contents unreadable, `stat` of the root returns `LOCKED`; snapshots and replication keep working on encrypted blocks |

The home key goes to the FS through init (accountd has no control
session): accountd sends init `UNLOCK_HOME { uid, key }` on its channel,
init passes it to fsd and zeroes the buffer. Until the FS supports
encryption, init answers `NOT_SUPPORTED`, accountd refuses to turn it on
and no wraps are stored (the wrap interface and its tests exist).

## The `accountproto` protocol

Message = operation code `u32` + fields (`libs/wire`, little-endian), like
the other services. An invalid message = `INVALID_ARGS`, never a panic.
Secrets (password, PIN) travel in a `secret` field and both sides zero
them after use. The channels are different capabilities:

| Channel | Who has it | What it may do |
|---|---|---|
| `login` | sign-in screen (system layer), init tests | list accounts for selection, authenticate, unlock |
| `self` | user session (settings) | own account: methods, phrase, encryption, lock, sign-out, request an administration action |
| `setup` | installer, only while no account exists | create the first account with the role |
| `grants` | services performing administration actions (devmgr, fwd, updates, ...) | check and consume a grant |
| `init` | init (private) | sessions, homes, home keys |

| Op | Channel | Name | Fields | Reply |
|---|---|---|---|---|
| 1 | login | `LIST` | - | accounts: uid, name, display name, methods (bits), flags |
| 2 | login | `PHRASE` | uid | phrase (shown after an account is picked) |
| 3 | login | `AUTH` | uid, method u8, secret | `OK` session u64 / `DENIED` wait ms, PIN attempts left / `LOCKED_OUT` ms |
| 4 | login | `UNLOCK` | session, method, secret | like `AUTH` |
| 5 | login | `LOGIN_LOCK` | session | - (lock from the screen; unlocking only with authentication) |
| 10 | self | `WHOAMI` | - | account record without secrets |
| 11 | self | `SET_PASSWORD` | old secret, new secret | - |
| 12 | self | `SET_PIN` | password secret, PIN secret (empty = delete) | - |
| 13 | self | `ENROLL_KEY` | password secret, key PIN secret, name | key id |
| 14 | self | `REMOVE_KEY` | password secret, id | - |
| 15 | self | `SET_PHRASE` | text | - |
| 16 | self | `ENCRYPT_HOME` | password secret | recovery key (only once) / `NOT_SUPPORTED` |
| 17 | self | `LOCK` | - | - |
| 18 | self | `LOGOUT` | - | - |
| 19 | self | `ELEVATE` | action u32, parameters (`accountproto::elevate::Params`) | after the dialog: the account action's result or a grant token; `CANCELED`, `ACCESS_DENIED`, without a compositor `NOT_SUPPORTED` |
| 20 | self | `SET_KEY_PIN` | PIN secret | - (a key that has no PIN yet) |
| 30 | setup | `CREATE_FIRST` | name, display name, password secret | uid |
| 40 | grants | `CHECK_GRANT` | token (16 B blob), action u32, parameters (blob) | - or `ACCESS_DENIED`; the token is consumed |
| 50 | init | `OPEN_SESSION` (accountd -> init, + self channel handle) | uid, groups, home | session u64 |
| 51 | init | `CLOSE_SESSION` | session | - |
| 52, 53 | init | `LOCK_SESSION`, `UNLOCK_SESSION` | session | - |
| 54, 55 | init | `CREATE_HOME`, `DELETE_HOME` | uid, name | - |
| 56 | init | `LIST_SESSIONS` | - | sessions: number, uid, locked |
| 57 | init | `UNLOCK_HOME` | uid, home key secret | - (`NOT_SUPPORTED` today) |
| 60 | init | `SESSION_CLOSED` (init -> accountd, event) | session | - |

Methods (`u8`): 1 password, 2 PIN, 3 security key (secret = key PIN, empty
= touch only), 4 development sign-in, 5 recovery key. A reply on the init
channel is a status `i32` (0 or a negative error code); an init event
starts with a positive operation code, so accountd tells them apart.
Administration actions (`ELEVATE`) see
[Administering the computer](#administering-the-computer).

Client (`accountproto::client`, feature `client`):

```rust
use accountproto::client::{Login, SelfService, session_of};
use accountproto::{AuthReply, Method, Secret};

let login = Login(channel);
for a in login.list()? {
    println!("{} ({})", a.display, a.name);
}
match login.auth(1000, Method::Password, Secret::from("password"))? {
    AuthReply::Ok { session } => println!("session {session}"),
    AuthReply::Denied { retry_after_ms, .. } => println!("next attempt in {} s", retry_after_ms / 1000),
    AuthReply::LockedOut { retry_after_ms } => println!("wait {} s", retry_after_ms / 1000),
}
```

## AI agent and identity

The agent workspace (the agents thread, `user/agentd`) runs as user 1000
hard-coded today. With accounts the agent gets the **signed-in user's**
session narrowed to the workspace (`NARROW` of the lease), identity = the
user's uid, marked as the agent in the audit and the task overview. The
agent gets no user number of its own (its files are the user's files).
The agent never confirms administration actions itself - `ELEVATE` from an
automated source always shows the dialog to a person. Agreed through the
coordinator.

## Tests

**On the host** (`cargo xtask test` also runs the library tests):

- `accountproto` (22 tests): encoding of all messages there and back,
  every truncation is an error, invalid messages, CBOR (RFC 8949 vectors,
  canonical CTAP2 key order, rejecting indefinite lengths, tags, duplicate
  keys and deep nesting), CTAPHID fragmentation and reassembly, PIN
  protocols 1 and 2, CTAP2 over the simulated key (registration and
  sign-in with and without a key PIN, `hmac-secret`, blocking after 8 wrong
  PINs, a key with a PIN requires a token, no touch, an unknown key, a
  signature by another key, `excludeList`, EdDSA, everything also over
  CTAPHID).
- `accounts` (22 tests): Argon2id according to the RFC 9106 vector
  (section 5.3) and the reference implementation vector, wraps (wrong
  secret, another user, damaged wrap), the recovery key (format, loose
  typing, typo detection), record format and damaged files, the slow-down
  table, the counter stored before verification (a failed store = no
  verification), password, NFC normalisation, PIN with the device key
  (another device key fails), PIN blocking and unblocking by password,
  a password change, recomputing weaker parameters, development sign-in
  only when allowed, security key with and without a PIN, a cloned key
  (counter does not grow), home encryption unwrapped by every method and by
  the recovery key, a damaged record does not take down the others,
  numbers of deleted accounts are not reused.

**In QEMU** (suite `ucty`, `user/init/src/tests/accounts.rs`, x86_64 and
aarch64):

| Test | Verifies |
|---|---|
| accountd running | `status`, the DB dataset belongs to accountd |
| creating the first account | `CREATE_FIRST` (short password refused, a second "first" account refused), role, home `/home/alice` is a dataset owned by the uid, `PROTECTED` without grants, `NOEXEC` |
| password sign-in | `AUTH` -> session, the session's FS writes to the home, `WHOAMI`, phrase, `ELEVATE` and `ENCRYPT_HOME` `NOT_SUPPORTED`, sign-out |
| PIN sign-in | `SET_PIN` (wrong password refused), `PIN_WEAK` flag, a wrong PIN with attempts left, sign-in with the PIN |
| session sees only its home | someone else's home and its files `NOT_FOUND`; listing `/home` shows the own home, not others or `.smazane`; the account DB `NOT_FOUND` |
| init does not mint a system identity | `OPEN_SESSION` for 0, 902, 999 and group 0 refused, no home for the system |
| lock and sign-out | lock, unlock with a wrong and the right password, after sign-out the session and its FS are gone |
| session events and job | opened/locked/unlocked/closed events, name and active session, a process in the session job, the job is gone after sign-out |
| raising privileges with the dialog | the test plays the compositor: dialog text, wrong password, cancel, allow with administration mode (the account is created), another action in the mode without a dialog, a grant only once and only for the same parameters |
| wrong password and slow-down | unknown account, 3rd failure = 30 s, during the wait even the right password fails |

## Risks and limitations

- ⚠️ **PIN without a TPM** protects only against guessing at the keyboard,
  not against disk theft; without a TPM the PIN wrap of the home key is
  equally weak. Allowed with a warning (the user's decision, see
  [PIN](#device-bound-pin)).
- ⚠️ **Administration mode** (option, 5 minutes): administration actions
  from the session without a dialog - for malware in the session too. Off
  by default.
- ⚠️ **Home encryption without a recovery key** = possible permanent data
  loss (see [Encryption](#home-encryption)). Until the FS supports
  encryption, the home **is not encrypted** and the disk can be read
  elsewhere (`osfs` runs as the system).
- ⚠️ The device key (`/db/device.key`) and the DB are protected only by FS
  permissions: against processes in the system yes, against booting
  another system from USB no.
- ⚠️ **The workaround with an unencrypted home:** the administration role can
  set a new password on an account and sign in as it; with an
  **unencrypted** home it thereby gets its data. With an encrypted home this
  is not possible (see [Home encryption](#home-encryption)). Related to
  [Organization management](organization-management.md#risks).
- A security key **without a key PIN**: whoever steals the key and knows
  the account name can sign in. Settings therefore offer a key PIN and warn
  without one.
- The sign-in screen and the administration dialog are only as secure as
  the compositor's system layer (applications do not draw into it, input
  goes only there) and the `inputd` secure attention shortcut.
- ⚠️ The fallback console (no GUI) has no sign-in yet: its terminal runs
  as the system, as before.

## Rollout status

| Part | Status |
|---|---|
| design (this page) | **[implemented]** |
| administration model | approved; **[implemented]** role, first account with the role, protection of the last role, `ELEVATE` with the dialog, account actions, grants, administration mode, audit; choosing the confirming account among several **[designed]** |
| `libs/accountproto` (messages, client, CTAP2, simulated key, administration actions) | **[implemented]**, host tests |
| `libs/accounts` (records, Argon2id, PIN, slow-down, keys, wraps, recovery key, password reset by administration) | **[implemented]**, host tests |
| `user/accountd` (DB on the FS, login/self/setup/grants channels, administration dialog, development sign-in) | **[implemented]**, QEMU tests |
| session manager and session job in init, desktop only after sign-in, terminal with the user identity, sign-out via `JOB_KILL` | **[implemented]** |
| sign-in and lock screen in the compositor | **[implemented]**, host tests; installer of the first account **[designed]** |
| CTAPHID transport in `usb-hid`, devmgr and init (`fido` -> `key`) | **[implemented]**; QEMU does not emulate a FIDO2 key, not yet verified on a real key |
| home encryption | wraps and recovery key **[implemented]**; FS vaults **[designed]** (FS phase 4) |
| account settings in the GUI | **[designed]** (Settings application) |
