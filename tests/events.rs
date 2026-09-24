// Offline fixtures only: no Music Assistant server is used.
#[path = "../src/events.rs"]
mod events;

use events::{translate, Event, Events};
use serde_json::json;

#[test]
fn the_event_socket_url_follows_the_api_base_and_refuses_unsafe_ones() {
    let url = |base| events::events_url(base).map(|u| u.to_string());
    assert_eq!(url("http://host:8095").unwrap(), "ws://host:8095/ws");
    assert_eq!(url("https://host").unwrap(), "wss://host/ws");
    // A reverse-proxy prefix has to survive, exactly as the HTTP client's does.
    assert_eq!(
        url("https://host/music/").unwrap(),
        "wss://host/music/ws",
        "a base path prefix is preserved"
    );
    for rejected in [
        "http://user:pass@host",
        "https://host/?token=x",
        "https://host/#fragment",
        "ftp://host",
        "not a url",
    ] {
        assert!(url(rejected).is_err(), "{rejected} must be refused");
    }
}

#[test]
fn only_events_this_application_acts_on_are_translated() {
    let event = |name, data| json!({"event": name, "object_id": "q1", "data": data});
    assert_eq!(
        translate(&event("player_updated", json!({}))),
        Some(Event::Players)
    );
    assert_eq!(
        translate(&event("player_added", json!({}))),
        Some(Event::Players)
    );
    assert_eq!(
        translate(&event("queue_updated", json!({}))),
        Some(Event::Queue("q1".into()))
    );
    assert_eq!(
        translate(&event("queue_items_updated", json!({}))),
        Some(Event::QueueItems("q1".into()))
    );
    // The one payload that is read: the server signals elapsed seconds itself.
    assert_eq!(
        translate(&event("queue_time_updated", json!(12.5))),
        Some(Event::Elapsed("q1".into(), 12.5))
    );
    assert_eq!(
        translate(&event("playlog_updated", json!({}))),
        Some(Event::Playlog)
    );

    // Anything unrecognised or malformed is ignored rather than guessed at.
    assert_eq!(translate(&event("dashboard_show", json!({}))), None);
    assert_eq!(translate(&json!({"no_event_field": 1})), None);
    assert_eq!(
        translate(&json!({"event":"queue_updated","object_id":""})),
        None,
        "an empty object id routes nowhere"
    );
    assert_eq!(
        translate(&event("queue_time_updated", json!("nonsense"))),
        None
    );
    assert_eq!(
        translate(&event("queue_time_updated", json!(f64::INFINITY))),
        None,
        "a non-finite position is not a position"
    );
}

#[test]
fn media_item_events_forward_full_items_and_ignore_empty_object_ids() {
    let item = json!({
        "item_id":"42",
        "provider":"library",
        "uri":"library://track/42",
        "name":"Song",
    });
    for name in ["media_item_updated", "media_item_added"] {
        assert_eq!(
            translate(&json!({"event":name,"object_id":"library://track/42","data":item})),
            Some(Event::MediaItem {
                uri: "library://track/42".into(),
                item: Some(item.clone()),
            })
        );
    }
    assert_eq!(
        translate(&json!({"event":"media_item_deleted","object_id":"library://track/42"})),
        Some(Event::MediaItem {
            uri: "library://track/42".into(),
            item: None,
        })
    );
    for name in [
        "media_item_updated",
        "media_item_added",
        "media_item_deleted",
    ] {
        assert_eq!(
            translate(&json!({"event":name,"object_id":"","data":item})),
            None
        );
    }
}

/// The real handshake: the server greets the connection before answering the
/// auth command, so the stream has to read past its own greeting.
#[tokio::test]
async fn the_stream_authenticates_past_the_server_greeting_and_forwards_events() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        // Music Assistant sends its server information first, unprompted.
        socket
            .send(Message::text(
                json!({"server_version":"2.10.2","schema_version":65}).to_string(),
            ))
            .await
            .unwrap();
        let Some(Ok(Message::Text(request))) = socket.next().await else {
            panic!("expected the auth command");
        };
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["command"], "auth");
        assert_eq!(request["args"]["token"], "fixture-token");
        let id = request["message_id"].as_str().unwrap().to_owned();
        socket
            .send(Message::text(
                json!({"message_id": id, "result": {"authenticated": true}}).to_string(),
            ))
            .await
            .unwrap();
        for event in [
            json!({"event":"queue_time_updated","object_id":"q1","data":31.0}),
            json!({"event":"player_updated","object_id":"p1","data":{}}),
        ] {
            socket.send(Message::text(event.to_string())).await.unwrap();
        }
        // Hold the socket open so the stream is not torn down mid-test.
        std::future::pending::<()>().await;
    });

    let (events, mut stream) =
        Events::start(&format!("http://{address}"), "fixture-token").unwrap();
    let mut seen = Vec::new();
    for _ in 0..3 {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), stream.recv())
            .await
            .expect("the stream must deliver")
            .unwrap();
        seen.push(event);
    }
    assert_eq!(
        seen,
        vec![
            Event::Online,
            Event::Elapsed("q1".into(), 31.0),
            Event::Players,
        ]
    );
    events.shutdown().await;
    server.abort();
}

/// A rejected token must not spin: the stream backs off instead of reconnecting
/// in a tight loop, and never reports itself online.
#[tokio::test]
async fn a_rejected_token_does_not_report_the_stream_online() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = attempts.clone();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            counted.fetch_add(1, std::sync::atomic::Ordering::Release);
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Some(Ok(Message::Text(request))) = socket.next().await else {
                continue;
            };
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            let id = request["message_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let _ = socket
                .send(Message::text(
                    json!({"message_id": id, "error_code": 6, "details": "invalid token"})
                        .to_string(),
                ))
                .await;
        }
    });

    let (events, mut stream) = Events::start(&format!("http://{address}"), "wrong").unwrap();
    let delivered = tokio::time::timeout(std::time::Duration::from_secs(2), stream.recv()).await;
    assert!(
        delivered.is_err(),
        "a refused stream reports nothing, least of all Online"
    );
    assert!(
        attempts.load(std::sync::atomic::Ordering::Acquire) < 5,
        "retries must back off rather than hammer the server"
    );
    events.shutdown().await;
    server.abort();
}
