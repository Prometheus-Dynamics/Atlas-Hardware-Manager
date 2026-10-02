# Raspberry Pi PiSP helper library
LIBPISP_VERSION = pios/1.3.0-1
LIBPISP_SITE = https://github.com/raspberrypi/libpisp.git
LIBPISP_SITE_METHOD = git
LIBPISP_LICENSE = BSD-2-Clause
LIBPISP_LICENSE_FILES = LICENSE
LIBPISP_INSTALL_STAGING = YES

# Logging auto-enables when Boost is present; keep the dependency explicit.
LIBPISP_DEPENDENCIES = \
	boost \
	json-for-modern-cpp

LIBPISP_CONF_OPTS = \
	-Dlogging=auto \
	-Dexamples=false

$(eval $(meson-package))
