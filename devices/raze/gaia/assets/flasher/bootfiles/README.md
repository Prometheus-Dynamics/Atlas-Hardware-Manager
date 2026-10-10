# raze-flasher's USB boot files

`2712/` is Raspberry Pi's BCM2712 second-stage bootloader and its data, as
the CM5's boot ROM asks for them during USB boot: byte for byte the
`2712/` entries of `firmware/bootfiles.bin` at raspberrypi/usbboot
`f905f2f5a92e086defa9607f8fd67633d96d5bd6`, the commit Atlas pins for its
own USB boot files (`scripts/fetch-usbboot-files.sh`). Raspberry Pi's
firmware, redistributable under its licence (raspberrypi/usbboot
`LICENSE`). Update them together with that pin.

SHA-256 (first 16 hex digits):

    bootcode5.bin  5f03b688c5c92d9d
    bootmain       12bfeae3fef82b34
    font.bin       36005b3c21bbec2f
    logo.bin       c588911933cd90ee
    mcb.bin        b9e8e49b7318cf43
    memsys00.bin   6ed473128233970c
    memsys01.bin   0a33130bf3dc6166
    memsys02.bin   3a777450d7126f0d
    memsys03.bin   ce23a54f5b6d01d4
