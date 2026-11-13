NOT COMMITTED: the Raze's device-tree overlay is generated, at build time, by `lemnos-ctl tree` from the
board file (components/raze-lemnos-board/files/board.toml). Nothing here is hand-written and nothing is compiled.

The generator writes three outputs (lemnos-ctl built from Lemnos 4a2d3b7d, the locked rev, and its SoC
map socs/bcm2712-rp1.toml; no output of the full file exists yet, see Status):
  1. board-overlay.dts      the one overlay: i2c-1 (GPIO10/11), the PWM1 block (fan, GPIO45), the PIO block,
                            the watchdog node, the USB controller in peripheral mode, and the boot-time pins;
  2. boot.txt               the loader fragment: the lines that hold the USB ports' power (GPIO16, GPIO20) high
                            from power-on, for config.txt;
  3. lemnos.conf            the modprobe.d file (options and blacklists).

Status: the generator REFUSES three devices of the board file at 4a2d3b7d (identity, watchdog, slots), so the
full file has no outputs yet (PR B18, "Lemnos refusals", the messages verbatim). The refusals are Lemnos Q2d, which
is not in 4a2d3b7d. Until Lemnos lands that, lemnosd on the image refuses to start, and the board has no overlay
for its devices. Not bootable into a working board until then.
