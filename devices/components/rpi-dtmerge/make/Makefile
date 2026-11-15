# dtmerge (raspberrypi/utils eac8944a) with libfdt (dtc v1.7.2), one static stripped program at $(OUT)/bin/dtmerge.
# Gaia builder "make": $(DEPS) holds the sources by name, $(OUT) is the build output; sandboxed and offline.
LIBFDT := fdt.c fdt_ro.c fdt_wip.c fdt_sw.c fdt_rw.c fdt_strerror.c fdt_empty_tree.c fdt_addresses.c fdt_overlay.c fdt_check.c
CFLAGS := -O2 -ffile-prefix-map=$(DEPS)=/deps -I$(DEPS)/dtc/libfdt -I$(DEPS)/utils/dtmerge

all: $(OUT)/bin/dtmerge

$(OUT)/bin/dtmerge: $(DEPS)/utils/dtmerge/dtmerge.c $(DEPS)/utils/dtmerge/dtoverlay.c $(addprefix $(DEPS)/dtc/libfdt/,$(LIBFDT))
	mkdir -p $(OUT)/bin
	cc $(CFLAGS) -static -s -o $@ $^
