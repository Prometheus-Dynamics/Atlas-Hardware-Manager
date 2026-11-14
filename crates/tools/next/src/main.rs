//! The `atlas-next` binary: runs `atlas-next slots ...` from the computer. See the library for
//! the commands and the pin rule.

use std::io::Write;
use std::sync::Arc;

use atlas_next::{Env, default_state_dir, execute, is_trial};
use tokio::sync::Notify;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let state_dir = default_state_dir(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"));
    let mut env = Env::new(state_dir);
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("atlas-next: the runtime cannot start: {error}");
            std::process::exit(1);
        }
    };
    let code = runtime.block_on(async {
        // Ctrl-C stops the following of a trial, and only a trial: any other command keeps the
        // default. A second Ctrl-C ends the process.
        if is_trial(&args) {
            let stop = Arc::new(Notify::new());
            env.stop = Some(Arc::clone(&stop));
            tokio::spawn(async move {
                let mut first = true;
                while tokio::signal::ctrl_c().await.is_ok() {
                    if !first {
                        std::process::exit(130);
                    }
                    first = false;
                    stop.notify_one();
                }
            });
        }
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        let code = execute(&args, &env, &mut out).await;
        let _ = out.flush();
        code
    });
    std::process::exit(code);
}
