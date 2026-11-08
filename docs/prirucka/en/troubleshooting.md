# When something does not work

The system is new, and some things may not work yet. Here is how to capture
what happened so we can fix it.

## 1. Diagnostic snapshot: `Ctrl+Alt+Shift+S`

When something on the screen behaves strangely (a window is drawn wrongly,
something does not respond), press **`Ctrl+Alt+Shift+S`**. The
**Print Screen** key works too (but over remote desktop or in a virtual
machine it often does not get through, so prefer the first one).

The system saves:

- a picture of the screen,
- a description of all windows and their state in the record (log).

You will find it in the folder `/system/log/` as `snimek-....bmp` (the
last 10 are kept). You can also take a snapshot from the palette: the
**Diagnostic snapshot** action.

⚠️ The snapshot contains what was on the screen. It is never sent anywhere
on its own. Look at it before you send it to anyone.

## 2. Report: the `report` command

Open the **Terminal** and type:

```text
report
```

This creates the file `/system/log/report-....txt` with a summary of the
system state and the latest records. Send it together with the snapshot and
a short description of what you were doing.

## 3. Records (logs)

| Where | What is there |
|---|---|
| `/system/log/boot-....log` | a record of the last 20 system starts |
| `/system/pady/` | records of crashes of the system and services |
| `/system/log/` | diagnostic snapshots and reports |

## System from USB (live USB)

When the system runs from a USB drive, it also saves copies of the records
to the visible partition **MO-LOGY** in the folder `MAGNUMOS`: `LOGxx.TXT`
(logs), `PADxx.TXT` (crashes), `REPORT.TXT` (report) and `SNIMxx.BMP`
(snapshots). Windows can read this partition too, so you can easily copy
the files and send them.

When the system from USB does not start at all: at startup, right after the
boot loader prompt, press the **L** key within 2 seconds. The system only
pulls the records out into the folder `ZACHRANA` on the MO-LOGY partition
and quits.

The live USB works only on computers with UEFI and Secure Boot turned off.
It only reads the computer's disks and never writes anything to them.

## When the system crashes

On a serious error, a screen appears with an error code (for example
`KERNEL_PANIC · #MO-7F3A-21C9`) and a QR code. The computer restarts by
itself after 60 seconds. Write the error code down or take a photo of it;
a crash record is saved, and after the next start you will find it in
`/system/pady/`.

## Where to send it

Send the snapshot, the report (`report-....txt` or `REPORT.TXT`) and a short
description to the project maintainer. Write what you were doing just before
the problem appeared.
