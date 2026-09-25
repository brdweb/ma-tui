use crate::api::{ApiClient, Control, Player, Queue, Track};
use crate::events::Event;
use crate::ui::Action;
use std::time::Duration;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

pub enum Update {
    Players(Vec<Player>),
    Queue(String, Result<Queue, String>),
    /// Playback position for a queue, straight from the event stream.
    Elapsed(String, f64),
    /// Whether the server is currently telling us about changes itself.
    Stream(bool),
    /// Listening progress changed somewhere in the library.
    Playlog,
    /// A library item changed on the server. `item` is None when it was deleted.
    MediaItem {
        uri: String,
        item: Option<serde_json::Value>,
    },
    /// The cover for what is playing, or none when there is not one.
    Artwork(Option<crate::artwork::Art>),
    Offline(String),
    Search(String, Result<Vec<Track>, String>),
    Browse(
        u64,
        Result<(Vec<crate::music::Media>, Option<crate::music::Target>), String>,
    ),
    /// Editable playlists for the picker, answering LoadPlaylists for `uri`.
    Playlists {
        uri: String,
        result: Result<Vec<crate::music::Media>, String>,
    },
    Notice(String),
}

pub struct Request {
    pub player: Option<String>,
    pub action: Action,
    pub issued: std::time::Instant,
}
impl Request {
    pub fn new(player: Option<String>, action: Action) -> Self {
        Self {
            player,
            action,
            issued: std::time::Instant::now(),
        }
    }
}

pub struct Controller {
    pub requests: mpsc::Sender<Request>,
    pub selection: watch::Sender<Option<String>>,
    pub updates: mpsc::Receiver<Update>,
    task: JoinHandle<()>,
}

/// One in-flight read, cancelled when a newer one supersedes it or the
/// controller stops, so an abandoned request does not hold a connection open.
#[derive(Default)]
struct Pending(Option<JoinHandle<()>>);

impl Pending {
    fn replace(&mut self, task: JoinHandle<()>) {
        if let Some(previous) = self.0.replace(task) {
            previous.abort();
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

/// How often to re-ask when the server is not telling us about changes, and
/// how often when it is. The live interval only catches a missed event or a
/// socket that died without saying so.
const POLL: Duration = Duration::from_secs(2);
const POLL_LIVE: Duration = Duration::from_secs(30);

/// Which reads the next pass owes. An event names what went stale rather than
/// forcing the whole snapshot to be re-read.
#[derive(Default, Clone, Copy)]
struct Stale {
    players: bool,
    queue: bool,
}

impl Stale {
    fn all() -> Self {
        Self {
            players: true,
            queue: true,
        }
    }
    fn any(self) -> bool {
        self.players || self.queue
    }
}

/// The event stream's next message, or never when there is no stream.
async fn next_event(events: &mut Option<mpsc::Receiver<Event>>) -> Option<Event> {
    match events {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Columns the cover is requested for. The proxy's smallest served size is 80,
/// which is already more than any panel this interface draws.
const ARTWORK_COLUMNS: u16 = 80;

impl Controller {
    pub fn start(api: ApiClient, events: Option<mpsc::Receiver<Event>>, artwork: bool) -> Self {
        let (selection, mut selected) = watch::channel::<Option<String>>(None);
        let (requests, mut commands) = mpsc::channel::<Request>(32);
        let (tx, updates) = mpsc::channel(16);
        let task = tokio::spawn(async move {
            let mut events = events;
            // Having a receiver does not mean the socket connected or
            // authenticated. Poll until the stream explicitly reports Online.
            let mut interval = tokio::time::interval(POLL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // The queue the last read was about, so an event for some other
            // player's queue costs nothing.
            let mut current: Option<String> = None;
            // Reading the library must never delay a transport key, so browse,
            // search and playlist-picker reads run beside this loop. Only the
            // newest of each matters: superseded requests are cancelled rather
            // than left to finish unread.
            let (mut browsing, mut searching, mut playlists) =
                (Pending::default(), Pending::default(), Pending::default());
            // The cover only changes when the item does, so it is fetched then
            // rather than on every queue read.
            let (mut covering, mut cover) = (Pending::default(), String::new());
            loop {
                let mut stale = Stale::all();
                tokio::select! {
                    _ = interval.tick() => {},
                    changed = selected.changed() => { if changed.is_err() { break; } },
                    event = next_event(&mut events) => {
                        let Some(event) = event else {
                            // The stream gave up for good; carry on by asking.
                            events = None;
                            interval = tokio::time::interval(POLL);
                            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                            let _ = tx.send(Update::Stream(false)).await;
                            continue;
                        };
                        stale = Stale::default();
                        // Take everything already waiting too, so a burst of
                        // events costs one read rather than one read each.
                        let mut next = Some(event);
                        while let Some(event) = next.take() {
                            match event {
                                Event::Players => stale.players = true,
                                Event::Queue(id) | Event::QueueItems(id) => {
                                    if current.as_deref() == Some(id.as_str()) { stale.queue = true; }
                                }
                                // The only payload read: no request needed at all.
                                Event::Elapsed(id, seconds) => {
                                    if current.as_deref() == Some(id.as_str())
                                        && tx.send(Update::Elapsed(id, seconds)).await.is_err() { return; }
                                }
                                // Anything shown may have changed while it was down.
                                Event::Online => {
                                    stale = Stale::all();
                                    interval = tokio::time::interval(POLL_LIVE);
                                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                                    if tx.send(Update::Stream(true)).await.is_err() { return; }
                                }
                                Event::Offline => {
                                    interval = tokio::time::interval(POLL);
                                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                                    if tx.send(Update::Stream(false)).await.is_err() { return; }
                                }
                                // Progress changed somewhere — possibly in
                                // Audiobookshelf or the web interface. The
                                // listing decides for itself whether it cares.
                                Event::Playlog => {
                                    if tx.send(Update::Playlog).await.is_err() { return; }
                                }
                                // Library events carry their full replacement, so no
                                // snapshot read is needed to forward them.
                                Event::MediaItem { uri, item } => {
                                    if tx.send(Update::MediaItem { uri, item }).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            next = events.as_mut().and_then(|rx| rx.try_recv().ok());
                        }
                        if !stale.any() { continue; }
                    },
                    command = commands.recv() => {
                        let Some(command) = command else { break; };
                        if command.issued.elapsed() > Duration::from_secs(3)
                            && !matches!(
                                command.action,
                                Action::Browse { .. }
                                    | Action::Search(_)
                                    | Action::LoadPlaylists { .. }
                            )
                        {
                            let _ = tx.send(Update::Notice("Command expired; press the key again".into())).await;
                            continue;
                        }
                        if let Action::Search(query) = command.action {
                            let (api, tx) = (api.clone(), tx.clone());
                            searching.replace(tokio::spawn(async move {
                                let result = api.search(&query).await.map_err(|e|e.to_string());
                                let _ = tx.send(Update::Search(query,result)).await;
                            }));
                            continue;
                        }
                        if let Action::Browse {generation, target} = command.action {
                            let (api, tx) = (api.clone(), tx.clone());
                            browsing.replace(tokio::spawn(async move {
                                let result = api.browse(&target).await.map_err(|e|e.to_string());
                                let _ = tx.send(Update::Browse(generation,result)).await;
                            }));
                            continue;
                        }
                        if let Action::LoadPlaylists { uri } = command.action {
                            let (api, tx) = (api.clone(), tx.clone());
                            playlists.replace(tokio::spawn(async move {
                                let result = api.editable_playlists().await.map_err(|e| e.to_string());
                                let _ = tx.send(Update::Playlists { uri, result }).await;
                            }));
                            continue;
                        }
                        if !matches!(command.action, Action::Refresh) {
                            let result = execute(&api,command).await;
                            let notice = match result {
                                Ok(Some(notice)) => notice.into(),
                                Ok(None) => "Command accepted; refreshing state".into(),
                                Err(e) => format!("Command failed (not retried): {e}"),
                            };
                            if tx.send(Update::Notice(notice)).await.is_err() { break; }
                        }
                    }
                }
                // The player list and the selected queue are independent reads,
                // so they cost one round trip together rather than two in turn.
                let id = selected.borrow().clone();
                let wants_queue = stale.queue.then_some(id.as_deref()).flatten();
                let (players, queue) = match (stale.players, wants_queue) {
                    (true, Some(id)) => {
                        let (players, queue) = tokio::join!(api.players(), api.queue(id));
                        (Some(players), Some(queue.map_err(|e| e.to_string())))
                    }
                    (true, None) => (Some(api.players().await), None),
                    (false, Some(id)) => {
                        (None, Some(api.queue(id).await.map_err(|e| e.to_string())))
                    }
                    (false, None) => (None, None),
                };
                if let Some(players) = players {
                    let players = match players {
                        Ok(players) => players,
                        Err(err) => {
                            if tx.send(Update::Offline(err.to_string())).await.is_err() {
                                break;
                            }
                            continue;
                        }
                    };
                    if tx.send(Update::Players(players)).await.is_err() {
                        break;
                    }
                }
                if let (Some(id), Some(queue)) = (id, queue) {
                    // Remember which queue is on screen so its events are the
                    // only ones that cost a read.
                    current = queue.as_ref().ok().map(|q| q.id.clone());
                    if artwork {
                        let next = queue
                            .as_ref()
                            .ok()
                            .and_then(|q| crate::artwork::proxy_id(&q.details["current_item"]))
                            .unwrap_or_default();
                        if next != cover {
                            cover = next.clone();
                            let (api, tx) = (api.clone(), tx.clone());
                            covering.replace(tokio::spawn(async move {
                                let art = match next.is_empty() {
                                    true => None,
                                    false => api.artwork(&next, ARTWORK_COLUMNS).await.ok(),
                                };
                                let _ = tx.send(Update::Artwork(art)).await;
                            }));
                        }
                    }
                    if tx.send(Update::Queue(id, queue)).await.is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            requests,
            selection,
            updates,
            task,
        }
    }
    pub async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

async fn execute(api: &ApiClient, request: Request) -> anyhow::Result<Option<&'static str>> {
    let Request { player, action, .. } = request;
    match action {
        Action::MarkPlayed { item, played } => {
            api.mark_played(item, played).await?;
            Ok(None)
        }
        Action::Favorite {
            uri,
            media_type,
            library_id,
            favorite,
        } => {
            api.set_favorite(&uri, &media_type, library_id.as_deref(), favorite)
                .await?;
            Ok(Some(if favorite {
                "Added to favourites"
            } else {
                "Removed from favourites"
            }))
        }
        Action::AddToLibrary { uri } => {
            api.add_to_library(&uri).await?;
            Ok(Some("Added to library"))
        }
        Action::AddToPlaylist { playlist_id, uri } => {
            api.add_to_playlist(&playlist_id, &uri).await?;
            Ok(Some(
                "Added to playlist (the server finishes adding in the background)",
            ))
        }
        Action::CreatePlaylist { name, uri } => {
            api.create_playlist(&name, uri.as_deref()).await?;
            Ok(Some("Playlist created"))
        }
        action => {
            let player = player.ok_or_else(|| anyhow::anyhow!("No player selected"))?;
            match action {
                Action::Toggle => api.control(&player, Control::Toggle).await?,
                Action::Next => api.control(&player, Control::Next).await?,
                Action::Previous => api.control(&player, Control::Previous).await?,
                // The interface resolves the target position, so holding the key
                // does not issue a queue request per keystroke.
                Action::Seek(position) => api.control(&player, Control::Seek(position)).await?,
                Action::Play(uri) => api.play_uri(&player, &uri).await?,
                Action::Enqueue(uri) => api.enqueue_uri(&player, &uri).await?,
                Action::PlayMany(uris) => api.play_uris(&player, &uris).await?,
                Action::EnqueueMany(uris) => api.enqueue_uris(&player, &uris).await?,
                Action::PlayFolder(target) => {
                    let uris = folder_uris(api, &target).await?;
                    api.play_uris(&player, &uris).await?;
                }
                Action::EnqueueFolder(target) => {
                    let uris = folder_uris(api, &target).await?;
                    api.enqueue_uris(&player, &uris).await?;
                }
                Action::PlayNext(uri) => api.play_next_uri(&player, &uri).await?,
                Action::StartRadio(uri) => {
                    api.start_radio(&player, &uri).await?;
                    return Ok(Some("Radio started"));
                }
                Action::Command(command) => api.playback_command(&player, command).await?,
                _ => {}
            }
            Ok(None)
        }
    }
}

async fn folder_uris(
    api: &ApiClient,
    target: &crate::music::Target,
) -> anyhow::Result<Vec<String>> {
    let (items, _) = api.browse(target).await?;
    let uris: Vec<_> = items
        .into_iter()
        .filter(|item| item.available && item.playable && !item.uri.is_empty())
        .map(|item| item.uri)
        .collect();
    if uris.is_empty() {
        return Err(anyhow::anyhow!("Folder has no available playable items"));
    }
    Ok(uris)
}
