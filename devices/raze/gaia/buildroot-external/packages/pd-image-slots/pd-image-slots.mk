################################################################################
#
# pd-image-slots
#
################################################################################

# Raze device package tool, built from the source next to this file (staged
# with it by Gaia).
PD_IMAGE_SLOTS_VERSION = 1.0
PD_IMAGE_SLOTS_SITE = $(PD_IMAGE_SLOTS_PKGDIR)/src
PD_IMAGE_SLOTS_SITE_METHOD = local
PD_IMAGE_SLOTS_LICENSE = MIT

define PD_IMAGE_SLOTS_BUILD_CMDS
	$(TARGET_CC) $(TARGET_CFLAGS) $(TARGET_LDFLAGS) -Wall -Wextra \
		-o $(@D)/pd-image-slots $(@D)/pd-image-slots.c
endef

define PD_IMAGE_SLOTS_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(@D)/pd-image-slots $(TARGET_DIR)/usr/bin/pd-image-slots
endef

$(eval $(generic-package))
