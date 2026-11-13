UBI fixtures copied unchanged from the image builder's repository, branch
origin/next at commit 361d085 (the fixtures last changed in 2262f6a, "E5: UBI"),
path crates/nodes/fs-ubi/tests/fixtures/. They are dev's writer's output
(see that repository's crates/nodes/fs-ubi/tests/ubi.rs for the inputs that
produced each one): nand-16k.ubi (16 KiB PEBs), nor-4k.ubi (4 KiB PEBs, VID
header at 64), spinand-32k.ubi (32 KiB PEBs, VID header at 2048).
