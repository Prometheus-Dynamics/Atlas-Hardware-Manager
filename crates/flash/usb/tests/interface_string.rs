//! Finding a board's Orion link by its interface string, on the simulated
//! bus. The board's gadget is one configuration with three functions: NCM
//! (two interfaces), one ACM console (two interfaces), and the vendor
//! interface that carries Orion's link (two bulk endpoints, class 0xFF).

use std::time::Duration;

use atlas_usb::sim::{Answer, SimBus, StringRequests};
use atlas_usb::{
    DeviceSummary, EndpointKind, EndpointSummary, InterfaceSummary, OpenOptions, Timeouts, UsbBus,
    UsbError, find_by_interface_string, open_interface,
};

const LINK: &str = "orion-link";

/// A vendor interface with a string and a bulk pair, as the board's gadget
/// presents it. Placeholder ids, as the other tests use.
fn link(number: u8, string: &str) -> InterfaceSummary {
    InterfaceSummary::new(number, 0xff, 0, 0)
        .with_string(string)
        .with_endpoint(EndpointSummary::new(0x81, EndpointKind::Bulk))
        .with_endpoint(EndpointSummary::new(0x01, EndpointKind::Bulk))
}

/// NCM (control and data), then the ACM console, then the link last. The
/// NCM data interface has bulk endpoints too, so only the string tells the
/// link apart.
fn three_functions(number_of_link: u8) -> Vec<InterfaceSummary> {
    let mut interfaces = vec![
        InterfaceSummary::new(0, 0x02, 0x0d, 0x00),
        InterfaceSummary::new(1, 0x0a, 0x00, 0x01)
            .with_endpoint(EndpointSummary::new(0x82, EndpointKind::Bulk))
            .with_endpoint(EndpointSummary::new(0x02, EndpointKind::Bulk)),
        InterfaceSummary::new(2, 0x02, 0x02, 0x01),
        InterfaceSummary::new(3, 0x0a, 0x00, 0x00)
            .with_endpoint(EndpointSummary::new(0x83, EndpointKind::Bulk))
            .with_endpoint(EndpointSummary::new(0x03, EndpointKind::Bulk)),
    ];
    interfaces.push(link(number_of_link, LINK));
    interfaces
}

/// A board with the three functions, attached at `location` with `serial`.
fn board(bus: &SimBus, location: &str, serial: &str) {
    bus.attach(DeviceSummary::new(location, 0x1234, 0x5678).with_serial(serial));
    assert!(bus.set_interfaces(location, three_functions(4)));
}

#[tokio::test]
async fn a_device_with_the_three_functions_is_found_by_its_string() {
    let bus = SimBus::new();
    board(&bus, "1-2", "A1");

    let scan = find_by_interface_string(&bus, LINK, Timeouts::default())
        .await
        .unwrap();
    assert!(scan.skipped.is_empty(), "{:?}", scan.skipped);
    assert_eq!(scan.matches.len(), 1);
    let found = &scan.matches[0];
    assert_eq!(found.device.location, "1-2");
    assert_eq!(found.interface.number, 4);
    assert_eq!(found.interface.class, 0xff);
    assert_eq!(found.interface.string.as_deref(), Some(LINK));
}

#[tokio::test]
async fn its_bulk_endpoints_do_io_and_only_its_interface_is_claimed() {
    let bus = SimBus::new();
    board(&bus, "1-2", "A1");
    bus.script("1-2", [Answer::Ok, Answer::Data(vec![0xde, 0xad])]);

    let mut io = open_interface(&bus, "1-2", LINK, &OpenOptions::default())
        .await
        .unwrap();
    io.bulk_out(&[7]).await.unwrap();
    assert_eq!(io.bulk_in(8).await.unwrap(), vec![0xde, 0xad]);

    // NCM and ACM stay with the operating system's drivers.
    assert_eq!(bus.claimed("1-2"), vec![4]);
}

#[tokio::test]
async fn a_device_without_the_string_is_not_matched() {
    let bus = SimBus::new();
    bus.attach(DeviceSummary::new("1-5", 0x1234, 0x5678).with_serial("C3"));
    // Only NCM and the console, and a vendor interface with another name.
    bus.set_interfaces(
        "1-5",
        [
            InterfaceSummary::new(0, 0x02, 0x0d, 0x00),
            link(1, "some-other-link"),
        ],
    );

    let scan = find_by_interface_string(&bus, LINK, Timeouts::default())
        .await
        .unwrap();
    assert!(scan.matches.is_empty());
    assert!(scan.skipped.is_empty());
    let error = open_interface(&bus, "1-5", LINK, &OpenOptions::default())
        .await
        .err();
    assert!(matches!(error, Some(UsbError::NotFound(_))), "{error:?}");
    assert!(bus.claimed("1-5").is_empty());
}

#[tokio::test]
async fn two_boards_are_told_apart_by_location_and_serial() {
    let bus = SimBus::new();
    board(&bus, "1-2", "A1");
    board(&bus, "1-3", "B2");
    bus.script("1-2", [Answer::Data(vec![0xa])]);
    bus.script("1-3", [Answer::Data(vec![0xb])]);

    let scan = find_by_interface_string(&bus, LINK, Timeouts::default())
        .await
        .unwrap();
    let found: Vec<(&str, Option<&str>)> = scan
        .matches
        .iter()
        .map(|m| (m.device.location.as_str(), m.device.serial.as_deref()))
        .collect();
    assert_eq!(found, vec![("1-2", Some("A1")), ("1-3", Some("B2"))]);

    // Opening by the second board's location reaches that board only.
    let mut second = open_interface(&bus, "1-3", LINK, &OpenOptions::default())
        .await
        .unwrap();
    assert_eq!(second.bulk_in(1).await.unwrap(), vec![0xb]);
    assert!(bus.claimed("1-2").is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_device_that_stalls_its_string_request_is_skipped_without_blocking_the_others() {
    let bus = SimBus::new();
    // 1-1 stalls the request, 1-2 never answers it, 1-3 is a good board.
    bus.attach(DeviceSummary::new("1-1", 0x1234, 0x5678));
    bus.set_interfaces("1-1", three_functions(4));
    bus.set_string_requests("1-1", StringRequests::Stall);
    bus.attach(DeviceSummary::new("1-2", 0x1234, 0x5678));
    bus.set_interfaces("1-2", three_functions(4));
    bus.set_string_requests("1-2", StringRequests::Silent);
    board(&bus, "1-3", "B2");

    let timeouts = Timeouts::default();
    let started = tokio::time::Instant::now();
    let scan = find_by_interface_string(&bus, LINK, timeouts)
        .await
        .unwrap();
    let waited = started.elapsed();

    let found: Vec<&str> = scan
        .matches
        .iter()
        .map(|m| m.device.location.as_str())
        .collect();
    assert_eq!(found, vec!["1-3"]);
    assert_eq!(
        scan.skipped,
        vec![
            atlas_usb::Skipped {
                location: "1-1".into(),
                error: UsbError::Stall,
            },
            atlas_usb::Skipped {
                location: "1-2".into(),
                error: UsbError::Timeout("the interface string request".into()),
            },
        ]
    );
    // The silent device costs one control timeout and no more.
    assert_eq!(waited, timeouts.control);
    assert!(waited < Duration::from_secs(10));
}

#[tokio::test]
async fn a_string_interface_without_bulk_endpoints_is_refused_at_open() {
    let bus = SimBus::new();
    bus.attach(DeviceSummary::new("1-4", 0x1234, 0x5678));
    bus.set_interfaces(
        "1-4",
        [InterfaceSummary::new(0, 0xff, 0, 0).with_string(LINK)],
    );
    let error = open_interface(&bus, "1-4", LINK, &OpenOptions::default())
        .await
        .err();
    assert!(matches!(error, Some(UsbError::Protocol(_))), "{error:?}");
}

#[tokio::test]
async fn a_bus_that_cannot_list_interfaces_skips_its_devices_with_a_reason() {
    struct Plain;

    #[async_trait::async_trait]
    impl UsbBus for Plain {
        async fn devices(&self) -> Result<Vec<DeviceSummary>, UsbError> {
            Ok(vec![DeviceSummary::new("1-1", 0x1234, 0x5678)])
        }

        async fn open(
            &self,
            location: &str,
            _options: &OpenOptions,
        ) -> Result<Box<dyn atlas_usb::UsbIo>, UsbError> {
            Err(UsbError::NotFound(location.to_string()))
        }
    }

    let scan = find_by_interface_string(&Plain, LINK, Timeouts::default())
        .await
        .unwrap();
    assert!(scan.matches.is_empty());
    assert_eq!(scan.skipped.len(), 1);
    assert!(matches!(scan.skipped[0].error, UsbError::Other(_)));
}
