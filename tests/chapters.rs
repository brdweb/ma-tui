use ma_tui::{
    api::{ApiClient, Queue},
    chapters::{self, Chapter, Chapters, Source},
    controller::{Controller, Update},
    presentation::apply,
    ui::App,
};
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

fn chapter(position: u64, name: &str, start: f64, end: Option<f64>) -> Chapter {
    Chapter {
        position: Some(position),
        name: name.into(),
        start,
        end,
    }
}

fn queue_with(media_item: Value) -> Value {
    json!({"queue_id":"q1","current_item":{"queue_item_id":"qi","media_item":media_item}})
}

#[test]
fn chapters_parse_sorted_and_cleaned() {
    let item = json!({"metadata":{"chapters":[
        {"position":2,"name":"Two","start":60.5,"end":120.0},
        {"position":1,"name":"One","start":0,"end":60.5},
        {"position":3,"name":"Three","start":120.0,"end":null},
    ]}});
    assert_eq!(
        chapters::parse(&item),
        vec![
            chapter(1, "One", 0.0, Some(60.5)),
            chapter(2, "Two", 60.5, Some(120.0)),
            chapter(3, "Three", 120.0, None),
        ]
    );
}

#[test]
fn malformed_chapters_are_dropped_or_repaired_not_errors() {
    let long = "x".repeat(chapters::NAME_LIMIT + 50);
    let item = json!({"metadata":{"chapters":[
        {"position":1,"name":"No start"},
        {"position":2,"name":"Negative","start":-1.0},
        {"position":3,"name":"Text start","start":"10"},
        {"position":4,"name":"\u{1b}[31mEscaped\u{7}","start":5.0,"end":2.0},
        {"name":long,"start":7.0,"end":"later"},
        "not an object",
    ]}});
    let parsed = chapters::parse(&item);
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        parsed[0].name, "[31mEscaped",
        "control characters are stripped"
    );
    assert_eq!(parsed[0].end, None, "an end before the start is dropped");
    assert_eq!(parsed[1].position, None);
    assert_eq!(parsed[1].name.chars().count(), chapters::NAME_LIMIT);
    assert_eq!(parsed[1].end, None);

    for empty in [
        json!({}),
        json!({"metadata":null}),
        json!({"metadata":{"chapters":null}}),
        json!({"metadata":{"chapters":{}}}),
        json!({"metadata":{"chapters":[]}}),
    ] {
        assert!(chapters::parse(&empty).is_empty(), "{empty}");
    }
}

#[test]
fn the_queue_item_decides_whether_a_fetch_is_needed() {
    let carried = queue_with(json!({
        "uri":"library://audiobook/1","media_type":"audiobook",
        "metadata":{"chapters":[{"position":1,"name":"One","start":0}]}
    }));
    assert_eq!(
        chapters::source(&carried),
        (
            "library://audiobook/1".into(),
            Source::Known(vec![chapter(1, "One", 0.0, None)])
        )
    );
    for media_type in ["audiobook", "podcast_episode"] {
        let slim = queue_with(json!({"uri":"x://item/1","media_type":media_type}));
        assert_eq!(
            chapters::source(&slim),
            ("x://item/1".into(), Source::Fetch),
            "{media_type}"
        );
    }
    let track = queue_with(json!({"uri":"library://track/1","media_type":"track"}));
    assert_eq!(
        chapters::source(&track),
        ("library://track/1".into(), Source::Known(vec![]))
    );
    assert_eq!(
        chapters::source(&json!({"queue_id":"q1","current_item":null})),
        (String::new(), Source::Known(vec![]))
    );
}

#[test]
fn chapters_are_only_read_for_the_item_they_were_loaded_for() {
    let loaded = Chapters {
        uri: "x://item/1".into(),
        list: vec![chapter(1, "One", 0.0, None)],
    };
    assert_eq!(
        loaded
            .current(&queue_with(json!({"uri":"x://item/1"})))
            .len(),
        1
    );
    assert!(loaded
        .current(&queue_with(json!({"uri":"x://item/2"})))
        .is_empty());
    assert!(loaded.current(&Value::Null).is_empty());
    assert!(Chapters::default()
        .current(&queue_with(json!({})))
        .is_empty());
}

fn showing(app: &mut App, uri: &str) {
    app.selected_id = Some("player".into());
    apply(
        app,
        Update::Queue(
            "player".into(),
            Ok(Queue {
                id: "q1".into(),
                details: queue_with(json!({"uri":uri,"media_type":"audiobook"})),
                current_title: "Book".into(),
                ..Default::default()
            }),
        ),
    );
}

#[test]
fn a_stale_fetch_cannot_overwrite_the_newer_items_chapters() {
    let mut app = App::default();
    showing(&mut app, "x://book/new");
    apply(
        &mut app,
        Update::Chapters {
            uri: "x://book/new".into(),
            chapters: vec![chapter(1, "New", 0.0, None)],
        },
    );
    // The slower fetch for the item that played before lands afterwards.
    apply(
        &mut app,
        Update::Chapters {
            uri: "x://book/old".into(),
            chapters: vec![chapter(1, "Old", 0.0, None)],
        },
    );
    let current = app.chapters.current(&app.queue_details);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].name, "New");

    // When the item changes, the old list stops being read at once.
    showing(&mut app, "x://book/next");
    assert!(app.chapters.current(&app.queue_details).is_empty());
}

/// A fixture serving one queue whose playing item is `media_item`, answering
/// `music/item_by_uri` with `full_item` and counting those lookups.
async fn fixture(media_item: Value, full_item: Value) -> (Controller, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = ApiClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "fixture-token",
    )
    .unwrap();
    let lookups = Arc::new(AtomicUsize::new(0));
    let counted = lookups.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (media_item, full_item, counted) =
                (media_item.clone(), full_item.clone(), counted.clone());
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let request: Value = loop {
                    let mut chunk = [0; 4096];
                    let read = socket.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&bytes);
                    if let Some((_, body)) = text.split_once("\r\n\r\n") {
                        if let Ok(request) = serde_json::from_str(body) {
                            break request;
                        }
                    }
                };
                let body = match request["command"].as_str().unwrap_or_default() {
                    "players/all" => json!([]),
                    "player_queues/get_active_queue" => {
                        json!({"queue_id":"q1","current_item":{"queue_item_id":"qi","media_item":media_item}})
                    }
                    "player_queues/items" => json!([]),
                    "music/item_by_uri" => {
                        assert_eq!(request["args"]["uri"], media_item["uri"]);
                        counted.fetch_add(1, Ordering::SeqCst);
                        full_item
                    }
                    other => panic!("unexpected command: {other}"),
                }
                .to_string();
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
    let controller = Controller::start(api, None, false);
    controller.selection.send(Some("player".into())).unwrap();
    (controller, lookups)
}

async fn next_chapters(controller: &mut Controller) -> (String, Vec<Chapter>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match controller.updates.recv().await {
                Some(Update::Chapters { uri, chapters }) => return (uri, chapters),
                Some(_) => continue,
                None => panic!("the controller stopped"),
            }
        }
    })
    .await
    .expect("chapters must be reported for the playing item")
}

#[tokio::test]
async fn chapters_on_the_queue_item_need_no_lookup() {
    let (mut controller, lookups) = fixture(
        json!({"uri":"library://audiobook/1","media_type":"audiobook",
               "metadata":{"chapters":[{"position":1,"name":"Opening","start":0,"end":30}]}}),
        json!({}),
    )
    .await;
    let (uri, chapters) = next_chapters(&mut controller).await;
    assert_eq!(uri, "library://audiobook/1");
    assert_eq!(chapters, vec![chapter(1, "Opening", 0.0, Some(30.0))]);
    assert_eq!(lookups.load(Ordering::SeqCst), 0);
    controller.shutdown().await;
}

#[tokio::test]
async fn chapters_only_on_the_full_item_are_fetched_once() {
    let (mut controller, lookups) = fixture(
        json!({"uri":"podcast_feed://podcast_episode/e1","media_type":"podcast_episode"}),
        json!({"uri":"podcast_feed://podcast_episode/e1","media_type":"podcast_episode",
        "metadata":{"chapters":[
            {"position":2,"name":"Interview","start":300.0},
            {"position":1,"name":"Intro","start":0.0},
        ]}}),
    )
    .await;
    let (uri, chapters) = next_chapters(&mut controller).await;
    assert_eq!(uri, "podcast_feed://podcast_episode/e1");
    assert_eq!(
        chapters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Intro", "Interview"]
    );
    // Later snapshots of the same item do not look it up again.
    tokio::time::sleep(Duration::from_millis(4500)).await;
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    controller.shutdown().await;
}

#[tokio::test]
async fn an_item_without_chapters_reports_none() {
    let (mut controller, lookups) = fixture(
        json!({"uri":"library://audiobook/2","media_type":"audiobook"}),
        json!({"uri":"library://audiobook/2","media_type":"audiobook","metadata":{"chapters":null}}),
    )
    .await;
    let (uri, chapters) = next_chapters(&mut controller).await;
    assert_eq!(uri, "library://audiobook/2");
    assert!(chapters.is_empty());
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    controller.shutdown().await;
}

#[tokio::test]
async fn a_track_costs_no_lookup() {
    let (mut controller, lookups) = fixture(
        json!({"uri":"library://track/9","media_type":"track"}),
        json!({}),
    )
    .await;
    let (uri, chapters) = next_chapters(&mut controller).await;
    assert_eq!(uri, "library://track/9");
    assert!(chapters.is_empty());
    assert_eq!(lookups.load(Ordering::SeqCst), 0);
    controller.shutdown().await;
}

#[tokio::test]
async fn the_chapter_picker_reads_the_full_item() {
    let (mut controller, lookups) = fixture(
        json!({"uri":"library://audiobook/3","media_type":"audiobook"}),
        json!({"uri":"library://audiobook/3","media_type":"audiobook",
               "metadata":{"chapters":[{"position":1,"name":"One","start":0}]}}),
    )
    .await;
    // Let the playing item's own lookup happen first.
    next_chapters(&mut controller).await;
    controller
        .requests
        .send(ma_tui::controller::Request::new(
            None,
            ma_tui::ui::Action::LoadChapters {
                uri: "library://audiobook/3".into(),
            },
        ))
        .await
        .unwrap();
    let (uri, result) = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Update::ChapterList { uri, result }) = controller.updates.recv().await {
                return (uri, result);
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(uri, "library://audiobook/3");
    assert_eq!(result.unwrap(), vec![chapter(1, "One", 0.0, None)]);
    assert_eq!(lookups.load(Ordering::SeqCst), 2);
    controller.shutdown().await;
}
