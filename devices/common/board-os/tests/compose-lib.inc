# shellcheck shell=sh
# compose_lib <dir>: /usr/lib/board as a Raze image has it, the shared scripts
# with the family's and the package's next to them. Needs $_devices (devices/).
compose_lib() {
	mkdir -p "$1"
	for _f in "$_devices"/common/board-os/gaia/assets/rootfs/usr/lib/board/* \
		"$_devices"/families/rpi-bcm2712/gaia/assets/rootfs/usr/lib/board/* \
		"$_devices"/raze/gaia/assets/rootfs/usr/lib/board/*; do
		ln -s "$_f" "$1/"
	done
}
