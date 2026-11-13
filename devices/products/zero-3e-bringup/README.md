# ZERO 3E bring-up: what a person should see

DEV ONLY. Nothing here has booted on a board yet; this list is what the image is built to do. Untested on board.

## Before powering it

Power the board from its own supply (USB-C is also its power input; the OTG port is the one that talks to
the PC). An Ethernet cable to a network with DHCP is optional: it is the second way in.

Connect the board's USB-C (the OTG port) to a PC with a data cable (not a power-only cable).

## In order

1. **Power on.** The green LED on the board comes on, and the status LED starts blinking once the kernel is up
   (the heartbeat). If the heartbeat never starts: the kernel did not start (before the kernel there is no sign).
2. **A new network interface on the PC.** It is a **CDC NCM** device: on Linux the kernel binds it with the
   `cdc_ncm` driver, and `ip link` shows a new `usb0`/`enx…` interface. `lsusb` shows vendor `1d6b`, product `0104`
   ("USB network"). If it does not appear: the controller is not up (check the cable is a data cable and that
   it is on the OTG port), or the USB network unit failed on the board (no way to see that from the PC).
   Windows and macOS do not bind CDC NCM natively; a missing interface there is not a board fault (not checked).
3. **The PC's address on that interface is `62:3a:62:a8:c9:a6`.** That is the host address derived from the board's
   id `radxa-zero-3e` and its serial `b3d1ffecafae5cec` (the rule in `scripts/`'s check and in Gaia's
   `usb-gadget-network`, PR B24). Check with `ip link show <interface>`. The board's own address for the
   same link is `62:3a:62:a8:c9:a7`. If the PC's address is different, the serial did not fit on the board
   and the addresses are random for this boot (the board says so in its log: `usb0: ... random`).
4. **A DHCP lease on the PC: `10.55.0.2`** from the board, which is at `10.55.0.1` (udhcpd on the board, from the
   `usb-gadget-network` unit). The PC's network profile for the interface should be set to DHCP. If there is no
   lease within about a minute: udhcpd is not running on the board (the unit failed), or the PC's interface is
   down. Set `10.55.0.2/24` by hand on the PC and ping `10.55.0.1`; if that answers, the DHCP server is the fault.
5. **SSH as root with the bench key** to `10.55.0.1` (over the USB network) or to the Ethernet lease. Key only,
   no password. If the network is there but SSH is refused: dropbear or its host key on the data partition did
   not come up.

## Stable addresses

When the serial fits (lowercase hex, 8 to 32 digits: the ZERO 3E's `b3d1ffecafae5cec` does), the two addresses
above are the same on every boot and after a restart, so the PC's network profile re-attaches. They are
derived from the board's id and serial, not from fixed values in the kernel command line.

## What this image does not have

- No serial console is attached for the bring-up; the debug UART is the only other way in and needs an adapter.
- No Lemnos and no Orion. The managed image's USB gadget (Lemnos) is a different gadget with its own addresses.
- No legacy `g_ether` (the built-in network gadget that bound the controller before userspace): the network is the
  configfs NCM function, started by the unit.
- No networkd DHCP server on the USB network: udhcpd is the one DHCP server on it.
