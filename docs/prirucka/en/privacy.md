# Privacy and permissions

## Trust badge

On the left of every window title is the **trust badge**: a shield, the
name of the app and icons of what the app is allowed to do (network,
folders, camera). The system provides the name, not the app, and the app
cannot redraw the badge. System apps have the word **System** in the badge.

## System dialogs

When the system needs your decision (for example to allow a new device),
it shows a **system dialog** over a dimmed screen. You can recognize it by
the **System** strip and your **security phrase**. A dialog without the
phrase is a fake.

- Change the choice with `Tab` or the arrows, `Enter` confirms, `Esc` declines.
- The dialog reacts only to a real keyboard and mouse.
- For the first 0.3 seconds after it appears it ignores key presses, so you
  do not confirm it by accident.

## A new keyboard or mouse

When you plug in a new input device (USB), the system asks **Allow device?**.
Until you decide, the device types nothing. This protects you from fake
"keyboards" that would type commands on their own.

## Secure screen: `Ctrl+Alt+Delete`

Only the system catches this shortcut, and no app can fake it. It opens a
system screen. You can close it only with a real keyboard or mouse.

## AI agent and automation

When an AI agent is doing something in the system, a coral tag appears in
the bar next to tasks. When a program controls the keyboard or mouse, the
bar shows **"Input controlled by automation"**.

Camera and microphone indicators, in-window questions about network access,
and an overview of permissions with the option to remove them are
*coming soon*.
