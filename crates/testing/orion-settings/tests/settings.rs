//! The images of this tree, and the rule on fixtures.

use std::collections::BTreeMap;
use std::path::PathBuf;

use atlas_orion_settings::{Image, LINKED, Service, Variable, assignments, check, images};

fn devices() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../devices")
}

/// Images whose settings this check cannot read, each with the reason. An image here is NOT checked.
///
/// OWED (coordinator 2026-11-23): the flasher gets this same check when it moves to the image's Orion (after its
/// faults are closed and D6F is in). Until then it stays on this list; take it off in that move.
const NOT_READ: [(&str, &str); 1] = [(
    "flasher-bcm2712/flasher-bcm2712",
    "the flasher pins its own Orion (components/flasher-orion) and makes /etc/orion/orion.conf at boot (S39flasher-net), with no systemd units",
)];

#[test]
fn every_image_gives_orion_only_settings_its_pinned_orion_reads() {
    let all = images(&devices()).expect("the descriptions are read");
    assert!(
        all.iter().any(|image| image.name.starts_with("raze-base/")),
        "the Raze's image is among them: {:?}",
        all.iter().map(|image| &image.name).collect::<Vec<_>>()
    );
    let mut problems = Vec::new();
    for image in &all {
        if NOT_READ.iter().any(|(name, _)| *name == image.name) {
            continue;
        }
        problems.extend(check(image));
    }
    for (name, _) in NOT_READ {
        assert!(
            all.iter().any(|image| image.name == name),
            "{name} is listed as not read but is no image of the tree: take it off the list"
        );
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

fn fixture(conf: &str, comm: &[(&str, Option<&str>)]) -> Image {
    let mut variables = BTreeMap::new();
    variables.insert(
        Service::Comm,
        comm.iter()
            .map(|(name, value)| Variable {
                name: (*name).to_owned(),
                value: value.map(str::to_owned),
                from: "fixture.conf".to_owned(),
            })
            .collect(),
    );
    Image {
        name: "fixture/image".to_owned(),
        pinned: LINKED.to_owned(),
        conf: Some(("orion.conf".to_owned(), conf.to_owned())),
        variables,
    }
}

const GOOD: &str = "node.name = board\nsocket.node_uids = 901\nstate.dir = /data/orion\n";

#[test]
fn settings_the_pinned_orion_reads_pass() {
    let image = fixture(GOOD, &[("ORION_PEER_LISTEN", Some("127.0.0.1:9200"))]);
    assert_eq!(check(&image), Vec::<String>::new());
}

#[test]
fn a_key_neither_service_reads_is_named_and_one_only_node_logic_reads_is_not() {
    // `files.max_size` is node logic's: the comm core leaves it, and it is not unknown.
    let image = fixture(
        &format!("{GOOD}files.max_size = 1024\nno.such.key = 1\n"),
        &[],
    );
    let problems = check(&image);
    assert!(
        problems.iter().any(|p| p.contains("`no.such.key`")),
        "{problems:#?}"
    );
    assert!(
        !problems.iter().any(|p| p.contains("files.max_size")),
        "{problems:#?}"
    );
}

#[test]
fn a_variable_the_pinned_orion_does_not_read_is_named_with_its_file() {
    let image = fixture(GOOD, &[("ORION_NO_SUCH_SETTING", Some("lemnos0, end0"))]);
    let problems = check(&image);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`ORION_NO_SUCH_SETTING`") && p.contains("fixture.conf")),
        "{problems:#?}"
    );
}

#[test]
fn a_variable_written_on_the_board_is_checked_by_its_name() {
    let known = fixture(GOOD, &[("ORION_PEER_LISTEN", None)]);
    assert_eq!(check(&known), Vec::<String>::new());
    let unknown = fixture(GOOD, &[("ORION_NO_SUCH_SETTING", None)]);
    let problems = check(&unknown);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`ORION_NO_SUCH_SETTING`") && p.contains("written on the board")),
        "{problems:#?}"
    );
}

#[test]
fn a_value_that_does_not_parse_is_a_failure() {
    let image = fixture(GOOD, &[("ORION_PEER_LISTEN", Some("not an address"))]);
    let problems = check(&image);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("cannot read its settings")),
        "{problems:#?}"
    );
}

#[test]
fn an_image_on_another_orion_is_not_passed_unread() {
    let mut image = fixture(GOOD, &[]);
    image.pinned = "0000000000000000000000000000000000000000".to_owned();
    let problems = check(&image);
    assert!(
        problems.iter().any(|p| p.contains("nothing was checked")),
        "{problems:#?}"
    );
}

#[test]
fn an_environment_line_is_split_as_systemd_splits_it() {
    assert_eq!(assignments("A=1 B=2"), ["A=1", "B=2"]);
    assert_eq!(assignments("\"A=lemnos0, end0\""), ["A=lemnos0, end0"]);
    // Unquoted, the list is cut at its space: the second word is no assignment, and systemd drops it.
    assert_eq!(assignments("A=lemnos0, end0"), ["A=lemnos0,", "end0"]);
}
