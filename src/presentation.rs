use crate::{
    controller::Update,
    controls::{Entry, InputKind, Menu, Prompt, PromptTarget},
    music::{Media, Target},
    ui::{Action, App, PlayerView, TrackView},
};

fn display(text: String) -> String {
    text.chars().filter(|c| !c.is_control()).take(512).collect()
}

fn codec_label(raw: &str) -> Option<&str> {
    if raw.len() > 32 {
        return None;
    }
    let raw = raw.trim();
    let codec = if raw
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("audio/"))
    {
        &raw[6..]
    } else {
        raw
    };
    let codec = if codec
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("x-"))
    {
        &codec[2..]
    } else {
        codec
    };
    let codec = if codec.eq_ignore_ascii_case("mpeg") {
        "mp3"
    } else {
        codec
    };
    (!codec.is_empty()
        && codec.len() <= 12
        && !codec.eq_ignore_ascii_case("unknown")
        && codec
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
    .then_some(codec)
}

fn audio_codec(format: &serde_json::Value) -> Option<&str> {
    format["codec_type"]
        .as_str()
        .and_then(codec_label)
        .or_else(|| format["content_type"].as_str().and_then(codec_label))
}

fn mapped_audio_format(mapping: &serde_json::Value) -> Option<&serde_json::Value> {
    mapping
        .get("audio_format")
        .filter(|format| audio_codec(format).is_some())
}

/// A short status for the local speaker, followed only by file information
/// supplied by the current MA queue item (never by the output stream).
pub fn local_audio_line(state: &str, queue_details: &serde_json::Value) -> String {
    use std::fmt::Write;

    let state = state.trim();
    let state = if !state.is_empty()
        && state.len() <= 16
        && state.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
    {
        state
    } else {
        "unknown"
    };
    let mut line = format!("Local audio · {state}");
    let current = &queue_details["current_item"];
    let media = &current["media_item"];
    // Streamdetails describes the queued file. A library mapping may describe
    // another version, so never use its quality while streamdetails is present.
    let format = if let Some(stream) = current.get("streamdetails").filter(|v| v.is_object()) {
        stream
            .get("audio_format")
            .filter(|v| audio_codec(v).is_some())
    } else {
        let mappings = media["provider_mappings"].as_array();
        // Prefer the current provider when identifiable; otherwise take the
        // first mapping with usable audio format information.
        mappings
            .and_then(|mappings| {
                let provider = media["provider"].as_str();
                let item_id = media["item_id"].as_str();
                mappings
                    .iter()
                    .filter(|mapping| {
                        provider.is_some_and(|provider| {
                            mapping["provider_instance"].as_str() == Some(provider)
                                || mapping["provider_domain"].as_str() == Some(provider)
                        }) && item_id.is_some_and(|id| mapping["item_id"].as_str() == Some(id))
                    })
                    .find_map(mapped_audio_format)
                    .or_else(|| mappings.iter().find_map(mapped_audio_format))
            })
            .or_else(|| {
                media
                    .pointer("/metadata/audio_format")
                    .filter(|v| audio_codec(v).is_some())
            })
    };
    let Some(format) = format else {
        return line;
    };
    let Some(codec) = audio_codec(format) else {
        return line;
    };

    line.push_str(" · ");
    line.extend(codec.bytes().map(|b| b.to_ascii_uppercase() as char));
    // Numeric limits keep server-supplied metadata sensible and the full line
    // under 80 visible characters, even when every optional field is present.
    if let Some(bps) = format["bit_rate"]
        .as_u64()
        .filter(|bps| (1_000..=10_000_000).contains(bps))
    {
        let _ = write!(line, " {} kbps", (bps + 500) / 1_000);
    }
    if let Some(rate) = format["sample_rate"]
        .as_u64()
        .filter(|rate| (8_000..=384_000).contains(rate))
    {
        if rate % 1_000 == 0 {
            let _ = write!(line, " {} kHz", rate / 1_000);
        } else if rate % 100 == 0 {
            let _ = write!(line, " {}.{} kHz", rate / 1_000, (rate % 1_000) / 100);
        }
    }
    if let Some(depth) = format["bit_depth"]
        .as_u64()
        .filter(|depth| (8..=64).contains(depth))
    {
        let _ = write!(line, " {depth}-bit");
    }
    match format["channels"].as_u64() {
        Some(1) => line.push_str(" mono"),
        Some(2) => line.push_str(" stereo"),
        Some(channels @ 3..=8) => {
            let _ = write!(line, " {channels}ch");
        }
        _ => {}
    }
    line
}

/// Return a fixed, safe reconnect reason. Runtime error text may include peer
/// input, so it is never rendered directly.
fn audio_status_hint(state: &str, detail: &str) -> Option<&'static str> {
    match (state, detail) {
        (
            "reconnecting",
            "Audio proxy connection failed"
            | "Audio proxy authentication send failed"
            | "Audio proxy authentication failed"
            | "Audio proxy authentication timed out"
            | "Sendspin handshake timed out"
            | "Sendspin handshake failed",
        ) => Some("connection failed"),
        ("reconnecting", "Audio proxy disconnected" | "Audio connection closed") => {
            Some("connection lost")
        }
        (
            "reconnecting",
            "Audio receive queue overflow" | "Audio worker queue full or unavailable",
        ) => Some("receiver overloaded"),
        ("failed", "Audio worker queue overflow") => Some("worker queue overflow"),
        ("failed", "Audio output failed" | "Audio worker stopped") => Some("output stopped"),
        _ => None,
    }
}

/// A local-audio status with an optional fixed reconnect reason. The reason is
/// deliberately a small allowlist rather than the raw runtime detail.
pub fn local_audio_status_line(
    state: &str,
    detail: &str,
    queue_details: &serde_json::Value,
) -> String {
    let mut line = local_audio_line(state, queue_details);
    let Some(hint) = audio_status_hint(state.trim(), detail) else {
        return line;
    };
    let state_end = line["Local audio · ".len()..]
        .find(" · ")
        .map_or(line.len(), |offset| "Local audio · ".len() + offset);
    line.insert_str(state_end, &format!(" · {hint}"));
    line
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
