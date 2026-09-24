//! Playback controls only: no provider, user, library-management or server-admin commands.
use crate::ui::{Action, App};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Player {
        name: &'static str,
        args: Value,
    },
    Queue {
        id: String,
        name: &'static str,
        args: Value,
    },
    Transfer {
        source: String,
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub target: PromptTarget,
    pub value: String,
    pub kind: InputKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PromptTarget {
    Command {
        command: Command,
        argument: &'static str,
    },
    CreatePlaylist {
        uri: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputKind {
    Text,
    Number { min: f64, max: f64, integer: bool },
}

impl Prompt {
    fn needs_player(&self) -> bool {
        matches!(&self.target, PromptTarget::Command { .. })
    }

    pub fn submit(&self) -> Result<Action, &'static str> {
        match &self.target {
            PromptTarget::CreatePlaylist { uri } => {
                let name = self.value.trim();
                if name.is_empty() {
                    return Err("Enter a value");
                }
                if name.chars().count() > 200 {
                    return Err("Playlist names cannot exceed 200 characters");
                }
                Ok(Action::CreatePlaylist {
                    name: name.into(),
                    uri: uri.clone(),
                })
            }
            PromptTarget::Command { command, argument } => {
                let value = match self.kind {
                    InputKind::Text if self.value.trim().is_empty() => {
                        return Err("Enter a value");
                    }
                    InputKind::Text => json!(self.value.trim()),
                    InputKind::Number { min, max, integer } => {
                        let n: f64 = self
                            .value
                            .trim()
                            .parse()
                            .map_err(|_| "Enter a number within the displayed range")?;
                        if !n.is_finite() || n < min || n > max || (integer && n.fract() != 0.0) {
                            return Err("Enter a number within the displayed range");
                        }
                        if integer {
                            json!(n as i64)
                        } else {
                            json!(n)
                        }
                    }
                };
                let mut command = command.clone();
                match &mut command {
                    Command::Player { args, .. } | Command::Queue { args, .. } => {
                        args[*argument] = value
                    }
                    _ => return Err("Invalid input action"),
                }
                Ok(Action::Command(command))
            }
        }
    }
}

/// One selectable action, grouped under a heading so a long menu stays
/// readable.
pub struct Entry {
    pub section: &'static str,
    pub label: String,
    pub action: Action,
}

/// Headings in the order they are shown; entries keep their order within one.
const SECTIONS: [&str; 10] = [
    PLAYBACK, VOLUME, QUEUE, GROUPING, SOURCES, OPTIONS, SLEEP, MEDIA, PLAYLISTS, NEW,
];
const PLAYBACK: &str = "Playback";
const VOLUME: &str = "Volume";
const QUEUE: &str = "Queue";
const GROUPING: &str = "Grouping";
const SOURCES: &str = "Sources and sound modes";
const OPTIONS: &str = "Player options";
const SLEEP: &str = "Sleep timer";
const MEDIA: &str = "Media by URI";
const PLAYLISTS: &str = "Playlists";
const NEW: &str = "New";

pub struct Menu {
    pub prompt: Option<Prompt>,
    pub error: String,
    pub player: Option<String>,
    pub title: String,
    pub entries: Vec<Entry>,
    /// Index into the entries the filter leaves visible, never into `entries`.
    pub cursor: usize,
    pub filter: String,
    pub filtering: bool,
}

impl Menu {
    /// Entry indices matching the filter, case-insensitively.
    pub fn visible(&self) -> Vec<usize> {
        if self.filter.trim().is_empty() {
            return (0..self.entries.len()).collect();
        }
        let needle = self.filter.trim().to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.label.to_lowercase().contains(&needle)
                    || entry.section.to_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// The action the cursor is on, if the filter leaves one there.
    pub fn selected(&self) -> Option<&Entry> {
        self.visible()
            .get(self.cursor)
            .and_then(|index| self.entries.get(*index))
    }

    fn add(&mut self, section: &'static str, label: String, action: Action) {
        self.entries.push(Entry {
            section,
            label,
            action,
        });
    }

    fn player_command(
        &mut self,
        section: &'static str,
        label: String,
        name: &'static str,
        args: Value,
    ) {
        self.add(
            section,
            label,
            Action::Command(Command::Player { name, args }),
        );
    }

    fn queue_command(
        &mut self,
        section: &'static str,
        id: &str,
        label: String,
        name: &'static str,
        args: Value,
    ) {
        self.add(
            section,
            label,
            Action::Command(Command::Queue {
                id: id.to_owned(),
                name,
                args,
            }),
        );
    }
}

impl Menu {
    pub fn new(app: &App) -> Self {
        let mut menu = Self {
            prompt: None,
            error: String::new(),
            player: app.selected_id.clone(),
            title: "Select a connected player first".into(),
            entries: vec![],
            cursor: 0,
            filter: String::new(),
            filtering: false,
        };
        let Some(player) = app
            .players
            .iter()
            .find(|p| Some(&p.id) == app.selected_id.as_ref() && p.available && app.connected)
        else {
            return menu;
        };
        menu.title = format!("Controls · {}", player.name);
        let p = &player.details;
        for (label, name) in [
            ("Play / resume", "play"),
            ("Pause", "pause"),
            ("Stop", "stop"),
            ("Next track", "next"),
            ("Previous track", "previous"),
        ] {
            menu.player_command(PLAYBACK, label.into(), name, json!({}));
        }
        if let Some(powered) = p["powered"].as_bool() {
            menu.player_command(
                PLAYBACK,
                if powered { "Power off" } else { "Power on" }.into(),
                "power",
                json!({"powered":!powered}),
            );
        }
        if let Some(muted) = p["volume_muted"].as_bool() {
            menu.player_command(
                VOLUME,
                if muted { "Unmute" } else { "Mute" }.into(),
                "volume_mute",
                json!({"muted":!muted}),
            );
        }
        for (label, name) in [
            ("Volume up", "volume_up"),
            ("Volume down", "volume_down"),
            ("Group volume up", "group_volume_up"),
            ("Group volume down", "group_volume_down"),
        ] {
            menu.player_command(VOLUME, label.into(), name, json!({}));
        }
        if let Some(muted) = p["group_volume_muted"].as_bool() {
            menu.player_command(
                VOLUME,
                if muted { "Unmute group" } else { "Mute group" }.into(),
                "group_volume_mute",
                json!({"muted":!muted}),
            );
        }
        menu.player_command(GROUPING, "Leave player group".into(), "ungroup", json!({}));
        for target in app
            .players
            .iter()
            .filter(|other| other.available && other.id != player.id)
        {
            if p["can_group_with"]
                .as_array()
                .is_some_and(|ids| ids.contains(&json!(target.id)))
            {
                menu.player_command(
                    GROUPING,
                    format!("Join group led by {}", target.name),
                    "group",
                    json!({"target_player":target.id}),
                );
            }
            if p["group_members"]
                .as_array()
                .is_some_and(|ids| ids.contains(&json!(target.id)))
            {
                menu.player_command(
                    GROUPING,
                    format!("Remove {} from this group", target.name),
                    "set_members",
                    json!({"player_ids_to_remove":[target.id]}),
                );
            }
        }
        for (field, name, argument, label) in [
            ("source_list", "select_source", "source", "Source"),
            (
                "sound_mode_list",
                "select_sound_mode",
                "sound_mode",
                "Sound mode",
            ),
        ] {
            for item in p[field]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["passive"] != true)
            {
                if let Some(id) = item["id"].as_str() {
                    menu.player_command(
                        SOURCES,
                        format!("{label}: {}", clean(&item["name"])),
                        name,
                        json!({argument:id}),
                    );
                }
            }
        }
        for option in p["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["read_only"] != true)
        {
            let Some(key) = option["key"].as_str() else {
                continue;
            };
            let label = clean(&option["name"]);
            if let Some(value) = option["value"].as_bool() {
                menu.player_command(
                    OPTIONS,
                    format!("{label}: {}", !value),
                    "set_option",
                    json!({"option_key":key,"option_value":!value}),
                );
            } else if let Some(options) = option["options"].as_array() {
                for choice in options {
                    menu.player_command(
                        OPTIONS,
                        format!("{label}: {}", clean(&choice["name"])),
                        "set_option",
                        json!({"option_key":key,"option_value":choice["value"]}),
                    );
                }
            } else if let Some(value) = option["value"].as_f64() {
                let step = option["step"].as_f64().unwrap_or(1.0);
                for delta in [-step, step] {
                    let next = value + delta;
                    if !next.is_finite()
                        || option["min_value"].as_f64().is_some_and(|v| next < v)
                        || option["max_value"].as_f64().is_some_and(|v| next > v)
                    {
                        continue;
                    }
                    let next = if option["type"] == "integer" {
                        json!(next as i64)
                    } else {
                        json!(next)
                    };
                    menu.player_command(
                        OPTIONS,
                        format!("{label}: {next}"),
                        "set_option",
                        json!({"option_key":key,"option_value":next}),
                    );
                }
            }
        }
        for minutes in [15, 30, 60, 90] {
            menu.player_command(
                SLEEP,
                format!("Sleep timer: {minutes} minutes"),
                "sleep_timer/set",
                json!({"seconds":minutes*60}),
            );
        }
        menu.player_command(
            SLEEP,
            "Cancel sleep timer".into(),
            "sleep_timer/clear",
            json!({}),
        );
        if !app.queue_id.is_empty() {
            let q = &app.queue_details;
            let id = app.queue_id.as_str();
            if q["is_dynamic"] != true {
                let shuffle = q["shuffle_enabled"].as_bool().unwrap_or(false);
                menu.queue_command(
                    QUEUE,
                    id,
                    format!("Shuffle: {}", if shuffle { "off" } else { "on" }),
                    "shuffle",
                    json!({"shuffle_enabled":!shuffle}),
                );
                for mode in ["off", "one", "all"] {
                    menu.queue_command(
                        QUEUE,
                        id,
                        format!("Repeat: {mode}"),
                        "repeat",
                        json!({"repeat_mode":mode}),
                    );
                }
            }
            for (label, field, name) in [
                ("Autoplay", "autoplay_enabled", "autoplay"),
                ("Crossfade", "crossfade_enabled", "crossfade"),
            ] {
                if let Some(value) = q[field].as_bool() {
                    menu.queue_command(
                        QUEUE,
                        id,
                        format!("{label}: {}", if value { "off" } else { "on" }),
                        name,
                        json!({field:!value}),
                    );
                }
            }
            menu.queue_command(
                QUEUE,
                id,
                "Clear queue and stop playback".into(),
                "clear",
                json!({}),
            );
            if !app.queue.is_empty() {
                menu.input(
                    QUEUE,
                    "Save queue as playlist…".into(),
                    Command::Queue {
                        id: app.queue_id.clone(),
                        name: "save_as_playlist",
                        args: json!({}),
                    },
                    "name",
                    InputKind::Text,
                );
            }
            if let Some(item) = app.queue.get(app.queue_cursor).filter(|t| !t.id.is_empty()) {
                menu.queue_command(
                    QUEUE,
                    id,
                    format!("Play queue item: {}", item.title),
                    "play_index",
                    json!({"index":item.id}),
                );
                menu.queue_command(
                    QUEUE,
                    id,
                    format!("Remove queue item: {}", item.title),
                    "delete_item",
                    json!({"item_id_or_index":item.id}),
                );
                for (label, shift) in [("Move up", -1), ("Move down", 1), ("Move to next", 0)] {
                    menu.queue_command(
                        QUEUE,
                        id,
                        format!("{label}: {}", item.title),
                        "move_item",
                        json!({"queue_item_id":item.id,"pos_shift":shift}),
                    );
                }
                menu.queue_command(
                    QUEUE,
                    id,
                    format!("Move to end: {}", item.title),
                    "move_item_end",
                    json!({"queue_item_id":item.id}),
                );
            }
            if let Some(item) = app.results.get(app.search_cursor) {
                for (label, option) in [
                    ("Play now (replace)", "replace"),
                    ("Play next", "next"),
                    ("Add to queue", "add"),
                    ("Play immediately, keep queue", "play"),
                ] {
                    menu.queue_command(
                        QUEUE,
                        id,
                        format!("{label}: {}", item.title),
                        "play_media",
                        json!({"media":item.uri,"option":option}),
                    );
                }
            }
            if matches!(
                q["current_item"]["media_item"]["media_type"].as_str(),
                Some("audiobook" | "podcast_episode")
            ) {
                for speed in [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
                    menu.queue_command(
                        QUEUE,
                        id,
                        format!("Playback speed: {speed}x"),
                        "set_playback_speed",
                        json!({"speed":speed}),
                    );
                }
            }
            for target in app
                .players
                .iter()
                .filter(|p| p.available && Some(&p.id) != app.selected_id.as_ref())
            {
                menu.add(
                    QUEUE,
                    format!(
                        "Transfer queue/playback to {} (replaces destination)",
                        target.name
                    ),
                    Action::Command(Command::Transfer {
                        source: app.queue_id.clone(),
                        target: target.id.clone(),
                    }),
                );
            }
        }
        for (section, label, name, argument, min, max) in [
            (
                VOLUME,
                "Set volume (0–100)",
                "volume_set",
                "volume_level",
                0.0,
                100.0,
            ),
            (
                VOLUME,
                "Set group volume (0–100)",
                "group_volume",
                "volume_level",
                0.0,
                100.0,
            ),
            (
                PLAYBACK,
                "Seek to seconds (0–86400)",
                "seek",
                "position",
                0.0,
                86400.0,
            ),
            (
                SLEEP,
                "Sleep timer in seconds (1–86400)",
                "sleep_timer/set",
                "seconds",
                1.0,
                86400.0,
            ),
        ] {
            menu.input(
                section,
                label.into(),
                Command::Player {
                    name,
                    args: json!({}),
                },
                argument,
                InputKind::Number {
                    min,
                    max,
                    integer: true,
                },
            );
        }
        for option in p["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["read_only"] != true && !v["options"].is_array())
        {
            if option["type"] == "string" {
                if let Some(key) = option["key"].as_str() {
                    menu.input(
                        OPTIONS,
                        format!("Set {}", clean(&option["name"])),
                        Command::Player {
                            name: "set_option",
                            args: json!({"option_key":key}),
                        },
                        "option_value",
                        InputKind::Text,
                    );
                }
            }
        }
        if !app.queue_id.is_empty() {
            for (label, option) in [
                ("Play media URI (replaces queue)", "replace"),
                ("Add media URI to queue", "add"),
                ("Play media URI next", "next"),
            ] {
                menu.input(
                    MEDIA,
                    label.into(),
                    Command::Queue {
                        id: app.queue_id.clone(),
                        name: "play_media",
                        args: json!({"option":option}),
                    },
                    "media",
                    InputKind::Text,
                );
            }
        }
        // Group the headings without disturbing the order inside each one.
        menu.entries.sort_by_key(|entry| {
            SECTIONS
                .iter()
                .position(|section| *section == entry.section)
                .unwrap_or(SECTIONS.len())
        });
        menu
    }

    fn input(
        &mut self,
        section: &'static str,
        label: String,
        command: Command,
        argument: &'static str,
        kind: InputKind,
    ) {
        let action = Action::Prompt(Prompt {
            label: label.clone(),
            target: PromptTarget::Command { command, argument },
            kind,
            value: String::new(),
        });
        self.add(section, label, action);
    }
}

fn clean(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect()
}

pub fn key(app: &mut App, key: KeyEvent) -> Action {
    let menu = app.menu.as_mut().unwrap();
    if let Some(prompt) = &mut menu.prompt {
        match key.code {
            KeyCode::Esc => {
                menu.prompt = None;
                menu.error.clear();
            }
            KeyCode::Enter => {
                let permitted = if prompt.needs_player() {
                    app.connected
                        && menu.player == app.selected_id
                        && app
                            .players
                            .iter()
                            .any(|p| p.available && Some(&p.id) == menu.player.as_ref())
                } else {
                    app.connected
                };
                if !permitted {
                    menu.error = if prompt.needs_player() {
                        "Player unavailable; cancel and reconnect"
                    } else {
                        "Server unavailable; cancel and reconnect"
                    }
                    .into();
                    return Action::None;
                }
                match prompt.submit() {
                    Ok(action) => {
                        app.menu = None;
                        return action;
                    }
                    Err(error) => menu.error = error.into(),
                }
            }
            KeyCode::Backspace => {
                prompt.value.pop();
            }
            KeyCode::Char(c) if !c.is_control() && prompt.value.len() < 2048 => {
                prompt.value.push(c)
            }
            _ => {}
        }
        return Action::None;
    }
    // While filtering, text keys narrow the list; navigation and Enter still
    // act on the selection, so the filter is a search box, not a mode switch.
    if menu.filtering {
        match key.code {
            KeyCode::Esc => {
                menu.filtering = false;
                menu.filter.clear();
                menu.cursor = 0;
                return Action::None;
            }
            KeyCode::Backspace => {
                menu.filter.pop();
                menu.cursor = 0;
                return Action::None;
            }
            KeyCode::Char(c) if !c.is_control() && menu.filter.len() < 64 => {
                menu.filter.push(c);
                menu.cursor = 0;
                return Action::None;
            }
            _ => {}
        }
    } else if key.code == KeyCode::Char('/') {
        menu.filtering = true;
        return Action::None;
    }
    let last = menu.visible().len().saturating_sub(1);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::F(1) => {
            app.menu = None;
        }
        KeyCode::Down | KeyCode::Char('j') => menu.cursor = (menu.cursor + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => menu.cursor = menu.cursor.saturating_sub(1),
        KeyCode::PageDown => menu.cursor = (menu.cursor + 10).min(last),
        KeyCode::PageUp => menu.cursor = menu.cursor.saturating_sub(10),
        KeyCode::Home => menu.cursor = 0,
        KeyCode::End => menu.cursor = last,
        KeyCode::Enter => {
            // A filter matching nothing has nothing to apply: keep the menu.
            if menu.selected().is_none() {
                return Action::None;
            }
            let chosen = menu
                .selected()
                .map(|entry| entry.action.clone())
                .unwrap_or(Action::None);
            // A library edit needs the server but no speaker; everything else
            // has to still be aimed at the player the menu was opened for.
            let needs_player = match &chosen {
                Action::Prompt(prompt) => prompt.needs_player(),
                _ => chosen.needs_player(),
            };
            let permitted = if needs_player {
                app.connected
                    && menu.player == app.selected_id
                    && app
                        .players
                        .iter()
                        .any(|p| Some(&p.id) == menu.player.as_ref() && p.available)
            } else {
                app.connected
            };
            let action = if permitted {
                chosen
            } else {
                app.status = "Player disconnected; reopen controls after reconnecting".into();
                Action::None
            };
            if let Action::Prompt(prompt) = action {
                menu.prompt = Some(prompt);
                return Action::None;
            }
            app.menu = None;
            return action;
        }
        _ => {}
    }
    Action::None
}

/// The menu is a pane, not a screen: it takes the browser's column and leaves
/// the player, the speakers and the queue where they are. That keeps what is
/// playing visible while choosing, and means nothing is ever drawn over the
/// cover — pixels written outside the cell grid are not the renderer's to
/// clear, so not covering them is worth more than repairing them afterwards.
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let menu = app.menu.as_ref().unwrap();
    let palette = app.palette;
    if let Some(prompt) = &menu.prompt {
        let area = crate::ui::heading(frame, area, palette, &prompt.label.to_uppercase(), true);
        frame.render_widget(
            Paragraph::new(format!(
                "{}▏\n\n{}\n\nEnter applies · Esc cancels",
                prompt.value, menu.error
            ))
            .wrap(ratatui::widgets::Wrap { trim: false }),
            area,
        );
        return;
    }
    let visible = menu.visible();
    let label = format!(
        "{} · {} OF {}",
        menu.title.to_uppercase(),
        visible.len(),
        menu.entries.len()
    );
    let area = crate::ui::heading(frame, area, palette, &label, true);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let hint = if menu.filtering {
        format!("Filter: {}▏  Esc clears · Enter applies", menu.filter)
    } else {
        "↑↓ choose · / filter · Enter applies · Esc returns".into()
    };
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(palette.secondary)),
        rows[0],
    );
    // Headings are drawn as their own rows; the cursor only ever lands on an
    // entry, so navigation never stops on one.
    let mut items: Vec<ListItem> = Vec::with_capacity(visible.len() + SECTIONS.len());
    let mut selected = None;
    let mut section = "";
    for (position, index) in visible.iter().enumerate() {
        let entry = &menu.entries[*index];
        if entry.section != section {
            section = entry.section;
            items.push(ListItem::new(Line::styled(
                section.to_string(),
                Style::default()
                    .fg(palette.secondary)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            )));
        }
        if position == menu.cursor {
            selected = Some(items.len());
        }
        items.push(ListItem::new(Line::from(format!("  {}", entry.label))));
    }
    let mut state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol("› ")
            .highlight_style(Style::default().fg(palette.accent).bg(palette.selection)),
        rows[1],
        &mut state,
    );
}
