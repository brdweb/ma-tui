//! Episode listings on servers that page them (2.11.0b6 and later) and on
//! servers that return every episode (2.10.x).
use ma_tui::{api::ApiClient, music::Target};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// `count` episodes of one show, as a provider lists them: newest first,
/// positions counting from the oldest.
fn show(count: u64) -> Vec<Value> {
    (1..=count)
        .rev()
        .map(|position| {
            json!({"item_id":format!("e{position}"),"provider":"feed","name":format!("Episode {position}"),
                   "media_type":"podcast_episode","uri":format!("feed://podcast_episode/e{position}"),
                   "position":position,"fully_played":position % 2 == 0})
        })
        .collect()
}

/// A server with `episodes` for every show. With a schema it behaves like
/// 2.11.0b8: newest first by position, `limit` defaulting to 500 and 0 meaning
/// all. Without one it behaves like 2.10.x and ignores the paging arguments.
async fn server(schema: Option<u32>, episodes: Vec<Value>) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (episodes, seen) = (episodes.clone(), seen.clone());
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let text = loop {
                    let mut chunk = [0; 8192];
                    let read = socket.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    if text.starts_with("GET ") && text.contains("\r\n\r\n") {
                        break text;
                    }
                    if let Some((_, body)) = text.split_once("\r\n\r\n") {
                        if serde_json::from_str::<Value>(body).is_ok() {
                            break text;
                        }
                    }
                };
                let body = if text.starts_with("GET /info ") {
                    json!({"server_version":"fixture","schema_version":schema})
                } else {
                    let request: Value =
                        serde_json::from_str(text.split_once("\r\n\r\n").unwrap().1).unwrap();
                    seen.lock().unwrap().push(request.clone());
                    let args = &request["args"];
                    match request["command"].as_str().unwrap() {
                        "music/podcasts/library_items" => json!([
                            {"item_id":"s1","provider":"feed","name":"Show","media_type":"podcast"}
                        ]),
                        "music/podcasts/podcast_episodes" if schema.is_some() => {
                            let limit = args["limit"].as_u64().unwrap_or(500) as usize;
                            let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                            let page = episodes.iter().skip(offset);
                            let page: Vec<&Value> = if limit == 0 {
                                page.collect()
                            } else {
                                page.take(limit).collect()
                            };
                            json!(page)
                        }
                        "music/podcasts/podcast_episodes" => json!(episodes),
                        other => panic!("unexpected command {other}"),
                    }
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

/// Every title reachable by following `next` from the first page.
async fn every_page(api: &ApiClient) -> (Vec<String>, usize) {
    let mut target = Some(Target::Podcast {
        id: "s1".into(),
        provider: "feed".into(),
        offset: 0,
    });
    let (mut titles, mut pages) = (Vec::new(), 0);
    while let Some(page) = target {
        let (items, next) = api.browse(&page).await.unwrap();
        assert!(items.len() <= ma_tui::music::PAGE_SIZE);
        titles.extend(items.into_iter().map(|item| item.title));
        target = next;
        pages += 1;
    }
    (titles, pages)
}

fn newest_to_oldest(count: u64) -> Vec<String> {
    (1..=count).rev().map(|n| format!("Episode {n}")).collect()
}

#[tokio::test]
async fn a_long_show_is_reachable_in_full_on_a_paging_server() {
    let (url, calls) = server(Some(101), show(600)).await;
    let api = ApiClient::new(&url, "fixture-token").unwrap();
    api.refresh_schema().await.unwrap();
    let (titles, pages) = every_page(&api).await;
    assert_eq!(
        titles,
        newest_to_oldest(600),
        "past the server's default 500"
    );
    // A full last page cannot tell there is nothing after it, so one more
    // (empty) page is asked for, as with library listings.
    assert_eq!(pages, 7);
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls[1]["args"],
        json!({"item_id":"s1","provider_instance_id_or_domain":"feed",
               "sort_field":"position","sort_direction":"desc","limit":100,"offset":100})
    );
}

#[tokio::test]
async fn an_older_server_is_paged_here_in_the_same_order() {
    let (url, calls) = server(None, show(250)).await;
    let api = ApiClient::new(&url, "fixture-token").unwrap();
    api.refresh_schema().await.unwrap();
    let (titles, pages) = every_page(&api).await;
    assert_eq!(titles, newest_to_oldest(250));
    assert_eq!(pages, 3);
    // Asking for everything also covers a paging server whose schema is not
    // known yet: limit 0 is "all" there.
    assert_eq!(calls.lock().unwrap()[0]["args"]["limit"], 0);
}

#[tokio::test]
async fn order_comes_from_position_not_arrival() {
    // Oldest first on the wire, as some providers list them.
    let mut oldest_first = show(5);
    oldest_first.reverse();
    let (url, _) = server(None, oldest_first).await;
    let api = ApiClient::new(&url, "fixture-token").unwrap();
    let (titles, _) = every_page(&api).await;
    assert_eq!(titles, newest_to_oldest(5));
}

#[tokio::test]
async fn unplayed_episodes_read_newest_first_on_both_generations() {
    for schema in [None, Some(101)] {
        // Newest first on the wire, as 2.11 and the 2.10.9 feed provider send them.
        let (url, _) = server(schema, show(6)).await;
        let api = ApiClient::new(&url, "fixture-token").unwrap();
        api.refresh_schema().await.unwrap();
        let (items, _) = api.browse(&Target::UnplayedEpisodes).await.unwrap();
        let titles: Vec<&str> = items.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Episode 5", "Episode 3", "Episode 1"],
            "schema {schema:?}"
        );
    }
}
