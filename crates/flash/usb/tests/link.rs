//! The byte link on the simulated bus: the async pair and the blocking halves
//! that implement `LinkRead` and `LinkWrite`. The async tests run on the paused
//! clock; the blocking tests use real time only where one thread must wait for
//! another, with deadlines generous enough for a loaded machine.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use atlas_usb::sim::{SIM_BUFFER_BYTES, SimBus};
use atlas_usb::{
    DeviceSummary, EndpointDirection, EndpointKind, EndpointSummary, InterfaceSummary, Link,
    LinkOptions, LinkRead, LinkWrite, LinkWriter, OpenOptions, UsbError, WRITE_IN_FLIGHT,
    WRITE_TRANSFER, ZeroLength, is_device_gone, open_link, usb_error,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Builder;

const LINK: &str = "orion-link";
const SIM_PACKET: usize = 512;

/// A vendor interface with one bulk IN and one bulk OUT endpoint, named by its
/// string. The interface number is a placeholder.
fn link_interface(number: u8) -> InterfaceSummary {
    InterfaceSummary::new(number, 0xff, 0, 0)
        .with_string(LINK)
        .with_endpoint(EndpointSummary::new(0x81, EndpointKind::Bulk))
        .with_endpoint(EndpointSummary::new(0x01, EndpointKind::Bulk))
}

fn attach(bus: &SimBus, location: &str) {
    bus.attach(DeviceSummary::new(location, 0x1234, 0x5678).with_serial("A1"));
    assert!(bus.set_interfaces(location, [link_interface(4)]));
}

async fn open(bus: &SimBus, location: &str, options: LinkOptions) -> Link {
    open_link(bus, location, LINK, &OpenOptions::default(), options)
        .await
        .expect("the link opens")
}

fn pattern(len: usize, seed: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 31 + seed) % 251) as u8).collect()
}

#[tokio::test(start_paused = true)]
async fn bytes_arrive_in_order_both_ways_across_packet_boundaries() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, mut writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");

    // Packets the board sends: short, full, zero-length, a run of full ones.
    let sizes = [1usize, 511, 512, 0, 512, 3, 0, 100, 256];
    let from_board: Vec<Vec<u8>> = sizes
        .iter()
        .enumerate()
        .map(|(i, len)| pattern(*len, i))
        .collect();
    let expected_from_board: Vec<u8> = from_board.concat();
    let to_board = pattern(3000, 9);

    let mut got_from_board = Vec::new();
    let mut got_to_board = Vec::new();
    tokio::join!(
        async {
            for packet in &from_board {
                peer.send_packet(packet).await.expect("send");
            }
        },
        async {
            let mut buf = [0u8; 300];
            while got_from_board.len() < expected_from_board.len() {
                let n = tokio::time::timeout(Duration::from_secs(60), reader.read(&mut buf))
                    .await
                    .expect("no stall")
                    .expect("read");
                assert!(n > 0, "a read returned the end of the stream");
                got_from_board.extend_from_slice(&buf[..n]);
            }
        },
        async {
            writer.write_all(&to_board).await.expect("write");
            writer.flush().await.expect("flush");
        },
        async {
            while got_to_board.len() < to_board.len() {
                let packet = peer.recv_packet().await.expect("the link is open");
                assert!(packet.len() <= SIM_PACKET);
                got_to_board.extend(packet);
            }
        },
    );
    assert_eq!(got_from_board, expected_from_board);
    assert_eq!(got_to_board, to_board);
}

#[tokio::test(start_paused = true)]
async fn a_read_returns_what_arrived_and_does_not_wait_for_a_full_buffer() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, _writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");
    peer.send_packet(&[1, 2, 3]).await.unwrap();
    let mut buf = [0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(1), reader.read(&mut buf))
        .await
        .expect("a read with bytes waiting does not block")
        .unwrap();
    assert_eq!(&buf[..n], &[1, 2, 3]);
}

#[tokio::test(start_paused = true)]
async fn a_zero_length_packet_is_not_the_end_of_the_stream() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, _writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");
    peer.send_packet(&[]).await.unwrap();
    peer.send_packet(&[9, 8]).await.unwrap();
    let mut buf = [0u8; 16];
    let n = tokio::time::timeout(Duration::from_secs(1), reader.read(&mut buf))
        .await
        .expect("the zero-length packet does not end the read")
        .unwrap();
    assert_eq!(&buf[..n], &[9, 8]);
}

#[tokio::test(start_paused = true)]
async fn large_writes_are_split_into_packets_and_arrive_whole() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (_reader, mut writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");
    let data = pattern(100_000, 3);
    let mut received = Vec::new();
    tokio::join!(
        async {
            writer.write_all(&data).await.expect("write");
            writer.flush().await.expect("flush");
        },
        async {
            while received.len() < data.len() {
                let packet = peer.recv_packet().await.expect("open");
                assert!(packet.len() <= SIM_PACKET, "a packet of {}", packet.len());
                received.extend(packet);
            }
        },
    );
    assert_eq!(received, data);
}

#[tokio::test(start_paused = true)]
async fn a_write_ending_on_a_full_packet_sends_no_zero_length_packet_by_default() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (_reader, mut writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");
    writer.write_all(&[7; SIM_PACKET]).await.unwrap();
    writer.flush().await.unwrap();
    writer.write_all(&[5; 3]).await.unwrap();
    writer.flush().await.unwrap();
    assert_eq!(peer.recv_packet().await.unwrap(), vec![7; SIM_PACKET]);
    assert_eq!(peer.recv_packet().await.unwrap(), vec![5; 3]);
}

#[tokio::test(start_paused = true)]
async fn on_flush_a_write_ending_on_a_full_packet_is_closed_by_a_zero_length_packet() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let options = LinkOptions {
        zero_length: ZeroLength::OnFlush,
        ..LinkOptions::default()
    };
    let (_reader, mut writer) = open(&bus, "1-2", options).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");
    writer.write_all(&[7; SIM_PACKET]).await.unwrap();
    writer.flush().await.unwrap();
    assert_eq!(peer.recv_packet().await.unwrap(), vec![7; SIM_PACKET]);
    assert_eq!(peer.recv_packet().await.unwrap(), Vec::<u8>::new());
    // A flush with nothing new sends nothing more.
    writer.write_all(&[1]).await.unwrap();
    writer.flush().await.unwrap();
    assert_eq!(peer.recv_packet().await.unwrap(), vec![1]);
}

#[tokio::test(start_paused = true)]
async fn unplug_while_a_read_is_pending_ends_both_halves_as_gone() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, mut writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    let (read, ()) = tokio::join!(
        async {
            let mut buf = [0u8; 16];
            reader.read(&mut buf).await
        },
        async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert!(bus.detach("1-2"));
        },
    );
    let error = read.expect_err("the read fails");
    assert!(is_device_gone(&error), "{error}");
    // The writer learns on its next transfer.
    writer.write_all(&[1]).await.ok();
    let error = writer.flush().await.expect_err("the flush fails");
    assert!(is_device_gone(&error), "{error}");
    // Once gone, every call says so, without waiting.
    let mut buf = [0u8; 4];
    let error = reader.read(&mut buf).await.expect_err("still gone");
    assert!(is_device_gone(&error));
}

#[tokio::test(start_paused = true)]
async fn unplug_while_a_write_is_pending_ends_both_halves_as_gone() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, mut writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    // Nobody reads on the board side, so the write waits for room.
    let data = pattern(1 << 20, 1);
    let (write, ()) = tokio::join!(writer.write_all(&data), async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(bus.detach("1-2"));
    },);
    let error = write.expect_err("the write fails");
    assert!(is_device_gone(&error), "{error}");
    let mut buf = [0u8; 4];
    let error = reader.read(&mut buf).await.expect_err("the read fails");
    assert!(is_device_gone(&error), "{error}");
}

#[tokio::test(start_paused = true)]
async fn dropping_one_half_leaves_the_other_working_and_dropping_both_releases_the_claim() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let gauges = bus.gauges();
    let peer = bus.peer("1-2").expect("a peer");
    let (reader, writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    assert_eq!(gauges.live().devices, 1);

    // The writer goes; the reader keeps receiving.
    drop(writer);
    let mut reader = reader;
    peer.send_packet(&[4, 2]).await.unwrap();
    let mut buf = [0u8; 8];
    let n = reader.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], &[4, 2]);
    assert_eq!(gauges.live().devices, 1, "the reader still holds the claim");

    // The reader goes; a writer from a fresh link keeps sending.
    drop(reader);
    assert_eq!(
        gauges.live().devices,
        0,
        "dropping the last half releases it"
    );
    let (_reader, mut writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    writer.write_all(&[6]).await.unwrap();
    writer.flush().await.unwrap();
    assert_eq!(peer.recv_packet().await.unwrap(), vec![6]);
}

#[tokio::test(start_paused = true)]
async fn dropping_the_writer_leaves_the_reader_of_the_same_link_working() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let peer = bus.peer("1-2").expect("a peer");
    let (mut reader, writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    drop(writer);
    peer.send_packet(&[1]).await.unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(reader.read(&mut buf).await.unwrap(), 1);
}

#[tokio::test]
async fn an_interface_with_the_wrong_endpoints_is_refused_and_not_claimed() {
    let bus = SimBus::new();
    let two_in = InterfaceSummary::new(4, 0xff, 0, 0)
        .with_string(LINK)
        .with_endpoint(EndpointSummary::new(0x81, EndpointKind::Bulk))
        .with_endpoint(EndpointSummary::new(0x82, EndpointKind::Bulk))
        .with_endpoint(EndpointSummary::new(0x01, EndpointKind::Bulk));
    let with_interrupt = InterfaceSummary::new(4, 0xff, 0, 0)
        .with_string(LINK)
        .with_endpoint(EndpointSummary::new(0x81, EndpointKind::Bulk))
        .with_endpoint(EndpointSummary::new(0x01, EndpointKind::Bulk))
        .with_endpoint(EndpointSummary::new(0x83, EndpointKind::Interrupt));
    let no_out = InterfaceSummary::new(4, 0xff, 0, 0)
        .with_string(LINK)
        .with_endpoint(EndpointSummary::new(0x81, EndpointKind::Bulk));
    let cases = [
        ("1-1", two_in, "2 bulk IN, 1 bulk OUT"),
        ("1-2", with_interrupt, "1 interrupt IN"),
        ("1-3", no_out, "1 bulk IN"),
    ];
    for (location, interface, named) in cases {
        bus.attach(DeviceSummary::new(location, 0x1234, 0x5678));
        assert!(bus.set_interfaces(location, [interface]));
        let error = open_link(
            &bus,
            location,
            LINK,
            &OpenOptions::default(),
            LinkOptions::default(),
        )
        .await
        .expect_err("refused");
        let UsbError::Protocol(message) = error else {
            panic!("expected a refusal, got {error:?}");
        };
        assert!(message.contains(named), "{message}");
        assert!(bus.claimed(location).is_empty(), "{location} was claimed");
    }
}

#[tokio::test(start_paused = true)]
async fn a_fast_writer_waits_and_memory_stays_bounded() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    // No write timeout: this test is about back-pressure, not about time.
    let options = LinkOptions {
        write_timeout: None,
        ..LinkOptions::default()
    };
    let (_reader, mut writer) = open(&bus, "1-2", options).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");
    let total = 1 << 20;
    let data = pattern(total, 5);
    let accepted = AtomicUsize::new(0);

    tokio::join!(
        async {
            for piece in data.chunks(4096) {
                writer.write_all(piece).await.expect("write");
                accepted.fetch_add(piece.len(), Ordering::SeqCst);
            }
            writer.flush().await.expect("flush");
        },
        async {
            // Let the writer run until it has to wait. The board reads nothing.
            tokio::time::sleep(Duration::from_secs(1)).await;
            let taken = accepted.load(Ordering::SeqCst);
            assert!(taken < total, "the writer was never held back");
            // The board side holds at most one buffer and one transfer.
            assert!(
                peer.queued_from_pc() <= SIM_BUFFER_BYTES + WRITE_TRANSFER,
                "{} bytes queued for the board",
                peer.queued_from_pc()
            );
            // The PC side holds at most the transfers it has queued.
            assert!(
                taken <= peer.queued_from_pc() + WRITE_IN_FLIGHT * WRITE_TRANSFER,
                "{taken} bytes accepted"
            );
            // The board reads now; the writer finishes and all bytes arrive.
            let mut got = Vec::new();
            while got.len() < total {
                got.extend(peer.recv_packet().await.expect("open"));
            }
            assert_eq!(got, data);
        },
    );
    assert_eq!(accepted.load(Ordering::SeqCst), total);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_read_is_reported_and_the_next_read_works() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, _writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");
    peer.set_stall(EndpointDirection::In, true);
    let mut buf = [0u8; 4];
    let error = reader.read(&mut buf).await.expect_err("stalled");
    assert_eq!(usb_error(&error), Some(&UsbError::Stall));
    assert!(!is_device_gone(&error));
    peer.set_stall(EndpointDirection::In, false);
    peer.send_packet(&[3]).await.unwrap();
    assert_eq!(reader.read(&mut buf).await.unwrap(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_write_is_sticky_because_the_stream_position_is_unknown() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (_reader, mut writer) = open(&bus, "1-2", LinkOptions::default()).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");
    peer.set_stall(EndpointDirection::Out, true);
    writer.write_all(&[1, 2, 3]).await.ok();
    let error = writer.flush().await.expect_err("stalled");
    assert_eq!(usb_error(&error), Some(&UsbError::Stall));
    peer.set_stall(EndpointDirection::Out, false);
    let error = writer.write_all(&[4]).await.expect_err("still failed");
    assert_eq!(usb_error(&error), Some(&UsbError::Stall));
}

#[tokio::test(start_paused = true)]
async fn a_read_timeout_loses_nothing_and_can_be_retried() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let options = LinkOptions {
        read_timeout: Some(Duration::from_secs(1)),
        ..LinkOptions::default()
    };
    let (mut reader, _writer) = open(&bus, "1-2", options).await.into_parts();
    let peer = bus.peer("1-2").expect("a peer");
    let mut buf = [0u8; 4];
    let error = reader.read(&mut buf).await.expect_err("timed out");
    assert!(matches!(usb_error(&error), Some(UsbError::Timeout(_))));
    assert!(!is_device_gone(&error));
    peer.send_packet(&[5]).await.unwrap();
    assert_eq!(reader.read(&mut buf).await.unwrap(), 1);
    assert_eq!(buf[0], 5);
}

#[tokio::test(start_paused = true)]
async fn a_write_timeout_is_sticky() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let options = LinkOptions {
        write_timeout: Some(Duration::from_secs(1)),
        ..LinkOptions::default()
    };
    let (_reader, mut writer) = open(&bus, "1-2", options).await.into_parts();
    // The board never reads: the second transfer waits and times out.
    writer.write_all(&pattern(40_000, 2)).await.ok();
    let error = writer.flush().await.expect_err("timed out");
    assert!(matches!(usb_error(&error), Some(UsbError::Timeout(_))));
    let error = writer.write_all(&[1]).await.expect_err("still timed out");
    assert!(matches!(usb_error(&error), Some(UsbError::Timeout(_))));
}

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread().enable_time().build().unwrap()
}

#[test]
fn the_blocking_halves_move_bytes_both_ways_and_are_send_and_static() {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<LinkWriter>();
    assert_send_static::<atlas_usb::LinkReader>();
    assert_send_static::<atlas_usb::BlockingReader>();
    assert_send_static::<atlas_usb::BlockingWriter>();

    use std::io::{Read, Write};
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let link = runtime().block_on(open(&bus, "1-2", LinkOptions::default()));
    let (mut reader, mut writer) = link.into_blocking().unwrap();
    let peer = bus.peer("1-2").expect("a peer");
    let board = std::thread::spawn(move || {
        runtime().block_on(async {
            let mut got = Vec::new();
            while got.len() < 10_000 {
                got.extend(peer.recv_packet().await.expect("open"));
            }
            for packet in pattern(10_000, 4).chunks(SIM_PACKET) {
                peer.send_packet(packet).await.unwrap();
            }
            got
        })
    });
    Write::write_all(&mut writer, &pattern(10_000, 8)).unwrap();
    Write::flush(&mut writer).unwrap();
    let mut back = vec![0u8; 10_000];
    Read::read_exact(&mut reader, &mut back).unwrap();
    assert_eq!(back, pattern(10_000, 4));
    assert_eq!(board.join().unwrap(), pattern(10_000, 8));
}

#[test]
fn link_read_blocks_until_bytes_arrive_and_returns_zero_for_a_zero_length_packet() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, _writer) = runtime()
        .block_on(open(&bus, "1-2", LinkOptions::default()))
        .into_blocking()
        .unwrap();
    let peer = bus.peer("1-2").expect("a peer");
    let board = std::thread::spawn(move || {
        runtime().block_on(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            peer.send_packet(&[]).await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            peer.send_packet(&[1, 2, 3]).await.unwrap();
        })
    });
    let mut buf = [0u8; 32];
    // The zero-length packet: nothing yet, not the end.
    assert_eq!(LinkRead::read(&mut reader, &mut buf).unwrap(), 0);
    // The bytes: returned as soon as they arrive.
    assert_eq!(LinkRead::read(&mut reader, &mut buf).unwrap(), 3);
    assert_eq!(&buf[..3], &[1, 2, 3]);
    board.join().unwrap();
}

#[test]
fn dropping_the_write_half_stops_a_blocked_read_and_the_claim_goes_with_both_halves() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let gauges = bus.gauges();
    let (mut reader, writer) = runtime()
        .block_on(open(&bus, "1-2", LinkOptions::default()))
        .into_blocking()
        .unwrap();
    assert_eq!(gauges.live().devices, 1);
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let read_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 8];
        let result = LinkRead::read(&mut reader, &mut buf).map_err(|e| e.to_string());
        let _ = result_tx.send(result);
    });
    // The read is blocked in its transfer: no bytes, no unplug.
    std::thread::sleep(Duration::from_millis(50));
    assert!(result_rx.try_recv().is_err(), "the read returned early");
    let started = std::time::Instant::now();
    drop(writer);
    let result = result_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the blocked read returned after the write half was dropped");
    let message = result.expect_err("a stopped read is an error");
    assert!(message.contains("write half"), "{message}");
    assert!(started.elapsed() < Duration::from_secs(5));
    read_thread.join().expect("the read thread ends");
    assert_eq!(gauges.live().devices, 0, "the interface claim was released");
}

#[test]
fn after_the_write_half_is_dropped_every_read_errors_at_once_even_with_none_pending() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let gauges = bus.gauges();
    let (mut reader, writer) = runtime()
        .block_on(open(&bus, "1-2", LinkOptions::default()))
        .into_blocking()
        .unwrap();
    // No read is pending when the write half goes.
    drop(writer);
    let mut buf = [0u8; 8];
    let started = std::time::Instant::now();
    for _ in 0..3 {
        let error = LinkRead::read(&mut reader, &mut buf).expect_err("stopped");
        assert!(!is_device_gone(&error));
    }
    assert!(started.elapsed() < Duration::from_secs(2), "a read waited");
    drop(reader);
    assert_eq!(gauges.live().devices, 0, "the claim was released");
}

#[test]
fn a_read_thread_blocked_when_the_write_half_drops_ends_and_a_later_read_errors_at_once() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, writer) = runtime()
        .block_on(open(&bus, "1-2", LinkOptions::default()))
        .into_blocking()
        .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut buf = [0u8; 8];
        let first = LinkRead::read(&mut reader, &mut buf).is_err();
        let second = LinkRead::read(&mut reader, &mut buf).is_err();
        let _ = tx.send((first, second));
    });
    std::thread::sleep(Duration::from_millis(50));
    drop(writer);
    let (first, second) = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the blocked read returned within the deadline");
    assert!(first && second);
    thread.join().expect("the thread ends");
}

#[test]
fn a_blocked_read_on_one_thread_does_not_delay_a_write_on_another() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, mut writer) = runtime()
        .block_on(open(&bus, "1-2", LinkOptions::default()))
        .into_blocking()
        .unwrap();
    let peer = bus.peer("1-2").expect("a peer");
    let read_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 16];
        let n = LinkRead::read(&mut reader, &mut buf).expect("the read");
        buf[..n].to_vec()
    });
    std::thread::sleep(Duration::from_millis(50));
    let started = std::time::Instant::now();
    LinkWrite::write_all(&mut writer, b"ping").expect("the write goes through");
    assert!(started.elapsed() < Duration::from_secs(2));
    let sent = runtime().block_on(peer.recv_packet()).unwrap();
    assert_eq!(sent, b"ping");
    runtime().block_on(peer.send_packet(b"pong")).unwrap();
    assert_eq!(read_thread.join().unwrap(), b"pong");
}

#[test]
fn link_read_reports_an_unplug_as_gone_and_does_not_return_zero_forever() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, _writer) = runtime()
        .block_on(open(&bus, "1-2", LinkOptions::default()))
        .into_blocking()
        .unwrap();
    let unplug = bus.clone();
    let pull = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        assert!(unplug.detach("1-2"));
    });
    let started = std::time::Instant::now();
    let mut buf = [0u8; 8];
    let error = LinkRead::read(&mut reader, &mut buf).expect_err("gone");
    pull.join().unwrap();
    assert!(is_device_gone(&error), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
    // Once gone, every later read says so too.
    let error = LinkRead::read(&mut reader, &mut buf).expect_err("still gone");
    assert!(is_device_gone(&error), "{error}");
}

#[test]
fn link_write_all_returns_once_every_byte_is_with_the_device_and_alternates_under_a_slow_peer() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (mut reader, mut writer) = runtime()
        .block_on(open(&bus, "1-2", LinkOptions::default()))
        .into_blocking()
        .unwrap();
    let peer = bus.peer("1-2").expect("a peer");

    // A board that answers each request after a pause, echoing it back.
    let rounds = 20;
    let board = std::thread::spawn(move || {
        runtime().block_on(async {
            for _ in 0..rounds {
                let request = peer.recv_packet().await.expect("open");
                tokio::time::sleep(Duration::from_millis(20)).await;
                peer.send_packet(&request).await.unwrap();
            }
        });
    });

    for round in 0..rounds {
        let request = format!("req {round}\n");
        LinkWrite::write_all(&mut writer, request.as_bytes()).expect("the write returns");
        let mut reply = Vec::new();
        while reply.len() < request.len() {
            let mut buf = [0u8; 64];
            let n = LinkRead::read(&mut reader, &mut buf).unwrap();
            reply.extend_from_slice(&buf[..n]);
        }
        assert_eq!(reply, request.as_bytes());
    }
    board.join().unwrap();
}

#[test]
fn link_write_sends_no_zero_length_packet() {
    let bus = SimBus::new();
    attach(&bus, "1-2");
    let (_reader, mut writer) = runtime()
        .block_on(open(&bus, "1-2", LinkOptions::default()))
        .into_blocking()
        .unwrap();
    let peer = bus.peer("1-2").expect("a peer");
    let board = std::thread::spawn(move || runtime().block_on(peer.recv_packet()));
    LinkWrite::write_all(&mut writer, &[9; SIM_PACKET]).unwrap();
    assert_eq!(board.join().unwrap(), Some(vec![9; SIM_PACKET]));
}
