//! Proof for the device catalog: every board in the fixture tree, and under
//! the repository's `devices/` when that folder exists, is recognised from
//! its own USB ids and hostname; the embedded catalog is the one read from
//! `devices/` at build time, and its size is printed.

use std::fs;
use std::path::{Path, PathBuf};

use atlas_catalog::{Catalog, UsbState, embedded, embedded_size, read_dir};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/devices");

fn repo_devices() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../devices")
}

/// Every board's own USB ids, in every state it lists them, must recognise
/// that board, and its hostname prefix must recognise it.
fn assert_every_board_recognised(catalog: &Catalog) {
    for board in &catalog.boards {
        for id in &board.usb {
            let found = catalog.recognise_usb(id.vid, id.pid, id.state);
            assert!(
                found
                    .iter()
                    .any(|m| m.model == board.model && m.state == id.state),
                "{} not recognised from {:04x}:{:04x} ({:?})",
                board.model,
                id.vid,
                id.pid,
                id.state
            );
        }
        if let Some(prefix) = &board.hostname_prefix {
            let host = format!("{prefix}1");
            assert!(
                catalog
                    .recognise_hostname(&host)
                    .iter()
                    .any(|b| b.model == board.model),
                "{} not recognised from hostname {host}",
                board.model
            );
        }
    }
}

#[test]
fn fixture_boards_are_recognised_from_their_ids() {
    let catalog = read_dir(Path::new(FIXTURES)).expect("fixtures read");
    let models: Vec<&str> = catalog.boards.iter().map(|b| b.model.as_str()).collect();
    assert_eq!(models, ["alpha-board", "beta-board", "gamma-board"]);
    // delta-board has no app/, and stray/ is outside boards/: neither is in.
    assert_every_board_recognised(&catalog);

    // A shared id is a candidate for both boards, and says what state it is in.
    let shared = catalog.recognise_usb(0x29f1, 0x0230, UsbState::Download);
    let mut names: Vec<&str> = shared.iter().map(|m| m.model.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["beta-board", "gamma-board"]);

    // The same ids in another state recognise nothing.
    assert!(
        catalog
            .recognise_usb(0x29f1, 0x0230, UsbState::Storage)
            .is_empty()
    );
    // Unknown ids recognise nothing.
    assert!(
        catalog
            .recognise_usb(0x1234, 0x5678, UsbState::Download)
            .is_empty()
    );

    // Hostnames: case does not matter, and the gamma board's own prefix.
    let host: Vec<&str> = catalog
        .recognise_hostname("GAMMA-7")
        .iter()
        .map(|b| b.model.as_str())
        .collect();
    assert_eq!(host, ["gamma-board"]);
    assert!(catalog.recognise_hostname("unrelated").is_empty());
}

#[test]
fn look_references_are_relative_to_devices() {
    let catalog = read_dir(Path::new(FIXTURES)).expect("fixtures read");
    let alpha = catalog.board("alpha-board").expect("alpha");
    assert_eq!(alpha.display_name, "Alpha Board");
    assert_eq!(
        alpha.picture.as_deref(),
        Some("boards/alpha-board/app/picture.png")
    );
    assert_eq!(
        alpha.case.as_deref(),
        Some("boards/alpha-board/app/case.glb")
    );
    let beta = catalog.board("beta-board").expect("beta");
    assert_eq!(beta.picture, None);
}

#[test]
fn every_board_under_repo_devices_is_recognised_when_present() {
    let dir = repo_devices();
    let catalog = read_dir(&dir).expect("repo devices read");
    println!(
        "repo devices/ ({}): {} boards with app data",
        if dir.is_dir() { "present" } else { "absent" },
        catalog.boards.len()
    );
    assert_every_board_recognised(&catalog);
}

#[test]
fn embedded_catalog_is_the_one_read_from_devices() {
    let size = embedded_size();
    let boards = embedded().boards.len();
    println!("embedded device catalog: {size} bytes, {boards} boards");
    assert_eq!(
        embedded(),
        &read_dir(&repo_devices()).expect("repo devices read")
    );
    assert_every_board_recognised(embedded());
}

#[test]
fn absent_or_empty_devices_is_an_empty_catalog() {
    let base = scratch("empty");
    let absent = base.join("nope");
    assert!(read_dir(&absent).expect("absent is fine").boards.is_empty());
    let empty = base.join("empty");
    fs::create_dir_all(empty.join("boards")).expect("mkdir");
    assert!(read_dir(&empty).expect("empty is fine").boards.is_empty());
    // A Gaia build file beside no app/ is not app data.
    fs::create_dir_all(empty.join("boards/x-board/gaia")).expect("mkdir");
    fs::write(empty.join("boards/x-board/gaia/board.toml"), "not app data").expect("write");
    assert!(read_dir(&empty).expect("ignored").boards.is_empty());
}

/// Writes `devices/usb-ids.json` and `devices/boards/<dir>/app/app.json`.
fn write_tree(dir: &Path, board_dir: &str, app: &str) {
    let usb = r#"{"schema":"urn:atlas:usb-ids:1","ids":{"m":{"state":"download","vid":"2207","pid":"350a","name":"M"}}}"#;
    fs::write(dir.join("usb-ids.json"), usb).expect("write");
    let app_dir = dir.join("boards").join(board_dir).join("app");
    fs::create_dir_all(&app_dir).expect("mkdir");
    fs::write(app_dir.join("app.json"), app).expect("write");
}

#[test]
fn bad_app_data_is_refused() {
    let good =
        r#"{"schema":"urn:atlas:device-app:1","model":"x-board","display_name":"X","usb":["m"]}"#;
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "unknown usb ref",
            "x-board",
            r#"{"schema":"urn:atlas:device-app:1","model":"x-board","display_name":"X","usb":["nope"]}"#,
            "usb id \"nope\" is not",
        ),
        (
            "wrong schema",
            "x-board",
            r#"{"schema":"urn:other:1","model":"x-board","display_name":"X"}"#,
            "schema must be",
        ),
        (
            "unknown field",
            "x-board",
            r#"{"schema":"urn:atlas:device-app:1","model":"x-board","display_name":"X","extra":1}"#,
            "unknown field",
        ),
        (
            "model differs from directory",
            "y-board",
            good,
            "must equal its board directory name",
        ),
        (
            "bad model",
            "x-board",
            r#"{"schema":"urn:atlas:device-app:1","model":"X Board","display_name":"X"}"#,
            "must equal its board directory name",
        ),
        (
            "picture escapes app folder",
            "x-board",
            r#"{"schema":"urn:atlas:device-app:1","model":"x-board","display_name":"X","look":{"picture":"../p.png"}}"#,
            "inside the board's app/ folder",
        ),
        (
            "absolute case",
            "x-board",
            r#"{"schema":"urn:atlas:device-app:1","model":"x-board","display_name":"X","look":{"case":"/etc/c.glb"}}"#,
            "inside the board's app/ folder",
        ),
        (
            "uppercase prefix",
            "x-board",
            r#"{"schema":"urn:atlas:device-app:1","model":"x-board","display_name":"X","network":{"hostname_prefix":"X-"}}"#,
            "hostname_prefix",
        ),
    ];
    for (name, dir_name, app, want) in cases {
        let dir = scratch(&name.replace(' ', "-"));
        write_tree(&dir, dir_name, app);
        let err = read_dir(&dir).expect_err(name);
        assert!(err.reason.contains(want), "{name}: {err}");
    }

    let dir = scratch("bad-hex");
    write_tree(&dir, "x-board", good);
    fs::write(dir.join("usb-ids.json"), r#"{"schema":"urn:atlas:usb-ids:1","ids":{"m":{"state":"download","vid":"35zz","pid":"350a","name":"M"}}}"#).expect("write");
    assert!(
        read_dir(&dir)
            .expect_err("hex")
            .reason
            .contains("4 hex digits")
    );

    let dir = scratch("bad-state");
    write_tree(&dir, "x-board", good);
    fs::write(dir.join("usb-ids.json"), r#"{"schema":"urn:atlas:usb-ids:1","ids":{"m":{"state":"console","vid":"2207","pid":"350a","name":"M"}}}"#).expect("write");
    assert!(read_dir(&dir).is_err());
}

/// A fresh folder under the test target's temporary directory.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("catalog")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}
