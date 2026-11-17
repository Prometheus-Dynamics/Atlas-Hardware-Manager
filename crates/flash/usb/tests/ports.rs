//! Port-path selection, the identity check, and the typed outcome of a stop, on
//! the simulated bus. Every case the brief names has a test here: no device at
//! the port (Absent), a wrong identity (Refused), a device present but silent
//! (Deaf), a device that vanishes mid-exchange (Absent), a device that reappears
//! on another port, a re-enumeration on the same port, two identical devices on
//! different ports, a stall and a refused access (Refused), and an expected
//! device that cannot be opened in time (TimedOut).

use std::time::Duration;

use atlas_usb::sim::{Answer, SimBus};
use atlas_usb::{
    DeviceSummary, EndpointKind, EndpointSummary, Expect, InterfaceSummary, OpenOptions, Outcome,
    Party, PortPath, Reason, UsbBus, UsbError, UsbId, observe, open_expected,
};

const BOARD: UsbId = UsbId {
    vendor: 0x1d6b,
    product: 0x0100,
};
const STAGE: UsbId = UsbId {
    vendor: 0x2e8a,
    product: 0x000c,
};

fn port(text: &str) -> PortPath {
    text.parse().unwrap()
}

fn device(location: &str, id: UsbId) -> DeviceSummary {
    DeviceSummary::new(location, id.vendor, id.product)
}

/// A vendor interface named `flash` with a bulk OUT and a bulk IN endpoint.
fn flash_interface() -> InterfaceSummary {
    InterfaceSummary::new(0, 0xff, 0, 0)
        .with_string("flash")
        .with_endpoint(EndpointSummary::new(0x81, EndpointKind::Bulk))
        .with_endpoint(EndpointSummary::new(0x01, EndpointKind::Bulk))
}

fn attach(bus: &SimBus, id: UsbId, location: &str) {
    bus.attach(device(location, id));
    bus.set_interfaces(location, [flash_interface()]);
}

fn opts() -> OpenOptions {
    OpenOptions::default()
}

fn expect(id: UsbId) -> Expect {
    Expect::Exactly(id)
}

#[tokio::test(start_paused = true)]
async fn the_port_is_opened_and_only_that_port() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    attach(&bus, BOARD, "1-3");
    let opened =
        match open_expected(&bus, &port("1-3"), expect(BOARD), &opts(), Duration::ZERO).await {
            Outcome::Done(opened) => opened,
            other => panic!("expected Done, got {other}"),
        };
    assert_eq!(opened.device.location, "1-3");
    assert_eq!(bus.claimed("1-3"), vec![0], "the device on 1-3 is claimed");
    assert!(
        bus.claimed("1-2").is_empty(),
        "the identical board on 1-2 is not touched"
    );
}

#[tokio::test(start_paused = true)]
async fn a_wrong_identity_at_the_port_is_refused_and_nothing_is_claimed() {
    let bus = SimBus::new();
    attach(&bus, STAGE, "1-2");
    let outcome = open_expected(&bus, &port("1-2"), expect(BOARD), &opts(), Duration::ZERO).await;
    match outcome {
        Outcome::Refused(refused) => {
            assert_eq!(refused.by, Party::Host);
            assert_eq!(
                refused.reason,
                Reason::WrongIdentity {
                    location: port("1-2"),
                    expected: BOARD,
                    found: STAGE,
                }
            );
        }
        other => panic!("expected a refusal, got {other}"),
    }
    assert!(bus.claimed("1-2").is_empty(), "nothing was claimed");
    assert!(bus.records("1-2").is_empty(), "nothing was sent");
}

#[tokio::test(start_paused = true)]
async fn nothing_on_the_port_is_absent() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-5");
    match open_expected(
        &bus,
        &port("1-2"),
        expect(BOARD),
        &opts(),
        Duration::from_secs(2),
    )
    .await
    {
        Outcome::Absent(absent) => {
            assert_eq!(absent.port, port("1-2"));
            assert_eq!(absent.looked_for, Some(BOARD));
            assert_eq!(absent.waited, Duration::from_secs(2));
        }
        other => panic!("expected Absent, got {other}"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_device_present_but_silent_is_deaf() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    let opened =
        match open_expected(&bus, &port("1-2"), expect(BOARD), &opts(), Duration::ZERO).await {
            Outcome::Done(opened) => opened,
            other => panic!("expected Done, got {other}"),
        };
    let mut io = opened.io;
    bus.script("1-2", [Answer::Timeout]);
    let error = io.bulk_in(64).await.expect_err("silent");
    assert!(matches!(error, UsbError::Timeout(_)));
    let outcome: Outcome<()> = observe(
        &bus,
        &port("1-2"),
        expect(BOARD),
        "reading the boot ROM's reply",
        Duration::from_secs(5),
        false,
        error,
    )
    .await;
    match outcome {
        Outcome::Deaf(deaf) => {
            assert_eq!(deaf.port, port("1-2"));
            assert_eq!(deaf.expected, Some(BOARD));
            assert_eq!(deaf.waited, Duration::from_secs(5));
            assert!(deaf.sent.contains("reading the boot ROM's reply"));
        }
        other => panic!("expected Deaf, got {other}"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_device_that_vanishes_mid_exchange_is_absent_and_the_write_may_have_applied() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    let mut io = open_expected(&bus, &port("1-2"), expect(BOARD), &opts(), Duration::ZERO)
        .await
        .into_result()
        .expect("opened")
        .io;
    bus.vanish_after("1-2", 2);
    io.bulk_out(&[1, 2, 3]).await.expect("first answer");
    io.bulk_out(&[4, 5, 6]).await.expect("second answer");
    let error = io
        .bulk_out(&[7])
        .await
        .expect_err("the device left the bus");
    assert_eq!(error, UsbError::Disconnected);
    let outcome: Outcome<()> = observe(
        &bus,
        &port("1-2"),
        expect(BOARD),
        "writing the image",
        Duration::from_secs(5),
        true,
        error,
    )
    .await;
    match outcome {
        Outcome::Absent(absent) => {
            assert_eq!(absent.port, port("1-2"));
            assert!(
                absent.may_have_applied,
                "the write was sent, its end not seen"
            );
            assert!(absent.during.contains("writing the image"));
        }
        other => panic!("expected Absent, got {other}"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_re_enumeration_on_the_same_port_follows_the_port_and_the_old_handle_dies() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    let mut old = open_expected(&bus, &port("1-2"), expect(BOARD), &opts(), Duration::ZERO)
        .await
        .into_result()
        .expect("opened")
        .io;
    // The boot ROM hands over to its stage: same port, new identity.
    attach(&bus, STAGE, "1-2");
    let opened = open_expected(&bus, &port("1-2"), expect(STAGE), &opts(), Duration::ZERO)
        .await
        .into_result()
        .expect("the stage on the same port");
    assert_eq!(opened.device.vendor, STAGE.vendor);
    assert_eq!(opened.device.product, STAGE.product);
    let stale = old
        .bulk_out(&[0])
        .await
        .expect_err("the old handle is dead");
    assert_eq!(stale, UsbError::Disconnected);
    // The boot ROM's identity is no longer what the port holds.
    assert!(matches!(
        open_expected(&bus, &port("1-2"), expect(BOARD), &opts(), Duration::ZERO).await,
        Outcome::Refused(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_re_enumeration_is_waited_for_within_the_given_wait() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    // The device leaves the port and comes back as the stage after two seconds.
    bus.detach("1-2");
    let options = opts();
    let here = port("1-2");
    let (opened, ()) = tokio::join!(
        open_expected(
            &bus,
            &here,
            expect(STAGE),
            &options,
            Duration::from_secs(30)
        ),
        async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            attach(&bus, STAGE, "1-2");
        }
    );
    assert!(
        opened.is_done(),
        "the stage was found on the port: {opened}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_device_that_reappears_on_another_port_is_absent_here_and_found_there() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    assert!(bus.move_device("1-2", "1-4"));
    assert!(matches!(
        open_expected(&bus, &port("1-2"), expect(BOARD), &opts(), Duration::ZERO).await,
        Outcome::Absent(_)
    ));
    let opened = open_expected(&bus, &port("1-4"), expect(BOARD), &opts(), Duration::ZERO).await;
    assert!(opened.is_done(), "found on 1-4: {opened}");
    assert_eq!(bus.claimed("1-4"), vec![0]);
}

#[tokio::test(start_paused = true)]
async fn a_stall_is_refused_by_the_device() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    let mut io = open_expected(&bus, &port("1-2"), expect(BOARD), &opts(), Duration::ZERO)
        .await
        .into_result()
        .expect("opened")
        .io;
    bus.script("1-2", [Answer::Stall]);
    let error = io.bulk_out(&[1]).await.expect_err("stalled");
    let outcome: Outcome<()> = observe(
        &bus,
        &port("1-2"),
        expect(BOARD),
        "writing",
        Duration::from_secs(5),
        true,
        error,
    )
    .await;
    match outcome {
        Outcome::Refused(refused) => {
            assert_eq!(refused.by, Party::Device);
            assert!(
                matches!(refused.reason, Reason::Message(ref text) if text.contains("stalled"))
            );
        }
        other => panic!("expected a refusal, got {other}"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_refused_access_is_refused_by_this_host_with_its_fix() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    bus.fail_opens(
        "1-2",
        1,
        UsbError::Access {
            message: "No permission.".into(),
            fix: "Install the rule.".into(),
        },
    );
    match open_expected(
        &bus,
        &port("1-2"),
        expect(BOARD),
        &opts(),
        Duration::from_secs(5),
    )
    .await
    {
        Outcome::Refused(refused) => {
            assert_eq!(refused.by, Party::Host);
            assert_eq!(
                refused.reason,
                Reason::Message("No permission. Install the rule.".into())
            );
        }
        other => panic!("expected a refusal, got {other}"),
    }
}

#[tokio::test(start_paused = true)]
async fn an_expected_device_that_cannot_be_opened_in_time_is_timed_out() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    bus.fail_opens("1-2", 1000, UsbError::Timeout("opening".into()));
    match open_expected(
        &bus,
        &port("1-2"),
        expect(BOARD),
        &opts(),
        Duration::from_secs(3),
    )
    .await
    {
        Outcome::TimedOut(timed) => {
            assert_eq!(timed.port, port("1-2"));
            assert_eq!(timed.bound, Duration::from_secs(3));
        }
        other => panic!("expected TimedOut, got {other}"),
    }
}

#[tokio::test(start_paused = true)]
async fn the_bus_listing_reports_the_device_on_its_port_only() {
    let bus = SimBus::new();
    attach(&bus, BOARD, "1-2");
    let listed = bus.devices().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        atlas_usb::at_port(&bus, &port("1-2"))
            .await
            .unwrap()
            .unwrap()
            .location,
        "1-2"
    );
    assert!(
        atlas_usb::at_port(&bus, &port("1-3"))
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_wrong_identity_error_names_the_port_and_both_ids() {
    let error = UsbError::WrongIdentity {
        location: port("3-4.2"),
        expected: BOARD,
        found: STAGE,
    };
    assert_eq!(
        error.to_string(),
        "the device at 3-4.2 is 2e8a:000c, not the expected 1d6b:0100"
    );
}

#[test]
fn each_outcome_says_what_it_observed_in_one_line() {
    let deaf: Outcome<()> = Outcome::Deaf(atlas_usb::Deaf {
        port: port("3-4.2.1.4.1"),
        expected: Some(BOARD),
        sent: "the reply".into(),
        waited: Duration::from_secs(5),
        may_have_applied: false,
    });
    assert_eq!(
        deaf.to_string(),
        "deaf: the 1d6b:0100 device at 3-4.2.1.4.1 is on the bus but did not answer the reply within 5s"
    );
}
