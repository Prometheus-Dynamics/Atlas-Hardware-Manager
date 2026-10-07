################################################################################
#
# rpi-utils
#
################################################################################

# Raze package (staged by Gaia; not in Buildroot). Replaces vcgencmd from
# rpi-userland, which Buildroot 2026.08 removed. The repository has no
# release tags; this is master as of 2026-10-02.
RPI_UTILS_VERSION = e0484c848f9c9d1aecc7bd0738930db97bcf18df
RPI_UTILS_SITE = $(call github,raspberrypi,utils,$(RPI_UTILS_VERSION))
RPI_UTILS_LICENSE = BSD-3-Clause
RPI_UTILS_LICENSE_FILES = LICENCE
# Build only vcgencmd: it has its own CMakeLists.txt and no dependencies.
# The top-level project would also build pinctrl, piolib, kdtc, eeptools and
# others, which need libfdt and more.
RPI_UTILS_SUBDIR = vcgencmd

# vcmailbox too: the device package's usb-boot uses it to set a one-time
# boot order (set_reboot_order, Pi 5/CM5). It is one C file with no
# dependencies, so it is compiled directly rather than through its CMake.
define RPI_UTILS_BUILD_VCMAILBOX
	$(TARGET_CC) $(TARGET_CFLAGS) $(TARGET_LDFLAGS) \
		-o $(@D)/vcmailbox/vcmailbox $(@D)/vcmailbox/vcmailbox.c
endef
RPI_UTILS_POST_BUILD_HOOKS += RPI_UTILS_BUILD_VCMAILBOX

define RPI_UTILS_INSTALL_VCMAILBOX
	$(INSTALL) -D -m 0755 $(@D)/vcmailbox/vcmailbox $(TARGET_DIR)/usr/bin/vcmailbox
endef
RPI_UTILS_POST_INSTALL_TARGET_HOOKS += RPI_UTILS_INSTALL_VCMAILBOX

$(eval $(cmake-package))
