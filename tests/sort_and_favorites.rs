//! Library sort arguments and per-user favourites across server generations.
use ma_tui::{
    api::ApiClient,
    controller::Update,
    events::{translate, Event},
    music::{Kind, Media, Order, Target, SORT_FIELDS_SCHEMA},
    presentation::apply,
    ui::{App, TrackView},
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[test]
fn every_order_has_both_spellings() {
    let shapes: Vec<(Value, Value)> = [
        Order::Name,
        Order::RecentlyAdded,
        Order::LastPlayed,
        Order::MostPlayed,
        Order::Year,
        Order::Random,
    ]
    .into_iter()
    .map(|order| {
        let (mut old, mut new) = (json!({}), json!({}));
        order.apply(&mut old, false);
        order.apply(&mut new, true);
        (old, new)
    })
    .collect();
    assert_eq!(
        shapes,
        [
            (
                json!({"order_by":"sort_name"}),
                json!({"sort_field":"sort_name","sort_direction":"asc"})
            ),
            (
                json!({"order_by":"timestamp_added_desc"}),
                json!({"sort_field":"timestamp_added","sort_direction":"desc"})
            ),
            (
                json!({"order_by":"last_played_desc"}),
                json!({"sort_field":"last_played","sort_direction":"desc"})
            ),
            (
                json!({"order_by":"play_count_desc"}),
                json!({"sort_field":"play_count","sort_direction":"desc"})
            ),
            (
                json!({"order_by":"year_desc"}),
                json!({"sort_field":"year","sort_direction":"desc"})
            ),
            (json!({"order_by":"random"}), json!({"sort_field":"random"})),
        ]
    );
}

/// A server reporting `schema`, recording each command it is sent.
async fn server(schema: Option<u32>) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let text = loop {
                    let mut chunk = [0; 4096];
                    let read = socket.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    match text.split_once("\r\n\r\n") {
                        Some(_) if text.starts_with("GET ") => break text,
                        Some((_, body)) if serde_json::from_str::<Value>(body).is_ok() => {
                            break text
                        }
                        _ => {}
                    }
                };
                let body = if text.starts_with("GET /info ") {
                    json!({"server_version":"fixture","schema_version":schema})
                } else {
                    let body = text.split_once("\r\n\r\n").unwrap().1;
                    seen.lock()
                        .unwrap()
                        .push(serde_json::from_str(body).unwrap());
                    json!([])
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
    (url, calls)
}

#[tokio::test]
async fn library_listings_send_the_spelling_the_server_reads() {
    let target = Target::Library {
        kind: Kind::Albums,
        offset: 0,
        favorite: false,
        search: None,
        order: Order::Year,
    };
    for (schema, expected) in [
        (
            None,
            json!({"limit":100,"offset":0,"favorite":null,"order_by":"year_desc"}),
        ),
        (
            Some(SORT_FIELDS_SCHEMA - 1),
            json!({"limit":100,"offset":0,"favorite":null,"order_by":"year_desc"}),
        ),
        (
            Some(SORT_FIELDS_SCHEMA),
            json!({"limit":100,"offset":0,"favorite":null,"sort_field":"year","sort_direction":"desc"}),
        ),
    ] {
        let (url, calls) = server(schema).await;
        let api = ApiClient::new(&url, "fixture-token").unwrap();
        api.refresh_schema().await.unwrap();
        api.browse(&target).await.unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0]["command"], "music/albums/library_items");
        assert_eq!(calls[0]["args"], expected, "schema {schema:?}");
    }
}

#[test]
fn a_favourite_update_becomes_a_library_item_change() {
    for (favorite, expected) in [
        (json!(true), true),
        (json!(false), false),
        (Value::Null, false),
    ] {
        let event = json!({"event":"favorite_updated","object_id":"library://track/1",
                           "data":{"uri":"library://track/1","media_type":"track","item_id":"1",
                                   "favorite":favorite,"user_id":"u1"}});
        assert_eq!(
            translate(&event),
            Some(Event::MediaItem {
                uri: "library://track/1".into(),
                item: Some(json!({ "favorite": expected })),
            })
        );
    }
    assert_eq!(
        translate(&json!({"event":"favorite_updated","object_id":"library://track/1"})),
        None
    );
}

fn favourite_row() -> Media {
    Media::parse(
        &json!({"name":"Song","item_id":"1","provider":"library","uri":"library://track/1",
                "media_type":"track","favorite":true}),
        "track",
    )
}

#[test]
fn a_library_event_without_favourite_state_keeps_the_heart() {
    let mut app = App::default();
    app.music.page.items = vec![favourite_row()];
    app.results = vec![TrackView {
        media: Some(favourite_row()),
        ..Default::default()
    }];
    // 2.11 sends library items without the per-user favourite.
    for item in [
        json!({"uri":"library://track/1","name":"Song (renamed)"}),
        json!({"uri":"library://track/1","favorite":null}),
    ] {
        apply(
            &mut app,
            Update::MediaItem {
                uri: "library://track/1".into(),
                item: Some(item),
            },
        );
        assert!(app.music.page.items[0].favorite);
        assert!(app.results[0].media.as_ref().unwrap().favorite);
    }
    // A real false still clears it.
    apply(
        &mut app,
        Update::MediaItem {
            uri: "library://track/1".into(),
            item: Some(json!({"favorite":false})),
        },
    );
    assert!(!app.music.page.items[0].favorite);
}

#[test]
fn only_true_is_a_favourite_row() {
    for (value, expected) in [
        (json!(true), true),
        (json!(false), false),
        (Value::Null, false),
    ] {
        let row = Media::parse(
            &json!({"name":"Song","uri":"library://track/1","media_type":"track","favorite":value}),
            "track",
        );
        assert_eq!(row.favorite, expected, "{value}");
    }
}
