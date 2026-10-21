use super::*;

/// A freshly flashed r20 board's /run/board/update.json, as HeliOS read it.
#[test]
fn status_reads_nulls_as_empty() {
    let status: Status = serde_json::from_str(
        r#"{"state":"idle","slot_active":"A","slot_staged":null,"version_active":"dev-v2027.0.0-alpha-2-94-gc9548d91+Dev-102-gb899b87","version_staged":null,"progress":0,"error":null,"version_previous":null,"started_by":null,"phase":null}"#,
    )
    .unwrap();
    assert_eq!(status.state, "idle");
    assert_eq!(
        status.version_active,
        "dev-v2027.0.0-alpha-2-94-gc9548d91+Dev-102-gb899b87"
    );
    assert_eq!(
        (
            status.version_staged.as_str(),
            status.progress,
            status.error.as_str()
        ),
        ("", 0, "")
    );
    let all_null: Status = serde_json::from_str(
        r#"{"state":null,"version_active":null,"version_staged":null,"progress":null,"error":null}"#,
    )
    .unwrap();
    assert_eq!(all_null.state, "");
}

#[test]
fn status_parses_the_writer_output() {
    let status: Status = serde_json::from_str(
        r#"{"state":"staged","slot_active":"A","slot_staged":"B","version_active":"1.0","version_staged":"2.0","progress":1000,"error":""}"#,
    )
    .unwrap();
    assert_eq!(status.state, "staged");
    assert_eq!(status.version_staged, "2.0");
    assert_eq!(status.progress, 1000);
}

#[test]
fn commands_run_as_atlas() {
    assert_eq!(
        as_atlas("/usr/lib/board/update status"),
        "export BOARD_EVENT_SOURCE=atlas; /usr/lib/board/update status"
    );
}

#[test]
fn ssh_failures_read_as_reasons() {
    assert!(matches!(
        ssh_error(Some(255), "root@x: Permission denied (publickey)."),
        DriverError::Incompatible(_)
    ));
    assert!(matches!(
        ssh_error(
            Some(255),
            "ssh: connect to host x port 22: No route to host"
        ),
        DriverError::Unreachable(_)
    ));
    assert!(matches!(
        ssh_error(Some(1), "update: nothing is staged"),
        DriverError::Other(reason) if reason == "update: nothing is staged"
    ));
}

#[test]
fn the_private_key_sits_next_to_the_public_one() {
    let dir = std::env::temp_dir().join(format!("atlas-board-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("id_test"), "private").unwrap();
    std::fs::write(dir.join("id_test.pub"), "ssh-ed25519 AAAA").unwrap();
    assert_eq!(
        private_key_for(&dir.join("id_test.pub")),
        Some(dir.join("id_test"))
    );
    assert_eq!(private_key_for(&dir.join("missing.pub")), None);
    let _ = std::fs::remove_dir_all(dir);
}
