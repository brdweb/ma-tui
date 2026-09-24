//! Music Assistant's event stream, so the interface learns about a change when
//! it happens instead of re-asking on a timer.
//!
//! Verified against server tag 2.10.2
//! `controllers/webserver/websocket_client.py`: the socket carries the same
//! command envelope as HTTP `/api`, the first command must be `auth`, and after
//! it succeeds the server subscribes the connection itself — there is no
//! subscribe command to send. Events then arrive unprompted as
//! `{"event":…, "object_id":…, "data":…}`.
//!
//! Event names and `object_id`s route updates. `queue_time_updated` carries
//! elapsed seconds (`player_queues/controller.py` signals
//! `data=queue.elapsed_time`), while media-item events carry a full library
//! item for the interface to apply directly. Everything else is treated as
//! "this went stale", and the existing HTTP reads remain the single place that
//! parses a player or a queue. Peer input never reaches an error message.

use anyhow::{anyhow, bail, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// What the server says has changed, reduced to what this application acts on.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A player appeared, changed or went away; the whole list is re-read.
    Players,
    /// This queue's own state changed (track, transport, shuffle, repeat).
    Queue(String),
    /// This queue's contents changed.
    QueueItems(String),
    /// Playback position for this queue, in seconds.
    Elapsed(String, f64),
    /// Played/resume state changed somewhere in the library.
    Playlog,
    /// A library item changed; deletion carries no replacement item.
    MediaItem {
        uri: String,
        item: Option<serde_json::Value>,
    },
    /// The stream is live again; anything shown may have been missed.
    Online,
    /// The stream dropped. Polling has to carry the interface until it returns.
    Offline,
}

/// The running stream. Holding this keeps it alive; dropping or shutting it
/// down ends it. Events themselves arrive on the receiver `start` hands back.
pub struct Events {
    task: JoinHandle<()>,
}

impl Events {
    /// Spawn the stream. Connecting happens in the background and retries, so a
    /// server that is not reachable yet never blocks starting the interface.
    pub fn start(server: &str, token: &str) -> Result<(Self, mpsc::Receiver<Event>)> {
        let url = events_url(server)?;
        let token = token.to_owned();
        let (tx, updates) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                // `session` reports Ok exactly when it reached the authenticated
                // stream, so that is also when the interface has to be told the
                // stream has gone. A failure reason is deliberately dropped: it
                // can carry peer input, and polling covers the outage regardless.
                if session(&url, &token, &tx).await.is_ok() {
                    if tx.send(Event::Offline).await.is_err() {
                        return;
                    }
                    backoff = Duration::from_secs(1);
                } else {
                    backoff = (backoff * 2).min(Duration::from_secs(15));
                }
                tokio::time::sleep(backoff).await;
            }
        });
        Ok((Self { task }, updates))
    }

    pub async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

/// One connection: authenticate, then forward events until it ends.
async fn session(url: &url::Url, token: &str, tx: &mpsc::Sender<Event>) -> Result<()> {
    let mut socket = tokio::time::timeout(Duration::from_secs(10), async {
        let (mut socket, _) = tokio_tungstenite::connect_async_with_config(
            url.as_str(),
            Some(
                tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                    .max_message_size(Some(4 * 1024 * 1024))
                    .max_frame_size(Some(4 * 1024 * 1024)),
            ),
            false,
        )
        .await
        .map_err(|_| anyhow!("Event stream connection failed"))?;
        socket
            .send(WsMessage::text(
                serde_json::json!({
                    "command": "auth",
                    "message_id": AUTH_ID,
                    "args": {"token": token},
                })
                .to_string(),
            ))
            .await
            .map_err(|_| anyhow!("Event stream authentication send failed"))?;
        // The server greets the connection with its own information before any
        // result, so read past anything that is not this command's answer.
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    let value: serde_json::Value = serde_json::from_str(&text)
                        .map_err(|_| anyhow!("Invalid event stream response"))?;
                    if value.get("message_id").and_then(|v| v.as_str()) != Some(AUTH_ID) {
                        continue;
                    }
                    if value.get("error_code").is_some() {
                        bail!("Event stream authentication rejected");
                    }
                    return Ok(socket);
                }
                Some(Ok(WsMessage::Ping(data))) => socket
                    .send(WsMessage::Pong(data))
                    .await
                    .map_err(|_| anyhow!("Event stream disconnected"))?,
                Some(Ok(_)) => continue,
                _ => bail!("Event stream authentication failed"),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("Event stream authentication timed out"))??;

    if tx.send(Event::Online).await.is_err() {
        return Ok(());
    }
    // Past this point the stream was live, so every way out is a plain end of
    // stream rather than a failure: the caller uses Ok to mean "was connected".
    while let Some(message) = socket.next().await {
        match message {
            Ok(WsMessage::Text(text)) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if let Some(event) = translate(&value) {
                    if tx.send(event).await.is_err() {
                        return Ok(());
                    }
                }
            }
            Ok(WsMessage::Ping(data)) => {
                if socket.send(WsMessage::Pong(data)).await.is_err() {
                    break;
                }
            }
            Ok(WsMessage::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        }
    }
    Ok(())
}

const AUTH_ID: &str = "ma-tui-auth";

/// Map one server event onto what this application re-reads. Unknown events are
/// ignored rather than guessed at.
pub fn translate(value: &serde_json::Value) -> Option<Event> {
    let name = value.get("event")?.as_str()?;
    let object = || {
        value
            .get("object_id")
            .and_then(|v| v.as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    };
    match name {
        "player_added" | "player_updated" | "player_removed" => Some(Event::Players),
        "queue_updated" => Some(Event::Queue(object()?)),
        "queue_items_updated" => Some(Event::QueueItems(object()?)),
        "queue_time_updated" => {
            let seconds = value.get("data")?.as_f64().filter(|s| s.is_finite())?;
            Some(Event::Elapsed(object()?, seconds.max(0.0)))
        }
        "playlog_updated" | "media_item_played" => Some(Event::Playlog),
        "media_item_updated" | "media_item_added" => {
            let item = value.get("data").filter(|item| item.is_object())?.clone();
            Some(Event::MediaItem {
                uri: object()?,
                item: Some(item),
            })
        }
        "media_item_deleted" => Some(Event::MediaItem {
            uri: object()?,
            item: None,
        }),
        _ => None,
    }
}

/// MA 2.10.2 serves the event socket at /ws, alongside the HTTP API.
pub(crate) fn events_url(base: &str) -> Result<url::Url> {
    let mut url = url::Url::parse(base).map_err(|_| anyhow!("Invalid server URL"))?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => bail!("Server URL must be HTTP(S)"),
    };
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Server URL must not carry credentials, query or fragment");
    }
    url.set_scheme(scheme)
        .map_err(|_| anyhow!("Invalid server URL"))?;
    let path = format!("{}/ws", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}
