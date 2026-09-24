use api::ApiClient;
use ma_tui::api;
use ma_tui::controls::Command;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// A fixture that answers each request from its body, for a read that issues
/// several and whose shape depends on the replies.
async fn server_with(
    reply: impl Fn(&str) -> Value + Send + Sync + 'static,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/prefix", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let end = loop {
                let mut b = [0; 1024];
                let n = socket.read(&mut b).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&b[..n]);
                if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
            let len: usize = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            while bytes.len() < end + len {
                let mut b = [0; 1024];
                let n = socket.read(&mut b).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&b[..n]);
            }
            let body = String::from_utf8_lossy(&bytes[end..end + len]).to_string();
            let answer = reply(&body).to_string();
            socket.write_all(format!("HTTP/1.1 200 Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len()).as_bytes()).await.unwrap();
        }
    });
    (url, task)
}

async fn server(replies: Vec<(u16, String)>) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/prefix", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let end;
            loop {
                let mut b = [0; 1024];
                let n = socket.read(&mut b).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&b[..n]);
                if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    end = i + 4;
                    break;
                }
            }
            let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
            assert!(headers.starts_with("post /prefix/api http/1.1"));
            assert!(headers.contains("authorization: bearer test-secret\r\n"));
            let len: usize = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            while bytes.len() < end + len {
                let mut b = [0; 1024];
                let n = socket.read(&mut b).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&b[..n]);
            }
            requests.push(serde_json::from_slice(&bytes[end..end + len]).unwrap());
            socket.write_all(format!("HTTP/1.1 {status} Test\r\nLocation: /prefix/api\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (url, task)
}

#[tokio::test]
async fn connection_test_checks_server_information_before_sending_a_token() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/dashboard", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        loop {
            let mut buf = [0; 1024];
            let count = socket.read(&mut buf).await.unwrap();
            assert!(count > 0);
            headers.extend_from_slice(&buf[..count]);
            if headers.windows(4).any(|v| v == b"\r\n\r\n") {
                break;
            }
        }
        let headers = String::from_utf8_lossy(&headers).to_lowercase();
        assert!(headers.starts_with("get /dashboard/info http/1.1"));
        assert!(!headers.contains("authorization:"));
        let body = "<html>Private dashboard text</html>";
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
    });
    let error = ApiClient::new(&url, "fixture-secret")
        .unwrap()
        .verify()
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("direct Music Assistant base URL"));
    assert!(!error.contains("fixture-secret"));
    assert!(!error.contains("Private dashboard"));
    task.await.unwrap();
}

#[tokio::test]
async fn queue_edits_are_bound_to_displayed_queue_and_player_commands_are_direct() {
    let (url, task) = server(vec![ok(json!({"queue_id":"new-leader"}))]).await;
    let api = ApiClient::new(&url, "test-secret").unwrap();
    assert!(api
        .playback_command(
            "member",
            Command::Queue {
                id: "old-leader".into(),
                name: "delete_item",
                args: json!({"item_id_or_index":"item"})
            }
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("changed"));
    assert_eq!(task.await.unwrap().len(), 1);
    for (name, args) in [
        ("volume_mute", json!({"muted":true})),
        ("power", json!({"powered":false})),
        ("group", json!({"target_player":"leader"})),
        ("sleep_timer/set", json!({"seconds":900})),
    ] {
        let (url, task) = server(vec![ok(Value::Null)]).await;
        ApiClient::new(&url, "test-secret")
            .unwrap()
            .playback_command(
                "member",
                Command::Player {
                    name,
                    args: args.clone(),
                },
            )
            .await
            .unwrap();
        let request = task.await.unwrap().remove(0);
        assert_eq!(request["args"]["player_id"], "member");
        for (key, value) in args.as_object().unwrap() {
            assert_eq!(&request["args"][key], value);
        }
        assert_eq!(
            request["command"],
            if name.contains('/') {
                format!("players/{name}")
            } else {
                format!("players/cmd/{name}")
            }
        );
    }
}

#[tokio::test]
async fn queue_edit_envelopes_and_transfer_resolve_group_leaders() {
    for (name, args) in [
        ("shuffle", json!({"shuffle_enabled":true})),
        ("repeat", json!({"repeat_mode":"all"})),
        ("play_index", json!({"index":"item"})),
        ("move_item", json!({"queue_item_id":"item","pos_shift":-1})),
        ("delete_item", json!({"item_id_or_index":"item"})),
        ("clear", json!({})),
    ] {
        let (url, task) = server(vec![ok(json!({"queue_id":"leader"})), ok(Value::Null)]).await;
        ApiClient::new(&url, "test-secret")
            .unwrap()
            .playback_command(
                "member",
                Command::Queue {
                    id: "leader".into(),
                    name,
                    args: args.clone(),
                },
            )
            .await
            .unwrap();
        let requests = task.await.unwrap();
        assert_eq!(requests[1]["command"], format!("player_queues/{name}"));
        assert_eq!(requests[1]["args"]["queue_id"], "leader");
        for (key, value) in args.as_object().unwrap() {
            assert_eq!(&requests[1]["args"][key], value);
        }
    }
    let (url, task) = server(vec![
        ok(json!({"queue_id":"source"})),
        ok(json!({"queue_id":"target-leader"})),
        ok(Value::Null),
    ])
    .await;
    ApiClient::new(&url, "test-secret")
        .unwrap()
        .playback_command(
            "member",
            Command::Transfer {
                source: "source".into(),
                target: "target-member".into(),
            },
        )
        .await
        .unwrap();
    let requests = task.await.unwrap();
    assert_eq!(
        requests[2]["args"],
        json!({"source_queue_id":"source","target_queue_id":"target-leader"})
    );
}
#[test]
fn rejects_unsafe_urls_and_tokens_without_echoing_them() {
    for url in [
        "ftp://localhost",
        "http://user:secret@localhost",
        "http://@localhost",
        "http://localhost/?secret",
        "http://localhost/#secret",
        "not a url",
    ] {
        assert!(
            ApiClient::new(url, "test-secret").is_err(),
            "accepted {url}"
        );
    }
    assert!(ApiClient::new("http://localhost", "bad\r\ntoken").is_err());
    assert!(ApiClient::new("http://localhost", "").is_err());
}

#[tokio::test]
async fn queue_resolves_group_owner_and_loads_items() {
    let item = json!({"queue_item_id":"i1","name":"Fallback","duration":120,"media_item":{"name":"Song","artists":[{"name":"Artist"},{"name":"Guest"}]}});
    let (url, task) = server(vec![ok(json!({"queue_id":"leader","display_name":"Group","state":"playing","elapsed_time":12.5,"items":1,"current_item":item})), ok(json!([item]))]).await;
    let q = ApiClient::new(&url, "test-secret")
        .unwrap()
        .queue("member")
        .await
        .unwrap();
    assert_eq!(
        (
            &*q.id,
            &*q.name,
            &*q.state,
            &*q.current_title,
            &*q.current_artist,
            q.elapsed,
            q.duration
        ),
        (
            "leader",
            "Group",
            "playing",
            "Song",
            "Artist, Guest",
            12.5,
            120.0
        )
    );
    assert_eq!(
        (
            &*q.items[0].id,
            &*q.items[0].title,
            &*q.items[0].artist,
            q.items[0].duration
        ),
        ("i1", "Song", "Artist, Guest", 120.0)
    );
    let r = task.await.unwrap();
    assert_eq!(r[0]["command"], "player_queues/get_active_queue");
    assert_eq!(r[0]["args"], json!({"player_id":"member"}));
    assert_eq!(r[1]["command"], "player_queues/items");
    assert_eq!(
        r[1]["args"],
        json!({"queue_id":"leader","limit":500,"offset":0})
    );
}

#[tokio::test]
async fn search_tracks_with_artist_names() {
    let (url, task) = server(vec![ok(
        json!({"tracks":[{"uri":"library://track/1","name":"Song","artists":[{"name":"Artist"}]}], "albums":[{"uri":"library://album/2","name":"Album name"}], "playlists":[{"uri":"library://playlist/3","name":"Playlist name"}]}),
    )])
    .await;
    let t = ApiClient::new(&url, "test-secret")
        .unwrap()
        .search("Song")
        .await
        .unwrap();
    assert_eq!(
        (&*t[0].uri, &*t[0].title, &*t[0].artist),
        ("library://track/1", "Song", "Artist")
    );
    assert_eq!(t[1].uri, "library://album/2");
    assert_eq!(t[1].title, "[Album] Album name");
    assert_eq!(t[2].uri, "library://playlist/3");
    let r = task.await.unwrap();
    assert_eq!(r[0]["command"], "music/search");
    assert_eq!(
        r[0]["args"],
        json!({"search_query":"Song","media_types":["track","album","artist","playlist","radio","audiobook","podcast"],"limit":50})
    );
}
#[tokio::test]
async fn controls_route_transport_to_the_active_queue() {
    use api::Control;
    for (action, command, args) in [
        (
            Control::Toggle,
            "player_queues/play_pause",
            json!({"queue_id":"leader"}),
        ),
        (
            Control::Next,
            "player_queues/next",
            json!({"queue_id":"leader"}),
        ),
        (
            Control::Previous,
            "player_queues/previous",
            json!({"queue_id":"leader"}),
        ),
        (
            Control::Seek(42.8),
            "player_queues/seek",
            json!({"queue_id":"leader","position":42}),
        ),
    ] {
        // Volume is a player command and goes through `playback_command`.
        let (url, task) = server(vec![ok(json!({"queue_id":"leader"})), ok(Value::Null)]).await;
        ApiClient::new(&url, "test-secret")
            .unwrap()
            .control("member", action)
            .await
            .unwrap();
        let r = task.await.unwrap();
        assert_eq!(r.last().unwrap()["command"], command);
        assert_eq!(r.last().unwrap()["args"], args);
    }
}
#[tokio::test]
async fn play_explicitly_replaces_while_enqueue_adds() {
    for option in ["replace", "add", "next"] {
        let (url, task) = server(vec![ok(json!({"queue_id":"leader"})), ok(Value::Null)]).await;
        let client = ApiClient::new(&url, "test-secret").unwrap();
        if option == "replace" {
            client
                .play_uri("member", "library://track/1")
                .await
                .unwrap();
        } else if option == "next" {
            client
                .play_next_uri("member", "library://track/1")
                .await
                .unwrap();
        } else {
            client
                .enqueue_uri("member", "library://track/1")
                .await
                .unwrap();
        }
        let r = task.await.unwrap();
        assert_eq!(r[1]["command"], "player_queues/play_media");
        assert_eq!(
            r[1]["args"],
            json!({"queue_id":"leader","media":"library://track/1","option":option})
        );
    }
}

#[tokio::test]
async fn favorites_add_by_uri() {
    let (url, task) = server(vec![ok(Value::Null)]).await;
    ApiClient::new(&url, "test-secret")
        .unwrap()
        .set_favorite("provider://track/1", "track", None, true)
        .await
        .unwrap();
    let requests = task.await.unwrap();
    assert_eq!(requests[0]["command"], "music/favorites/add_item");
    assert_eq!(requests[0]["args"], json!({"item":"provider://track/1"}));
}

#[tokio::test]
async fn favorites_remove_uses_known_library_id() {
    let (url, task) = server(vec![ok(Value::Null)]).await;
    ApiClient::new(&url, "test-secret")
        .unwrap()
        .set_favorite("library://track/42", "track", Some("42"), false)
        .await
        .unwrap();
    let requests = task.await.unwrap();
    assert_eq!(requests[0]["command"], "music/favorites/remove_item");
    assert_eq!(
        requests[0]["args"],
        json!({"media_type":"track","library_item_id":"42"})
    );
}

#[tokio::test]
async fn favorites_remove_resolves_library_ids_and_refuses_provider_only_items() {
    let (url, task) = server(vec![
        ok(json!({"provider":"library","item_id":"42"})),
        ok(Value::Null),
    ])
    .await;
    ApiClient::new(&url, "test-secret")
        .unwrap()
        .set_favorite("provider://track/1", "track", None, false)
        .await
        .unwrap();
    let requests = task.await.unwrap();
    assert_eq!(requests[0]["command"], "music/item_by_uri");
    assert_eq!(requests[0]["args"], json!({"uri":"provider://track/1"}));
    assert_eq!(requests[1]["command"], "music/favorites/remove_item");
    assert_eq!(
        requests[1]["args"],
        json!({"media_type":"track","library_item_id":"42"})
    );

    let (url, task) = server(vec![ok(json!({"provider":"provider","item_id":"1"}))]).await;
    let error = ApiClient::new(&url, "test-secret")
        .unwrap()
        .set_favorite("provider://track/1", "track", None, false)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "This item is not in your library");
    assert_eq!(task.await.unwrap().len(), 1);
}

#[tokio::test]
async fn add_to_library_uses_item_uri() {
    let (url, task) = server(vec![ok(json!({"item_id":"42"}))]).await;
    ApiClient::new(&url, "test-secret")
        .unwrap()
        .add_to_library("provider://album/42")
        .await
        .unwrap();
    let requests = task.await.unwrap();
    assert_eq!(requests[0]["command"], "music/library/add_item");
    assert_eq!(requests[0]["args"], json!({"item":"provider://album/42"}));
}

#[tokio::test]
async fn start_radio_enqueues_a_dynamic_radio_playlist_on_the_active_queue() {
    let (url, task) = server(vec![ok(json!({"queue_id":"leader"})), ok(Value::Null)]).await;
    ApiClient::new(&url, "test-secret")
        .unwrap()
        .start_radio("member", "library://track/1")
        .await
        .unwrap();
    let requests = task.await.unwrap();
    assert_eq!(requests[0]["command"], "player_queues/get_active_queue");
    assert_eq!(requests[0]["args"], json!({"player_id":"member"}));
    assert_eq!(requests[1]["command"], "player_queues/play_media");
    assert_eq!(
        requests[1]["args"],
        json!({
            "queue_id":"leader",
            "media":"radio_playlist://playlist/library://track/1",
            "option":"replace",
        })
    );
}

#[tokio::test]
async fn editable_playlists_filter_noneditable_rows() {
    let (url, task) = server(vec![ok(json!([
        {
            "item_id":"editable",
            "provider":"library",
            "uri":"library://playlist/editable",
            "name":"Editable",
            "media_type":"playlist",
            "is_editable":true,
        },
        {
            "item_id":"readonly",
            "provider":"library",
            "uri":"library://playlist/readonly",
            "name":"Read-only",
            "media_type":"playlist",
            "is_editable":false,
        },
    ]))])
    .await;
    let playlists = ApiClient::new(&url, "test-secret")
        .unwrap()
        .editable_playlists()
        .await
        .unwrap();
    assert_eq!(playlists.len(), 1);
    assert_eq!(playlists[0].id, "editable");
    let requests = task.await.unwrap();
    assert_eq!(requests[0]["command"], "music/playlists/library_items");
    assert_eq!(
        requests[0]["args"],
        json!({"limit":500,"offset":0,"order_by":"sort_name"})
    );
}

#[tokio::test]
async fn create_playlist_seeds_the_created_library_playlist() {
    let (url, task) = server(vec![ok(json!({"item_id":"new"})), ok(Value::Null)]).await;
    ApiClient::new(&url, "test-secret")
        .unwrap()
        .create_playlist("  New playlist  ", Some("library://track/1"))
        .await
        .unwrap();
    let requests = task.await.unwrap();
    assert_eq!(requests[0]["command"], "music/playlists/create_playlist");
    assert_eq!(requests[0]["args"], json!({"name":"New playlist"}));
    assert_eq!(
        requests[1]["command"],
        "music/playlists/add_playlist_tracks"
    );
    assert_eq!(
        requests[1]["args"],
        json!({"db_playlist_id":"new","uris":["library://track/1"]})
    );
}

#[tokio::test]
async fn create_playlist_rejects_blank_names_before_network() {
    let (url, task) = server(vec![]).await;
    let error = ApiClient::new(&url, "test-secret")
        .unwrap()
        .create_playlist(" \n\t ", None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Playlist name must be 1–200 characters");
    assert!(task.await.unwrap().is_empty());
}
#[tokio::test]
async fn redirects_are_not_followed_and_client_can_retry() {
    let (url, task) = server(vec![(307, "test-secret".into()), ok(json!([]))]).await;
    let client = ApiClient::new(&url, "test-secret").unwrap();
    assert!(client.players().await.is_err());
    assert!(client.players().await.unwrap().is_empty());
    assert_eq!(task.await.unwrap().len(), 2);
}
#[tokio::test]
async fn stalled_requests_time_out_without_exposing_url_or_token() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/private-server-path",
        listener.local_addr().unwrap()
    );
    let task = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    });
    let client = ApiClient::new(&url, "test-secret").unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(12), client.players()).await;
    task.abort();
    let error = format!(
        "{:#}",
        result
            .expect("client must enforce its own timeout")
            .unwrap_err()
    );
    assert!(!error.contains("test-secret") && !error.contains("private-server-path"));
}
#[tokio::test]
async fn queue_paginates_beyond_first_500_items() {
    let page: Vec<Value> = (0..500)
        .map(|i| json!({"queue_item_id":i.to_string(),"name":"Song"}))
        .collect();
    let (url, task) = server(vec![
        ok(json!({"queue_id":"leader","items":501})),
        ok(json!(page)),
        ok(json!([{"queue_item_id":"500","name":"Last"}])),
    ])
    .await;
    let q = ApiClient::new(&url, "test-secret")
        .unwrap()
        .queue("member")
        .await
        .unwrap();
    assert_eq!(q.items.len(), 501);
    assert_eq!(q.items[500].title, "Last");
    assert_eq!(task.await.unwrap()[2]["args"]["offset"], 500);
}
#[tokio::test]
async fn invalid_control_values_are_rejected_before_network() {
    for action in [
        api::Control::Seek(-1.0),
        api::Control::Seek(f64::NAN),
        api::Control::Seek(f64::INFINITY),
    ] {
        let (url, task) = server(vec![ok(json!({"queue_id":"leader"})), ok(Value::Null)]).await;
        let error = ApiClient::new(&url, "test-secret")
            .unwrap()
            .control("member", action)
            .await;
        task.abort();
        assert!(error.is_err());
    }
}
#[test]
fn client_debug_is_redacted() {
    let client = ApiClient::new("http://localhost/private-path", "test-secret").unwrap();
    let debug = format!("{client:?}");
    assert!(!debug.contains("test-secret") && !debug.contains("private-path"));
}
// Supplemental regression cases for the already exercised transport/parser.
#[tokio::test]
async fn http_errors_malformed_json_and_wrong_envelopes_are_redacted() {
    for response in [
        (401, "test-secret private-body".into()),
        (500, "test-secret private-body".into()),
        (200, "test-secret private-body".into()),
        ok(json!({"result":[],"error":"test-secret private-body"})),
    ] {
        let (url, task) = server(vec![response]).await;
        let error = ApiClient::new(&url, "test-secret")
            .unwrap()
            .players()
            .await
            .unwrap_err();
        for message in [format!("{error:#}"), format!("{error:?}")] {
            assert!(!message.contains("test-secret") && !message.contains("private-body"));
        }
        task.await.unwrap();
    }
}
#[tokio::test]
async fn no_active_queue_is_an_error_not_a_guessed_player_queue() {
    let (url, task) = server(vec![ok(Value::Null)]).await;
    assert!(ApiClient::new(&url, "test-secret")
        .unwrap()
        .play_uri("member", "library://track/1")
        .await
        .is_err());
    assert_eq!(task.await.unwrap().len(), 1);
}
#[tokio::test]
async fn optional_metadata_can_be_missing_or_null() {
    let (url, task) = server(vec![
        ok(json!([{"player_id":"p","name":"Offline","available":false,"volume_level":null}])),
        ok(json!({"queue_id":"q","current_item":null,"items":1})),
        ok(json!([{"queue_item_id":"radio","name":"Station","duration":null,"media_item":null}])),
    ])
    .await;
    let client = ApiClient::new(&url, "test-secret").unwrap();
    let p = client.players().await.unwrap();
    assert_eq!(p[0].volume, None);
    assert!(!p[0].available);
    let q = client.queue("p").await.unwrap();
    assert!(q.current_title.is_empty());
    assert_eq!(q.items[0].title, "Station");
    assert_eq!(q.items[0].duration, 0.0);
    task.await.unwrap();
}
fn ok(v: Value) -> (u16, String) {
    (200, v.to_string())
}

#[tokio::test]
async fn library_browse_paginates_and_preserves_query_order_and_favorite_filter() {
    use ma_tui::music::{Kind, Order, Target, PAGE_SIZE};
    let rows:Vec<_>=(0..PAGE_SIZE).map(|i|json!({"item_id":i.to_string(),"provider":"library","media_type":"track","name":"Fixture","uri":format!("library://track/{i}")})).collect();
    let (url, task) = server(vec![ok(json!(rows)), ok(json!([]))]).await;
    let api = ApiClient::new(&url, "test-secret").unwrap();
    let target = Target::Library {
        kind: Kind::Tracks,
        offset: 0,
        favorite: true,
        search: Some("Fixture".into()),
        order: Order::MostPlayed,
    };
    let (items, next) = api.browse(&target).await.unwrap();
    assert_eq!(items.len(), 100);
    assert!(items[0].playable);
    let next = next.expect("a full page has a next target");
    assert_eq!(
        next,
        Target::Library {
            kind: Kind::Tracks,
            offset: PAGE_SIZE,
            favorite: true,
            search: Some("Fixture".into()),
            order: Order::MostPlayed,
        }
    );
    let (items, next) = api.browse(&next).await.unwrap();
    assert!(items.is_empty());
    assert!(next.is_none());
    let calls = task.await.unwrap();
    assert_eq!(
        calls[0]["args"],
        json!({"limit":100,"offset":0,"order_by":"play_count_desc","favorite":true,"search":"Fixture"})
    );
    assert_eq!(
        calls[1]["args"],
        json!({"limit":100,"offset":100,"order_by":"play_count_desc","favorite":true,"search":"Fixture"})
    );
}

#[tokio::test]
async fn browse_routes_collections_and_provider_folders_without_player_commands() {
    use ma_tui::music::{Kind, Order, Target};
    let args = json!({"item_id":"fixture","provider_instance_id_or_domain":"provider"});
    for (target, command, expected) in [
        (
            Target::Album {
                id: "fixture".into(),
                provider: "provider".into(),
            },
            "music/albums/album_tracks",
            args.clone(),
        ),
        (
            Target::Playlist {
                id: "fixture".into(),
                provider: "provider".into(),
            },
            "music/playlists/playlist_tracks",
            args.clone(),
        ),
        (
            Target::Artist {
                id: "fixture".into(),
                provider: "provider".into(),
            },
            "music/artists/artist_albums",
            args.clone(),
        ),
        (
            Target::ArtistTracks {
                id: "fixture".into(),
                provider: "provider".into(),
            },
            "music/artists/artist_tracks",
            args,
        ),
        (
            Target::Providers {
                path: Some("provider://browse/folder".into()),
            },
            "music/browse",
            json!({"path":"provider://browse/folder"}),
        ),
        (
            Target::Library {
                kind: Kind::Radio,
                offset: 0,
                favorite: false,
                search: None,
                order: Order::Name,
            },
            "music/radios/library_items",
            json!({"limit":100,"offset":0,"order_by":"sort_name","favorite":null}),
        ),
        (
            Target::RecentlyPlayed,
            "music/recently_played_items",
            json!({"limit":50}),
        ),
    ] {
        let (url, task) = server(vec![ok(json!([]))]).await;
        let (items, _) = ApiClient::new(&url, "test-secret")
            .unwrap()
            .browse(&target)
            .await
            .unwrap();
        if matches!(target, Target::Artist { .. }) {
            assert!(matches!(items[0].open, Some(Target::ArtistTracks { .. })));
        }
        let calls = task.await.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["command"], command);
        assert_eq!(calls[0]["args"], expected);
    }
}

#[tokio::test]
async fn players_reads_bare_http_result_not_websocket_envelope() {
    let (url, task) = server(vec![ok(json!([{"player_id":"p1","name":"Kitchen","playback_state":"playing","volume_level":37,"available":true}]))]).await;
    let players = ApiClient::new(&url, "test-secret")
        .unwrap()
        .clone()
        .players()
        .await
        .unwrap();
    assert_eq!(players.len(), 1);
    assert_eq!(
        (
            &*players[0].id,
            &*players[0].name,
            &*players[0].state,
            players[0].volume,
            players[0].available
        ),
        ("p1", "Kitchen", "playing", Some(37), true)
    );
    let requests = task.await.unwrap();
    assert_eq!(requests[0]["command"], "players/all");
    assert!(requests[0]["message_id"].is_string());
}

/// Unplayed episodes are assembled here because MA 2.10.2 has no filter for
/// them: the shows are listed, then each is asked for its episodes.
#[tokio::test]
async fn unplayed_episodes_are_gathered_across_every_show() {
    use ma_tui::music::Target;
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let (url, task) = server_with(move |body: &str| {
        seen.lock().unwrap().push(body.to_owned());
        if body.contains("podcasts/library_items") {
            return json!([
                {"item_id":"s1","provider":"abs","name":"Show One","media_type":"podcast"},
                {"item_id":"s2","provider":"abs","name":"Show Two","media_type":"podcast"},
                // A show missing its identity cannot be asked for episodes.
                {"item_id":"","provider":"abs","name":"Broken","media_type":"podcast"},
            ]);
        }
        if body.contains("\"s1\"") {
            return json!([
                {"item_id":"e1","provider":"abs","name":"One first","media_type":"podcast_episode",
                 "uri":"library://podcast_episode/e1","fully_played":true},
                {"item_id":"e2","provider":"abs","name":"One second","media_type":"podcast_episode",
                 "uri":"library://podcast_episode/e2"},
                {"item_id":"e3","provider":"abs","name":"One third","media_type":"podcast_episode",
                 "uri":"library://podcast_episode/e3","resume_position_ms":5000},
            ]);
        }
        json!([
            {"item_id":"e4","provider":"abs","name":"Two only","media_type":"podcast_episode",
             "uri":"library://podcast_episode/e4","fully_played":false}
        ])
    })
    .await;

    let api = ApiClient::new(&url, "test-secret").unwrap();
    let (items, next) = api.browse(&Target::UnplayedEpisodes).await.unwrap();
    assert_eq!(next, None, "the list is not paged");

    let titles: Vec<&str> = items.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["One third", "One second", "Two only"],
        "finished episodes are dropped and each show reads newest first"
    );
    // A part-played episode is unfinished, and says where it stopped.
    assert!(items[0].detail.contains("resume 0:05"));

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 3, "one listing, then one read per usable show");
    assert!(
        !calls.iter().any(|c| c.contains("\"item_id\":\"\"")),
        "a show with no identity is not asked for"
    );
    task.abort();
}

#[tokio::test]
async fn failed_episode_reads_are_reported_instead_of_an_empty_unplayed_list() {
    use ma_tui::music::Target;
    let (url, task) = server(vec![
        ok(json!([
            {"item_id":"s1","provider":"abs"},
            {"item_id":"s2","provider":"abs"},
        ])),
        (503, "{}".into()),
        (503, "{}".into()),
    ])
    .await;
    let error = ApiClient::new(&url, "test-secret")
        .unwrap()
        .browse(&Target::UnplayedEpisodes)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("2 of 2 shows"), "{error}");
    assert!(error.contains("HTTP 503"), "{error}");
    assert_eq!(task.await.unwrap().len(), 3);
}

#[tokio::test]
async fn partially_failed_unplayed_reads_are_reported_even_after_the_item_limit() {
    use ma_tui::music::Target;
    // The first successful show fills the shelf. A later failure must still
    // be visible instead of being hidden by truncating the successful rows.
    let episodes: Vec<_> = (0..300)
        .map(|index| json!({"name":format!("Episode {index}"),"fully_played":false}))
        .collect();
    let (url, task) = server(vec![
        ok(json!([
            {"item_id":"s1","provider":"abs"},
            {"item_id":"s2","provider":"abs"},
        ])),
        ok(json!(episodes)),
        (503, "{}".into()),
    ])
    .await;
    let error = ApiClient::new(&url, "test-secret")
        .unwrap()
        .browse(&Target::UnplayedEpisodes)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("incomplete"), "{error}");
    assert!(error.contains("1 of 2 shows"), "{error}");
    assert_eq!(task.await.unwrap().len(), 3);
}

#[tokio::test]
async fn malformed_episode_listings_are_reported_as_failed_reads() {
    use ma_tui::music::Target;
    let (url, task) = server(vec![
        ok(json!([{"item_id":"s1","provider":"abs"}])),
        ok(json!({"unexpected":"response"})),
    ])
    .await;
    let error = ApiClient::new(&url, "test-secret")
        .unwrap()
        .browse(&Target::UnplayedEpisodes)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("1 of 1 shows"), "{error}");
    assert!(error.contains("Invalid podcast episode listing"), "{error}");
    assert_eq!(task.await.unwrap().len(), 2);
}

#[tokio::test]
async fn empty_subscriptions_and_empty_episode_listings_remain_successful() {
    use ma_tui::music::Target;
    for replies in [
        vec![ok(json!([]))],
        vec![
            ok(json!([{"item_id":"s1","provider":"abs"}])),
            ok(json!([])),
        ],
    ] {
        let expected_requests = replies.len();
        let (url, task) = server(replies).await;
        let (items, next) = ApiClient::new(&url, "test-secret")
            .unwrap()
            .browse(&Target::UnplayedEpisodes)
            .await
            .unwrap();
        assert!(items.is_empty());
        assert_eq!(next, None);
        assert_eq!(task.await.unwrap().len(), expected_requests);
    }
}
