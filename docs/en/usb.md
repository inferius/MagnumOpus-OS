# USB

USB is handled by three kinds of processes that talk over channels:

- **`xhci`**: the xHCI controller driver and the USB core. It sets up
  devices (address, descriptors, configuration, endpoints) and performs
  transfers. It is the only one with access to the controller.
- **`usb-hub`**: the hub class driver. It serves the controller's root
  ports (a virtual root hub) as well as external hubs.
- **`usb-hid`**: the keyboard and mouse driver. It sends events to the
  [input service](input.md) the same way virtio-input and PS/2 do.

Class drivers have no Resource at all. They get only the channel of their
interface and see neither the controller, nor memory, nor other devices. A
bug or malice in one class driver therefore cannot endanger the others.

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="USB processes">
<defs><marker id="us1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="160" height="70" rx="6"/><text class="t b" x="90" y="45" text-anchor="middle">xHCI controller</text><text class="m" x="90" y="64" text-anchor="middle">PCI 0C/03/30</text><text class="m" x="90" y="80" text-anchor="middle">ports, DMA, MSI-X</text>
<rect class="acc" x="220" y="20" width="200" height="90" rx="6"/><text class="t b" x="320" y="45" text-anchor="middle">xhci (process)</text><text class="m" x="320" y="64" text-anchor="middle">rings, slots, enumeration</text><text class="m" x="320" y="80" text-anchor="middle">virtual root hub</text><text class="m" x="320" y="96" text-anchor="middle">Resource of this controller only</text>
<path class="ln" d="M170 55 L218 55" marker-end="url(#us1)"/>
<rect class="box" x="480" y="20" width="130" height="56" rx="6"/><text class="t b" x="545" y="43" text-anchor="middle">devmgr</text><text class="m" x="545" y="62" text-anchor="middle">starts drivers</text>
<path class="ln" d="M420 45 L478 45" marker-end="url(#us1)"/><text class="m" x="424" y="38">usb-interface</text>
<rect class="box" x="220" y="180" width="160" height="70" rx="6"/><text class="t b" x="300" y="205" text-anchor="middle">usb-hub</text><text class="m" x="300" y="224" text-anchor="middle">root hub and external</text><text class="m" x="300" y="240" text-anchor="middle">port reset, hotplug</text>
<rect class="box" x="430" y="180" width="160" height="70" rx="6"/><text class="t b" x="510" y="205" text-anchor="middle">usb-hid</text><text class="m" x="510" y="224" text-anchor="middle">boot protocol</text><text class="m" x="510" y="240" text-anchor="middle">keyboard, mouse</text>
<rect class="ok" x="630" y="180" width="120" height="70" rx="6"/><text class="t b" x="690" y="210" text-anchor="middle">inputd</text><text class="m" x="690" y="229" text-anchor="middle">IDEV / IEVT</text>
<path class="ln" d="M300 112 L300 178" marker-end="url(#us1)"/><text class="m" x="306" y="150">usbproto (hub channel)</text>
<path class="ln" d="M380 112 L480 178" marker-end="url(#us1)"/><text class="m" x="430" y="140">usbproto</text>
<path class="ln" d="M545 78 L520 176" marker-end="url(#us1)"/><path class="ln" d="M545 78 L320 176" marker-end="url(#us1)"/>
<path class="ln" d="M590 215 L628 215" marker-end="url(#us1)"/>
<text class="m" x="10" y="290">Class drivers only have the channel of their interface: no Resource, no DMA, no other devices.</text>
<text class="m" x="10" y="310">usb-hub sees the controller's root ports as an ordinary hub, so device discovery and hotplug work the same everywhere.</text>
</svg>
<figcaption>USB processes: controller, USB core, class drivers and the input service.</figcaption>
</figure>

## Start-up and device discovery

1. `devmgr` finds a class 0C/03/30 controller on PCI and starts `xhci`. It
   gives it a restricted Resource (BARs, configuration, interrupts, DMA)
   and a channel back.
2. `xhci` takes the controller over from the firmware (USB Legacy Support,
   so the firmware stops emulating PS/2), resets it and sets up the
   contexts, the command ring and the event ring. Then it announces the
   **virtual root hub** with `usb-interface 09 00 00 0000 0000` and a
   channel.
3. `devmgr` starts `usb-hub`. It powers the ports and walks them. On a port
   with a device attached it resets the port (a USB 3 port enables itself)
   and then tells `xhci` to set the device up (`ATTACH`).
4. `xhci` sets the device up:
   - Enable Slot and Address Device,
   - the first 8 bytes of the device descriptor (EP0 packet size, with
     Evaluate Context if needed), the full descriptor and the whole
     configuration,
   - SET_CONFIGURATION and Configure Endpoint for the interrupt endpoints.

   It then announces every interface: `usb-interface <class> <subclass>
   <protocol> <vid> <pid>`.
5. `devmgr` starts `usb-hub` (09) or `usb-hid` (03) by class. `usb-hid`
   also gets a channel to the input service, whose other end goes to init
   in an `input` message.
6. After the first pass over the ports `usb-hub` sends `SCAN_DONE`, `xhci`
   sends `usb-ready`, and only then does `devmgr` send init `done`. So the
   devices attached at boot are in the input service before init says
   `settled`.

Boot log:

```text
xhci 00:08.0: 8 portu (4 USB 3), 64 slotu, kontext 32 B
usb-hub: root hub, 8 portu
xhci: port 5: zarizeni 0627:0001 (high speed), slot 1
usb-hid: USB klavesnice 0627:0001 (port [5], high speed)
inputd: zarizeni 'USB klavesnice 0627:0001' pripojeno
```

## The virtual root hub

`xhci` presents the root ports as a USB 2.0 hub. It answers hub class
requests and translates them to the `PORTSC` registers:

| Request | What it does |
|---|---|
| `GET_DESCRIPTOR(HUB)` | the controller's port count, per-port power |
| port `GET_STATUS` | `wPortStatus` from `PORTSC`: CCS, PED, OCA, PR, PP, speed (low / high; super speed in reserved bit 15); `wPortChange` from CSC, PEC, OCC, PRC+WRC |
| `SET_FEATURE(PORT_POWER / PORT_RESET)` | sets PP / PR |
| `CLEAR_FEATURE(C_PORT_*)` | clears the change (RW1C), keeps the other bits |
| `CLEAR_FEATURE(PORT_ENABLE)` | disables the port (PED) |
| `INTERRUPT_IN` on 0x81 | waits for a port change and replies with a bitmap (bit N = port N) |

That is why `usb-hub` has a single code path for root and external hubs.
Hotplug works the same way: a port change completes the wait on 0x81 and
the hub driver walks the port again.

## The `usbproto` protocol

A request is a 16-byte header (`op`, `id`, `a`, `b`, all u32) plus data.
A reply is `id`, `status` (0 or an [error](errors.md) code), `completion`
(the xHCI completion code), `actual` (bytes transferred) and data. All
little-endian. Every request gets one reply with the same `id`, but the
order of replies may differ from the order of requests. At most one request
may wait on one endpoint, otherwise `ALREADY_EXISTS`.

| `op` | Who | Meaning |
|---|---|---|
| 1 `INFO` | all | speed, interface number, port path, device descriptor, the whole configuration |
| 2 `CONTROL` | all | control transfer on EP0: data = 8-byte setup + OUT data; reply = IN data. `SET_ADDRESS` and `SET_CONFIGURATION` = `ACCESS_DENIED` (the controller handles them) |
| 3 `INTERRUPT_IN` | all | `a` = endpoint (e.g. 0x81), `b` = byte limit; the reply comes when data arrives. Only endpoints of the caller's own interface, otherwise `ACCESS_DENIED` |
| 4 `INTERRUPT_OUT` | all | `a` = endpoint, data = payload |
| 16 `ATTACH` | hub | a device was attached to port `a` (the port is enabled after reset); reply `actual` = slot |
| 17 `DETACH` | hub | the device on port `a` is gone: the controller closes its interface channels and releases the slot |
| 18 `SCAN_DONE` | hub | the first pass over the ports is complete |

A transfer error is `status = IO_ERROR`, and `completion` says why, e.g.
`6 = STALL` when the device does not support the request. `xhci` recovers
a halted endpoint itself (Reset Endpoint, Set TR Dequeue). Class drivers
wrap the requests with the `usbclient` library:

```rust
let mut usb = UsbClient::new(channel);
let info = usb.info()?;
usb.control(hid::set_boot_protocol(info.interface), &[])?;
let report = usb.call(op::INTERRUPT_IN, 0x81, 8, &[], None)?;
```

## HID: keyboard and mouse (boot protocol)

`usb-hid` switches the interface to the boot protocol (`SET_PROTOCOL(0)`)
and turns off report repeating (`SET_IDLE(0)`, the input service repeats
keys). Then it reads the interrupt endpoint. A STALL on these requests does
no harm, because the device already uses the boot protocol.

- **Keyboard**, 8-byte report: modifiers (bit 0 left Ctrl ... bit 7 right
  Windows key), reserved, up to 6 pressed keys (HID usages of page 0x07).
  Events come from the difference against the previous report: releases
  first, then presses. A "rollover" report (usage 1-3, too many keys) is
  ignored. `MSC_SCAN` = `0x70000 | usage`, so for example `u` has scan code
  `0x70018`.
- **Mouse**, 3-4 byte report: buttons (5), dx, dy (i8, Y positive down as
  in evdev), wheel (i8, positive up).
- **Unplugging:** the interface channel closes, the driver sends a release
  for every held key and button and exits. The input service then removes
  the source.

The usage-to-evdev mapping and the report differencing live in the
`usbproto::hid` library, tested on the host.

## Verifying a new keyboard (against BadUSB)

A USB device can pretend to be a keyboard and type commands by itself
(BadUSB, "Rubber Ducky"). The **security option** "verify new keyboards"
stops this:

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="New keyboard verification">
<defs><marker id="us2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="40" width="170" height="70" rx="6"/><text class="t b" x="95" y="65" text-anchor="middle">new keyboard</text><text class="m" x="95" y="84" text-anchor="middle">attached after boot</text><text class="m" x="95" y="100" text-anchor="middle">(settled)</text>
<rect class="warn" x="250" y="40" width="200" height="70" rx="6"/><text class="t b" x="350" y="65" text-anchor="middle">pending verification</text><text class="m" x="350" y="84" text-anchor="middle">input only to the secure channel</text><text class="m" x="350" y="100" text-anchor="middle">no shortcut, no repeat</text>
<rect class="ok" x="540" y="10" width="210" height="56" rx="6"/><text class="t b" x="645" y="33" text-anchor="middle">trusted (trust)</text><text class="m" x="645" y="52" text-anchor="middle">input to the focus owner</text>
<rect class="box" x="540" y="90" width="210" height="56" rx="6"/><text class="t b" x="645" y="113" text-anchor="middle">rejected (reject)</text><text class="m" x="645" y="132" text-anchor="middle">input is dropped</text>
<path class="ln" d="M180 75 L248 75" marker-end="url(#us2)"/>
<path class="ln" d="M450 65 L538 40" marker-end="url(#us2)"/>
<path class="ln" d="M450 90 L538 115" marker-end="url(#us2)"/>
<text class="m" x="10" y="180">A system dialog shows a code; the user types it on the new keyboard (only the system sees it), then the dialog sends trust.</text>
</svg>
<figcaption>A new keyboard gets a say only after the system has verified it.</figcaption>
</figure>

1. After boot, init sends `inputd` the `settled` command. Keyboards
   attached up to that moment are trusted, because the user had them
   plugged in at power-on.
2. With the option on (`policy verify-new-keyboards on`), every further
   **physical** keyboard gets the `pending` state. The focus owner and the
   secure channel get `NewKeyboard { source, Pending }`. A client may show,
   for example, "a new keyboard was connected".
3. All events of that keyboard go **only to the secure channel**, not to
   the focus owner. The secure attention shortcut does not work from it,
   and neither does key repeat.
4. The system shows its own dialog (drawn by the compositor in a layer
   applications cannot paint over) with a random code. The user types the
   code on the new keyboard. The dialog sees it, because the input goes to
   `secure`, and sends `trust <id>`. If they do not match, it sends
   `reject <id>`, and the keyboard's input is dropped from then on.
5. After the decision comes `NewKeyboard { source, Trusted / Rejected }`.

A device that merely pretends to be a keyboard does not know the code. It
types nothing until it guesses it. The option belongs to the
[security profiles](fs-v3-design.md#defaults-and-profiles):

| Profile | Default |
|---|---|
| Strict | on |
| Standard | option (off by default) |
| Relaxed | off |

It can also be switched on on its own in a custom security option. The
mechanism is done in `inputd`. The dialog with the code comes with the GUI
(6c). Until then the test verifies directly with a `trust` message.

## Tests

| Where | What |
|---|---|
| `cargo test -p usbproto` | descriptors (configuration, alternate settings, broken lengths), setup packets, messages, hub (descriptor, port status, speed), HID boot (report differencing, rollover, usage table, mouse) |
| `cargo xtask test`, suite `usb` | the USB keyboard from boot is trusted; keys from QEMU all the way to the service (source, HID scan code); mouse hotplug (attach, motion, wheel, detach); a new keyboard waits for verification (input only on `secure`, after `trust` to the focus owner) |

QEMU has `qemu-xhci` with a `usb-kbd` keyboard bound to the console of a
second graphics card (`secondary-vga,id=video1`). Xtask sends it events
over QMP with `device=video1`. The mouse and the second keyboard are
attached by the test at run time (`device_add`, `device_del`). A QEMU USB
mouse cannot be bound to a console and would take the events meant for
PS/2.

## Limits and next phases

| Phase | State | What |
|---|---|---|
| 1 | done | xHCI controller: takeover from the firmware, reset, contexts, rings, MSI-X, root ports, control and interrupt transfers |
| 2 | done | HID boot protocol (keyboard, mouse), hotplug, new keyboard verification |
| 3 | plan | external hubs: route string, hub bit, port count and the TT for low/full speed behind a high-speed hub in the slot context. The hub driver is ready and the controller now refuses `ATTACH` from an external hub |
| 4 | plan | a full HID report descriptor parser (multimedia keys, mice with high-resolution wheels, tablets: extending `inputproto` with `EV_ABS`) |
| 5 | plan | I2C-HID (laptop touchpads: needs AML and an I2C driver) |

Other limits of the first version:

- no bulk or isochronous endpoints yet (disks, audio), one interrupter,
- 4 KiB pages only, interrupts target the boot CPU,
- the USB keyboard LEDs (`SET_REPORT`) stay off for now,
- devices attached at run time are passed by init to the input service
  only during the test (`pump_hotplug`). The init event loop (supervisor)
  will do it permanently.
