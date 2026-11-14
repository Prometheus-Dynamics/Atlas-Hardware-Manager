# ROCK 5C bring-up: what a person should see

DEV ONLY. Nothing here has booted on a board yet; this list is what the image is built to do.

## Before powering it

**Do not connect any of the board's USB ports to a PC.** This image enables no USB device port and has
no USB network. Every USB port on the board is a host port and supplies 5 V. Connect only devices
(a keyboard, a drive), never a computer. Reach the board over Ethernet.

Power it from its own supply, with the SD card in and an Ethernet cable to a network that has DHCP.

## In order

1. **Power LED on.** If not: power.
2. **A DHCP lease on Ethernet** (eth0) within about a minute. Find the address on the router or DHCP
   server. If there is none: the kernel did not start or the Ethernet driver did not bind; there is no
   USB network to fall back on, and the serial console (the debug UART) is the only other way in.
3. **SSH as root with the bench key** to that address (key only, no password). If the lease is there
   but SSH is refused: the SSH server or its host key on the data partition did not come up.

## What this image does not have

- No USB gadget and no USB network. The board's device-mode port is not enabled; its entry in the
  board's Lemnos file stays commented out until the VBUS check with a meter has been done on a board.
- No Lemnos and no Orion.

## What the signs say (B31: the LEDs and the watchdog)

Planned, not built: no image was built or booted for this PR. Two LEDs: `led-0` is the green power LED, `led-1` is the blue
heartbeat LED (rk3588s-rock-5c.dts:50-65). `led-1` is the one this image's signs use, and the watchdog is armed by systemd
(components/bringup-alive: RuntimeWatchdogSec 16 s).

| State | Green (`led-0`) | Blue (`led-1`) | What it means |
|---|---|---|---|
| Power on, before the kernel | off | off | nothing has loaded |
| Kernel alive, before systemd starts | on | beats (the tree's heartbeat trigger) | the kernel runs |
| systemd up, eth0 has no lease | on | beats | userspace runs; no DHCP lease yet |
| eth0 has its lease | on | 1 s blink | the link you reach the board by has its address |
| Hung (PID 1 stalled) | on | the last pattern keeps going (kernel triggers) | not visible as a hang; after about 22 s the watchdog resets the board and the cycle starts again |

USB with no configuration: this image has NO USB gadget and no USB network (no device port is enabled). Nothing is
configured on the USB ports, and they are host ports. Never connect a USB port of this board to a PC (see the warning above).

Reboot to the loader (no button): NOT wired on this board. The reboot-mode node is not in the tree for RK3588S and the
PMU GRF base U-Boot uses (0xfd588000, U-Boot rk3588/Kconfig:418) is not a node in the kernel's rk3588 dtsi. See docs/prs/B31.md.
