# Raspberry Pi PiSP helper library
# Tag v1.7.0, by commit: the repository also has a branch named v1.7.0.
LIBPISP_VERSION = f8a5eb2af4c5dea76442785ef42b2fb1aa9e62f9
LIBPISP_SITE = https://github.com/raspberrypi/libpisp.git
LIBPISP_SITE_METHOD = git
LIBPISP_LICENSE = BSD-2-Clause
LIBPISP_LICENSE_FILES = LICENSE
LIBPISP_INSTALL_STAGING = YES

# No Boost: libpisp uses it only for its logging (Boost.Log), which
# Buildroot's boost package doesn't build unless BR2_PACKAGE_BOOST_LOG is set,
# so `-Dlogging=auto` never found it and the dependency only cost build time
# (extracting, installing and copying Boost's headers into every dependent's
# per-package tree). libcamera logs the PiSP pipeline itself.
LIBPISP_DEPENDENCIES = json-for-modern-cpp

LIBPISP_CONF_OPTS = \
	-Dlogging=disabled \
	-Dexamples=false

$(eval $(meson-package))
