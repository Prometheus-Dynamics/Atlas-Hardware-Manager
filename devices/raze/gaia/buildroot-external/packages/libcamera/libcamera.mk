################################################################################
#
# libcamera
#
################################################################################
LIBCAMERA_SITE = https://github.com/raspberrypi/libcamera.git
# LIBCAMERA_SITE = https://github.com/libcamera-org/libcamera.git
LIBCAMERA_SITE_METHOD = git
# Raspberry Pi libcamera snapshot on upstream 0.7.2 (soname libcamera.so.0.7:
# anything linked against 0.6, such as PhotonVision's libcamera GL driver, must
# be rebuilt). Needs libpisp >= 1.7.0 (libpisp.mk).
# Tag v0.7.2+rpt20260817, by commit so the archive (and its hash) is stable.
LIBCAMERA_VERSION = 6c1dd9d55573010f710c9e190a73e7e76f0d9432
LIBCAMERA_DEPENDENCIES = \
	host-openssl \
	host-pkgconf \
	host-python-jinja2 \
	host-python-ply \
	host-python-pyyaml \
	libyaml
LIBCAMERA_CONF_OPTS = \
    -Dauto_features=disabled \
    -Dandroid=disabled \
    -Ddocumentation=disabled \
    -Dtest=false \
    -Dwerror=false \
    -Db_lto=false \
    --wrap-mode=default
LIBCAMERA_BUILDDIR = $(@D)/buildroot-build
LIBCAMERA_INSTALL_STAGING = YES
LIBCAMERA_LICENSE = \
	LGPL-2.1+ (library), \
	GPL-2.0+ (utils), \
	MIT (qcam/assets/feathericons), \
	BSD-2-Clause (raspberrypi), \
	GPL-2.0 with Linux-syscall-note or BSD-3-Clause (linux kernel headers), \
	CC0-1.0 (meson build system), \
	CC-BY-SA-4.0 (doc)
LIBCAMERA_LICENSE_FILES = \
	LICENSES/LGPL-2.1-or-later.txt \
	LICENSES/GPL-2.0-or-later.txt \
	LICENSES/MIT.txt \
	LICENSES/BSD-2-Clause.txt \
	LICENSES/GPL-2.0-only.txt \
	LICENSES/Linux-syscall-note.txt \
	LICENSES/BSD-3-Clause.txt \
	LICENSES/CC0-1.0.txt \
	LICENSES/CC-BY-SA-4.0.txt

ifeq ($(BR2_TOOLCHAIN_GCC_AT_LEAST_7),y)
LIBCAMERA_CXXFLAGS = -faligned-new
endif

# IPA module signatures are verified with gnutls or OpenSSL's libcrypto (meson tries gnutls
# first). Unsigned or badly signed modules run isolated in a proxy process per camera.
ifeq ($(BR2_PACKAGE_GNUTLS),y)
LIBCAMERA_DEPENDENCIES += gnutls
else
LIBCAMERA_DEPENDENCIES += openssl
endif

ifeq ($(BR2_PACKAGE_LIBCAMERA_PYTHON),y)
LIBCAMERA_DEPENDENCIES += python3 python-pybind
LIBCAMERA_CONF_OPTS += -Dpycamera=enabled
else
LIBCAMERA_CONF_OPTS += -Dpycamera=disabled
endif

ifeq ($(BR2_PACKAGE_LIBCAMERA_V4L2),y)
LIBCAMERA_CONF_OPTS += -Dv4l2=enabled
else
LIBCAMERA_CONF_OPTS += -Dv4l2=disabled
endif

ifeq ($(BR2_PACKAGE_LIBCAMERA_PIPELINE_RPI_PISP),y)
LIBCAMERA_DEPENDENCIES += \
	libpisp \
	libyuv \
	bzip2 \
	jpeg
endif

LIBCAMERA_PIPELINES-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_IMX8_ISI) += imx8-isi
LIBCAMERA_PIPELINES-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_IPU3) += ipu3
LIBCAMERA_PIPELINES-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_MALI_C55) += mali-c55
LIBCAMERA_PIPELINES-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_RKISP1) += rkisp1
LIBCAMERA_PIPELINES-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_RPI_PISP) += rpi/pisp
LIBCAMERA_PIPELINES-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_RPI_VC4) += rpi/vc4
LIBCAMERA_PIPELINES-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_SIMPLE) += simple
LIBCAMERA_PIPELINES-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_UVCVIDEO) += uvcvideo
LIBCAMERA_PIPELINES-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_VIMC) += vimc

# Select IPAs to match the enabled pipelines. Some pipelines (e.g. uvcvideo)
# do not have an IPA and will be ignored safely by meson.
LIBCAMERA_CONF_OPTS += -Dpipelines=$(subst $(space),$(comma),$(LIBCAMERA_PIPELINES-y))

# Map enabled pipelines to IPAs (uvcvideo has no IPA)
LIBCAMERA_IPAS-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_IPU3) += ipu3
LIBCAMERA_IPAS-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_MALI_C55) += mali-c55
LIBCAMERA_IPAS-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_RKISP1) += rkisp1
LIBCAMERA_IPAS-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_RPI_VC4) += rpi/vc4
LIBCAMERA_IPAS-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_SIMPLE) += simple
LIBCAMERA_IPAS-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_VIMC) += vimc
# Allow future Pi5 pipeline if selected
LIBCAMERA_IPAS-$(BR2_PACKAGE_LIBCAMERA_PIPELINE_RPI_PISP) += rpi/pisp

LIBCAMERA_CONF_OPTS += -Dipas=$(subst $(space),$(comma),$(LIBCAMERA_IPAS-y))

ifeq ($(BR2_PACKAGE_LIBCAMERA_COMPLIANCE),y)
LIBCAMERA_DEPENDENCIES += gtest libevent
LIBCAMERA_CONF_OPTS += -Dlc-compliance=enabled
else
LIBCAMERA_CONF_OPTS += -Dlc-compliance=disabled
endif

# gstreamer-video-1.0, gstreamer-allocators-1.0
ifeq ($(BR2_PACKAGE_GSTREAMER1)$(BR2_PACKAGE_GST1_PLUGINS_BASE),yy)
LIBCAMERA_CONF_OPTS += -Dgstreamer=enabled
LIBCAMERA_DEPENDENCIES += gstreamer1 gst1-plugins-base
else
LIBCAMERA_CONF_OPTS += -Dgstreamer=disabled
endif

ifeq ($(BR2_PACKAGE_LIBEVENT),y)
LIBCAMERA_CONF_OPTS += -Dcam=enabled
LIBCAMERA_DEPENDENCIES += libevent
else
LIBCAMERA_CONF_OPTS += -Dcam=disabled
endif

# Explicitly disable qcam unless a separate option is introduced.
LIBCAMERA_CONF_OPTS += -Dqcam=disabled

ifeq ($(BR2_PACKAGE_ELFUTILS),y)
# Optional dependency on libdw
LIBCAMERA_DEPENDENCIES += elfutils
endif

ifeq ($(BR2_PACKAGE_JPEG),y)
LIBCAMERA_DEPENDENCIES += jpeg
endif

ifeq ($(BR2_PACKAGE_LIBDRM),y)
LIBCAMERA_DEPENDENCIES += libdrm
endif

ifeq ($(BR2_PACKAGE_LIBUNWIND),y)
LIBCAMERA_DEPENDENCIES += libunwind
endif

ifeq ($(BR2_PACKAGE_SDL2),y)
LIBCAMERA_DEPENDENCIES += sdl2
endif

ifeq ($(BR2_PACKAGE_TIFF),y)
LIBCAMERA_DEPENDENCIES += tiff
endif

ifeq ($(BR2_PACKAGE_HAS_UDEV),y)
LIBCAMERA_CONF_OPTS += -Dudev=enabled
LIBCAMERA_DEPENDENCIES += udev
endif

ifeq ($(BR2_PACKAGE_LIBCAMERA_TRACING),y)
LIBCAMERA_CONF_OPTS += -Dtracing=enabled
LIBCAMERA_DEPENDENCIES += lttng-libust
else
LIBCAMERA_CONF_OPTS += -Dtracing=disabled
endif

ifeq ($(BR2_PACKAGE_LIBEXECINFO),y)
LIBCAMERA_DEPENDENCIES += libexecinfo
LIBCAMERA_LDFLAGS = $(TARGET_LDFLAGS) -lexecinfo
endif

# libcamera only loads an IPA module in-process when its .sign matches the installed file;
# otherwise it runs the IPA in a separate proxy process per camera. Meson signs the modules
# at install time, but target-finalize strips them afterwards, which invalidates the
# signatures. Strip them here (strip is idempotent, so target-finalize leaves them
# unchanged) and sign the stripped files with the key built into libcamera.
define LIBCAMERA_SIGN_TARGET_IPA_SO
	for f in $(TARGET_DIR)/usr/lib/libcamera/ipa/ipa_*.so; do \
		test -f "$$f" || continue; \
		$(STRIPCMD) "$$f" && \
		$(HOST_DIR)/bin/openssl dgst -sha256 \
			-sign $(LIBCAMERA_BUILDDIR)/src/ipa-priv-key.pem \
			-out "$$f.sign" "$$f" && \
		chmod 0644 "$$f.sign" || exit 1; \
	done
endef

LIBCAMERA_POST_INSTALL_TARGET_HOOKS += LIBCAMERA_SIGN_TARGET_IPA_SO

# Ensure pipeline modules land in the target filesystem (RP1 needs rpi/pisp).
# Meson sometimes skips installing pipeline/IPA data (notably rpi/pisp); force copy from build tree to staging/target.
define LIBCAMERA_INSTALL_TARGET_CMDS
	$(MESON_INSTALL_TARGET)
	# Ensure IPA and pipeline data/files are copied to target even if meson skips them
	if test -d $(STAGING_DIR)/usr/share/libcamera/ipa; then \
		$(INSTALL) -d $(TARGET_DIR)/usr/share/libcamera/ipa; \
		cd $(STAGING_DIR)/usr/share/libcamera/ipa && find . -type f | while read f; do \
			$(INSTALL) -D -m 0644 $(STAGING_DIR)/usr/share/libcamera/ipa/$$f $(TARGET_DIR)/usr/share/libcamera/ipa/$$f; \
		done; \
	fi; \
	if test -d $(STAGING_DIR)/usr/share/libcamera/pipeline; then \
		$(INSTALL) -d $(TARGET_DIR)/usr/share/libcamera/pipeline; \
		cd $(STAGING_DIR)/usr/share/libcamera/pipeline && find . -type f | while read f; do \
			$(INSTALL) -D -m 0644 $(STAGING_DIR)/usr/share/libcamera/pipeline/$$f $(TARGET_DIR)/usr/share/libcamera/pipeline/$$f; \
		done; \
	fi; \
	# Keep IPA .so files already installed by meson in libcamera/ipa
	if test -d $(STAGING_DIR)/usr/lib/libcamera/ipa; then \
		$(INSTALL) -d $(TARGET_DIR)/usr/lib/libcamera/ipa; \
		$(INSTALL) -m 0755 $(STAGING_DIR)/usr/lib/libcamera/ipa/* $(TARGET_DIR)/usr/lib/libcamera/ipa/; \
		chmod 0755 $(TARGET_DIR)/usr/lib/libcamera/ipa/*.so; \
	fi; \
	# Ensure IPA proxy worker executables are present in the runtime image.
	# RPi pipelines fail with "Failed to get proxy worker path" when these are missing.
	if test -d $(STAGING_DIR)/usr/libexec/libcamera; then \
		$(INSTALL) -d $(TARGET_DIR)/usr/libexec/libcamera; \
		cd $(STAGING_DIR)/usr/libexec/libcamera && find . -maxdepth 1 -type f | while read f; do \
			$(INSTALL) -D -m 0755 $(STAGING_DIR)/usr/libexec/libcamera/$$f $(TARGET_DIR)/usr/libexec/libcamera/$$f; \
		done; \
	fi; \
	# Ensure main shared libs land in the image; meson install can occasionally skip when using staging
	if test -d $(STAGING_DIR)/usr/lib; then \
		$(INSTALL) -d $(TARGET_DIR)/usr/lib; \
		cp -a $(STAGING_DIR)/usr/lib/libcamera*.so* $(TARGET_DIR)/usr/lib/ 2>/dev/null || true; \
	fi; \
	# Some components look for libcamera libs in /lib64; mirror the shared objects there
	if test -d $(TARGET_DIR)/usr/lib; then \
		mkdir -p $(TARGET_DIR)/lib64; \
		cp -a $(TARGET_DIR)/usr/lib/libcamera*.so* $(TARGET_DIR)/lib64/ 2>/dev/null || true; \
	fi
endef

$(eval $(meson-package))
