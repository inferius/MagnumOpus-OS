# Organization management

How a company or school can manage **its own** computers without the
system ceasing to protect the people who use them: enrolling a computer in
organization management, company backup of the home key, endpoint
monitoring (EDR) and the risks, stated explicitly. The whole page is
**[designed]** (approved by the user, no code yet).

It builds on [User accounts](accounts.md) (administering the computer
without root, home encryption), on the [Security](architecture.md#security)
model and on [Service recovery](service-recovery.md) (the supervisor
restarts the agent).

## Principles

1. **Never a personal computer.** The features on this page exist only on
   a computer that was explicitly **enrolled** in organization management.
   None of them is on in the default system and none can be switched on
   remotely on a computer that is not enrolled.
2. **The user always sees what happens.** An enrolled computer shows it in
   settings ("This computer is managed by organization X, monitored: ...")
   and with a permanent indicator on the bar. Hidden management does not
   exist.
3. **No organization code in the kernel.** The organization's agent is an
   ordinary userspace process with a capability; no kernel module and no
   third-party driver is added (the lesson of
   [CrowdStrike 2024](#endpoint-monitoring-edr)).
4. **Strong permissions are recorded and visible.** Event subscription,
   recovery of the home key and a policy change all go into the audit and
   the user sees them.
5. **The organization's private keys are not in the computer.** Stealing
   the computer or the account database gives nothing to the organization
   nor to an attacker.

## Enrolling a computer in organization management

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Enrolling a computer in organization management">
<defs><marker id="so1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="so1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="210" height="110" rx="6"/><text class="t b" x="115" y="45" text-anchor="middle">organization X</text><text class="m" x="115" y="66" text-anchor="middle">organization public key</text><text class="m" x="115" y="84" text-anchor="middle">policies (signed)</text><text class="m" x="115" y="102" text-anchor="middle">one-time enrollment code</text><text class="m" x="115" y="120" text-anchor="middle">private key: off the network</text>
<rect class="warn" x="290" y="20" width="200" height="110" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">system dialog</text><text class="m" x="390" y="66" text-anchor="middle">"Enroll this computer in</text><text class="m" x="390" y="82" text-anchor="middle">organization X?" + what</text><text class="m" x="390" y="98" text-anchor="middle">will be monitored; confirmed</text><text class="m" x="390" y="114" text-anchor="middle">by an admin role (ELEVATE)</text>
<rect class="acc" x="560" y="20" width="190" height="110" rx="6"/><text class="t b" x="655" y="45" text-anchor="middle">/system/org</text><text class="m" x="655" y="66" text-anchor="middle">enrollment record</text><text class="m" x="655" y="84" text-anchor="middle">public key (trust anchor)</text><text class="m" x="655" y="102" text-anchor="middle">policies, signature-verified</text><text class="m" x="655" y="120" text-anchor="middle">FS session for this service only</text>
<path class="ln" d="M220 75 L288 75" marker-end="url(#so1)"/>
<path class="ln" d="M490 75 L558 75" marker-end="url(#so1)"/>
<rect class="ok" x="10" y="190" width="230" height="110" rx="6"/><text class="t b" x="125" y="215" text-anchor="middle">user settings</text><text class="m" x="125" y="236" text-anchor="middle">"Managed by organization X,</text><text class="m" x="125" y="252" text-anchor="middle">monitored: ..."</text><text class="m" x="125" y="274" text-anchor="middle">policy list, audit</text>
<rect class="ok" x="290" y="190" width="200" height="110" rx="6"/><text class="t b" x="390" y="215" text-anchor="middle">bar (compd)</text><text class="m" x="390" y="236" text-anchor="middle">permanent "managed"</text><text class="m" x="390" y="252" text-anchor="middle">indicator; a click shows</text><text class="m" x="390" y="268" text-anchor="middle">what is monitored and by whom</text>
<rect class="box" x="560" y="190" width="190" height="110" rx="6"/><text class="t b" x="655" y="215" text-anchor="middle">personal computer</text><text class="m" x="655" y="236" text-anchor="middle">no record: none of these</text><text class="m" x="655" y="252" text-anchor="middle">features, nobody issues</text><text class="m" x="655" y="268" text-anchor="middle">an event subscription</text>
<path class="ln-acc" d="M655 130 L655 160 L125 160 L125 188" marker-end="url(#so1a)"/>
<path class="ln-acc" d="M655 160 L390 160 L390 188" marker-end="url(#so1a)"/>
<text class="m" x="520" y="154" text-anchor="middle">read by accountd / compd</text>
</svg>
<figcaption>A person at the computer confirms the enrollment; from then on the user sees it in settings and on the bar.</figcaption>
</figure>

- **Enrollment** is a whole-computer action: it goes through `ELEVATE` and
  a system dialog like the other administration
  ([Administering the computer](accounts.md#administering-the-computer)).
  The dialog shows the organization and the **list of what will be
  monitored**; an application or malware cannot forge it. The enrollment
  code is one-time and time-limited.
- **The enrollment record** (`/system/org`) holds the organization's
  public key as the trust anchor and policies signed with the
  organization's private key. The computer accepts a policy only when the
  signature matches. The organization name and the list of what is
  monitored are taken by settings and the bar **from this record**, not
  from the agent, so the agent cannot display less than it monitors.
- **Organization policies** are data: which event categories are
  monitored, the default `AUTH` behavior when the agent is unavailable
  (fail-open / fail-closed), whether the home key is backed up, allowed
  applications. A policy change goes into the audit and the user sees it.
- **Removing the computer from the register** is also an `ELEVATE` action
  with confirmation. Whether the user may do it on a company computer
  without the organization is decided by policy (open question 1).
- **A personal computer** has no record: nobody issues the event
  subscription capability or the company key wrap, and the services that
  would use them do not start.

## Company backup of the home key (escrow)

The home key is already random (256 bits) and has a **wrap for every
authentication method** ([Home encryption](accounts.md#home-encryption)).
The company backup adds **one more wrap**: the home key wrapped with the
**organization's public key** (asymmetric, e.g. HPKE with X25519,
RFC 9180).

<figure class="diagram">
<svg viewBox="0 0 760 340" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Company wrap of the home key">
<defs><marker id="so2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="so2a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="20" width="170" height="80" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">home key</text><text class="m" x="95" y="64" text-anchor="middle">256 bits, random</text><text class="m" x="95" y="80" text-anchor="middle">(as today)</text>
<rect class="box" x="250" y="20" width="210" height="110" rx="6"/><text class="t b" x="355" y="43" text-anchor="middle">user wraps (in the DB)</text><text class="m" x="355" y="64" text-anchor="middle">password, PIN, security key,</text><text class="m" x="355" y="80" text-anchor="middle">recovery key</text><text class="m" x="355" y="102" text-anchor="middle">unchanged</text>
<rect class="acc" x="250" y="150" width="210" height="80" rx="6"/><text class="t b" x="355" y="173" text-anchor="middle">organization wrap (new)</text><text class="m" x="355" y="193" text-anchor="middle">HPKE: organization public key</text><text class="m" x="355" y="209" text-anchor="middle">only on an enrolled computer</text>
<path class="ln" d="M180 50 L248 55" marker-end="url(#so2)"/>
<path class="ln-acc" d="M120 100 L120 190 L248 190" marker-end="url(#so2a)"/>
<rect class="warn" x="530" y="20" width="220" height="210" rx="6"/><text class="t b" x="640" y="43" text-anchor="middle">private part: never in the PC</text><text class="m" x="640" y="68" text-anchor="middle">A  offline (safe)</text><text class="m" x="640" y="88" text-anchor="middle">B  HSM</text><text class="m" x="640" y="108" text-anchor="middle">C  YubiKey</text><text class="m" x="640" y="128" text-anchor="middle">D  Shamir k of n among</text><text class="m" x="640" y="144" text-anchor="middle">several people</text><text class="m" x="640" y="176" text-anchor="middle">theft of the PC or the</text><text class="m" x="640" y="192" text-anchor="middle">account DB gives nothing</text>
<path class="ln-acc" d="M460 190 L528 190" marker-end="url(#so2a)"/><text class="m" x="494" y="182" text-anchor="middle">recovery</text>
<rect class="ok" x="10" y="270" width="740" height="56" rx="6"/><text class="t b" x="380" y="292" text-anchor="middle">Recovery = an audit record + a notice to the owner at the next sign-in</text><text class="m" x="380" y="312" text-anchor="middle">the owner learns that their home was unlocked with the company key, even though they did nothing</text>
</svg>
<figcaption>One more wrap of the home key uses the organization's public key; the private part lies outside the computer.</figcaption>
</figure>

- **The organization's private key is never in the computer or in the
  account database.** Options: offline storage (a safe), an HSM, a
  YubiKey, or **Shamir secret sharing k of n** among several people, so
  recovery needs several persons. The computer knows only the public key;
  **stealing the computer or the account database gives nothing**, neither
  to an attacker nor to the organization.
- **Recovery** is an exceptional event: it unlocks the home of one
  account, is written to the audit (who, when, why) and the owner gets a
  **notice at the next sign-in**. Recovery does not allow reading data
  secretly.
- **Comparison with existing solutions.** FileVault on macOS has an
  "institutional recovery key" with the organization's public certificate
  (the private part outside the computer): the same idea as here.
  BitLocker with escrow to a directory stores recovery keys on the
  organization's server; it is weaker because **compromising the
  directory reveals all the keys**. Decision: use an asymmetric wrap.
- **Recommendation:** company data belongs mainly on **company storage**
  (a server, shared volumes); escrow is the emergency path, not the main
  way of working with data.
- Escrow depends on home encryption, which still waits for the FS
  encrypted vaults (phase 4,
  [Encrypted vaults](fs-v3-design.md#encrypted-vaults)); until the home is
  encrypted, escrow has nothing to back up.
- **Key backup** is switched on by policy; settings show that it is on
  and for whom.

## Endpoint monitoring (EDR)

Organizations want to see what happens on their computers (like tools
such as CrowdStrike). The system will offer it as a **system layer**, not
as kernel code.

> **The lesson of July 2024:** a faulty content update loaded by the
> CrowdStrike driver in the Windows kernel took down roughly 8.5 million
> computers into a restart loop. Third-party code in the kernel means its
> bug is a bug of the whole system. Therefore the agent **must not** be in
> the kernel or a driver here. The model is Apple's **Endpoint Security
> framework**: the system supplies events and the agent is an ordinary
> process outside the kernel.

<figure class="diagram">
<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Event service and the organization agent">
<defs><marker id="so3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="so3a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="190" height="200" rx="6"/><text class="t b" x="105" y="43" text-anchor="middle">kernel and services</text><text class="m" x="105" y="68" text-anchor="middle">processes: start, exit,</text><text class="m" x="105" y="84" text-anchor="middle">exec, exception</text><text class="m" x="105" y="108" text-anchor="middle">files: open, write,</text><text class="m" x="105" y="124" text-anchor="middle">delete, execute</text><text class="m" x="105" y="148" text-anchor="middle">network: connect, DNS</text><text class="m" x="105" y="172" text-anchor="middle">sign-in, ELEVATE,</text><text class="m" x="105" y="188" text-anchor="middle">USB, rule changes</text>
<rect class="acc" x="280" y="20" width="200" height="200" rx="6"/><text class="t b" x="380" y="43" text-anchor="middle">auditd (event service)</text><text class="m" x="380" y="68" text-anchor="middle">structured events</text><text class="m" x="380" y="88" text-anchor="middle">NOTIFY: a copy only</text><text class="m" x="380" y="108" text-anchor="middle">AUTH: allow / deny</text><text class="m" x="380" y="124" text-anchor="middle">with a deadline</text><text class="m" x="380" y="148" text-anchor="middle">verdict cache</text><text class="m" x="380" y="172" text-anchor="middle">default policy when the</text><text class="m" x="380" y="188" text-anchor="middle">agent is late or crashed</text>
<rect class="warn" x="560" y="20" width="190" height="200" rx="6"/><text class="t b" x="655" y="43" text-anchor="middle">organization agent</text><text class="m" x="655" y="68" text-anchor="middle">ordinary userspace</text><text class="m" x="655" y="84" text-anchor="middle">process</text><text class="m" x="655" y="108" text-anchor="middle">subscription capability</text><text class="m" x="655" y="124" text-anchor="middle">(enrolled computer only)</text><text class="m" x="655" y="148" text-anchor="middle">no access to the memory</text><text class="m" x="655" y="164" text-anchor="middle">of other processes</text><text class="m" x="655" y="188" text-anchor="middle">sends data to the org</text>
<path class="ln" d="M200 120 L278 120" marker-end="url(#so3)"/>
<path class="ln-acc" d="M480 100 L558 100" marker-end="url(#so3a)"/><text class="m" x="519" y="92" text-anchor="middle">event</text>
<path class="ln-acc" d="M558 150 L482 150" marker-end="url(#so3a)"/><text class="m" x="519" y="168" text-anchor="middle">verdict</text>
<rect class="ok" x="10" y="260" width="740" height="84" rx="6"/><text class="t b" x="380" y="282" text-anchor="middle">Agent crashed or missed the deadline? The system keeps running.</text><text class="m" x="380" y="302" text-anchor="middle">the default policy applies (fail-open: allow and log; fail-closed: deny only selected categories)</text><text class="m" x="380" y="320" text-anchor="middle">the supervisor (init) restarts the agent; the subscription is restored through a lease</text>
</svg>
<figcaption>The system supplies events and the agent is an ordinary process; its crash does not bring the system down.</figcaption>
</figure>

- **The event service** (working name `auditd`) receives **structured
  events** from the kernel and from services:
  - **processes:** start, exit, `exec`, exception (the kernel already has
    [exception channels](syscalls.md#exception_channel) this can be built
    on),
  - **files:** `open`, write, delete, execute (reported by `fsd`),
  - **network:** connections and DNS (reported by `netd`, building on the
    [firewall audit](firewall.md)),
  - **sign-in, `ELEVATE`, USB, rule changes** (reported by `accountd`,
    `devmgr`, `fwd`).
- **Two kinds of subscription:**
  - `NOTIFY`: the agent gets a copy of the event and decides nothing.
  - `AUTH`: the agent **allows or denies** the event, always **with a
    deadline**. If it misses the deadline or crashed, the **default
    policy** applies (fail-open or fail-closed per the organization's
    policy) and **the system keeps running**. Verdicts are **cached** (the
    same operation of the same program does not ask again).
- **The agent is an ordinary userspace process** with a subscription
  capability: never kernel code, never a driver, **no access to the
  memory of other processes**. The supervisor restarts it after a crash
  ([Service recovery](service-recovery.md)).
- **Only an enrolled computer issues the subscription capability** (init,
  after verifying the record in `/system/org`). A subscription is a
  **strong permission**: it is recorded in the audit and visible in
  settings and on the bar.
- The policy delimits the scope of events and the categories that may be
  monitored; what is not in the policy, the agent does not get.

## Screen and key recording

**It is not in the default monitoring scope.** Recording the screen or
key presses is a **separate, explicitly announced category**: it must be
in the policy, the user sees it in settings at enrollment and a **visible
indicator** (the bar) shows it for the whole time. Without the category
switched on, no capability is issued for it. Input to system dialogs and
password fields is never recorded.

## Risks

- ⚠️ **Legal.** Monitoring employees is governed by the GDPR (lawful
  basis, proportionality, information) and in the Czech Republic by the
  Labour Code, section 316: an employer may monitor employees only with a
  **serious reason** and after **informing** them. The system helps with
  **transparency** (the user sees what is monitored and gets a notice
  about key recovery), but **legal responsibility stays with the
  organization**. This page is not legal advice.
- ⚠️ **Performance.** `AUTH` slows the monitored operations: every
  uncached operation waits for the agent's verdict. The design limits this
  with a **deadline** (after it the default policy applies) and a
  **verdict cache** (a repeated operation does not ask). Estimate: a cache
  hit is cheap (no communication with the agent), a missing entry costs
  one message round trip to the agent plus its work. The concrete
  deadline and impact will be **measured** at rollout **[designed]**; the
  default `AUTH` scope is therefore narrow (typically program launch),
  not every `open`.
- ⚠️ **Abuse.** Event subscription and key recovery are strong
  permissions: a compromised agent or an organization administrator sees a
  lot. Against that stand: a capability only for an enrolled computer, an
  audit and visibility for the user, policies signed by a key outside the
  computer, Shamir sharing for key recovery and no agent access to the
  memory of other processes.
- ⚠️ **The workaround with an unencrypted home.** The administration role
  can set a new password on an account and sign in as it; with an
  **unencrypted** home it thereby gets its data. With an encrypted home
  this is not possible (the new password does not get the home key).
  Recorded as a known limitation in
  [Accounts](accounts.md#risks-and-limitations).

## Rollout order and dependencies

Implementation starts **only after the current order** for adopting
existing software
([std, `cstd`, memory management, dynamic linking, ...](architecture.md#adopting-existing-software-order));
this page does not change it. What must exist before each part:

| Part | Needs |
|---|---|
| enrollment (`/system/org`, dialog) | `accountd` (`ELEVATE`, role, audit), `fsd` (session for `/system/org`), `compd` (dialog, bar indicator) |
| company wrap of the home key | FS encrypted vaults (phase 4), `accountd` (wraps), HPKE cryptography (RustCrypto) |
| event service `auditd` | `logd` (storage and lookup), `fsd` and `netd` as event sources, kernel (exception channels, process events), init (capability, supervisor) |
| agent and `AUTH` subscription | `auditd`, init (restart through leases), policies from the enrollment record |
| screen and key recording | `compd` (indicator), input layer (`inputd`), policy |

## Open questions

1. May a user remove a company computer from the register alone, or only
   with the organization (policy)? Options: A) always (loses the company
   features), B) only with the organization, C) decided by policy.
2. Who may confirm enrollment on a shared computer with several accounts
   (only the administration role, or every account owner for their own
   home)?
3. What is the policy format and how does the organization deliver it
   (pull on every connection, or push)?

## Rollout status

| Part | Status |
|---|---|
| enrollment, policies, indicator | **[designed]** |
| company wrap of the home key (escrow) | **[designed]** (after the FS encrypted vaults) |
| event service `auditd`, agent, `NOTIFY` and `AUTH` subscriptions | **[designed]** |
| screen and key recording | **[designed]** |
