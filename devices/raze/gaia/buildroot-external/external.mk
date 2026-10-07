# Linux extensions in linux/ are found by Buildroot. Package overrides in
# packages/ are staged into Buildroot by Gaia.

# Hash of the pinned Raspberry Pi kernel tarball. raspberrypicm5io_defconfig
# sets BR2_DOWNLOAD_FORCE_CHECK_HASHES, and the defconfig's own linux.hash does
# not cover this tarball. Adding the file here, instead of through
# BR2_GLOBAL_PATCH_DIR, works wherever this tree is mounted. LINUX_HASH_FILES is
# expanded when the download runs, after this file is read, and Buildroot
# accepts the tarball when any listed hash file matches it.
LINUX_HASH_FILES += $(BR2_EXTERNAL_RAZE_DEVICE_PATH)/linux/linux.hash

# The Raze kernel fragment goes after any the OS lists in
# BR2_LINUX_KERNEL_CONFIG_FRAGMENT_FILES, so the package's decisions win
# (later fragments override earlier ones). LINUX_KCONFIG_FRAGMENT_FILES is a
# recursive variable that linux.mk defines before this file is read.
LINUX_KCONFIG_FRAGMENT_FILES += $(BR2_EXTERNAL_RAZE_DEVICE_PATH)/linux/raze.config

# Fail rather than ship a kernel whose core configuration drifted: the page
# size (4 KiB) and a built-in EROFS are package decisions (raze.config,
# kernel.toml).
define RAZE_CHECK_KERNEL_CONFIG
	@grep -qx 'CONFIG_ARM64_4K_PAGES=y' $(LINUX_DIR)/.config || \
		{ echo "Raze: the kernel must use 4 KiB pages (CONFIG_ARM64_4K_PAGES, BR2_ARM64_PAGE_SIZE_4K); something changed it" >&2; exit 1; }
	@grep -qx 'CONFIG_EROFS_FS=y' $(LINUX_DIR)/.config || \
		{ echo "Raze: EROFS must be built in (CONFIG_EROFS_FS=y)" >&2; exit 1; }
endef
LINUX_POST_CONFIGURE_HOOKS += RAZE_CHECK_KERNEL_CONFIG

# The CM5 defconfig uses Bootlin's external toolchain, whose tools are named
# aarch64-linux-*. OpenJDK's configure only looks for $(GNU_TARGET_NAME)-*
# (aarch64-buildroot-linux-gnu-*), and when it finds none it silently falls
# back to the host's objcopy/strip; jlink then fails to strip the aarch64
# binaries. Hand it the target tools explicitly, as configure arguments: it
# ignores these variables when they come from the environment. The configure
# recipe expands OPENJDK_CONF_OPTS at build time, so appending here takes
# effect.
ifeq ($(BR2_PACKAGE_OPENJDK),y)
OPENJDK_CONF_OPTS += \
	OBJCOPY=$(TARGET_OBJCOPY) \
	STRIP=$(TARGET_STRIP) \
	NM=$(TARGET_NM) \
	AR=$(TARGET_AR)
endif

# The stock overlays raze-device.txt loads are built from the kernel being
# built, not taken from rpi-firmware: the firmware release carries overlays
# from its own kernel series (1.20260915: 6.18), and 7.2 changed some of
# them (ws2812-pio no longer sets the pin function). boot.toml copies them
# from images/raze-overlays/.
RAZE_KERNEL_OVERLAYS = dwc2 i2c1-pi5 i2c-gpio ws2812-pio vc4-kms-v3d-pi5

ifeq ($(BR2_LINUX_KERNEL_EXT_OV9782),y)
define RAZE_BUILD_KERNEL_OVERLAYS
	$(LINUX_MAKE_ENV) $(BR2_MAKE) $(LINUX_MAKE_FLAGS) -C $(LINUX_DIR) \
		$(foreach o,$(RAZE_KERNEL_OVERLAYS),overlays/$(o).dtbo)
endef
LINUX_POST_BUILD_HOOKS += RAZE_BUILD_KERNEL_OVERLAYS

define RAZE_INSTALL_KERNEL_OVERLAYS
	$(INSTALL) -d $(BINARIES_DIR)/raze-overlays
	$(foreach o,$(RAZE_KERNEL_OVERLAYS),\
		$(INSTALL) -m 0644 $(LINUX_ARCH_PATH)/boot/dts/overlays/$(o).dtbo \
			$(BINARIES_DIR)/raze-overlays/$(o).dtbo$(sep))
endef
LINUX_POST_INSTALL_IMAGES_HOOKS += RAZE_INSTALL_KERNEL_OVERLAYS
endif
