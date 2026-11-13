# ROCK 4D bring-up: what a person should see

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
