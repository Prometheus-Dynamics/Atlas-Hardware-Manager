use super::*;

#[test]
fn events_read_leniently() {
    let event: DeviceEvent = serde_json::from_str(
        r#"{"t":5,"boot_id":"b","kind":"boot","source":"martian","message":"m","data":{"count":3,"ok":true,"x":[1],"slot":"A"}}"#,
    )
    .unwrap();
    assert_eq!(event.source, EventSource::Unknown);
    assert_eq!(event.data["count"], "3");
    assert_eq!(event.data["ok"], "true");
    assert!(!event.data.contains_key("x"));
    let bare: DeviceEvent = serde_json::from_str(r#"{"t":1,"kind":"x","data":null}"#).unwrap();
    assert_eq!(bare.source, EventSource::Unknown);
    assert!(bare.data.is_empty());
}

#[test]
fn controls_read_as_names_or_in_full() {
    let device: HardwareDevice = serde_json::from_str(
        r#"{"id":"fan","controls":["duty",{"name":"speed","value":0.5,"min":0,"max":1,"unit":""}]}"#,
    )
    .unwrap();
    assert_eq!(device.controls[0], HardwareControl::named("duty"));
    assert_eq!(device.controls[1].value, Some(0.5));
    assert_eq!(device.controls[1].max, Some(1.0));
}

#[test]
fn numbered_events_are_the_same_by_boot_and_seq() {
    let event: DeviceEvent = serde_json::from_str(
        r#"{"t":5,"boot_id":"b","seq":7,"uptime_s":3,"kind":"boot","message":"boot 2"}"#,
    )
    .unwrap();
    assert_eq!(
        (event.seq, event.uptime_s, event.at_ms),
        (Some(7), Some(3), None)
    );
    assert_eq!(event.when_ms(), 5_000);
    // Same boot and seq: the same event, whatever the clock said.
    let mut again = event.clone();
    again.t = 9;
    assert!(event.same_as(&again));
    let mut next = event.clone();
    next.seq = Some(8);
    assert!(
        !event.same_as(&next),
        "the same second and words, another seq"
    );
    let mut corrected = event.clone();
    corrected.at_ms = Some(42_000);
    assert_eq!(corrected.when_ms(), 42_000);
}

#[test]
fn severity_follows_the_kind() {
    let event = |kind: &str, data: &[(&str, &str)]| DeviceEvent {
        t: 0,
        boot_id: String::new(),
        kind: kind.into(),
        source: EventSource::Local,
        message: String::new(),
        data: data
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        seq: None,
        uptime_s: None,
        at_ms: None,
    };
    assert_eq!(event("update.trial-failed", &[]).severity(), "error");
    assert_eq!(event("update.confirmed", &[]).severity(), "success");
    assert_eq!(
        event("boot", &[("previous_clean", "false")]).severity(),
        "warning"
    );
    assert_eq!(
        event("boot", &[("previous_clean", "true")]).severity(),
        "info"
    );
    assert_eq!(event("selftest", &[("ok", "false")]).severity(), "warning");
}
