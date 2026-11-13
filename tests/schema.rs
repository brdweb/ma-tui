use ma_tui::{api::ApiClient, controller::Controller, events::Event};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// A server answering `GET /info` with `info` and any command with `[]`,
/// counting the `/info` reads.
async fn server(info: Value) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let reads = Arc::new(AtomicUsize::new(0));
    let counted = reads.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (info, counted) = (info.clone(), counted.clone());
            tokio::spawn(async move {
                let mut buffer = vec![0; 8192];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]);
                let body = if request.starts_with("GET /info ") {
                    counted.fetch_add(1, Ordering::SeqCst);
                    info.to_string()
                } else {
                    "[]".into()
                };
                let _ = socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            });
        }
    });
    (url, reads)
}

async fn schema_for(info: Value) -> Option<u32> {
    let (url, _) = server(info).await;
    let api = ApiClient::new(&url, "fixture-token").unwrap();
    api.refresh_schema().await.unwrap();
    api.schema()
}

#[tokio::test]
async fn the_schema_is_read_from_server_information() {
    let (url, _) = server(json!({"server_version":"2.11.0b8","schema_version":101})).await;
    let api = ApiClient::new(&url, "fixture-token").unwrap();
    assert_eq!(api.schema(), None, "unknown before /info is read");
    assert!(!api.schema_at_least(1));
    api.refresh_schema().await.unwrap();
    assert_eq!(api.schema(), Some(101));
    assert!(api.schema_at_least(101));
    assert!(!api.schema_at_least(102));
    // Every clone shares what was learned.
    assert_eq!(api.clone().schema(), Some(101));
}

#[tokio::test]
async fn a_missing_or_malformed_schema_means_the_oldest_behaviour() {
    for info in [
        json!({"server_version":"2.10.5"}),
        json!({"server_version":"2.10.5","schema_version":null}),
        json!({"server_version":"2.10.5","schema_version":"101"}),
        json!({"server_version":"2.10.5","schema_version":-1}),
        json!({"server_version":"2.10.5","schema_version":65.5}),
        json!({"server_version":"2.10.5","schema_version":0}),
        json!({"server_version":"2.10.5","schema_version":5_000_000_000u64}),
    ] {
        assert_eq!(schema_for(info.clone()).await, None, "{info}");
    }
    assert_eq!(
        schema_for(json!({"server_version":"2.10.9","schema_version":65})).await,
        Some(65)
    );
}

#[tokio::test]
async fn a_reconnect_reads_the_schema_again() {
    let (url, reads) = server(json!({"server_version":"2.11.0b8","schema_version":101})).await;
    let api = ApiClient::new(&url, "fixture-token").unwrap();
    let (events, stream) = tokio::sync::mpsc::channel(4);
    let controller = Controller::start(api.clone(), Some(stream), false);
    events.send(Event::Online).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while api.schema().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the controller re-reads /info when the stream comes back");
    assert_eq!(api.schema(), Some(101));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    controller.shutdown().await;
}
