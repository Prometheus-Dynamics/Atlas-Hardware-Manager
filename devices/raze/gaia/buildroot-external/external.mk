# Linux extensions in linux/ are found by Buildroot. Package overrides in
# packages/ are staged into Buildroot by Gaia.

# Hash of the pinned Raspberry Pi kernel tarball. raspberrypicm5io_defconfig
# sets BR2_DOWNLOAD_FORCE_CHECK_HASHES, and the defconfig's own linux.hash does
# not cover this tarball. Adding the file here, instead of through
# BR2_GLOBAL_PATCH_DIR, works wherever this tree is mounted. LINUX_HASH_FILES is
# expanded when the download runs, after this file is read, and Buildroot
# accepts the tarball when any listed hash file matches it.
LINUX_HASH_FILES += $(BR2_EXTERNAL_RAZE_DEVICE_PATH)/linux/linux.hash

# With Mesa's EGL (gpu.toml), keep rpi-userland out of staging. On aarch64
# rpi-userland builds no EGL/GLES libraries, but its install still copies the
# old Broadcom EGL/GLES/KHR headers into staging, where they can overwrite
# Mesa's depending on build order; anything compiled against the sysroot
# (e.g. PhotonVision's libcamera GL driver) then sees the wrong headers.
# vcgencmd and friends are still installed to the target. The staging recipe
# expands this variable at build time, so redefining it here, after the
# package makefiles, takes effect.
ifeq ($(BR2_aarch64)$(BR2_PACKAGE_RPI_USERLAND)$(BR2_PACKAGE_MESA3D_OPENGL_EGL),yyy)
define RPI_USERLAND_INSTALL_STAGING_CMDS
	@echo "rpi-userland: not installed to staging (keeps Mesa's EGL/GLES headers)"
endef
endif

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
