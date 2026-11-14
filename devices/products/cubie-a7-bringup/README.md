# Planned, not built. Cannot build today: the TF-A, U-Boot and kernel patch series this family names are applied by nothing.
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

| State | Green LED (`power-led`, PM2, green) | Blue LED (`status-led`, PJ26, blue) | What it means |
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

EXCUSED in the gate rule (named there). The A733 has no reboot-mode node in these sources, and the DockSeed series has no reboot-to-FEL request
(a grep of the pinned series patches finds none). UNVERIFIED: a person would have to read the Radxa U-Boot tree
(radxa/u-boot at 9da4b3e) for a boot-mode or FEL request before this can be stated. See docs/prs/B31.md, point 2.

## In order (SSH)

1. The blue LED beats: the kernel is up.
2. The blue LED blinks once usb0 has 10.55.0.1: the USB network link is up. Then `ssh root@10.55.0.1` with the bench key
   (key only, no password). If the blink never starts: the gadget did not come up (no address on usb0).
3. The second way in is Ethernet (eth0 by DHCP).

## The watchdog, as the board will behave (UNVERIFIED on a board)

- Requested: `RuntimeWatchdogSec=16s` (components/bringup-alive/files/10-watchdog.conf). systemd pings every 8 s.
- RK3566, RK3588S, RK3576 (this board's SoC): the driver rounds 16 s UP to the smallest TOP that is at least 16 s
  (drivers/watchdog/dw_wdt.c:123-143, dw_wdt_find_best_top). The TOPs are 2^(16+i) cycles of a 24 MHz clock (dw_wdt.c:52-53;
  the clock is xin24m: clk-rk3568.c:1208, clk-rk3588.c:1060, clk-rk3576.c:727). TOP 12 is 11.2 s (below 16); TOP 13 is
  2^29 / 24 MHz = 22.4 s. **Effective: 22.4 s** after the last ping. A PID 1 stall of about 22 s resets the board.
- A733 (Cubie): sunxi_wdt.c:25 gives a 16 s hardware maximum (WDT_MAX_TIMEOUT). **Effective: 16 s, UNVERIFIED, never run.**
- `RebootWatchdogSec=2min`: longer than the hardware maximum on every SoC, so the watchdog core keeps it fed while the kernel
  runs (drivers/watchdog/watchdog_dev.c:79-125). The A733 needs the series patch sunxi-wdt-heartbeat for it to be served.

