//! The local server and a viewer, in one process over loopback: what the host sets reaches the
//! viewer, what the viewer writes reaches the server, a write of the wrong type is refused on
//! the viewer's side, and dropping the viewer closes its connection on the server.

use std::time::Duration;

use atlas_nt::{Error, LocalServer, LocalServerConfig, Update, Value, Viewer, ViewerConfig};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);

async fn until<T>(viewer: &mut Viewer, mut wanted: impl FnMut(&Update) -> Option<T>) -> T {
    timeout(WAIT, async {
        loop {
            let update = viewer.next().await.expect("the viewer runs");
            if let Some(found) = wanted(&update) {
                return found;
            }
        }
    })
    .await
    .expect("the update came")
}

#[tokio::test]
async fn a_viewer_reads_and_writes_the_local_server() {
    let mut server = LocalServer::start(LocalServerConfig {
        bind: "127.0.0.1:0".parse().expect("addr"),
        persist_path: None,
    })
    .await
    .expect("the server starts");
    server
        .publish("/Cam/exposure", "double", false)
        .await
        .expect("publish");
    server
        .set("/Cam/exposure", Value::Double(20.0))
        .await
        .expect("set");

    let mut viewer = Viewer::connect(ViewerConfig {
        host: "127.0.0.1".into(),
        port: server.local_addr().port(),
        name: "bench".into(),
    })
    .await
    .expect("the viewer starts");

    until(&mut viewer, |u| {
        matches!(
            u,
            Update::Connection {
                connected: true,
                ..
            }
        )
        .then_some(())
    })
    .await;
    let announced = until(&mut viewer, |u| match u {
        Update::Announced(topic) if topic.name == "/Cam/exposure" => Some(topic.clone()),
        _ => None,
    })
    .await;
    assert_eq!(announced.type_name, "double");
    assert!(!announced.persistent());
    let sample = until(&mut viewer, |u| match u {
        Update::Value { name, sample } if name == "/Cam/exposure" => Some(sample.clone()),
        _ => None,
    })
    .await;
    assert_eq!(sample.value, Value::Double(20.0));
    assert_eq!(
        viewer.topic("/Cam/exposure").and_then(|t| t.sample.clone()),
        Some(sample)
    );

    // A write of the wrong type is refused here and never reaches the server.
    let refused = viewer.edit("/Cam/exposure", Value::Int(3)).await;
    assert!(
        matches!(refused, Err(Error::TypeMismatch { ref expected, got: "int", .. }) if expected == "double"),
        "{refused:?}"
    );
    assert!(matches!(
        viewer.edit("/Cam/missing", Value::Double(1.0)).await,
        Err(Error::UnknownTopic(_))
    ));

    viewer
        .edit("/Cam/exposure", Value::Double(5.5))
        .await
        .expect("a fitting edit is sent");
    let written = timeout(WAIT, async {
        loop {
            if let atlas_nt::ServerUpdate::Written { name, value, .. } =
                server.next_event().await.expect("the server runs")
            {
                return (name, value);
            }
        }
    })
    .await
    .expect("the server saw the write");
    assert_eq!(written, ("/Cam/exposure".to_owned(), Value::Double(5.5)));
    assert_eq!(
        server
            .topic("/Cam/exposure")
            .and_then(|t| t.sample)
            .map(|s| s.value),
        Some(Value::Double(5.5))
    );

    // Dropping the viewer closes its connection: the server sees the client leave.
    assert_eq!(server.clients().len(), 1);
    drop(viewer);
    timeout(WAIT, async {
        while !server.clients().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the client left the server");
}
