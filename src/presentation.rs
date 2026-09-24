use crate::{
    controller::Update,
    controls::{Entry, InputKind, Menu, Prompt, PromptTarget},
    music::{Media, Target},
    ui::{Action, App, PlayerView, TrackView},
};

fn display(text: String) -> String {
    text.chars().filter(|c| !c.is_control()).take(512).collect()
}

/// A library event names its library URI, while rows may be provider mappings.
fn matches_media(media: &Media, uri: &str, item: Option<&serde_json::Value>) -> bool {
    (!uri.is_empty() && media.uri == uri)
        || item
            .and_then(|item| item["provider_mappings"].as_array())
            .is_some_and(|mappings| {
                !media.id.is_empty()
                    && !media.provider.is_empty()
                    && mappings.iter().any(|mapping| {
                        mapping["item_id"].as_str() == Some(media.id.as_str())
                            && (mapping["provider_instance"].as_str()
                                == Some(media.provider.as_str())
                                || mapping["provider_domain"].as_str()
                                    == Some(media.provider.as_str()))
                    })
            })
}

/// MA can expose the embedded Sendspin endpoint behind a universal player.
/// Select its public player ID, keeping control/queue routing on that wrapper.
pub fn select_local(app: &mut App, endpoint: &str) -> Option<String> {
    if app.selected_id.is_some() || !app.connected || endpoint.is_empty() {
        return None;
    }
    let index = app
        .players
        .iter()
        .position(|p| p.available && matches_endpoint(p, endpoint))?;
    let id = app.players[index].id.clone();
    app.selected_id = Some(id.clone());
    app.player_cursor = index;
    Some(id)
}

/// Whether a player is MA-TUI's own endpoint, directly or as the universal
/// wrapper MA puts in front of it. Display names are never identity matches.
pub fn matches_endpoint(player: &PlayerView, endpoint: &str) -> bool {
    !endpoint.is_empty()
        && (player.id == endpoint
            || player.details["output_protocols"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|v| v["output_protocol_id"].as_str() == Some(endpoint)))
}

/// Apply network snapshots only to the player/query they were requested for.
/// Returns work the update implies, for the caller to submit.
pub fn apply(app: &mut App, event: Update) -> Option<crate::ui::Action> {
    match event {
        Update::Browse(generation, result) => app.music.apply(generation, result),
        Update::Players(players) => {
            let cursor_id = app.players.get(app.player_cursor).map(|p| p.id.clone());
            app.players = players
                .into_iter()
                .map(|p| PlayerView {
                    details: p.details,
                    id: p.id,
                    name: display(p.name),
                    state: display(p.state),
                    volume: p.volume,
                    available: p.available,
                })
                .collect();
            app.player_cursor = cursor_id
                .and_then(|id| app.players.iter().position(|p| p.id == id))
                .unwrap_or(0);
            if !app.connected {
                app.status = "Connected · select a player; controls act on that player".into();
            }
            app.connected = true;
        }
        Update::Queue(id, result) if app.selected_id.as_deref() == Some(id.as_str()) => {
            match result {
                Ok(queue) => {
                    let highlighted = app.queue.get(app.queue_cursor).map(|t| t.id.clone());
                    app.queue_id = queue.id;
                    app.queue_details = queue.details;
                    app.title = if queue.current_title.is_empty() {
                        "Nothing playing".into()
                    } else {
                        display(queue.current_title)
                    };
                    app.artist = display(queue.current_artist);
                    app.elapsed = queue.elapsed;
                    app.elapsed_at = Some(std::time::Instant::now());
                    app.duration = queue.duration;
                    app.queue = queue
                        .items
                        .into_iter()
                        .map(|t| TrackView {
                            id: t.id,
                            title: display(t.title),
                            artist: display(t.artist),
                            duration: t.duration,
                            ..Default::default()
                        })
                        .collect();
                    app.queue_cursor = highlighted
                        .and_then(|id| app.queue.iter().position(|t| t.id == id))
                        .unwrap_or(app.queue_cursor.min(app.queue.len().saturating_sub(1)));
                }
                Err(error) => {
                    app.queue_id.clear();
                    app.queue_details = serde_json::Value::Null;
                    app.queue.clear();
                    app.title = "Queue unavailable".into();
                    app.artist.clear();
                    app.elapsed = 0.0;
                    app.elapsed_at = None;
                    app.duration = 0.0;
                    app.status = error;
                }
            }
        }
        Update::Search(query, result) if query == app.query.trim() => match result {
            Ok(tracks) => {
                app.results = tracks
                    .into_iter()
                    .map(|t| TrackView {
                        uri: t.uri,
                        media: t.media,
                        title: display(t.title),
                        artist: display(t.artist),
                        ..Default::default()
                    })
                    .collect();
                app.search_cursor = 0;
                app.status = format!(
                    "Search complete · {} results (up to 50 per type)",
                    app.results.len()
                );
            }
            Err(error) => {
                app.results.clear();
                app.status = error;
            }
        },
        Update::MediaItem { uri, item } => {
            let item = item.as_ref();
            let showing_favorites = matches!(
                &app.music.page.target,
                Target::Library { favorite: true, .. }
            );
            let mut page_matched = false;
            let mut page_favorite_changed = false;
            for media in &mut app.music.page.items {
                if !matches_media(media, &uri, item) {
                    continue;
                }
                page_matched = true;
                if let Some(item) = item {
                    let favorite = item["favorite"] == true;
                    page_favorite_changed |= media.favorite != favorite;
                    media.favorite = favorite;
                    media.in_library = true;
                } else {
                    media.in_library = false;
                }
            }
            for result in &mut app.results {
                let Some(media) = &mut result.media else {
                    continue;
                };
                if !matches_media(media, &uri, item) {
                    continue;
                }
                if let Some(item) = item {
                    media.favorite = item["favorite"] == true;
                    media.in_library = true;
                } else {
                    media.in_library = false;
                }
            }
            if let Some(current) = app.queue_details.pointer_mut("/current_item/media_item") {
                let media = Media::parse(current, "track");
                if matches_media(&media, &uri, item) {
                    if let Some(item) = item {
                        current["favorite"] = serde_json::Value::Bool(item["favorite"] == true);
                    }
                }
            }
            if showing_favorites
                && (page_favorite_changed
                    || (!page_matched && item.is_some_and(|item| item["favorite"] == true)))
            {
                return Some(app.music.reload());
            }
        }
        Update::Stream(live) => app.live = live,
        Update::Artwork(art) => {
            app.artwork = art;
            app.artwork_generation = app.artwork_generation.wrapping_add(1);
        }
        // The server's own clock, for the queue currently on screen.
        Update::Elapsed(queue_id, seconds) if app.queue_id == queue_id => {
            app.elapsed = seconds;
            app.elapsed_at = Some(std::time::Instant::now());
        }
        Update::Offline(error) => {
            app.connected = false;
            app.status = format!("Disconnected · data stale · retrying: {error}");
        }
        Update::Playlists { uri, result } => {
            if app.settings.is_some() {
                return None;
            }
            if app.menu.as_ref().is_some_and(|menu| menu.prompt.is_some()) {
                app.status = "Playlists loaded; close the menu to choose".into();
                return None;
            }
            match result {
                Ok(playlists) => {
                    let mut entries: Vec<Entry> = playlists
                        .into_iter()
                        .map(|playlist| Entry {
                            section: "Playlists",
                            label: playlist.title,
                            action: Action::AddToPlaylist {
                                playlist_id: playlist.id,
                                uri: uri.clone(),
                            },
                        })
                        .collect();
                    entries.push(Entry {
                        section: "New",
                        label: "New playlist…".into(),
                        action: Action::Prompt(Prompt {
                            label: "New playlist name".into(),
                            target: PromptTarget::CreatePlaylist { uri: Some(uri) },
                            kind: InputKind::Text,
                            value: String::new(),
                        }),
                    });
                    app.menu = Some(Menu {
                        player: app.selected_id.clone(),
                        title: "Add to playlist".into(),
                        entries,
                        cursor: 0,
                        prompt: None,
                        error: String::new(),
                        filter: String::new(),
                        filtering: false,
                    });
                }
                Err(error) => app.status = format!("Could not load playlists: {error}"),
            }
        }
        Update::Notice(text) => app.status = text,
        Update::Playlog => {
            return app.music.progress_changed(std::time::Instant::now());
        }
        _ => {}
    }
    None
}
