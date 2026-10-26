use super::*;

#[test]
fn the_node_is_on_the_host_atlas_reached_the_board_at() {
    for (address, url) in [
        ("10.0.0.5", "orion+tcp://10.0.0.5:9200"),
        (
            "http://172.31.250.1:5899/.well-known/pd-device",
            "orion+tcp://172.31.250.1:9200",
        ),
        ("http://raze.local/", "orion+tcp://raze.local:9200"),
        ("10.0.0.7:22", "orion+tcp://10.0.0.7:9200"),
        ("fe80::1", "orion+tcp://[fe80::1]:9200"),
        ("[fe80::1]:5899", "orion+tcp://[fe80::1]:9200"),
        ("http://[fe80::1]:5899/x", "orion+tcp://[fe80::1]:9200"),
        ("orion+tcp://127.0.0.1:4100", "orion+tcp://127.0.0.1:4100"),
    ] {
        assert_eq!(LiveStreams::url_for(address), url, "{address}");
    }
}
