What a package carries besides the two programs.

- `linux/install-kind-deb`, `linux/install-kind-rpm`: one word, installed as
  `/usr/lib/atlas-hardware-manager/install-kind`. The app reads it to know which installer
  replaces it (`src/update.rs`, `install_kind`); no package tool is run to find out.
- `linux/atlas-hardware-manager.desktop`: the menu entry of the fast `.deb` (`scripts/deb-fast.sh`).
  Tauri's bundler writes its own for the packages `scripts/bundle.sh` makes.

Not carried from `dev`, each for its reason: the udev rules and their post-install script (they
give access to boards in a USB boot ROM, and this build registers no boot ROM driver in the real
mode yet; they come with the item that does), the `atlas` command wrapper (F8), and the Windows
driver tool and its installer hooks (the same boot ROM access, on Windows).
