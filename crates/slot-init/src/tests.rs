use super::*;

const SLOT: &str = r#"{"schema":1,"id":"photonvision-2026.4.0-styx-1a2b3c4d","family":"photonvision",
    "version":"2026.4.0","variant":"styx","kernel":"7.2.9-v8-16k",
    "image":{"size":471859200,"sha256":"00"},
    "verity":{"root_hash":"ab","salt":"5eed","hash_offset":467664896,"data_blocks":114176,
              "data_block_size":4096,"hash_block_size":4096}}"#;

#[test]
fn a_manifest_gives_the_verity_table() {
    let m = Manifest::parse(SLOT).unwrap();
    assert_eq!(m.kernel, "7.2.9-v8-16k");
    let root = root_hash(&"AB".repeat(32)).unwrap();
    let (sectors, params) = verity_table(m.verity.as_ref().unwrap(), "7:0", &root);
    assert_eq!(sectors, 114176 * 8);
    assert_eq!(
        params,
        format!("1 7:0 7:0 4096 4096 114176 114177 sha256 {root} 5eed")
    );
}

fn layered(layers: &str) -> String {
    format!(
        r#"{{"schema":2,"id":"helios-2026.2.0-styx-1a2b3c4d","family":"helios","kernel":"7.2.9-v8-16k","layers":[{layers}]}}"#
    )
}

fn layer(name: &str, id: char) -> String {
    format!(
        r#"{{"name":"{name}","id":"{}","size":8192,"verity":{{"salt":"00","hash_offset":4096,"data_blocks":1}}}}"#,
        id.to_string().repeat(64)
    )
}

#[test]
fn a_layered_manifest_lists_its_layers_lowest_first() {
    let m = Manifest::parse(&layered(&format!(
        "{},{},{}",
        layer("base", 'a'),
        layer("platform", 'b'),
        layer("app", 'c')
    )))
    .unwrap();
    assert!(m.verity.is_none());
    let names: Vec<_> = m.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["base", "platform", "app"]);
    assert_eq!(m.layers[0].verity.tree_start(), 8192);
    let roots = root_hashes(&format!("{},{}", "A".repeat(64), "b".repeat(64))).unwrap();
    assert_eq!(roots, ["a".repeat(64), "b".repeat(64)]);
    assert!(root_hashes(&format!("{},", "a".repeat(64))).is_err());
}

#[test]
fn layer_lines_for_the_writer() {
    let with = |n: &str, c: char| {
        layer(n, c).replace(
            r#""verity":{"#,
            &format!(
                r#""sha256":"{}","verity":{{"root_hash":"{}","#,
                "5".repeat(64),
                "D".repeat(64)
            ),
        )
    };
    let text = layered(&format!("{},{}", with("base", 'a'), with("app", 'c')));
    let lines = layer_lines(&text).unwrap();
    assert_eq!(
        lines,
        [
            format!(
                "base {} 8192 {} {}",
                "a".repeat(64),
                "5".repeat(64),
                "d".repeat(64)
            ),
            format!(
                "app {} 8192 {} {}",
                "c".repeat(64),
                "5".repeat(64),
                "d".repeat(64)
            ),
        ]
    );
    // A layer without its file's sha256 or its root hash can't be installed.
    assert!(layer_lines(&layered(&layer("base", 'a'))).is_err());
    assert!(layer_lines(SLOT).unwrap().is_empty());
}

#[test]
fn a_pushed_layer_replaces_the_one_of_its_name() {
    let with = |n: &str, c: char| {
        layer(n, c).replace(
            r#""verity":{"#,
            &format!(
                r#""sha256":"{}","verity":{{"root_hash":"{}","#,
                c.to_string().repeat(64),
                "d".repeat(64)
            ),
        )
    };
    let slot = layered(&format!("{},{}", with("base", 'a'), with("app", 'c')));
    let pushed = set_layer(&slot, &with("app", 'e')).unwrap();
    let lines = layer_lines(&pushed).unwrap();
    assert!(lines[0].starts_with(&format!("base {}", "a".repeat(64))));
    assert!(lines[1].starts_with(&format!("app {}", "e".repeat(64))));
    // No layer of that name, a one-image slot, a layer missing its sha256.
    assert!(set_layer(&slot, &with("platform", 'e')).is_err());
    assert!(set_layer(SLOT, &with("app", 'e')).is_err());
    assert!(set_layer(&slot, &layer("app", 'e')).is_err());
}

#[test]
fn a_bundle_takes_its_layers_from_gaias_layers_json() {
    // Gaia's record: erofs_size and erofs_sha256, and more besides.
    let gaia = |n: &str, c: char| {
        format!(
            r#"{{"name":"{n}","id":"{0}","tar":"layers/{n}.tar","tar_sha256":"{1}","tar_size":10,"erofs":"layers/{n}.erofs","erofs_sha256":"{0}","erofs_size":8192,"inputs":"x","verity":{{"root_hash":"{2}","salt":"00","hash_offset":4096,"data_blocks":1,"data_block_size":4096,"hash_block_size":4096}}}}"#,
            c.to_string().repeat(64),
            "9".repeat(64),
            "d".repeat(64)
        )
    };
    let layers = format!(
        r#"{{"order":["base","app"],"layers":[{},{}]}}"#,
        gaia("base", 'a'),
        gaia("app", 'c')
    );
    let slot = SLOT.replace("\"schema\":1", "\"schema\":2");
    let merged = merge_layers(&slot, &layers).unwrap();
    let m = Manifest::parse(&merged).unwrap();
    assert!(m.verity.is_none() && m.image.is_none());
    assert_eq!(m.layers.len(), 2);
    assert_eq!(m.layers[1].size, 8192);
    let lines = layer_lines(&merged).unwrap();
    assert_eq!(
        lines[1],
        format!("app {0} 8192 {0} {1}", "c".repeat(64), "d".repeat(64))
    );
    // A layer built without verity can't boot.
    let bare = layers.replacen(&format!(r#""verity":{{"root_hash":"{}","salt":"00","hash_offset":4096,"data_blocks":1,"data_block_size":4096,"hash_block_size":4096}}"#, "d".repeat(64)), r#""verity":null"#, 1);
    assert!(merge_layers(&slot, &bare).is_err());
    assert!(merge_layers(&slot, "{}").is_err());
}

#[test]
fn bad_layered_manifests_are_refused() {
    // Both kinds at once, neither, a layer named twice, a bad id, a bad name.
    let both = layered(&layer("base", 'a')).replace(
        r#""layers""#,
        r#""verity":{"salt":"00","hash_offset":4096,"data_blocks":1},"layers""#,
    );
    assert!(Manifest::parse(&both).is_err());
    assert!(Manifest::parse(&layered("")).is_err());
    assert!(
        Manifest::parse(&layered(&format!(
            "{},{}",
            layer("base", 'a'),
            layer("base", 'b')
        )))
        .is_err()
    );
    assert!(Manifest::parse(&layered(&layer("base", 'g'))).is_err());
    assert!(Manifest::parse(&layered(&layer("base", 'A'))).is_err());
    assert!(Manifest::parse(&layered(&layer("../x", 'a'))).is_err());
    let overlapping =
        layered(&layer("base", 'a')).replace("\"hash_offset\":4096", "\"hash_offset\":0");
    assert!(Manifest::parse(&overlapping).is_err());
    let many: Vec<String> = (0..=MAX_LAYERS)
        .map(|n| layer(&format!("l{n}"), 'a'))
        .collect();
    assert!(Manifest::parse(&layered(&many.join(","))).is_err());
}

#[test]
fn bad_manifests_and_hashes_are_refused() {
    let overlapping = SLOT.replace("467664896", "4096");
    assert!(Manifest::parse(&overlapping).is_err());
    let odd = SLOT.replace("\"hash_block_size\":4096", "\"hash_block_size\":3000");
    assert!(Manifest::parse(&odd).is_err());
    assert!(root_hash("abc").is_err());
    assert!(root_hash(&"g".repeat(64)).is_err());
}

#[test]
fn data_dirs_ids_and_state() {
    let with = SLOT.replace(
        "\"variant\":\"styx\",",
        "\"variant\":\"styx\",\"data_dirs\":[\"photonvision_config\"],",
    );
    let m = Manifest::parse(&with).unwrap();
    assert_eq!(m.family, "photonvision");
    assert_eq!(m.data_dirs, ["photonvision_config"]);
    for bad in ["../etc", "/etc", "a/../b", ""] {
        assert!(
            Manifest::parse(&SLOT.replace(
                "\"variant\":\"styx\",",
                &format!("\"variant\":\"styx\",\"data_dirs\":[\"{bad}\"],")
            ))
            .is_err(),
            "{bad}"
        );
    }
    assert!(valid_id("photonvision-2026.4.0-styx-1a2b3c4d"));
    assert!(!valid_id("../x") && !valid_id("A") && !valid_id(".hidden"));
    let state =
        State::parse(r#"{"schema":1,"default":"a","previous":"b","trial":null,"slots":{}}"#)
            .unwrap();
    assert_eq!(state.previous.as_deref(), Some("b"));
}

#[test]
fn fields_and_boot_files() {
    let text = r#"{"id":"x","image":{"size":12,"sha256":"aa"},"pinned":true,"boot":{"files":{"kernel8.img":"0000000000000000000000000000000000000000000000000000000000000000","overlays/a.dtbo":"1111111111111111111111111111111111111111111111111111111111111111"}}}"#;
    assert_eq!(field(text, "image.size").unwrap().as_deref(), Some("12"));
    assert_eq!(field(text, "image.sha256").unwrap().as_deref(), Some("aa"));
    assert_eq!(field(text, "pinned").unwrap().as_deref(), Some("true"));
    assert_eq!(field(text, "nope.x").unwrap(), None);
    let files = boot_files(text).unwrap();
    assert_eq!(files.len(), 2);
    assert!(boot_files(&text.replace("overlays/a.dtbo", "../a.dtbo")).is_err());
}

#[test]
fn fields_are_one_line_each() {
    let text =
        r#"{"image":{"size":12},"family":"helios","pinned":true,"boot":{"files":{}},"blank":""}"#;
    // A missing field, an object and an empty string are all empty lines.
    let got = fields(
        text,
        &["image.size", "nope", "family", "pinned", "boot", "blank"],
    );
    assert_eq!(got.unwrap(), ["12", "", "helios", "true", "", ""]);
    // A value that wouldn't fit its line is refused, so the caller reads it alone.
    assert!(fields(&text.replace("helios", r"a\nb"), &["family"]).is_err());
    assert!(fields("not json", &["family"]).is_err());
}

#[test]
fn devices_and_cmdline() {
    // dm-3 on major 253: new_encode_dev.
    assert_eq!(decode_dev((253 << 8) | 3), (253, 3));
    assert_eq!(decode_dev((7 << 8) | 0x34 | (0x100 << 12)), (7, 0x134));
    let cmdline = "console=ttyAMA10 atlas.slot=pv-1 atlas.roothash=abcd panic=5";
    assert_eq!(cmdline_arg(cmdline, "atlas.slot"), Some("pv-1"));
    assert_eq!(cmdline_arg(cmdline, "atlas.data"), None);
}
