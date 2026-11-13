//! Proof for the device catalog: every board in the fixture tree, and under
//! the repository's `devices/` when that folder exists, is recognised from
//! its own USB ids and hostname; the embedded catalog is the one read from
//! `devices/` at build time, and its size is printed.

use std::fs;
use std::path::{Path, PathBuf};

use atlas_catalog::{Catalog, Overrides, UsbState, embedded, embedded_size, read_dir};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/devices");

fn repo_devices() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../devices")
}

/// Every board's own USB ids, in every state it lists them, must recognise
/// that board, and its hostname prefix must recognise it.
fn assert_every_board_recognised(catalog: &Catalog) {
    for board in &catalog.boards {
        for id in &board.usb {
            // A vendor-wide id has no product to recognise by; see
            // `a_vendor_wide_id_matches_any_product_below_an_exact_one`.
            let Some(pid) = id.pid else { continue };
            let found = catalog.recognise_usb(id.vid, pid, id.state);
            assert!(
                found
                    .iter()
                    .any(|m| m.model == board.model && m.state == id.state),
                "{} not recognised from {:04x}:{:04x} ({:?})",
                board.model,
                id.vid,
                pid,
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

/// Writes a `devices/` tree with the given `usb-ids.json` and one board.
fn write_devices(dir: &Path, usb: &str, board_dir: &str, app: &str) {
    fs::write(dir.join("usb-ids.json"), usb).expect("write");
    let app_dir = dir.join("boards").join(board_dir).join("app");
    fs::create_dir_all(&app_dir).expect("mkdir");
    fs::write(app_dir.join("app.json"), app).expect("write");
}

const USB_WITH_VENDOR: &str = r#"{"schema":"urn:atlas:usb-ids:1","ids":{
    "exact":{"state":"download","vid":"2207","pid":"350b","name":"Exact"},
    "vendor":{"state":"download","vid":"2207","name":"Vendor"}}}"#;

#[test]
fn an_old_app_file_still_reads_and_its_json_has_no_new_keys() {
    // The fixtures are the format as it was before the optional fields. Alpha
    // has a picture and a case; beta has neither picture nor orientation.
    let catalog = read_dir(Path::new(FIXTURES)).expect("fixtures read");
    let alpha = catalog.board("alpha-board").expect("alpha");
    assert_eq!(alpha.pictures, ["boards/alpha-board/app/picture.png"]);
    assert!(alpha.ports.is_empty() && alpha.variants.is_empty());
    assert!(alpha.orientation.is_none() && alpha.found_by.is_none());

    let beta = catalog.board("beta-board").expect("beta");
    assert!(beta.pictures.is_empty() && beta.picture.is_none());
    let json = serde_json::to_string(beta).expect("serializes");
    for key in [
        "ports",
        "pictures",
        "picture",
        "variants",
        "orientation",
        "found_by",
    ] {
        assert!(!json.contains(key), "{key} must be left out: {json}");
    }
}

#[test]
fn a_board_names_ports_pictures_variants_orientation_and_found_by() {
    let dir = scratch("new-fields");
    let app = r#"{"schema":"urn:atlas:device-app:1","model":"x-board","display_name":"X",
        "usb":["exact"],
        "look":{"picture":"front.png","pictures":["back.png"],"case":"case.glb",
                "orientation":{"case_in_imu":[0,0.7071067811865476,-0.7071067811865476,0],
                               "ring_led0_deg":157.5,"ring_direction":-1}},
        "ports":{"usb-c":{"kind":"usb-c","label":"USB-C"},"eth":{"kind":"ethernet","label":"Ethernet"}},
        "variants":[{"key":"wide","display_name":"Wide lens","confirmed":false,
                     "overrides":{"pictures":["wide.png"],"ports":{"eth":{"kind":"ethernet","label":"2.5 GbE"}}}},
                    {"key":"tele","display_name":"Tele lens"}],
        "found_by":"  robot  "}"#;
    write_devices(&dir, USB_WITH_VENDOR, "x-board", app);
    let catalog = read_dir(&dir).expect("reads");
    let board = catalog.board("x-board").expect("board");

    // Ports are sorted by name, each with its kind and label.
    let names: Vec<&str> = board.ports.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["eth", "usb-c"]);
    assert_eq!(board.ports[1].kind, "usb-c");
    assert_eq!(board.ports[1].label, "USB-C");

    // Pictures in order; the first is also `picture`, for one-picture readers.
    assert_eq!(
        board.pictures,
        [
            "boards/x-board/app/front.png",
            "boards/x-board/app/back.png"
        ]
    );
    assert_eq!(
        board.picture.as_deref(),
        Some("boards/x-board/app/front.png")
    );

    let orientation = board.orientation.as_ref().expect("orientation");
    assert_eq!(orientation.ring_direction, Some(-1));
    assert_eq!(orientation.ring_led0_deg, Some(157.5));

    // Variants sorted by key; `confirmed` defaults to true.
    let keys: Vec<&str> = board.variants.iter().map(|v| v.key.as_str()).collect();
    assert_eq!(keys, ["tele", "wide"]);
    let tele = &board.variants[0];
    assert!(tele.confirmed && tele.overrides == Overrides::default());
    let wide = &board.variants[1];
    assert!(!wide.confirmed);
    assert_eq!(
        wide.overrides.pictures.as_deref(),
        Some(["boards/x-board/app/wide.png".to_string()].as_slice())
    );
    assert_eq!(wide.overrides.ports.as_ref().map(Vec::len), Some(1));

    // Free text, trimmed.
    assert_eq!(board.found_by.as_deref(), Some("robot"));
}

#[test]
fn bad_new_fields_are_refused() {
    let base = |extra: &str| {
        format!(
            r#"{{"schema":"urn:atlas:device-app:1","model":"x-board","display_name":"X","usb":["exact"],{extra}}}"#
        )
    };
    let cases: &[(&str, String, &str)] = &[
        (
            "port kind not a name",
            base(r#""ports":{"p":{"kind":"USB C","label":"P"}}"#),
            "kind",
        ),
        (
            "port without a label",
            base(r#""ports":{"p":{"kind":"usb","label":" "}}"#),
            "label is empty",
        ),
        (
            "same picture twice",
            base(r#""look":{"picture":"a.png","pictures":["a.png"]}"#),
            "listed twice",
        ),
        (
            "picture escapes app folder",
            base(r#""look":{"pictures":["../b.png"]}"#),
            "inside the board's app/ folder",
        ),
        (
            "quaternion not unit",
            base(r#""look":{"orientation":{"case_in_imu":[1,1,0,0]}}"#),
            "unit quaternion",
        ),
        (
            "direction not 1 or -1",
            base(r#""look":{"orientation":{"ring_direction":2}}"#),
            "ring_direction",
        ),
        (
            "angle out of range",
            base(r#""look":{"orientation":{"ring_led0_deg":360.0}}"#),
            "ring_led0_deg",
        ),
        (
            "orientation with an unknown key",
            base(r#""look":{"orientation":{"tilt":1}}"#),
            "unknown field",
        ),
        (
            "variant key repeated",
            base(r#""variants":[{"key":"a","display_name":"A"},{"key":"a","display_name":"B"}]"#),
            "listed twice",
        ),
        (
            "variant key not a name",
            base(r#""variants":[{"key":"A","display_name":"A"}]"#),
            "variant key",
        ),
        (
            "variant without a name",
            base(r#""variants":[{"key":"a","display_name":""}]"#),
            "display_name is empty",
        ),
        (
            "found_by empty",
            base(r#""found_by":"   ""#),
            "found_by is empty",
        ),
    ];
    for (name, app, want) in cases {
        let dir = scratch(&name.replace(' ', "-"));
        write_devices(&dir, USB_WITH_VENDOR, "x-board", app);
        let err = read_dir(&dir).expect_err(name);
        assert!(err.reason.contains(want), "{name}: {err}");
    }
}

#[test]
fn a_vendor_wide_id_matches_any_product_below_an_exact_one() {
    let dir = scratch("vendor-wide");
    write_devices(
        &dir,
        USB_WITH_VENDOR,
        "x-board",
        r#"{"schema":"urn:atlas:device-app:1","model":"x-board","display_name":"X","usb":["vendor"]}"#,
    );
    write_devices(
        &dir,
        USB_WITH_VENDOR,
        "y-board",
        r#"{"schema":"urn:atlas:device-app:1","model":"y-board","display_name":"Y","usb":["exact"]}"#,
    );
    fs::create_dir_all(dir.join("boards/y-board/app")).expect("mkdir");
    let y = r#"{"schema":"urn:atlas:device-app:1","model":"y-board","display_name":"Y","usb":["exact"]}"#;
    fs::write(dir.join("boards/y-board/app/app.json"), y).expect("write");
    // z-board lists the vendor id too, so it shares the vendor-only rank with x.
    let z = dir.join("boards/z-board/app");
    fs::create_dir_all(&z).expect("mkdir");
    fs::write(
        z.join("app.json"),
        r#"{"schema":"urn:atlas:device-app:1","model":"z-board","display_name":"Z","usb":["vendor"]}"#,
    )
    .expect("write");
    let catalog = read_dir(&dir).expect("reads");

    // An unknown product of the vendor: the vendor-only boards, in model order.
    let unknown = catalog.recognise_usb(0x2207, 0x1234, UsbState::Download);
    let models: Vec<(&str, bool)> = unknown
        .iter()
        .map(|m| (m.model.as_str(), m.exact))
        .collect();
    assert_eq!(models, [("x-board", false), ("z-board", false)]);

    // The exact product: the board that lists it comes first, flagged exact;
    // the vendor-only boards follow it.
    let exact = catalog.recognise_usb(0x2207, 0x350b, UsbState::Download);
    let models: Vec<(&str, bool)> = exact.iter().map(|m| (m.model.as_str(), m.exact)).collect();
    assert_eq!(
        models,
        [("y-board", true), ("x-board", false), ("z-board", false)]
    );

    // The vendor id in another state recognises nothing.
    assert!(
        catalog
            .recognise_usb(0x2207, 0x1234, UsbState::Storage)
            .is_empty()
    );
}

#[test]
fn the_repo_app_data_carries_the_raze_orientation_and_the_ai_cam_lenses() {
    let catalog = embedded();
    let raze = catalog.board("raze").expect("raze");
    let o = raze.orientation.as_ref().expect("raze orientation");
    assert_eq!(o.ring_direction, Some(-1));
    assert_eq!(o.ring_led0_deg, Some(157.5));
    assert_eq!(o.case_in_imu.map(|q| q[0]), Some(0.0));

    let cam = catalog.board("canmv-k230d-ai-cam").expect("ai-cam");
    let lenses: Vec<(&str, &str, bool)> = cam
        .variants
        .iter()
        .map(|v| (v.key.as_str(), v.display_name.as_str(), v.confirmed))
        .collect();
    assert_eq!(
        lenses,
        [
            ("lens-140", "140° lens", false),
            ("lens-72", "72° lens", false)
        ]
    );

    // The vendor-wide Rockchip id is in the shared file.
    let vendor = repo_devices().join("usb-ids.json");
    let text = fs::read_to_string(vendor).expect("usb-ids.json");
    assert!(text.contains("\"rockchip-vendor\""));
}
