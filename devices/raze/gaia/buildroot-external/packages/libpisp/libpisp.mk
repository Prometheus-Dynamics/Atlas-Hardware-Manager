# Raspberry Pi PiSP helper library
# Tag v1.7.0, by commit: the repository also has a branch named v1.7.0.
LIBPISP_VERSION = f8a5eb2af4c5dea76442785ef42b2fb1aa9e62f9
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
