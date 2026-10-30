use super::*;

fn args(pairs: &[(&str, TypedConfigValue)]) -> BTreeMap<String, TypedConfigValue> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[test]
fn update_arguments_are_checked() {
    let text = |value: &str| TypedConfigValue::String(value.into());
    let good = args(&[
        ("image_url", text("http://10.0.0.5:7700/images/t/x.img.xz")),
        ("sha256", text(&"AB".repeat(32))),
        ("size", TypedConfigValue::UInt(42)),
    ]);
    let request = StageRequest::from_args(&good).unwrap();
    assert_eq!(request.sha256, "ab".repeat(32));
    assert_eq!(request.size, 42);

    let mut bad = good.clone();
    bad.insert("image_url".into(), text("file:///etc/shadow"));
    assert!(StageRequest::from_args(&bad).unwrap_err().contains("http"));
    // An upload on the board (HeliOS's /v1/update/uploads), only there.
    let mut upload = good.clone();
    upload.insert(
        "image_url".into(),
        text("file:///data/updates/42/helios.slot.tar"),
    );
    assert!(StageRequest::from_args(&upload).is_ok());
    for url in [
        "file:///data/updates/",
        "file:///data/updates/../board/x.tar",
        "file:///data/updates/42/../../etc/shadow",
        "file:///data/updatesx/y.tar",
    ] {
        let mut bad = good.clone();
        bad.insert("image_url".into(), text(url));
        assert!(StageRequest::from_args(&bad).is_err(), "{url}");
    }
    let mut bad = good.clone();
    bad.insert("sha256".into(), text("abc"));
    assert!(
        StageRequest::from_args(&bad)
            .unwrap_err()
            .contains("sha256")
    );
    let mut bad = good;
    bad.remove("size");
    assert!(StageRequest::from_args(&bad).unwrap_err().contains("size"));
}
