use ma_tui::{
    api::ApiClient,
    controller::{Controller, Request, Update},
    music::Target,
    ui::Action,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn worker_polls_without_blocking_caller_and_can_cancel_stalled_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = ApiClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "fixture-token",
    )
    .unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        assert!(socket.read(&mut buffer).await.unwrap() > 0);
        let body = r#"[{"player_id":"test","name":"Fixture player","available":true}]"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let (_socket, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let mut controller = Controller::start(client, None, false);
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), controller.updates.recv())
        .await
        .unwrap()
        .unwrap();
    match event {
        Update::Players(players) => assert_eq!(players[0].id, "test"),
        _ => panic!("expected players"),
    }
    controller.selection.send(Some("test".into())).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), controller.shutdown())
        .await
        .unwrap();
    server.abort();
}

/// A library read that never answers must not hold up transport or polling.
#[tokio::test]
async fn a_stalled_browse_does_not_block_the_poll() {
    use ma_tui::{controller::Request, music::Target, ui::Action};
    use std::time::Duration;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = ApiClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "fixture-token",
    )
    .unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buffer = vec![0; 8192];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                if String::from_utf8_lossy(&buffer[..read]).contains("music/browse") {
                    // The slow read: answer it never.
                    std::future::pending::<()>().await;
                }
                let body = r#"[{"player_id":"test","name":"Fixture player","available":true}]"#;
                let _ = socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await;
            });
        }
    });
    let mut controller = Controller::start(client, None, false);
    let first = tokio::time::timeout(Duration::from_secs(2), controller.updates.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(first, Update::Players(_)), "the poll starts");

    controller
        .requests
        .send(Request::new(
            None,
            Action::Browse {
                generation: 1,
                target: Target::Providers { path: None },
            },
        ))
        .await
        .unwrap();

    let players = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match controller.updates.recv().await {
                Some(Update::Players(players)) => return players,
                Some(_) => continue,
                None => panic!("the controller stopped"),
            }
        }
    })
    .await
    .expect("the poll must keep running while a library read is outstanding");
    assert_eq!(players[0].id, "test");

    tokio::time::timeout(Duration::from_secs(1), controller.shutdown())
        .await
        .unwrap();
    server.abort();
}

/// Events drive the reads: a position for the queue on screen costs no request
/// at all, and a queue that is not on screen costs nothing either.
#[tokio::test]
async fn events_route_by_queue_and_a_position_needs_no_request() {
    use ma_tui::events::Event;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = ApiClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "fixture-token",
    )
    .unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let counted = counted.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 8192];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                counted.fetch_add(1, Ordering::Release);
                let body = if request.contains("get_active_queue") {
                    r#"{"queue_id":"q1","display_name":"Kitchen","state":"playing","items":0,"elapsed_time":5.0,"current_item":{}}"#
                } else if request.contains("player_queues/items") {
                    "[]"
                } else {
                    r#"[{"player_id":"p1","name":"Kitchen","available":true}]"#
                };
                let _ = socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await;
            });
        }
    });

    let (feed, stream) = tokio::sync::mpsc::channel(16);
    let mut controller = Controller::start(client, Some(stream), false);
    feed.send(Event::Online).await.unwrap();
    controller.selection.send(Some("p1".into())).unwrap();

    // Wait until a queue has actually been read, so the controller knows which
    // queue is on screen.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Update::Queue(_, Ok(queue))) = controller.updates.recv().await {
                assert_eq!(queue.id, "q1");
                return;
            }
        }
    })
    .await
    .expect("the selected queue is read once");
    // Let the initial tick, Online refresh and selection reads finish before
    // counting. The live interval is far away.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = requests.load(Ordering::Acquire);

    // A position for a different queue is not ours to show.
    feed.send(Event::Elapsed("other".into(), 99.0))
        .await
        .unwrap();
    // The one for our queue is, and it arrives without asking the server.
    feed.send(Event::Elapsed("q1".into(), 42.0)).await.unwrap();
    let elapsed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match controller.updates.recv().await {
                Some(Update::Elapsed(id, seconds)) => return (id, seconds),
                Some(_) => continue,
                None => panic!("the controller stopped"),
            }
        }
    })
    .await
    .expect("a position event must reach the interface");
    assert_eq!(
        elapsed,
        ("q1".into(), 42.0),
        "only the displayed queue's position is shown"
    );
    assert_eq!(
        requests.load(Ordering::Acquire),
        settled,
        "a position costs no request"
    );

    tokio::time::timeout(Duration::from_secs(2), controller.shutdown())
        .await
        .unwrap();
    server.abort();
}

/// An event receiver is created before connecting/authenticating; a socket
/// that never connects must not slow HTTP updates to the live-stream interval.
#[tokio::test]
async fn an_event_stream_that_never_opens_keeps_fallback_polling() {
    let (_feed, stream) = tokio::sync::mpsc::channel(16);
    let (mut controller, server) = polling_fixture(Some(stream)).await;
    expect_players(&mut controller).await;
    expect_players(&mut controller).await;
    controller.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn polling_tracks_online_offline_and_closed_event_streams() {
    use ma_tui::events::Event;

    let (feed, stream) = tokio::sync::mpsc::channel(16);
    let (mut controller, server) = polling_fixture(Some(stream)).await;
    expect_players(&mut controller).await;

    feed.send(Event::Online).await.unwrap();
    expect_live_polling(&mut controller).await;
    feed.send(Event::Offline).await.unwrap();
    expect_stream(&mut controller, false).await;
    expect_players(&mut controller).await;
    expect_players(&mut controller).await;

    feed.send(Event::Online).await.unwrap();
    expect_live_polling(&mut controller).await;

    // Closing the receiver's source is another form of outage, even when it
    // cannot send an Offline event first.
    drop(feed);
    expect_stream(&mut controller, false).await;
    expect_players(&mut controller).await;
    expect_players(&mut controller).await;
    controller.shutdown().await;
    server.abort();
}

async fn expect_live_polling(controller: &mut Controller) {
    use std::time::Duration;

    expect_stream(controller, true).await;
    // Reconnecting immediately refreshes the snapshot. Let those reads
    // finish before checking that the fallback timer has been replaced.
    expect_players(controller).await;
    while let Ok(update) =
        tokio::time::timeout(Duration::from_millis(100), controller.updates.recv()).await
    {
        assert!(matches!(update, Some(Update::Players(_))));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(2500), controller.updates.recv())
            .await
            .is_err(),
        "an authenticated live stream must not keep polling every two seconds"
    );
}

async fn polling_fixture(
    stream: Option<tokio::sync::mpsc::Receiver<ma_tui::events::Event>>,
) -> (Controller, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = ApiClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "fixture-token",
    )
    .unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            assert!(socket.read(&mut buffer).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]")
                .await
                .unwrap();
        }
    });
    (Controller::start(client, stream, false), server)
}

async fn expect_players(controller: &mut Controller) {
    let update = tokio::time::timeout(std::time::Duration::from_secs(5), controller.updates.recv())
        .await
        .expect("fallback polling must refresh player state within five seconds");
    assert!(matches!(update, Some(Update::Players(_))));
}

async fn expect_stream(controller: &mut Controller, online: bool) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match controller.updates.recv().await {
                Some(Update::Stream(actual)) => {
                    assert_eq!(actual, online);
                    return;
                }
                Some(Update::Players(_)) => continue,
                _ => panic!("expected a stream status update"),
            }
        }
    })
    .await
    .expect("the event stream status must reach the interface");
}

async fn queue_fixture(
    listing: serde_json::Value,
) -> (
    Controller,
    tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
    tokio::task::JoinHandle<()>,
) {
    use serde_json::{json, Value};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = ApiClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "fixture-token",
    )
    .unwrap();
    let (sent, received) = tokio::sync::mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let end = loop {
                let mut chunk = [0; 1024];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(header_end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break header_end + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            while bytes.len() < end + length {
                let mut chunk = [0; 1024];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
            }
            let request: Value = serde_json::from_slice(&bytes[end..end + length]).unwrap();
            let body = match request["command"].as_str().unwrap() {
                "players/all" => "[]".into(),
                "music/browse" => listing.to_string(),
                "player_queues/get_active_queue" => {
                    json!({"queue_id":"selected-queue"}).to_string()
                }
                "player_queues/play_media" => "null".into(),
                other => panic!("unexpected command: {other}"),
            };
            if request["command"] != "players/all" {
                sent.send(request).unwrap();
            }
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    (Controller::start(api, None, false), received, server)
}

async fn queue_notice(controller: &mut Controller) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match controller.updates.recv().await {
                Some(Update::Notice(notice)) => return notice,
                Some(_) => continue,
                None => panic!("controller stopped before reporting the command"),
            }
        }
    })
    .await
    .unwrap()
}

fn queue_calls(
    received: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
) -> Vec<serde_json::Value> {
    std::iter::from_fn(|| received.try_recv().ok()).collect()
}

#[tokio::test]
async fn selected_player_batch_append_uses_its_active_queue_in_visible_order() {
    use serde_json::json;
    let (mut controller, mut received, server) = queue_fixture(json!([])).await;
    controller
        .requests
        .send(Request::new(
            Some("selected-member".into()),
            Action::EnqueueMany(vec![
                "provider://track/z".into(),
                "library://track/a".into(),
            ]),
        ))
        .await
        .unwrap();
    assert_eq!(
        queue_notice(&mut controller).await,
        "Command accepted; refreshing state"
    );
    let calls = queue_calls(&mut received);
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0]["command"], "player_queues/get_active_queue");
    assert_eq!(calls[0]["args"], json!({"player_id":"selected-member"}));
    for (call, uri) in calls[1..]
        .iter()
        .zip(["provider://track/z", "library://track/a"])
    {
        assert_eq!(call["command"], "player_queues/play_media");
        assert_eq!(
            call["args"],
            json!({"queue_id":"selected-queue","media":uri,"option":"add"})
        );
    }
    controller.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn selected_player_batch_replace_uses_its_active_queue_in_visible_order() {
    use serde_json::json;
    let (mut controller, mut received, server) = queue_fixture(json!([])).await;
    controller
        .requests
        .send(Request::new(
            Some("selected-member".into()),
            Action::PlayMany(vec![
                "provider://track/z".into(),
                "library://track/a".into(),
            ]),
        ))
        .await
        .unwrap();
    assert_eq!(
        queue_notice(&mut controller).await,
        "Command accepted; refreshing state"
    );
    let calls = queue_calls(&mut received);
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0]["command"], "player_queues/get_active_queue");
    assert_eq!(calls[0]["args"], json!({"player_id":"selected-member"}));
    for (index, (call, uri)) in calls[1..]
        .iter()
        .zip(["provider://track/z", "library://track/a"])
        .enumerate()
    {
        assert_eq!(call["command"], "player_queues/play_media");
        assert_eq!(
            call["args"],
            json!({"queue_id":"selected-queue","media":uri,"option":if index == 0 { "replace" } else { "add" }})
        );
    }
    controller.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn folder_append_browses_once_and_adds_only_immediate_available_playable_items() {
    use serde_json::json;
    let listing = json!([
        {"media_type":"track","uri":"provider://track/last"},
        {"media_type":"folder","path":"provider://browse/nested","uri":"provider://browse/nested"},
        {"media_type":"track","uri":"provider://track/unavailable","available":false},
        {"media_type":"track","uri":"provider://track/blocked","is_playable":false},
        {"media_type":"track","uri":"provider://track/first"},
        {"media_type":"radio","uri":"provider://radio/live","is_playable":true}
    ]);
    let (mut controller, mut received, server) = queue_fixture(listing).await;
    controller
        .requests
        .send(Request::new(
            Some("selected-member".into()),
            Action::EnqueueFolder(Target::Providers {
                path: Some("provider://browse/folder".into()),
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        queue_notice(&mut controller).await,
        "Command accepted; refreshing state"
    );
    let calls = queue_calls(&mut received);
    assert_eq!(calls.len(), 5);
    assert_eq!(calls[0]["command"], "music/browse");
    assert_eq!(calls[0]["args"], json!({"path":"provider://browse/folder"}));
    assert_eq!(calls[1]["command"], "player_queues/get_active_queue");
    assert_eq!(calls[1]["args"], json!({"player_id":"selected-member"}));
    for (call, uri) in calls[2..].iter().zip([
        "provider://track/last",
        "provider://track/first",
        "provider://radio/live",
    ]) {
        assert_eq!(call["command"], "player_queues/play_media");
        assert_eq!(
            call["args"],
            json!({"queue_id":"selected-queue","media":uri,"option":"add"})
        );
    }
    controller.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn folder_replace_browses_once_and_plays_immediate_items_in_source_order() {
    use serde_json::json;
    let listing = json!([
        {"media_type":"track","uri":"provider://track/last"},
        {"media_type":"folder","path":"provider://browse/nested","uri":"provider://browse/nested"},
        {"media_type":"track","uri":"provider://track/unavailable","available":false},
        {"media_type":"track","uri":"provider://track/blocked","is_playable":false},
        {"media_type":"track","uri":"","is_playable":true},
        {"media_type":"track","uri":"provider://track/first"},
        {"media_type":"radio","uri":"provider://radio/live","is_playable":true}
    ]);
    let (mut controller, mut received, server) = queue_fixture(listing).await;
    controller
        .requests
        .send(Request::new(
            Some("selected-member".into()),
            Action::PlayFolder(Target::Providers {
                path: Some("provider://browse/folder".into()),
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        queue_notice(&mut controller).await,
        "Command accepted; refreshing state"
    );
    let calls = queue_calls(&mut received);
    assert_eq!(calls.len(), 5);
    assert_eq!(calls[0]["command"], "music/browse");
    assert_eq!(calls[0]["args"], json!({"path":"provider://browse/folder"}));
    assert_eq!(calls[1]["command"], "player_queues/get_active_queue");
    assert_eq!(calls[1]["args"], json!({"player_id":"selected-member"}));
    for (index, (call, uri)) in calls[2..]
        .iter()
        .zip([
            "provider://track/last",
            "provider://track/first",
            "provider://radio/live",
        ])
        .enumerate()
    {
        assert_eq!(call["command"], "player_queues/play_media");
        assert_eq!(
            call["args"],
            json!({"queue_id":"selected-queue","media":uri,"option":if index == 0 { "replace" } else { "add" }})
        );
    }
    controller.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn empty_folder_actions_report_no_playable_items_without_queue_commands() {
    use serde_json::json;
    let listing = json!([
        {"media_type":"folder","path":"provider://browse/nested"},
        {"media_type":"track","uri":"provider://track/unavailable","available":false},
        {"media_type":"track","uri":"provider://track/blocked","is_playable":false},
        {"media_type":"track","uri":"","is_playable":true}
    ]);
    let (mut controller, mut received, server) = queue_fixture(listing).await;
    for action in [
        Action::EnqueueFolder(Target::Providers {
            path: Some("provider://browse/empty".into()),
        }),
        Action::PlayFolder(Target::Providers {
            path: Some("provider://browse/empty".into()),
        }),
    ] {
        controller
            .requests
            .send(Request::new(Some("selected-member".into()), action))
            .await
            .unwrap();
        assert_eq!(
            queue_notice(&mut controller).await,
            "Command failed (not retried): Folder has no available playable items"
        );
        let calls = queue_calls(&mut received);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["command"], "music/browse");
        assert_eq!(calls[0]["args"], json!({"path":"provider://browse/empty"}));
    }
    controller.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn batch_and_folder_actions_require_a_player_at_execution_time() {
    use serde_json::json;
    let (mut controller, mut received, server) = queue_fixture(json!([])).await;
    for action in [
        Action::EnqueueMany(vec!["provider://track/1".into()]),
        Action::PlayMany(vec!["provider://track/1".into()]),
        Action::EnqueueFolder(Target::Providers {
            path: Some("provider://browse/folder".into()),
        }),
        Action::PlayFolder(Target::Providers {
            path: Some("provider://browse/folder".into()),
        }),
    ] {
        controller
            .requests
            .send(Request::new(None, action))
            .await
            .unwrap();
        assert_eq!(
            queue_notice(&mut controller).await,
            "Command failed (not retried): No player selected"
        );
    }
    assert!(queue_calls(&mut received).is_empty());
    controller.shutdown().await;
    server.abort();
}
