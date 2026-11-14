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
   `cdc_ncm` driver, and `ip link` shows a new `usb0`/`enx…` interface. `lsusb` shows vendor `1d6b`, product `0103`
   (Gaia's bring-up USB network; `0104` is the managed gadget, not this one: components/usb-gadget-network at 7177d72 presents 1d6b:0103). If it does not appear: the controller is not up (check the cable is a data cable and that
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

## What the signs say (B31: the LED and the watchdog)

Planned, not built: no image was built or booted for this PR. The LED is `led-green` (rk3566-radxa-zero-3.dtsi:29, the only
LED). The watchdog is armed by systemd (components/bringup-alive: RuntimeWatchdogSec 16 s; a hang of PID 1 resets the board).

| State | Green LED (`led-green`, colour green, the only LED) | What it means |
|---|---|---|
| Power on, before the kernel | off | nothing has loaded |
| Kernel alive, before systemd starts | beats (the kernel's own heartbeat trigger, from the tree's dts) | the kernel runs |
| systemd up, usb0 has no address | beats | userspace runs; the gadget has not come up |
| usb0 has 10.55.0.1 (the link you reach the board by) | 1 s blink | the USB network link has its address |
| Hung (PID 1 stalled) | the last pattern keeps going: the triggers run in the kernel | not visible as a hang; after about 22 s the watchdog resets the board, the LED goes off, and it starts again at "kernel alive" |

USB with no configuration: the device port (USB-C 1, dr_mode peripheral, gadget dts) starts the configfs NCM function only
(usb-gadget-network). The PC needs `cdc_ncm`. No ACM, no mass storage, no serial gadget. Nothing to configure.

Reboot to the loader (no button): `reboot loader` writes 0xEF08A53C to the boot-mode register at pmugrf offset 0x200 (the node is in
assets/dts/rockchip/rk3566-radxa-zero-3e-gadget.dts; that file is marked for removal, and the node moves with it). U-Boot's SPL returns to
the BootROM download for that value (CONFIG_SPL_ROCKCHIP_BACK_TO_BROM, families/rk35xx/assets/u-boot.fragment). UNVERIFIED on a board.

## The watchdog, as the board will behave (UNVERIFIED on a board)

- Requested: `RuntimeWatchdogSec=16s` (components/bringup-alive/files/10-watchdog.conf). systemd pings every 8 s.
- RK3566, RK3588S, RK3576 (this board's SoC): the driver rounds 16 s UP to the smallest TOP that is at least 16 s
  (drivers/watchdog/dw_wdt.c:123-143, dw_wdt_find_best_top). The TOPs are 2^(16+i) cycles of a 24 MHz clock (dw_wdt.c:52-53;
  the clock is xin24m: clk-rk3568.c:1208, clk-rk3588.c:1060, clk-rk3576.c:727). TOP 12 is 11.2 s (below 16); TOP 13 is
  2^29 / 24 MHz = 22.4 s. **Effective: 22.4 s** after the last ping. A PID 1 stall of about 22 s resets the board.
- A733 (Cubie): sunxi_wdt.c:25 gives a 16 s hardware maximum (WDT_MAX_TIMEOUT). **Effective: 16 s, UNVERIFIED, never run.**
- `RebootWatchdogSec=2min`: longer than the hardware maximum on every SoC, so the watchdog core keeps it fed while the kernel
  runs (drivers/watchdog/watchdog_dev.c:79-125). The A733 needs the series patch sunxi-wdt-heartbeat for it to be served.
