//! `board-agent`: see the library docs. Settings come from the environment
//! (`board_agent::Config`); it takes no arguments.

use std::time::Duration;

use board_agent::{Config, log, run};

fn main() {
    if std::env::args().nth(1).is_some() {
        log(
            "takes no arguments; settings come from the environment (BOARD_AGENT_*, BOARD_RUN_DIR)",
        );
        std::process::exit(2);
    }
    let config = Config::from_lookup(|key| std::env::var(key).ok());
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log(&format!("can't start: {error}"));
            std::process::exit(1);
        }
    };
    runtime.block_on(async {
        loop {
            // Returns only when the node refused the claim or the
            // connection failed past the retry policy; start over.
            if let Err(error) = run(config.clone()).await {
                log(&format!("orion-node: {error}; trying again"));
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}
