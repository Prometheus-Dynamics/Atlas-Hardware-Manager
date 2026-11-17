# ROCK 5C bring-up: what a person should see

DEV ONLY. Nothing here has booted on a board yet; this list is what the image is built to do.

## Before powering it

**Do not connect any of the board's USB ports to a PC.** This image enables no USB device port and has
no USB network. Every USB port on the board is a host port and supplies 5 V. Connect only devices
(a keyboard, a drive), never a computer. Reach the board over Ethernet.

Power it from its own supply, with the SD card in and an Ethernet cable to a network that has DHCP.

## In order

1. **Power LED on.** If not: power.
2. **A DHCP lease on Ethernet** (the board's one Ethernet device, whatever its name: eth0 or end0) within about a minute. Find the address on the router or DHCP
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

| State | Green LED (`led-0`, colour green, function POWER) | Blue LED (`led-1`, colour blue, function HEARTBEAT) | What it means |
|---|---|---|---|
| Power on, before the kernel | off | off | nothing has loaded |
| Kernel alive, before systemd starts | on | beats (the tree's heartbeat trigger) | the kernel runs |
| systemd up, the Ethernet has no lease | on | beats | userspace runs; no DHCP lease yet |
| the Ethernet (any Type=ether link) has its lease | on | 1 s blink | the Ethernet, the link you reach the board by, has its address |
| Hung (PID 1 stalled) | on | the last pattern keeps going (kernel triggers) | not visible as a hang; after about 22 s the watchdog resets the board and the cycle starts again |

USB with no configuration: this image has NO USB gadget and no USB network (no device port is enabled). Nothing is
configured on the USB ports, and they are host ports. Never connect a USB port of this board to a PC (see the warning above).

Reboot to the loader (no button): `reboot loader` writes 0xEF08A53C at offset 0x80 of a syscon at 0xfd588000 (assets/dts/rockchip/
rk3588s-rock-5c-loader.dts; that syscon is added there because no kernel node contains the register U-Boot reads, 0xfd588080,
U-Boot rk3588/Kconfig:418). The base 0xfd588000 was inferred from the register (no RK3588 TRM on disk); it is ESTABLISHED from
U-Boot's side: the node's register in the built dtb is 0xfd588080, U-Boot's own default. UNVERIFIED on a board.

Checked in the BUILT image (Gaia, 2026-11-14, from next be263ac1; not booted): the reboot-mode node is in the dtb with
`mode-loader = 0xef08a53c`, its register equals U-Boot v2026.10's ROCKCHIP_BOOT_MODE_REG default for this SoC, and
"Returning to boot ROM" is in u-boot-rockchip.bin, so CONFIG_SPL_ROCKCHIP_BACK_TO_BROM took. One effect of that option to
check at the first boot: U-Boot's `spl_boot_device()` then answers BOOTROM as the fallback when no device in
`u-boot,spl-boot-order` works, so the SPL returns to the ROM instead of trying MMC itself. Probably harmless. Still not
proven by anything: that the boot ROM enters its download mode after the reset.

## The watchdog, as the board will behave (UNVERIFIED on a board)

- Requested: `RuntimeWatchdogSec=16s` (components/bringup-alive/files/10-watchdog.conf). systemd pings every 8 s.
- RK3566, RK3588S, RK3576 (this board's SoC): the driver rounds 16 s UP to the smallest TOP that is at least 16 s
  (drivers/watchdog/dw_wdt.c:123-143, dw_wdt_find_best_top). The TOPs are 2^(16+i) cycles of a 24 MHz clock (dw_wdt.c:52-53;
  the clock is xin24m: clk-rk3568.c:1208, clk-rk3588.c:1060, clk-rk3576.c:727). TOP 12 is 11.2 s (below 16); TOP 13 is
  2^29 / 24 MHz = 22.4 s. **Effective: 22.4 s** after the last ping. A PID 1 stall of about 22 s resets the board.
- A733 (Cubie): sunxi_wdt.c:25 gives a 16 s hardware maximum (WDT_MAX_TIMEOUT). **Effective: 16 s, UNVERIFIED, never run.**
- `RebootWatchdogSec=2min`: longer than the hardware maximum on every SoC, so the watchdog core keeps it fed while the kernel
  runs (drivers/watchdog/watchdog_dev.c:79-125). The A733 needs the series patch sunxi-wdt-heartbeat for it to be served.

LED silkscreen: the dts gives each LED a colour and a function, not a silkscreen label. Read the board's own legend for the
names; the state table uses the dts names. The blue LED beats from kernel boot (`led-1` default trigger, set in the image's own
dts, rk3588s-rock-5c-loader.dts); the green LED is the power LED and stays on.
