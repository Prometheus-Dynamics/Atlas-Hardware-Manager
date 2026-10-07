################################################################################
#
# board-image-slots
#
################################################################################

# Raze device package tool, built from the source next to this file (staged
# with it by Gaia).
BOARD_IMAGE_SLOTS_VERSION = 1.0
BOARD_IMAGE_SLOTS_SITE = $(BOARD_IMAGE_SLOTS_PKGDIR)/src
BOARD_IMAGE_SLOTS_SITE_METHOD = local
BOARD_IMAGE_SLOTS_LICENSE = MIT

define BOARD_IMAGE_SLOTS_BUILD_CMDS
	$(TARGET_CC) $(TARGET_CFLAGS) $(TARGET_LDFLAGS) -Wall -Wextra \
		-o $(@D)/board-image-slots $(@D)/board-image-slots.c
endef

define BOARD_IMAGE_SLOTS_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(@D)/board-image-slots $(TARGET_DIR)/usr/bin/board-image-slots
endef

$(eval $(generic-package))
