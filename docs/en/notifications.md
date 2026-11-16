# Notifications: the notifyd service and pop-up cards

Apps and services send notifications ("Download finished", "Flash drive
attached", "Battery 5 %") to the `notifyd` service. The compositor shows
them as a pop-up card at the top right and keeps them in the notification
centre (the bell in the bar). An app cannot fake who sent a notification
or flood the screen.

Owner page for: the `libs/notifyproto` protocol, the `user/notifyd`
service (logic in `libs/notifycore`), pop-up cards in the compositor
(`libs/comp/src/toast.rs`). The look of cards is in
[Appearance](appearance.md#notifications), the notification centre in the
bell panel belongs to [GUI](gui.md).

## Overview

<svg viewBox="0 0 760 260" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Notification path">
<defs><marker id="on1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="80" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">app, service</text><text class="m" x="110" y="64" text-anchor="middle">sender channel (NOTIFY</text><text class="m" x="110" y="80" text-anchor="middle">tag) with identity from init</text>
<path class="ln" d="M210 60 L288 60" marker-end="url(#on1)"/><text class="m" x="222" y="52">Send</text>
<rect class="acc" x="290" y="20" width="200" height="80" rx="6"/><text class="t b" x="390" y="45" text-anchor="middle">notifyd</text><text class="m" x="390" y="64" text-anchor="middle">identity, limits, Do not</text><text class="m" x="390" y="80" text-anchor="middle">disturb, per-user visibility</text>
<path class="ln" d="M490 60 L568 60" marker-end="url(#on1)"/><text class="m" x="500" y="52">ViewReply</text>
<rect class="ok" x="570" y="20" width="180" height="80" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">compd (comp::toast)</text><text class="m" x="660" y="64" text-anchor="middle">card at the top right,</text><text class="m" x="660" y="80" text-anchor="middle">notification centre</text>
<path class="ln" d="M660 100 L660 150 L390 150 L390 102" marker-end="url(#on1)"/><text class="m" x="430" y="142">click, close, Seen (View)</text>
<path class="ln" d="M290 90 L212 90" marker-end="url(#on1)"/><text class="m" x="222" y="110">action, closed</text>
<rect class="box" x="290" y="180" width="200" height="60" rx="6"/><text class="t b" x="390" y="205" text-anchor="middle">init</text><text class="m" x="390" y="224" text-anchor="middle">only one minting identities</text>
<path class="ln" d="M390 180 L390 102" marker-end="url(#on1)"/>
</svg>

- A **sender** gets its channel from init: an app at start with the
  `abi::params::tag::NOTIFY` tag, the identity from its manifest (id, name)
  and the session user; system services the `system` identity (user 0)
  **[verified in QEMU]** (test `oznameni: protokol: identita,
  viditelnost, akce, nahrazeni, Nerusit`).
- A **viewer** (the compositor for every session with the `notify <uid>`
  command, tests) sees the notifications of its user and system ones
  **[verified in QEMU]**.
- notifyd accepts control commands (`sender`, `viewer`, `status`) only
  from init on the bootstrap channel.

## The notifyproto protocol

Message = operation code `u32` + fields (`libs/wire`). An invalid message
returns `INVALID_ARGS`, the service keeps running **[verified in QEMU]**.

| Side | Message | Fields | Reply |
|---|---|---|---|
| sender | `NOTIFY` (1) | key u64, title, body, priority, actions (at most 3: id, label) | `SENT` (key, pops up) or `RESULT` with an error |
| sender | `CLOSE` (2) | key | - |
| to sender | `ACTION` (102) | key, action id (empty = click on the notification) | - |
| to sender | `CLOSED` (103) | key, reason (user, sender, limit) | - |
| viewer | `SUBSCRIBE` (10) | - | `SNAPSHOT` (110, 32 notifications each) and `DND` (113), then changes |
| viewer | `DISMISS` (11), `INVOKE` (12), `DISMISS_ALL` (13) | notification number, action | `RESULT` (100) |
| viewer | `SET_DND` (14) | end (wall time ns, 0 = off, `u64::MAX` = until turned off) | `RESULT`, `DND` to all viewers of the user |
| viewer | `SEEN` (15) | number | `RESULT`: the user saw the notification (the card went to the centre, the centre was opened); it will not pop up again, `seen` in the notification |
| to viewer | `ADDED` (111), `REMOVED` (112) | notification, or number and reason | - |

- The notification **key** is in the identity's namespace: the same key
  replaces the notification in place (download progress, `replaced`, the
  card is only redrawn), even after the app restarts **[verified in
  QEMU]**.
- **Priority:** `Low` only to the centre, `Normal` pops up for 6 s, `High`
  stays until closed, `Urgent` goes through Do not disturb and only a
  system identity may use it (an app's is lowered to `High`) **[verified
  in QEMU]**.
- **Texts** are shortened (title 120 characters, body 1000, action label
  40) and control characters removed; an empty title or two actions with
  the same id = `INVALID_ARGS` **[implemented]** (`cargo test -p
  notifycore`).

## Limits and Do not disturb

- At most **20 notifications per identity** and **200 per user**; the
  oldest makes room (`CLOSED` with reason limit) **[implemented]**.
- **Pop-up rate:** a bucket of 5 cards per identity, one refilled every
  3 s. Over the limit the notification goes quietly to the centre (spam
  does not fill the screen) **[implemented]** (`cargo test -p notifycore`).
- **Do not disturb** is per user (`SET_DND`): notifications go only to the
  centre and do not pop up even after it ends; `Urgent` does **[verified
  in QEMU]**.
- **Full screen** (the compositor's automatic Do not disturb,
  [GUI](gui.md)): the compositor does not show the card and sends `SEEN`,
  except `Urgent` **[implemented]** (test
  `fullscreen_dnd_keeps_quiet_except_urgent`).
- The login and lock screens do not draw cards (privacy).

## Pop-up cards

`libs/comp/src/toast.rs` **[implemented]** (`cargo test -p comp toast`):

- a card of the `size.notification_w` token width at the top right, slides
  in from the right (24 px in 240 milliseconds, easing), at most 3 cards stacked,
  others wait;
- content: app name, title, at most 2 body lines, action buttons (the
  first highlighted), a close cross; an AI agent card has a left stripe in
  the `ai` colour;
- `Normal` goes to the centre after 6 s (`SEEN`), `High` and `Urgent`
  stay;
- a click on the card = `INVOKE` without an action, on a button = `INVOKE`
  with the action, the cross = `DISMISS`. **Physical input only**: a
  synthetic click (`SYNTHETIC`) does not run a notification action (test
  `synthetic_click_does_not_invoke`).

API for the notification centre (bell panel, GUI): `Comp::notifications`,
`notify_dismiss`, `notify_invoke`, `notify_dismiss_all`, `notify_set_dnd`,
`notify_dnd`, `notify_unseen` (the unseen count for the badge) and
`notify_mark_seen` (the centre was opened) **[implemented]** (test
`unseen_count_and_mark_seen`). notifyd keeps the `seen` flag, so it
survives a compositor restart.

## Limitations

- notifyd state is only in memory: a crash of the service loses
  notifications (the supervisor watches it, restart **[designed]**); the
  compositor replaces its state after a new subscription.
- App icons on cards, notification sounds, grouping by app and history
  across restarts **[designed]**.
- Screen reader (the `status` region) **[designed]**.
