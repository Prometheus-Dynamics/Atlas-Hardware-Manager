# Linux extensions in linux/ are found by Buildroot. Package overrides in
# packages/ are staged into Buildroot by Gaia.

# Hash of the pinned Raspberry Pi kernel tarball. raspberrypicm5io_defconfig
# sets BR2_DOWNLOAD_FORCE_CHECK_HASHES, and the defconfig's own linux.hash does
# not cover this tarball. Adding the file here, instead of through
# BR2_GLOBAL_PATCH_DIR, works wherever this tree is mounted. LINUX_HASH_FILES is
# expanded when the download runs, after this file is read, and Buildroot
# accepts the tarball when any listed hash file matches it.
LINUX_HASH_FILES += $(BR2_EXTERNAL_RAZE_DEVICE_PATH)/linux/linux.hash
