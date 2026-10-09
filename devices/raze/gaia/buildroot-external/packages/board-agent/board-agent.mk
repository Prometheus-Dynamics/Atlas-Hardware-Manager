################################################################################
#
# board-agent
#
################################################################################

# The Orion device agent, crates/board-agent in the Atlas repository this
# device package comes from. Gaia imports the package from an Atlas checkout
# (`@source:atlas/devices/raze/gaia/buildroot-external`), so the crate is
# four levels above this external tree, at the same commit. A vendored copy
# of devices/raze (sync-device.sh) has no crate next to it: point
# BOARD_AGENT_WORKSPACE at an Atlas checkout, or set
# BR2_PACKAGE_BOARD_AGENT=n.
#
# It builds from that checkout with the workspace's Cargo.lock
# (`cargo build -p board-agent --locked`) rather than through cargo-package:
# a local site method gets no vendoring step, and vendoring the whole
# workspace would fetch the desktop app's dependencies too. Cargo fetches
# only what board-agent needs into $(DL_DIR)/br-cargo-home (Buildroot's
# CARGO_HOME), including the Orion git dependency; once that cache is
# filled, the build works offline. For a first offline build, fill it on a
# connected machine with the same command and copy br-cargo-home along.
BOARD_AGENT_WORKSPACE ?= $(abspath $(BR2_EXTERNAL_RAZE_DEVICE_PATH)/../../../..)
BOARD_AGENT_SITE = $(BOARD_AGENT_WORKSPACE)/crates/board-agent
BOARD_AGENT_SITE_METHOD = local
# The commit that last changed the crate or the lock file, so a new agent
# shows up as a new version (and is rebuilt).
BOARD_AGENT_VERSION = $(or $(shell git -C $(BOARD_AGENT_WORKSPACE) log -1 --format=%h -- crates/board-agent Cargo.lock 2>/dev/null),local)
BOARD_AGENT_LICENSE = MIT
BOARD_AGENT_DEPENDENCIES = host-rustc

define BOARD_AGENT_BUILD_CMDS
	@test -f $(BOARD_AGENT_WORKSPACE)/Cargo.toml || \
		{ echo "board-agent: no Atlas workspace at $(BOARD_AGENT_WORKSPACE); set BOARD_AGENT_WORKSPACE or BR2_PACKAGE_BOARD_AGENT=n" >&2; exit 1; }
	$(TARGET_MAKE_ENV) \
		$(TARGET_CONFIGURE_OPTS) \
		$(PKG_CARGO_ENV) \
		cargo build \
			--release \
			--locked \
			--package board-agent \
			--manifest-path $(BOARD_AGENT_WORKSPACE)/Cargo.toml \
			--target-dir $(@D)/target
endef

define BOARD_AGENT_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(@D)/target/$(RUSTC_TARGET_NAME)/release/board-agent \
		$(TARGET_DIR)/usr/bin/board-agent
endef

$(eval $(generic-package))
