################################################################################
# Linux extension: add OV9782 support to the upstream OV9282 driver
#
# OV9782 is handled as a variant of drivers/media/i2c/ov9282.c.  Do not stage
# or build the old standalone ov9782.c driver from this extension.
################################################################################

OV9782_EXT_DIR := $(dir $(lastword $(MAKEFILE_LIST)))

define OV9782_PREPARE_KERNEL
	@echo "[ov9782] patching OV9282 driver with OV9782 variant support"
	@if ! grep -q 'ovti,ov9782' $(LINUX_DIR)/drivers/media/i2c/ov9282.c; then \
		patch -d $(LINUX_DIR) -p1 < $(OV9782_EXT_DIR)/ov9782/0001-media-i2c-ov9282-add-ov9782-variant-draft.patch; \
	else \
		echo "[ov9782] OV9282 driver already has OV9782 variant support"; \
	fi
endef

LINUX_PRE_PATCH_HOOKS += OV9782_PREPARE_KERNEL

# Ensure the shared OV9282 symbol is enabled after kernel configuration is generated.
define OV9782_ENABLE_CONFIG
	@if [ -x $(LINUX_DIR)/scripts/config ]; then \
		$(LINUX_DIR)/scripts/config --file $(LINUX_DIR)/.config --module VIDEO_OV9282; \
	else \
		$(MAKE) -C $(LINUX_DIR) $(LINUX_MAKE_FLAGS) scripts; \
		$(LINUX_DIR)/scripts/config --file $(LINUX_DIR)/.config --module VIDEO_OV9282; \
	fi
	$(LINUX_DIR)/scripts/config --file $(LINUX_DIR)/.config --disable VIDEO_OV9782
	$(MAKE) -C $(LINUX_DIR) $(LINUX_MAKE_FLAGS) olddefconfig
endef

LINUX_POST_CONFIGURE_HOOKS += OV9782_ENABLE_CONFIG$(sep)
