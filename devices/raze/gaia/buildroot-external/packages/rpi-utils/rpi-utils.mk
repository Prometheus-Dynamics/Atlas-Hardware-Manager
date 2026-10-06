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

$(eval $(cmake-package))
