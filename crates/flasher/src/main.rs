//! `atlas-flasher`: the raze-flasher daemon. /init starts it with the
//! FunctionFS mount and the eMMC:
//!
//!     atlas-flasher /dev/ffs-flasher /dev/mmcblk0
//!
//! It brings up the interface, prints `ready` once ep1/ep2 exist (so /init
//! can bind the gadget), then serves Atlas until asked to reboot.

#[cfg(target_os = "linux")]
fn main() {
    use std::path::PathBuf;
    use std::time::Duration;

    use atlas_flasher_proto::RebootInto;
    use flasher::board::Board;
    use flasher::ffs::Function;
    use flasher::session::{self, Context, Outcome};

    fn log(message: &str) {
        let line = format!("atlas-flasher: {message}\n");
        if std::fs::write("/dev/kmsg", &line).is_err() {
            let _ = std::io::Write::write_all(&mut std::io::stderr(), line.as_bytes());
        }
    }

    let mut args = std::env::args_os().skip(1);
    let ffs = PathBuf::from(args.next().unwrap_or_else(|| "/dev/ffs-flasher".into()));
    let disk = PathBuf::from(args.next().unwrap_or_else(|| "/dev/mmcblk0".into()));
    let function = match Function::start(&ffs) {
        Ok(function) => function,
        Err(problem) => {
            log(&format!(
                "can't start FunctionFS at {}: {problem}",
                ffs.display()
            ));
            std::process::exit(1);
        }
    };
    log("ready");
    let context = Context {
        disk,
        board: Board::detect(),
        version: env!("CARGO_PKG_VERSION").into(),
        drop_caches: true,
    };
    loop {
        if !function.enabled() {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let (to_host, from_host) = match function.endpoints() {
            Ok(endpoints) => endpoints,
            Err(problem) => {
                log(&format!("endpoints: {problem}"));
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        };
        log("Atlas connected");
        match session::serve(from_host, to_host, &context) {
            Outcome::Closed => {
                log("Atlas went away");
                std::thread::sleep(Duration::from_millis(200));
            }
            Outcome::Reboot(RebootInto::Emmc | RebootInto::UsbBoot) => {
                log("rebooting to the eMMC");
                // Let the reply reach Atlas before the link drops.
                std::thread::sleep(Duration::from_millis(300));
                nix::unistd::sync();
                let _ = nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_AUTOBOOT);
                std::process::exit(0);
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr(),
        "atlas-flasher runs on the board (Linux) only"
    );
    std::process::exit(1);
}
