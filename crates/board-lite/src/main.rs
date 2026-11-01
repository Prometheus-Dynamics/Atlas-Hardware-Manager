//! board-lite: a small board's identity contract (see the library).
//!
//! ```text
//! board-lite --model <model> [--rev <rev>] [--print]
//! ```
//!
//! Serves `GET :5899/.well-known/pd-device` and its mDNS advertisement until
//! killed. `--print` prints the document and exits.

use std::io::Write;
use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;
use std::process::ExitCode;

use board_lite::identity::{self, Options, document, facts};
use board_lite::{PATH, PORT, mdns, server};

fn say(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut model = None;
    let mut rev = None;
    let mut print = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model = args.next(),
            "--rev" => rev = args.next(),
            "--print" => print = true,
            other => {
                say(&format!("board-lite: unexpected {other}"));
                return ExitCode::FAILURE;
            }
        }
    }
    let Some(model) = model else {
        say("usage: board-lite --model <model> [--rev <rev>] [--print]");
        return ExitCode::FAILURE;
    };
    let options = Options {
        model,
        rev,
        package_version: env!("ATLAS_BUILD_VERSION").to_string(),
        root: PathBuf::from("/"),
    };
    if print {
        say(&document(&options, &facts(&options.root)));
        return ExitCode::SUCCESS;
    }

    let board = facts(&options.root);
    let tail = |text: &str| text[text.len().saturating_sub(8)..].to_string();
    let advert = mdns::Advert {
        instance: format!("{}-{}", options.model, tail(&board.serial)),
        host: board.hostname.clone(),
        port: PORT,
        txt: [
            ("contract", "1"),
            ("model", options.model.as_str()),
            ("rev", options.rev.as_deref().unwrap_or("")),
            ("serial", board.serial.as_str()),
            ("os", board.os_name.as_deref().unwrap_or("")),
            ("os_ver", board.os_version.as_deref().unwrap_or("")),
            ("path", PATH),
        ]
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .to_vec(),
    };
    std::thread::spawn(move || {
        let error = mdns::run(&advert);
        say(&format!("board-lite: no mDNS: {error}"));
    });

    let listener = match TcpListener::bind((Ipv4Addr::UNSPECIFIED, PORT)) {
        Ok(listener) => listener,
        Err(error) => {
            say(&format!("board-lite: port {PORT}: {error}"));
            return ExitCode::FAILURE;
        }
    };
    server::serve(&listener, &|| {
        identity::document(&options, &facts(&options.root))
    });
    ExitCode::SUCCESS
}
