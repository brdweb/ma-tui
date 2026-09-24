use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::theme::Palette;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    OpenSettings,
    SaveSettings,
    Command(crate::controls::Command),
    Prompt(crate::controls::Prompt),
    Quit,
    Refresh,
    Select(String),
    Search(String),
    Browse {
        generation: u64,
        target: crate::music::Target,
    },
    Toggle,
    Next,
    Previous,
    /// Absolute position in seconds, resolved by the interface.
    Seek(f64),
    Play(String),
    Enqueue(String),
    PlayNext(String),
    /// Mark a library item played or unplayed. Carries the item's identity
    /// because Music Assistant names the item itself, not a URI. This is a
    /// library edit, so it needs no speaker.
    MarkPlayed {
        item: serde_json::Value,
        played: bool,
    },
    /// Favourite or unfavourite an item. Adding names the item by URI;
    /// removing needs its library id, which is known only for library rows.
    /// A library edit, so it needs no speaker.
    Favorite {
        uri: String,
        media_type: String,
        library_id: Option<String>,
        favorite: bool,
    },
    /// Save a provider item into the library. A library edit.
    AddToLibrary {
        uri: String,
    },
    /// Replace the queue with a radio station seeded from an item.
    StartRadio(String),
    /// Read the playlists this user can add to, for the playlist picker.
    LoadPlaylists {
        uri: String,
    },
    /// Append one item to an editable library playlist.
    AddToPlaylist {
        playlist_id: String,
        uri: String,
    },
    /// Create a library playlist, seeded with one item when given.
    CreatePlaylist {
        name: String,
        uri: Option<String>,
    },
}

impl Action {
    /// Whether this is aimed at a speaker. A library edit is not: it changes
    /// what the server stores, so it stands on its own.
    pub fn needs_player(&self) -> bool {
        !matches!(
            self,
            Action::MarkPlayed { .. }
                | Action::Favorite { .. }
                | Action::AddToLibrary { .. }
                | Action::LoadPlaylists { .. }
                | Action::AddToPlaylist { .. }
                | Action::CreatePlaylist { .. }
        )
    }
}

impl App {
    /// A queue mode change for the displayed queue. Dynamic queues report no
    /// shuffle or repeat state, so the key says so rather than guessing.
    fn queue_mode(&mut self, name: &'static str, args: serde_json::Value) -> Action {
        if self.queue_id.is_empty() {
            self.status = "No active queue for this player yet".into();
            return Action::None;
        }
        if self.queue_details["is_dynamic"] == true {
            self.status = "A dynamic queue has no shuffle or repeat setting".into();
            return Action::None;
        }
        Action::Command(crate::controls::Command::Queue {
            id: self.queue_id.clone(),
            name,
            args,
        })
    }

    /// Seek relative to the position already on screen, so repeated presses
    /// accumulate without a queue request each time. The next poll corrects it.
    fn seek(&mut self, delta: f64) -> Action {
        if !self.duration.is_finite() || self.duration <= 0.0 || !self.elapsed.is_finite() {
            self.status = "This item has no seekable duration".into();
            return Action::None;
        }
        self.elapsed = (self.elapsed + delta).clamp(0.0, self.duration);
        // Keep running from the new position rather than the old snapshot.
        self.elapsed_at = Some(std::time::Instant::now());
        Action::Seek(self.elapsed)
    }

    /// Whether the selected speaker reports that it is playing.
    fn playing(&self) -> bool {
        self.players.iter().any(|p| {
            Some(&p.id) == self.selected_id.as_ref() && p.available && p.state == "playing"
        })
    }

    /// Carry the displayed position forward between server snapshots, which
    /// arrive far too rarely to animate a progress bar. Every snapshot replaces
    /// the value outright, so this is a display estimate and drift cannot
    /// accumulate across polls.
    pub fn advance(&mut self, now: std::time::Instant) {
        let Some(anchor) = self.elapsed_at else {
            return;
        };
        self.elapsed_at = Some(now);
        if !self.playing() || !self.elapsed.is_finite() {
            return;
        }
        self.elapsed += now.saturating_duration_since(anchor).as_secs_f64();
        if self.duration.is_finite() && self.duration > 0.0 {
            self.elapsed = self.elapsed.min(self.duration);
        }
    }

    /// Why the visualizer has no samples, in terms of the selected speaker.
    /// A remote speaker's audio never reaches this machine.
    fn silence(&self) -> String {
        let selected = self
            .players
            .iter()
            .find(|p| Some(&p.id) == self.selected_id.as_ref());
        let local = selected
            .zip(self.local_endpoint.as_deref())
            .is_some_and(|(p, endpoint)| crate::presentation::matches_endpoint(p, endpoint));
        match selected {
            Some(player) if local => format!(
                "no local audio · this speaker is {}",
                if player.state.is_empty() {
                    "idle"
                } else {
                    player.state.as_str()
                }
            ),
            Some(player) => format!("no local audio · playing on {}", player.name),
            None => "no local audio · select MA-TUI's own speaker".into(),
        }
    }

    /// Pasted text never becomes shortcuts, field navigation or submission.
    pub fn paste(&mut self, text: &str) {
        if let Some(settings) = &mut self.settings {
            settings.paste(text);
        } else if let Some(menu) = &mut self.menu {
            if let Some(prompt) = &mut menu.prompt {
                if !append_paste(&mut prompt.value, text, 2048) {
                    menu.error = "Paste exceeds this field's 2048-byte limit".into();
                }
            }
        } else if self.music.filtering {
            if !append_paste(&mut self.music.filter_input, text, 128) {
                self.status = "Paste exceeds the filter's 128-byte limit".into();
            }
        } else if self.editing && !append_paste(&mut self.query, text, 256) {
            self.status = "Paste exceeds the search field's 256-byte limit".into();
        }
    }

    pub fn key(&mut self, key: crossterm::event::KeyEvent) -> Action {
        use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::Quit;
        }
        if let Some(settings) = &mut self.settings {
            return settings.key(key);
        }
        if self.menu.is_some() {
            return crate::controls::key(self, key);
        }
        if self.editing {
            match key.code {
                KeyCode::Esc => self.editing = false,
                KeyCode::Enter => {
                    self.editing = false;
                    self.search_cursor = 0;
                    if !self.query.trim().is_empty() {
                        return Action::Search(self.query.trim().into());
                    }
                }
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) if !c.is_control() && self.query.len() < 256 => self.query.push(c),
                _ => {}
            }
            return Action::None;
        }
        if self.focus == Focus::Music {
            if let Some(action) = crate::music::key(self, key) {
                return action;
            }
        }
        if self.focus == Focus::Search
            && (matches!(
                key.code,
                KeyCode::Enter | KeyCode::Char('P') | KeyCode::Char('a') | KeyCode::Char('N')
            ) || (key.code == KeyCode::Char('f') && key.modifiers.is_empty()))
        {
            let media = self
                .results
                .get(self.search_cursor)
                .and_then(|track| track.media.clone());
            if key.code == KeyCode::Char('f') {
                return media
                    .as_ref()
                    .and_then(|media| media.favorite_action())
                    .unwrap_or_else(|| {
                        self.status = "This item cannot be a favourite".into();
                        Action::None
                    });
            }
            if let Some(media) = media {
                if key.code == KeyCode::Enter {
                    if let Some(target) = media.open.clone() {
                        self.focus = Focus::Music;
                        self.content = Focus::Music;
                        return self.music.navigate(target, media.title);
                    }
                }
                if matches!(key.code, KeyCode::Enter | KeyCode::Char('P')) {
                    return crate::music::choose(self, &media);
                }
                // The queue shortcuts still need a speaker of their own; the
                // menu can now open without one, for progress alone.
                if !self
                    .players
                    .iter()
                    .any(|p| Some(&p.id) == self.selected_id.as_ref() && p.available)
                {
                    self.status = "Select an available speaker first".into();
                    return Action::None;
                }
                crate::music::choose(self, &media);
                if self.menu.take().is_some() && media.playable && media.available {
                    return if key.code == KeyCode::Char('a') {
                        Action::Enqueue(media.uri)
                    } else {
                        Action::PlayNext(media.uri)
                    };
                }
                return Action::None;
            }
        }
        match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::F(3) | KeyCode::Char('b') => {
                self.focus = Focus::Music;
                self.content = Focus::Music;
                Action::None
            }
            KeyCode::F(4) => {
                self.focus = Focus::Queue;
                Action::None
            }
            KeyCode::F(2) => Action::OpenSettings,
            KeyCode::Char('?') | KeyCode::F(1) => {
                self.menu = Some(crate::controls::Menu::new(self));
                Action::None
            }
            KeyCode::Char('/') => {
                self.editing = true;
                self.focus = Focus::Search;
                self.content = Focus::Search;
                self.query.clear();
                Action::None
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if key.code == KeyCode::BackTab {
                    match self.focus {
                        Focus::Players => Focus::Search,
                        Focus::Music => Focus::Players,
                        Focus::Queue => Focus::Music,
                        Focus::Search => Focus::Queue,
                    }
                } else {
                    match self.focus {
                        Focus::Players => Focus::Music,
                        Focus::Music => Focus::Queue,
                        Focus::Queue => Focus::Search,
                        Focus::Search => Focus::Players,
                    }
                };
                // Music and search share the right pane; the queue has its own.
                if matches!(self.focus, Focus::Music | Focus::Search) {
                    self.content = self.focus;
                }
                Action::None
            }
            KeyCode::Esc => {
                match self.focus {
                    // Leave search results for the browser they came from.
                    Focus::Search => {
                        self.focus = Focus::Music;
                        self.content = Focus::Music;
                    }
                    Focus::Music if !self.music.history.is_empty() => self.music.back(),
                    _ => {}
                }
                Action::None
            }
            KeyCode::Down
            | KeyCode::Char('j')
            | KeyCode::Up
            | KeyCode::Char('k')
            | KeyCode::PageDown
            | KeyCode::PageUp
            | KeyCode::Home
            | KeyCode::End => {
                let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));
                let (cursor, len) = match self.focus {
                    Focus::Players => (&mut self.player_cursor, self.players.len()),
                    Focus::Queue => (&mut self.queue_cursor, self.queue.len()),
                    Focus::Search => (&mut self.search_cursor, self.results.len()),
                    Focus::Music => (&mut self.music.page.cursor, self.music.page.items.len()),
                };
                *cursor = if key.code == KeyCode::Home {
                    0
                } else if key.code == KeyCode::End {
                    len.saturating_sub(1)
                } else if key.code == KeyCode::PageDown {
                    cursor.saturating_add(10).min(len.saturating_sub(1))
                } else if key.code == KeyCode::PageUp {
                    cursor.saturating_sub(10)
                } else if down {
                    cursor.saturating_add(1).min(len.saturating_sub(1))
                } else {
                    cursor.saturating_sub(1)
                };
                Action::None
            }
            KeyCode::Enter if self.focus == Focus::Players && self.connected => {
                if let Some(p) = self.players.get(self.player_cursor).filter(|p| p.available) {
                    self.selected_id = Some(p.id.clone());
                    self.queue.clear();
                    self.queue_id.clear();
                    self.queue_details = serde_json::Value::Null;
                    self.title = "Loading queue…".into();
                    self.artist.clear();
                    self.elapsed = 0.0;
                    self.elapsed_at = None;
                    self.duration = 0.0;
                    self.focus = Focus::Music;
                    self.content = Focus::Music;
                    Action::Select(p.id.clone())
                } else {
                    Action::None
                }
            }
            KeyCode::Char('r') => Action::Refresh,
            KeyCode::Char('F') if self.connected => {
                let media = crate::music::Media::parse(
                    &self.queue_details["current_item"]["media_item"],
                    "track",
                );
                media.favorite_action().unwrap_or_else(|| {
                    self.status = "Nothing playing to favourite".into();
                    Action::None
                })
            }
            _ if !self.connected
                || !self
                    .players
                    .iter()
                    .any(|p| Some(&p.id) == self.selected_id.as_ref() && p.available) =>
            {
                Action::None
            }
            KeyCode::Char(' ') | KeyCode::Char('p') => Action::Toggle,
            KeyCode::Char('n') | KeyCode::Char('>') | KeyCode::Char('.') => Action::Next,
            KeyCode::Char('<') | KeyCode::Char(',') => Action::Previous,
            KeyCode::Char('s') => Action::Command(crate::controls::Command::Player {
                name: "stop",
                args: serde_json::json!({}),
            }),
            // z and l keep shuffle and repeat off any shifted pair.
            KeyCode::Char('z') => self.queue_mode(
                "shuffle",
                serde_json::json!({
                    "shuffle_enabled": self.queue_details["shuffle_enabled"] != true
                }),
            ),
            KeyCode::Char('l') => {
                // off → all → one → off, matching the controls menu's modes.
                let next = match self.queue_details["repeat_mode"].as_str() {
                    Some("all") => "one",
                    Some("one") => "off",
                    _ => "all",
                };
                self.queue_mode("repeat", serde_json::json!({ "repeat_mode": next }))
            }
            KeyCode::Char('m') => {
                let muted = self
                    .players
                    .iter()
                    .find(|p| Some(&p.id) == self.selected_id.as_ref())
                    .and_then(|p| p.details["volume_muted"].as_bool());
                muted
                    .map(|v| {
                        Action::Command(crate::controls::Command::Player {
                            name: "volume_mute",
                            args: serde_json::json!({"muted":!v}),
                        })
                    })
                    .unwrap_or(Action::None)
            }
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('-') => {
                Action::Command(crate::controls::Command::Player {
                    name: if key.code == KeyCode::Char('-') {
                        "volume_down"
                    } else {
                        "volume_up"
                    },
                    args: serde_json::json!({}),
                })
            }
            KeyCode::Left | KeyCode::Right => self.seek(if key.code == KeyCode::Left {
                -10.0
            } else {
                10.0
            }),
            KeyCode::Enter | KeyCode::Delete | KeyCode::Char('J') | KeyCode::Char('K')
                if self.focus == Focus::Queue =>
            {
                let Some(item) = self
                    .queue
                    .get(self.queue_cursor)
                    .filter(|t| !t.id.is_empty())
                else {
                    return Action::None;
                };
                let (name, args) = match key.code {
                    KeyCode::Enter => ("play_index", serde_json::json!({"index":item.id})),
                    KeyCode::Delete => (
                        "delete_item",
                        serde_json::json!({"item_id_or_index":item.id}),
                    ),
                    KeyCode::Char('J') => (
                        "move_item",
                        serde_json::json!({"queue_item_id":item.id,"pos_shift":1}),
                    ),
                    _ => (
                        "move_item",
                        serde_json::json!({"queue_item_id":item.id,"pos_shift":-1}),
                    ),
                };
                Action::Command(crate::controls::Command::Queue {
                    id: self.queue_id.clone(),
                    name,
                    args,
                })
            }
            KeyCode::Enter | KeyCode::Char('a') if self.focus == Focus::Search => {
                if let Some(t) = self.results.get(self.search_cursor) {
                    if key.code == KeyCode::Enter {
                        Action::Play(t.uri.clone())
                    } else {
                        Action::Enqueue(t.uri.clone())
                    }
                } else {
                    Action::None
                }
            }
            _ => Action::None,
        }
    }
}

/// Single-line inputs exclude terminal control characters. Reject oversized
/// pastes atomically rather than silently truncating a URL or credential.
pub(crate) fn append_paste(value: &mut String, text: &str, limit: usize) -> bool {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    if text.len() > limit.saturating_sub(value.len()) {
        return false;
    }
    value.push_str(&text);
    true
}

#[derive(Clone, Default)]
pub struct PlayerView {
    pub details: serde_json::Value,
    pub id: String,
    pub name: String,
    pub state: String,
    pub available: bool,
    pub volume: Option<u8>,
}

#[derive(Clone, Default)]
pub struct TrackView {
    pub media: Option<crate::music::Media>,
    pub id: String,
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub duration: f64,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Players,
    Music,
    Queue,
    Search,
}

pub struct App {
    pub music: crate::music::Browser,
    /// Decoded local samples, when MA-TUI itself is a speaker this run.
    pub spectrum: Option<crate::visualizer::Analyzer>,
    pub visualizer: crate::visualizer::Meter,
    /// Persistent identity of MA-TUI's own endpoint, for explaining an empty
    /// visualizer when a different speaker is selected.
    pub local_endpoint: Option<String>,
    pub content: Focus,
    pub menu: Option<crate::controls::Menu>,
    pub queue_id: String,
    pub queue_details: serde_json::Value,
    pub palette: Palette,
    pub settings: Option<crate::settings::Settings>,
    pub exit: bool,
    pub players: Vec<PlayerView>,
    pub queue: Vec<TrackView>,
    pub results: Vec<TrackView>,
    pub selected_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub elapsed: f64,
    /// When `elapsed` was last set, so the position can be carried forward
    /// between snapshots. `None` means there is nothing to carry.
    pub elapsed_at: Option<std::time::Instant>,
    pub duration: f64,
    pub status: String,
    pub audio_status: String,
    pub focus: Focus,
    pub player_cursor: usize,
    pub queue_cursor: usize,
    pub search_cursor: usize,
    pub editing: bool,
    pub query: String,
    pub demo: bool,
    pub connected: bool,
    /// Whether the server is pushing changes rather than being asked for them.
    pub live: bool,
    /// Steps of marquee motion, advanced by the render loop rather than read
    /// from the clock here, so drawing stays a function of state.
    pub tick: u64,
    /// Set while drawing when something on screen is mid-scroll, so the loop
    /// knows this frame is not the final one.
    pub scrolling: bool,
    /// The cover for what is playing, when there is one and it is wanted.
    pub artwork: Option<crate::artwork::Art>,
    /// Bumped whenever the cover changes, so a renderer that writes outside the
    /// cell grid knows when what it drew is stale.
    pub artwork_generation: u64,
    /// Draw covers as sixel rather than half blocks.
    pub sixel: bool,
    /// Where the cover was laid out this frame, for a renderer that has to
    /// write into it after the cells have been flushed.
    pub artwork_area: Option<Rect>,
    /// How the spectrum is drawn, from configuration.
    pub spectrum_style: crate::config::Spectrum,
    /// Set while drawing when the spectrum is on screen. It is driven by the
    /// audio rather than by state changes, so it needs frames of its own; the
    /// rest of the interface still costs nothing when nothing has changed.
    pub animating: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            music: crate::music::Browser::default(),
            spectrum: None,
            visualizer: crate::visualizer::Meter::default(),
            local_endpoint: None,
            content: Focus::Music,
            menu: None,
            queue_id: String::new(),
            queue_details: serde_json::Value::Null,
            palette: Palette::default(),
            settings: None,
            exit: false,
            players: vec![],
            queue: vec![],
            results: vec![],
            selected_id: None,
            title: "No player selected".into(),
            artist: "Select a player and press Enter".into(),
            elapsed: 0.0,
            elapsed_at: None,
            duration: 0.0,
            status: "Disconnected".into(),
            audio_status: "Local audio disabled".into(),
            focus: Focus::Players,
            player_cursor: 0,
            queue_cursor: 0,
            search_cursor: 0,
            editing: false,
            query: String::new(),
            demo: false,
            connected: false,
            live: false,
            tick: 0,
            artwork: None,
            artwork_generation: 0,
            sixel: false,
            artwork_area: None,
            spectrum_style: crate::config::Spectrum::default(),
            scrolling: false,
            animating: false,
        }
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let palette = app.palette;
    if let Some(settings) = &app.settings {
        settings.draw(frame, palette);
        return;
    }
    let area = frame.area();
    frame.render_widget(
        Block::default().style(
            Style::default()
                .bg(palette.background)
                .fg(palette.foreground),
        ),
        area,
    );
    if area.width < 50 || area.height < 16 {
        frame.render_widget(
            Paragraph::new("MA-TUI\nResize to 50 x 16\nq: quit")
                .style(Style::default().fg(palette.accent)),
            area,
        );
        return;
    }
    app.scrolling = false;
    app.artwork_area = None;
    // Chrome is two header rows, the player, two for status and two for hints;
    // everything else belongs to the lists. The player carries the spectrum
    // when this run has local audio and the terminal can spare the rows.
    let strip = strip_rows(app, area.height);
    // A cover needs rows of its own: it must not depend on the spectrum being
    // there, and four rows of player would leave it too small to recognise.
    let cover = if app.artwork.is_some() && area.height >= 24 {
        10
    } else {
        0
    };
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length((4 + strip).max(cover)),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(2),
    ])
    .split(area);
    let mode = if app.demo {
        "OFFLINE DEMO · sample data"
    } else {
        "MUSIC ASSISTANT · LOCAL + REMOTE"
    };
    // Say which way state is arriving, so a stream that quietly fell back to
    // polling is visible rather than indistinguishable.
    let feed = match (app.demo, app.connected, app.live) {
        (true, ..) => String::new(),
        (_, true, true) => "  ·  live".into(),
        (_, true, false) => "  ·  polling".into(),
        _ => String::new(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                concat!("  MA-TUI v", env!("CARGO_PKG_VERSION"), "  "),
                Style::default()
                    .fg(palette.background)
                    .bg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {mode}")),
            Span::styled(feed, Style::default().fg(palette.secondary)),
        ]))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(palette.secondary)),
        ),
        rows[0],
    );
    draw_now_playing(frame, app, rows[1], strip);
    rule(frame, rows[2], palette);
    rule(frame, rows[4], palette);

    // A divider between the columns rather than a box around each: the panes
    // are separated, but nothing is fenced in.
    let cols = Layout::horizontal([
        Constraint::Percentage(35),
        Constraint::Length(3),
        Constraint::Min(10),
    ])
    .split(rows[3]);
    divider(frame, cols[1], palette);
    let cols = [cols[0], cols[2]];
    // Players are few and short; the queue takes the rest of the column so it
    // stays visible while browsing.
    let listed = (app.players.len() * 2 + 1) as u16;
    let side = Layout::vertical([
        Constraint::Length(listed.clamp(3, (cols[0].height / 2).max(3))),
        Constraint::Length(1),
        Constraint::Min(3),
    ])
    .split(cols[0]);
    draw_players(frame, app, side[0]);
    rule(frame, side[1], palette);
    draw_queue(frame, app, side[2]);
    if app.menu.is_some() {
        crate::controls::draw(frame, app, cols[1]);
    } else if app.content == Focus::Search || app.focus == Focus::Search || app.editing {
        draw_search(frame, app, cols[1]);
    } else {
        crate::music::draw(frame, app, cols[1]);
    }

    let message = if app.editing {
        format!("Search: {}▏  [Enter: submit · Esc: cancel]", app.query)
    } else {
        format!("{}\n{}", app.status, app.audio_status)
    };
    frame.render_widget(Paragraph::new(message), rows[5]);
    frame.render_widget(
        Paragraph::new(format!("{}\n{}", hints(app), TRANSPORT_HINTS))
            .style(Style::default().fg(palette.secondary)),
        rows[6],
    );
}

/// Keys for the focused pane. The transport line below it never changes.
fn hints(app: &App) -> &'static str {
    if app.menu.is_some() {
        return "Enter applies · / filter · Esc returns · the player keeps running";
    }
    if app.music.filtering {
        return "Enter applies filter · Esc cancels";
    }
    if app.editing {
        return "Enter submits the search · Esc cancels";
    }
    match app.focus {
        Focus::Players => "Enter select speaker · ↑↓ move · Tab pane · b music · / search · F2 settings",
        Focus::Queue => "Enter play item · Delete remove · Shift-J/K move · Tab pane · b music",
        Focus::Music => {
            "Enter open · P play · a add · N next · f fav · Backspace · r reload · [ ] page · o sort · ^F filter"
        }
        Focus::Search => "Enter play · a add · N play next · f fav · / new search · Esc back to music",
    }
}

// Kept to 102 columns so it survives a narrow terminal; everything else lives
// in the controls menu.
const TRANSPORT_HINTS: &str =
    "Space/p pause · </> track · s stop · +/- vol · m mute · z shuffle · l repeat · ? all keys";

/// Rows the spectrum strip takes as part of the player, or none when this run
/// has no local audio to analyze or the terminal is too short to spare them.
fn strip_rows(app: &App, height: u16) -> u16 {
    if app.spectrum.is_some() && height >= 26 {
        5
    } else {
        0
    }
}

/// The player: what is on, what it is doing, and what it sounds like.
fn draw_now_playing(frame: &mut Frame, app: &mut App, area: Rect, strip: u16) {
    let palette = app.palette;
    let dim = Style::default().fg(palette.secondary);
    let player = app
        .players
        .iter()
        .find(|p| Some(&p.id) == app.selected_id.as_ref());
    // A cell is about twice as tall as it is wide and carries two pixels, so a
    // square cover wants twice as many columns as rows.
    let area = match app.artwork.as_ref().filter(|_| area.height >= 6) {
        Some(art) => {
            let columns = (area.height * 2).min(area.width / 3);
            let split =
                Layout::horizontal([Constraint::Length(columns), Constraint::Min(20)]).spacing(2);
            let parts = split.split(area);
            if app.sixel {
                // Sixel is written after the cells are flushed, so the region
                // is only claimed here: left blank so the cell renderer has
                // nothing to put back over the image on a later frame.
                app.artwork_area = Some(parts[0]);
            } else {
                frame.render_widget(
                    Paragraph::new(art.half_blocks(parts[0].width, parts[0].height)),
                    parts[0],
                );
            }
            parts[1]
        }
        None => area,
    };
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(strip),
        Constraint::Length(1),
    ])
    .split(area);

    // A label row: where it is playing, and how the queue is ordered.
    let mut order: Vec<String> = Vec::new();
    if !app.queue_id.is_empty() && app.queue_details["is_dynamic"] != true {
        order.push(format!(
            "SHUFFLE {}",
            on_off(app.queue_details["shuffle_enabled"] == true)
        ));
        order.push(
            match app.queue_details["repeat_mode"].as_str() {
                Some("all") => "REPEAT ALL",
                Some("one") => "REPEAT ONE",
                _ => "REPEAT OFF",
            }
            .into(),
        );
    }
    let where_playing = match player {
        Some(p) if !p.available => format!("NOW PLAYING · {} · UNAVAILABLE", p.name.to_uppercase()),
        Some(p) => format!("NOW PLAYING · {}", p.name.to_uppercase()),
        None => "NO SPEAKER SELECTED".into(),
    };
    let labels = Layout::horizontal([Constraint::Min(10), Constraint::Length(24)]).split(rows[0]);
    frame.render_widget(Paragraph::new(where_playing).style(dim), labels[0]);
    frame.render_widget(
        Paragraph::new(order.join(" · "))
            .style(dim)
            .alignment(ratatui::layout::Alignment::Right),
        labels[1],
    );

    // The title carries the most weight on screen, so it scrolls rather than
    // being cut off.
    let (title, scrolling) = marquee(&app.title, rows[1].width as usize, app.tick);
    app.scrolling |= scrolling;
    frame.render_widget(
        Paragraph::new(title).style(Style::default().add_modifier(Modifier::BOLD)),
        rows[1],
    );

    let second = Layout::horizontal([Constraint::Min(10), Constraint::Length(16)]).split(rows[2]);
    frame.render_widget(Paragraph::new(app.artist.clone()).style(dim), second[0]);
    frame.render_widget(
        Paragraph::new(format!(
            "{} / {}",
            duration(app.elapsed),
            duration(app.duration)
        ))
        .style(dim)
        .alignment(ratatui::layout::Alignment::Right),
        second[1],
    );

    if strip > 0 {
        let reason = spectrum(app, rows[3].width);
        // Bars follow the audio, not the state of the interface, so this frame
        // is never the last one while they are on screen.
        app.animating |= reason.is_none();
        crate::visualizer::render(
            frame,
            rows[3],
            palette,
            &app.visualizer,
            reason.as_deref(),
            app.spectrum_style,
        );
    }
    draw_transport(frame, app, rows[4]);
}

fn on_off(value: bool) -> &'static str {
    if value {
        "ON"
    } else {
        "OFF"
    }
}

/// Transport controls, the position within the item, and the volume. All four
/// controls are always shown; the one matching the current state is filled, so
/// the row reads as state rather than only as buttons.
fn draw_transport(frame: &mut Frame, app: &App, area: Rect) {
    let palette = app.palette;
    let player = app
        .players
        .iter()
        .find(|p| Some(&p.id) == app.selected_id.as_ref());
    let state = player.map_or("", |p| p.state.as_str());
    let columns = Layout::horizontal([
        Constraint::Length(12),
        Constraint::Min(10),
        Constraint::Length(12),
    ])
    .split(area);

    let dim = Style::default().fg(palette.secondary);
    // Nothing here is clickable, so buttons would be decoration. The state is
    // what the row is actually for, and it is said rather than drawn.
    let label = match state {
        "playing" => "PLAYING",
        "paused" => "PAUSED",
        "" => "IDLE",
        _ => "STOPPED",
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{} ", transport(state)),
                Style::default().fg(palette.accent),
            ),
            Span::styled(
                label,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        columns[0],
    );
    frame.render_widget(
        Gauge::default()
            .ratio(progress(app))
            .gauge_style(Style::default().fg(palette.accent).bg(palette.selection))
            .label(""),
        columns[1],
    );
    let volume = match player {
        Some(p) if p.details["volume_muted"] == true => "  MUTED".into(),
        Some(p) => p.volume.map_or("  VOL —".into(), |v| format!("  VOL {v}")),
        None => String::new(),
    };
    frame.render_widget(Paragraph::new(volume).style(dim), columns[2]);
}

fn draw_players(frame: &mut Frame, app: &App, area: Rect) {
    let palette = app.palette;
    let area = heading(frame, area, palette, "PLAYERS", app.focus == Focus::Players);
    let items: Vec<ListItem> = app
        .players
        .iter()
        .map(|p| {
            let selected = app.selected_id.as_deref() == Some(p.id.as_str());
            ListItem::new(vec![
                Line::from(format!("{} {}", if selected { "▶" } else { " " }, p.name)),
                Line::styled(
                    format!(
                        "  {} · {}",
                        if p.available {
                            p.state.as_str()
                        } else {
                            "unavailable"
                        },
                        p.volume.map_or("volume —".into(), |v| format!("vol {v}%"))
                    ),
                    Style::default().fg(palette.secondary),
                ),
            ])
        })
        .collect();
    let mut state = ListState::default().with_selected(
        (!app.players.is_empty())
            .then_some(app.player_cursor.min(app.players.len().saturating_sub(1))),
    );
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(Style::default().bg(palette.selection).fg(palette.accent))
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

/// Track rows: a number, the title over its artist, and a right-aligned state
/// column carrying either what the item is doing or how long it runs.
fn track_items<'a>(
    tracks: &'a [TrackView],
    palette: Palette,
    numbered: bool,
    playing: &str,
    width: u16,
) -> Vec<ListItem<'a>> {
    // The cursor takes two columns, the number four, the state column eight.
    let state_width = 8usize;
    let title_width =
        (width as usize).saturating_sub(2 + if numbered { 4 } else { 0 } + state_width);
    tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let position = if numbered {
                format!("{:02}  ", index + 1)
            } else {
                String::new()
            };
            let state = if !playing.is_empty() && track.id == playing {
                "PLAYING".into()
            } else if track.duration > 0.0 {
                duration(track.duration)
            } else {
                String::new()
            };
            let marker = if track.media.as_ref().is_some_and(|media| media.favorite) {
                " ♥"
            } else {
                ""
            };
            let title_text_width = title_width.saturating_sub(marker.chars().count());
            let title = truncate(&track.title, title_text_width);
            ListItem::new(vec![
                Line::from(vec![
                    Span::raw(format!("{position}{title:<title_text_width$}{marker}")),
                    Span::styled(
                        format!("{state:>state_width$}"),
                        Style::default().fg(if state == "PLAYING" {
                            palette.accent
                        } else {
                            palette.secondary
                        }),
                    ),
                ]),
                Line::styled(
                    format!(
                        "{}{}",
                        " ".repeat(position.len()),
                        truncate(&track.artist, title_width)
                    ),
                    Style::default().fg(palette.secondary),
                ),
            ])
        })
        .collect()
}

fn truncate(text: &str, width: usize) -> String {
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= width {
        return text.to_owned();
    }
    characters
        .into_iter()
        .take(width.saturating_sub(1))
        .chain(['…'])
        .collect()
}

fn draw_list(
    frame: &mut Frame,
    area: Rect,
    items: Vec<ListItem>,
    cursor: usize,
    palette: Palette,
    empty: &str,
) {
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(empty).style(Style::default().fg(palette.secondary)),
            area,
        );
        return;
    }
    let mut state =
        ListState::default().with_selected(Some(cursor.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(Style::default().bg(palette.selection))
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

/// Column headings for a track table, so the state column is readable as one.
fn columns_header(frame: &mut Frame, area: Rect, palette: Palette, numbered: bool) -> Rect {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let lead = if numbered { "  #   TITLE" } else { "  TITLE" };
    let width = rows[0].width as usize;
    let heading = format!("{lead:<0$}{1:>8}", width.saturating_sub(8), "STATE");
    frame.render_widget(
        Paragraph::new(heading).style(Style::default().fg(palette.secondary)),
        rows[0],
    );
    rows[1]
}

fn draw_queue(frame: &mut Frame, app: &App, area: Rect) {
    let palette = app.palette;
    let label = format!("QUEUE · {}", count(app.queue.len(), "item").to_uppercase());
    let area = heading(frame, area, palette, &label, app.focus == Focus::Queue);
    let playing = app.queue_details["current_item"]["queue_item_id"]
        .as_str()
        .unwrap_or_default();
    let body = columns_header(frame, area, palette, true);
    draw_list(
        frame,
        body,
        track_items(&app.queue, palette, true, playing, body.width),
        app.queue_cursor,
        palette,
        "  Queue is empty or not loaded",
    );
}

fn draw_search(frame: &mut Frame, app: &App, area: Rect) {
    let palette = app.palette;
    let label = format!(
        "SEARCH · {}",
        count(app.results.len(), "result").to_uppercase()
    );
    let area = heading(
        frame,
        area,
        palette,
        &label,
        app.focus == Focus::Search || app.editing,
    );
    draw_list(
        frame,
        area,
        track_items(&app.results, palette, false, "", area.width),
        app.search_cursor,
        palette,
        "  / search for tracks across providers",
    );
}

fn progress(app: &App) -> f64 {
    if app.duration > 0.0 && app.elapsed.is_finite() && app.duration.is_finite() {
        (app.elapsed / app.duration).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn transport(state: &str) -> &'static str {
    match state {
        "playing" => "▶",
        "paused" => "⏸",
        _ => "■",
    }
}

/// Advance the visualizer for this frame and report why it is empty when it
/// is. Bars are only ever drawn from decoded samples this process is playing.
fn spectrum(app: &mut App, width: u16) -> Option<String> {
    let (bars, _, _) = crate::visualizer::columns(width, app.spectrum_style);
    let now = std::time::Instant::now();
    let captured = app.spectrum.as_ref().map(|analyzer| analyzer.capture(now));
    app.visualizer.update(
        match captured {
            Some(Ok(bands)) => Some(bands),
            _ => None,
        },
        bars,
        now,
    );
    match captured {
        Some(Ok(_)) => None,
        Some(Err("no local audio")) => Some(app.silence()),
        Some(Err(reason)) => Some(reason.into()),
        None => Some("local audio is off · MA-TUI is not a speaker this run".into()),
    }
}

/// A horizontal rule separating one band of the interface from the next.
fn rule(frame: &mut Frame, area: Rect, palette: Palette) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new("─".repeat(area.width as usize))
            .style(Style::default().fg(palette.secondary)),
        area,
    );
}

/// A vertical rule between two columns.
fn divider(frame: &mut Frame, area: Rect, palette: Palette) {
    let middle = Rect {
        x: area.x + area.width / 2,
        width: 1.min(area.width),
        ..area
    };
    frame.render_widget(
        Paragraph::new(vec![Line::raw("│"); area.height as usize])
            .style(Style::default().fg(palette.secondary)),
        middle,
    );
}

/// Draw a pane's heading and return the area left for its content. There are no
/// boxes anywhere in this interface: a dim label and the space around it do the
/// separating that borders used to, which is quieter and gives back two columns
/// and two rows per pane.
pub(crate) fn heading(
    frame: &mut Frame,
    area: Rect,
    palette: Palette,
    label: &str,
    active: bool,
) -> Rect {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    // A cell grid has one type size, so the focused pane is marked by filling
    // its heading rather than enlarging it: a bar of colour is findable at a
    // glance in a way a colour change alone is not.
    let style = if active {
        Style::default()
            .fg(palette.background)
            .bg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(palette.secondary)
            .add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(Line::styled(format!(" {label} "), style)),
        rows[0],
    );
    rows[1]
}

/// Scroll text too wide for its column, holding at each end long enough to read
/// it. Returns whether it is mid-scroll so the loop keeps drawing.
pub fn marquee(text: &str, width: usize, step: u64) -> (String, bool) {
    let characters: Vec<char> = text.chars().collect();
    if width == 0 || characters.len() <= width {
        return (text.to_owned(), false);
    }
    const HOLD: u64 = 10;
    let travel = (characters.len() - width) as u64;
    let phase = step % (travel + HOLD * 2);
    let offset = phase.saturating_sub(HOLD).min(travel) as usize;
    (
        characters[offset..offset + width].iter().collect(),
        phase < travel + HOLD * 2,
    )
}

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

pub fn duration(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0) as u64
    } else {
        0
    };
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
