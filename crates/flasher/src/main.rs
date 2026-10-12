//! `atlas-flasher`: the raze-flasher daemon. /init starts it with the
//! FunctionFS mount and the eMMC:
//!
//!     atlas-flasher /dev/ffs-flasher /dev/mmcblk0
//!
//! It brings up the interface, prints `ready` once ep1/ep2 exist (so /init
//! can bind the gadget), then serves Atlas until asked to reboot, showing
//! each phase on the status ring. It never strands a board:
//!
//! - nothing written and no Atlas for 5 minutes: back to the eMMC (it is
//!   untouched, so it boots as before);
//! - written and verified, and Atlas went away without asking: back to the
//!   eMMC after a few seconds;
//! - a write that failed or stopped partway: it stays here (red), since the
//!   eMMC may not boot, for Atlas to try again.

#[cfg(target_os = "linux")]
fn main() {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use atlas_flasher_proto::RebootInto;
    use flasher::board::Board;
    use flasher::ffs::Function;
    use flasher::ring::{Emmc, Look, Ring, Show};
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
    /// No Atlas for this long, with the eMMC untouched: back to it.
    const IDLE: Duration = Duration::from_secs(5 * 60);
    /// After a verified write, once Atlas went away.
    const DONE: Duration = Duration::from_secs(5);
    fn reboot(why: &str) -> ! {
        log(&format!("rebooting to the eMMC: {why}"));
        nix::unistd::sync();
        let _ = nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_AUTOBOOT);
        std::process::exit(0);
    }
    let ring: Arc<dyn Show> = Arc::new(Ring::raze());
    ring.show(Look::Waiting);
    let emmc = Arc::new(Mutex::new(Emmc::Untouched));
    let context = Context {
        disk,
        board: Board::detect(),
        version: env!("CARGO_PKG_VERSION").into(),
        drop_caches: true,
        ring: ring.clone(),
        emmc: emmc.clone(),
    };
    let mut idle_since = Instant::now();
    loop {
        let state = *emmc.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match state {
            Emmc::Untouched if idle_since.elapsed() >= IDLE => {
                reboot("no Atlas for 5 minutes; the eMMC is as it was")
            }
            Emmc::Verified if idle_since.elapsed() >= DONE => {
                reboot("written and verified; Atlas went away")
            }
            _ => {}
        }
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
                idle_since = Instant::now();
                std::thread::sleep(Duration::from_millis(200));
            }
            Outcome::Reboot(RebootInto::Emmc | RebootInto::UsbBoot) => {
                // Let the reply reach Atlas before the link drops.
                std::thread::sleep(Duration::from_millis(300));
                reboot("Atlas asked");
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
