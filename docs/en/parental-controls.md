# Parental controls

How a parent (or another adult with the administration role) can restrict a
child's account: time limits, web filtering, approval of applications and an
activity overview. The whole page is **[designed]** (approved by the user,
no code yet).

It is the **same mechanism as [Organization management](organization-management.md)**
in small: an account policy set by the administration role through an
administration action
([Administering the computer](accounts.md#administering-the-computer)) and
requests that go **to the administrator, not to the child**. The right to
install applications (levels From the catalog / None) is described in
[Organization management](organization-management.md#the-right-to-install-applications);
this page covers the rest.

## Principles

1. **Prompt fatigue is a security problem** ([query budget](architecture.md#security)).
   The child should not have to ask what is allowed; the system tells them
   in advance and offers a **request to the parent** when a limit is hit.
2. **Requests go to the parent, not to the child.** The child never gets
   an "allow?" dialog that they could click through themselves.
3. **Transparency.** The child sees that the account is under parental
   controls and **what is monitored**; hidden control does not exist.
4. **No screen recording and no message content.** Time, applications and
   blocked attempts are monitored, nothing more.
5. **Resistance to bypass by the child**, with an honest list of the limits
   ([Privacy and risks](#privacy-and-risks)).

## The common mechanism

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Account policy and requests">
<defs><marker id="rk1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="rk1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="warn" x="10" y="20" width="200" height="100" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">parent (admin role)</text><text class="m" x="110" y="66" text-anchor="middle">sets the account policy:</text><text class="m" x="110" y="82" text-anchor="middle">time, web, applications</text><text class="m" x="110" y="102" text-anchor="middle">administration action (ELEVATE)</text>
<rect class="acc" x="290" y="20" width="200" height="100" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">accountd</text><text class="m" x="390" y="66" text-anchor="middle">the child's account policy</text><text class="m" x="390" y="82" text-anchor="middle">time budget, requests,</text><text class="m" x="390" y="98" text-anchor="middle">audit</text>
<rect class="ok" x="570" y="20" width="180" height="100" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">enforcement</text><text class="m" x="660" y="66" text-anchor="middle">session manager (time)</text><text class="m" x="660" y="82" text-anchor="middle">netd / fwd / dnsd (web)</text><text class="m" x="660" y="98" text-anchor="middle">FS and packages (apps)</text>
<path class="ln" d="M210 70 L288 70" marker-end="url(#rk1)"/>
<path class="ln" d="M490 70 L568 70" marker-end="url(#rk1)"/>
<rect class="box" x="10" y="200" width="200" height="100" rx="6"/><text class="t b" x="110" y="225" text-anchor="middle">child</text><text class="m" x="110" y="246" text-anchor="middle">sees: the account is controlled</text><text class="m" x="110" y="262" text-anchor="middle">and what is monitored; time</text><text class="m" x="110" y="278" text-anchor="middle">left; "Ask for ..."</text>
<rect class="ok" x="290" y="200" width="200" height="100" rx="6"/><text class="t b" x="390" y="225" text-anchor="middle">request</text><text class="m" x="390" y="246" text-anchor="middle">"more time", "allow a site",</text><text class="m" x="390" y="262" text-anchor="middle">"install an application"</text><text class="m" x="390" y="282" text-anchor="middle">goes to the parent, not the child</text>
<path class="ln-acc" d="M210 250 L288 250" marker-end="url(#rk1a)"/>
<path class="ln-acc" d="M390 200 L390 122" marker-end="url(#rk1a)"/>
<path class="ln-acc" d="M290 90 L215 190" marker-end="url(#rk1a)"/><text class="m" x="224" y="150" text-anchor="start">notice in the parent's session</text>
</svg>
<figcaption>The parent sets the account policy, the child offers a request when a limit is hit and it goes to the parent.</figcaption>
</figure>

- **The account policy** belongs to `accountd` and changes only through an
  administration action (`ELEVATE`, confirmed by a person with the role).
  The child cannot change it, not even by reinstalling an application.
- **Requests** ("more time", "allow a site", "install application X") are
  stored by `accountd` and the parent sees them as a **notice in their
  session** (at sign-in or immediately if it is running). **Notices on
  another device** **[designed]** later. Approval completes the request by
  itself (the time is extended, the domain is allowed, the application is
  installed).
- **Without an organization** this is the only mechanism for administering
  another account; **in an organization** the same policy is set by
  organization management.

## Time limits

<figure class="diagram">
<svg viewBox="0 0 760 270" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Time budget and warnings">
<defs><marker id="rk2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="740" height="40" rx="6"/><text class="m" x="20" y="54">allowed window (e.g. 7:00 to 20:00)</text>
<rect class="ok" x="10" y="100" width="420" height="40" rx="6"/><text class="t b" x="220" y="125" text-anchor="middle">daily budget: time remains</text>
<rect class="warn" x="430" y="100" width="90" height="40" rx="6"/><text class="m" x="475" y="125" text-anchor="middle">10 min</text>
<rect class="warn" x="520" y="100" width="60" height="40" rx="6"/><text class="m" x="550" y="125" text-anchor="middle">1 min</text>
<rect class="acc" x="580" y="100" width="170" height="40" rx="6"/><text class="t b" x="665" y="125" text-anchor="middle">session lock</text>
<path class="ln" d="M430 100 L430 82" marker-end="url(#rk2)"/><text class="m" x="440" y="90">advance warning</text>
<rect class="ok" x="10" y="180" width="740" height="70" rx="6"/><text class="t b" x="380" y="203" text-anchor="middle">the lock offers "Ask for more time" -> a request to the parent</text><text class="m" x="380" y="223" text-anchor="middle">active session time is measured (idleness does not count); the budget is held by accountd / the session manager, not by apps</text><text class="m" x="380" y="239" text-anchor="middle">exceptions: school and communication with parents are always allowed</text>
</svg>
<figcaption>A daily budget inside the allowed window, advance warnings, a lock at expiry and a request for more time.</figcaption>
</figure>

- **A daily time budget** per day of the week (different for a school day and
  a weekend).
- **An allowed time window** (e.g. 7 to 20 h): outside it the session does
  not start or unlock.
- **Limits per application and category** (games, video, social networks)
  next to the overall budget; the category comes from the application's
  manifest.
- **Exceptions:** school applications and **communication with parents is
  always allowed** (it does not consume the budget).
- **Advance warnings** 10 minutes and 1 minute before the end. At expiry
  the session is **locked** (application state stays, the child does not
  lose work) and the lock screen offers **"Ask for more time"**; the
  request goes to the parent.
- **Measurement:** **active session time** is counted, idleness (locked
  screen, no input) does not count. **The budget is held by `accountd` and
  the session manager in init, not by applications**, so an application
  cannot delay or reset it.
- **Resistance to changing the clock:** the child gains nothing by moving
  the clock. The budget is measured with **monotonic time**
  (`CLOCK_MONOTONIC`, kernel), the allowed window is evaluated from real
  time from the `timed` service and **changing the system time is an
  administration action** ([Administering the computer](accounts.md#administering-the-computer)),
  so the child cannot do it without the parent
  ([Real time](time.md#setting-the-clock-clock_set)).

## Web filtering

- **Where:** in the network layer that an application cannot bypass:
  `netd` (the only network stack) with `fwd` rules
  ([Firewall](firewall.md#dns-filter)) and `dnsd` (encrypted DNS,
  [Network](network.md#encrypted-dns-and-dnssec-dnsd)). New: **rules per
  user** (today they are per application and network profile) **[designed]**.
- **Categories** (adult, gambling, ...) from a list, **custom allowed and
  blocked domains**, **safe search enforced at the DNS level** (the search
  engine is redirected to its safe variant).
- **A blocked page** shows a **clear message** ("A parent blocked this
  page") and **"Ask for permission"**, which goes to the parent. Not a
  silently refused lookup.
- ⚠️ **Limits that must be stated explicitly:**
  - A domain filter **does not see HTTPS content**: it blocks the name, not
    the page. A harmless domain may contain unsuitable content.
  - **VPNs, proxies and applications with their own DoH** can bypass it.
    The policy therefore **forbids foreign DNS and VPNs for the account**
    (per-user firewall rules: only the system resolver `dnsd`, no tunnels).
  - **Applications with their own browser** (own DNS, own network code) must
    go through the same rules, otherwise they would bypass the filter; such
    applications fall under the right to install
    ([From the catalog / None](organization-management.md#the-right-to-install-applications)).
  - No filter is perfect; parental controls complement a conversation, they
    do not replace it.

## Activity overview for the parent

- **The parent sees:** **time per application** (day, week), **requests**
  (pending and resolved) and **blocked attempts** (domain, application,
  time).
- **The child sees:** that the account is under parental controls, **what is
  monitored** (time, applications, blocked attempts) and their own usage
  overview; the remaining time is shown on the bar.
- **Not monitored:** **screen recording**, key presses, **message content**
  and page content. These categories do not exist for parental controls
  (unlike [organization management](organization-management.md#screen-and-key-recording),
  where it is an explicitly announced option).

## Privacy and risks

- **Activity data is local only** and visible only to the **administration
  role** (and to the child about themselves); it is sent nowhere. **Retention
  is limited** (e.g. 30 days), then it is deleted.
- ⚠️ **Bypass by the child:** another computer or a phone outside the
  system; a VPN or own DNS where the rules do not apply; physical access
  (reinstalling the system or removing the disk). The control does not
  protect against what lies outside the device.
- ⚠️ **Abuse by the parent:** a strong role over another account. Hence the
  policy is visible to the child and changes go into the account audit;
  hidden control does not exist.
- ⚠️ **A false sense of safety:** a domain filter and a time limit do not
  replace a conversation with the child; the page says so explicitly.
- **Legal and age questions** (consent, age of majority) depend on the
  country; this page is not legal advice.

## Rollout order and dependencies

It starts **only after the current order** for adopting existing software
([std, `cstd`, ...](architecture.md#adopting-existing-software-order)) and
after organization management, with which it shares the account policy.

| Part | Needs |
|---|---|
| account policy and requests | `accountd` (role, `ELEVATE`, audit), `fsd`, `compd` (notices, lock screen) |
| time limits | the session manager in init, `timed` (real time), kernel (monotonic time), `compd` (warnings on the bar) |
| web filtering | `netd`, `fwd`, `dnsd`, per-user rules in `netproto` / `netfilter` |
| activity overview | `accountd`, time-per-application measurement (candidate `taskd`, [task overview](architecture.md#task-overview-agreed)), `logd` (retention) |
| the right to install | [Organization management](organization-management.md#the-right-to-install-applications), catalog and packages |

## Rollout status

| Part | Status |
|---|---|
| account policy and requests to the parent | **[designed]** |
| time limits, warnings, lock and request for more time | **[designed]** |
| web filtering (categories, custom lists, safe search, per-user rules) | **[designed]** |
| activity overview and transparency for the child | **[designed]** |
