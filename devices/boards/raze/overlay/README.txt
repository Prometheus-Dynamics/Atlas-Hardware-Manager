NOT COMMITTED: the Raze's device-tree overlay is generated, at build time, by `lemnos-ctl tree` from the
board file (boards/raze/lemnos-board.toml). Nothing here is hand-written and nothing is compiled.

The generator writes these outputs (PR B27 passes -o, --modprobe, --cmdline; --boot is not passed) (lemnos-ctl built from Lemnos d6b9361d, the L1 rev, and its SoC map
socs/bcm2712-rp1.toml; the lock is d6b9361d, PR B23, see Status):
  1. board-overlay.dts      the one overlay: i2c-1 (GPIO10/11), i2c-10 (GPIO38/39), the PWM1 block (fan, GPIO45),
                            the CSI receiver (owned by the module), the PiSP back end, the PIO block, the watchdog
                            node, the USB controller in peripheral mode, the camera regulator's base node disabled;
  2. boot.txt               the loader fragment: the lines that hold USB port power (GPIO16, GPIO20) and the fan
                            header (GPIO45) high from power-on, for config.txt. PR B27 does NOT pass --boot: Gaia has
                            no key for config.txt lines; Lemnos writes them in its own later steps;
  3. lemnos.conf            the modprobe.d file (options for the kernel module and the stock-module blacklists);
                            installed by the Raze's raze-lemnos-board component (PR B27).
  The command line (--cmdline) is a fourth output at PR B27 (cmdline.words); it has no consumer until Gaia's
  `[generate.provides]` rev. The overlay (-o) is declared too; its consumer comes with the same key.

Status (history): the file was written to Q2d (PR B20, docs/prs/B20.md). At the old lock 4a2d3b7d `lemnos-ctl tree` refused it at
the camera I2C bus (the map has no pin set gpio38_39 for i2c10 there; Q2d corrected the map). The lock moves to a
rev with Q2d in a later PR, then the generator output exists and is checked on the build. Not bootable into a
working board until then; the bench run is G1.
