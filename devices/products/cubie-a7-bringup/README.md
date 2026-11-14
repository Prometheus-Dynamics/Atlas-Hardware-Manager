# Radxa Cubie A7S bring-up: what a person should see

DEV ONLY. Nothing here has booted on a board yet. Planned, not built: no image was built or booted for this PR.
Untested on board. The base OS and the kernel are the sun60i family's (DockSeed's series; see families/sun60i/family.toml).

## Before powering it

Power the board from its own supply with the SD card in. Connect USB-C 1 (the device-capable port) to the PC with a
data cable for the USB network. Ethernet to a network with DHCP is the second way in.

## The signs (B31: the LEDs and the watchdog)

Two LEDs: `power-led` is the green PM2 LED (kept on); `status-led` is the blue PJ26 LED, which the bring-up dts sets to
the heartbeat trigger (board/assets/dts). The watchdog is armed by systemd (components/bringup-alive: RuntimeWatchdogSec
16 s, the A733 watchdog's hardware maximum; RebootWatchdogSec 2 min, served by the watchdog core through the series patch
sunxi-wdt-heartbeat).

| State | Green (`power-led`) | Blue (`status-led`) | What it means |
|---|---|---|---|
| Power on, before the kernel | off | off | nothing has loaded |
| Kernel alive, before systemd starts | on | beats (the bring-up dts's heartbeat) | the kernel runs |
| systemd up, usb0 has no address | on | beats | userspace runs; the gadget has not come up |
| usb0 has 10.55.0.1 (the link you reach the board by) | on | 1 s blink | the USB network link has its address |
| Hung (PID 1 stalled) | on | the last pattern keeps going (kernel triggers) | not visible as a hang; the watchdog resets the board |

## USB with no configuration

The USB-C 1 device port starts the configfs NCM function only (usb-gadget-network). The PC needs `cdc_ncm`. Nothing else
is configured; there is no ACM, mass storage or serial gadget.

## Reboot to the loader (no button)

NOT wired. The A733 has no reboot-mode node in these sources, and the DockSeed series has no reboot-to-FEL request
(a grep of the pinned series patches finds none). UNVERIFIED: a person would have to read the Radxa U-Boot tree
(radxa/u-boot at 9da4b3e) for a boot-mode or FEL request before this can be stated. See docs/prs/B31.md, point 2.

## In order (SSH)

1. The blue LED beats: the kernel is up.
2. The blue LED blinks once usb0 has 10.55.0.1: the USB network link is up. Then `ssh root@10.55.0.1` with the bench key
   (key only, no password). If the blink never starts: the gadget did not come up (no address on usb0).
3. The second way in is Ethernet (eth0 by DHCP).
