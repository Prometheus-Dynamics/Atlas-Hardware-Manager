use std::fs;
use std::path::PathBuf;

use atlas_log::{Config, Logger};

/// A fresh directory under the target dir, so scratch stays on the build disk.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("atlas-log-{name}"));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_failed_call_and_a_ui_error_land_with_their_context() {
    let dir = scratch("context");
    let logger = Logger::open(&dir, Config::default()).expect("open");

    logger.notice("Atlas started");
    logger.breadcrumb("opened the Devices page");
    logger.failed_call(
        "device.flash",
        "device=board-a image=update.img",
        "timeout after 5 s",
    );
    logger.ui_error("Devices page", "Flash button failed: timeout after 5 s");
    assert!(logger.flush());

    let text = fs::read_to_string(dir.join("atlas.log")).expect("current file");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "one line per entry: {text}");

    let call = lines
        .iter()
        .find(|line| line.contains("CALL_FAILED"))
        .expect("failed call line");
    assert!(call.contains("device.flash"), "{call}");
    assert!(
        call.contains("asked [device=board-a image=update.img]"),
        "{call}"
    );
    assert!(call.contains("error: timeout after 5 s"), "{call}");

    let ui = lines
        .iter()
        .find(|line| line.contains("UI_ERROR"))
        .expect("UI error line");
    assert!(ui.contains("Devices page: Flash button failed"), "{ui}");

    // Entries keep the order they were recorded in.
    let order: Vec<&str> = lines
        .iter()
        .map(|line| {
            line.split_whitespace()
                .nth(1)
                .expect("a kind after the timestamp")
        })
        .collect();
    assert_eq!(order, ["NOTICE", "CRUMB", "CALL_FAILED", "UI_ERROR"]);

    // The support bundle reads the same text.
    let bundle = logger.read_all().expect("read");
    assert_eq!(bundle, text);
}

#[test]
fn the_file_rotates_by_size_and_keeps_the_configured_number() {
    let dir = scratch("rotation");
    let config = Config {
        max_bytes: 1024,
        keep: 2,
        ..Config::default()
    };
    let logger = Logger::open(&dir, config).expect("open");

    for index in 0..200 {
        logger.failed_call("device.read", &format!("request={index}"), "no answer");
    }
    assert!(logger.flush());

    let names: Vec<String> = logger
        .files()
        .iter()
        .map(|path| {
            path.file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, ["atlas.log.2", "atlas.log.1", "atlas.log"]);

    for name in &names {
        let size = fs::metadata(dir.join(name)).expect("metadata").len();
        assert!(size <= 1024, "{name} is {size} bytes");
    }

    // The newest entry is in the current file; the oldest was dropped.
    let current = fs::read_to_string(dir.join("atlas.log")).expect("current");
    assert!(current.contains("request=199"), "{current}");
    let everything = logger.read_all().expect("read");
    assert!(everything.contains("request=199"));
    assert!(
        !everything.contains("request=0]"),
        "oldest entry should be gone"
    );
}

#[test]
fn keep_zero_keeps_only_the_current_file() {
    let dir = scratch("keep-zero");
    let config = Config {
        max_bytes: 1024,
        keep: 0,
        ..Config::default()
    };
    let logger = Logger::open(&dir, config).expect("open");
    for index in 0..100 {
        logger.breadcrumb(&format!("step {index}"));
    }
    assert!(logger.flush());
    assert_eq!(logger.files().len(), 1);
    let size = fs::metadata(dir.join("atlas.log")).expect("metadata").len();
    assert!(size <= 1024);
}

#[test]
fn long_and_multiline_entries_stay_bounded_on_one_line() {
    let dir = scratch("bounded");
    let logger = Logger::open(&dir, Config::default()).expect("open");

    let huge = format!("first line\nsecond line\n{}", "x".repeat(100_000));
    logger.failed_call("device.write", &huge, "bad\r\nresult");
    assert!(logger.flush());

    let text = fs::read_to_string(dir.join("atlas.log")).expect("current");
    assert_eq!(text.lines().count(), 1, "a multi-line entry is one line");
    assert!(text.len() < 4096, "line is {} bytes", text.len());
    assert!(text.contains("first line second line"));
    assert!(text.contains("..."));
}

#[test]
fn a_disabled_logger_records_nothing_and_never_fails() {
    let logger = Logger::disabled();
    logger.ui_error("anywhere", "nothing");
    logger.failed_call("call", "args", "error");
    logger.breadcrumb("crumb");
    assert!(logger.flush());
    assert!(logger.files().is_empty());
    assert_eq!(logger.read_all().expect("read"), "");
    assert!(logger.path().is_none());
}

#[test]
fn open_refuses_bad_configs_and_unusable_directories() {
    let dir = scratch("refusals");
    let nested = Config {
        file_name: "logs/atlas.log".to_owned(),
        ..Config::default()
    };
    assert!(Logger::open(&dir, nested).is_err());

    let tiny = Config {
        max_bytes: 16,
        ..Config::default()
    };
    assert!(Logger::open(&dir, tiny).is_err());

    // A file where the directory should be cannot be opened.
    fs::create_dir_all(dir.parent().expect("parent")).expect("parent");
    fs::write(&dir, "not a directory").expect("blocker");
    assert!(Logger::open(&dir, Config::default()).is_err());
    let _ = fs::remove_file(&dir);
}
