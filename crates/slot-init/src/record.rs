//! Recording [`Ops`] for tests/slot-init.sh: each step /init takes is a
//! line in `calls` (as the commands the shell /init used to run), and
//! nothing is mounted: the test prepares the directories the mounts would
//! show. `fail` names steps that fail: `verity`, `erofs`, `move`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::Manifest;
use crate::boot::Ops;

pub struct Record {
    pub calls: PathBuf,
    pub fail: Vec<String>,
}

impl Record {
    fn note(&self, line: &str) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.calls)
        {
            let _ = writeln!(file, "{line}");
        }
    }

    fn fails(&self, step: &str) -> bool {
        self.fail.iter().any(|f| f == step)
    }
}

impl Ops for Record {
    fn mount(&self, fstype: &str, opts: &str, source: &Path, target: &Path) -> Result<(), String> {
        self.note(&format!(
            "mount -t {fstype} -o {opts} {} {}",
            source.display(),
            target.display()
        ));
        if self.fails(fstype) {
            return Err(format!("no {fstype} here"));
        }
        Ok(())
    }

    fn bind(&self, source: &Path, target: &Path) -> Result<(), String> {
        self.note(&format!(
            "mount --bind {} {}",
            source.display(),
            target.display()
        ));
        Ok(())
    }

    fn move_mount(&self, source: &Path, target: &Path) -> Result<(), String> {
        self.note(&format!(
            "mount --move {} {}",
            source.display(),
            target.display()
        ));
        if self.fails("move") {
            return Err("no /data in the image".into());
        }
        Ok(())
    }

    fn umount(&self, target: &Path) {
        self.note(&format!("umount {}", target.display()));
    }

    fn load_module(&self, name: &str, release: &str) -> Result<(), String> {
        self.note(&format!("modprobe {name} ({release})"));
        Ok(())
    }

    fn open_slot(&self, image: &Path, manifest: &Manifest, root: &str) -> Result<String, String> {
        self.note(&format!("open {} {} {root}", image.display(), manifest.id));
        if self.fails("verity") {
            return Err("DM_TABLE_LOAD: Invalid argument".into());
        }
        Ok("/dev/dm-0".into())
    }

    fn kernel_log(&self) -> String {
        "kernel log\n".into()
    }

    fn sync(&self) {}

    fn reboot(&self) {
        self.note("reboot");
    }

    fn reboot_tryboot(&self) {
        self.note("reboot-tryboot");
    }

    fn switch_root(&self, new_root: &Path) -> Result<(), String> {
        self.note(&format!("switch_root {} /sbin/init", new_root.display()));
        Ok(())
    }

    fn sleep(&self, _time: Duration) {}
}
