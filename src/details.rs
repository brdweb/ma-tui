//! Show notes and facts about a podcast, episode or audiobook.
//!
//! The pane replaces the browser column, like the controls menu, and takes the
//! keys while it is open. Rows are slim summaries that keep no description, so
//! the pane opens at once with the row's title and fills in from the full item
//! (`music/item_by_uri`). Every value is remote text and goes through
//! `text::plain` before it is kept.

use crate::ui::{Action, App};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use serde_json::Value;

/// Longest description kept, in characters. Show notes can be long; a book's
/// blurb rarely is.
pub const DESCRIPTION_LIMIT: usize = 20_000;
/// Longest single fact kept, in characters.
const FACT_LIMIT: usize = 300;

/// Media types that have details worth reading.
pub fn offered(kind: &str) -> bool {
    matches!(kind, "podcast" | "podcast_episode" | "audiobook")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub uri: String,
    pub title: String,
    /// Labelled one-line facts, in display order.
    pub facts: Vec<(&'static str, String)>,
    pub description: String,
    pub loading: bool,
    pub error: String,
    /// First wrapped row shown.
    pub scroll: u16,
}

impl Details {
    /// The pane as it opens, before the full item arrives.
    pub fn loading(uri: &str, title: &str) -> Self {
        Self {
            uri: uri.into(),
            title: title.into(),
            loading: true,
            ..Self::default()
        }
    }

    /// Fill in from the full item.
    pub fn fill(&mut self, item: &Value) {
        let title = line(item["name"].as_str().unwrap_or_default());
        if !title.is_empty() {
            self.title = title;
        }
        self.facts = facts(item);
        self.description = crate::text::plain(
            item["metadata"]["description"].as_str().unwrap_or_default(),
            DESCRIPTION_LIMIT,
        );
        self.loading = false;
        self.error.clear();
    }
}

/// Open the pane for an item, or say why not. Returns the read to send.
pub fn open(app: &mut App, uri: &str, title: &str, kind: &str) -> Action {
    if !offered(kind) || uri.is_empty() {
        app.status = "Details are shown for podcasts, episodes and audiobooks".into();
        return Action::None;
    }
    app.details = Some(Details::loading(uri, title));
    Action::LoadDetails {
        uri: uri.into(),
        title: title.into(),
    }
}

/// One line of untrusted text.
fn line(text: &str) -> String {
    crate::text::plain(text, FACT_LIMIT).replace('\n', " ")
}

/// Names from a list of strings or of objects with a `name`.
fn names(list: &Value) -> String {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.as_str().or_else(|| entry["name"].as_str()))
        .map(line)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The calendar date of `metadata.release_date`, an ISO 8601 timestamp, as
/// sent. The same rule as the episode rows' date (#44); fold the two together
/// once both are merged.
fn published(item: &Value) -> Option<String> {
    let date = item["metadata"]["release_date"].as_str()?.get(..10)?;
    let shaped = date.bytes().enumerate().all(|(index, byte)| match index {
        4 | 7 => byte == b'-',
        _ => byte.is_ascii_digit(),
    });
    shaped.then(|| date.to_owned())
}

/// Whole seconds as `h:mm:ss`, or `m:ss` under an hour.
fn length(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The facts the server gave, leaving out any it did not.
pub fn facts(item: &Value) -> Vec<(&'static str, String)> {
    let metadata = &item["metadata"];
    let chapters = crate::chapters::parse(item).len();
    let links: Vec<String> = metadata["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|link| link["url"].as_str())
        .map(line)
        .filter(|url| !url.is_empty())
        .collect();
    let candidates = [
        (
            "Show",
            line(item["podcast"]["name"].as_str().unwrap_or_default()),
        ),
        ("Authors", names(&item["authors"])),
        ("Narrators", names(&item["narrators"])),
        ("Published", published(item).unwrap_or_default()),
        (
            "Duration",
            item["duration"]
                .as_u64()
                .filter(|seconds| *seconds > 0)
                .map(length)
                .unwrap_or_default(),
        ),
        ("People", names(&metadata["performers"])),
        (
            "Publisher",
            line(item["publisher"].as_str().unwrap_or_default()),
        ),
        (
            "Chapters",
            if chapters > 0 {
                chapters.to_string()
            } else {
                String::new()
            },
        ),
        (
            "Explicit",
            if metadata["explicit"] == true {
                "yes".into()
            } else {
                String::new()
            },
        ),
        ("Links", links.join(" · ")),
    ];
    candidates
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .collect()
}

/// Keys while the pane is open: scroll, or close it.
pub fn key(app: &mut App, key: KeyEvent) -> Action {
    let Some(details) = &mut app.details else {
        return Action::None;
    };
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('i') | KeyCode::Backspace => {
            app.details = None;
        }
        KeyCode::Down | KeyCode::Char('j') => details.scroll = details.scroll.saturating_add(1),
        KeyCode::Up | KeyCode::Char('k') => details.scroll = details.scroll.saturating_sub(1),
        KeyCode::PageDown | KeyCode::Char(' ') => {
            details.scroll = details.scroll.saturating_add(10)
        }
        KeyCode::PageUp => details.scroll = details.scroll.saturating_sub(10),
        KeyCode::Home => details.scroll = 0,
        _ => {}
    }
    Action::None
}

/// Rows a line takes when wrapped at word boundaries, as the paragraph does.
fn wrapped(text: &str, width: usize) -> usize {
    let (mut rows, mut used) = (1, 0);
    for word in text.split(' ') {
        let length = word.chars().count();
        if used > 0 && used + 1 + length > width {
            rows += 1;
            used = 0;
        }
        used += length + usize::from(used > 0);
        while used > width {
            rows += 1;
            used -= width;
        }
    }
    rows
}

/// The pane, in the browser column.
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let palette = app.palette;
    let Some(details) = &mut app.details else {
        return;
    };
    let area = crate::ui::heading(frame, area, palette, "DETAILS", true);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let dim = Style::default().fg(palette.secondary);
    frame.render_widget(
        Paragraph::new("↑↓ scroll · PgUp/PgDn page · Esc returns").style(dim),
        rows[0],
    );
    let mut lines = vec![
        Line::styled(
            details.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::default(),
    ];
    for (label, value) in &details.facts {
        lines.push(Line::from(vec![
            Span::styled(format!("{label}: "), dim),
            Span::raw(value.clone()),
        ]));
    }
    if details.loading {
        lines.push(Line::styled("Loading details…", dim));
    } else if !details.error.is_empty() {
        lines.push(Line::raw(format!(
            "Could not load details: {}",
            details.error
        )));
    } else if details.description.is_empty() {
        lines.push(Line::default());
        lines.push(Line::styled("No description", dim));
    } else {
        lines.push(Line::default());
        lines.extend(
            details
                .description
                .lines()
                .map(|text| Line::raw(text.to_owned())),
        );
    }
    // Wrapped height, so scrolling stops at the end rather than past it.
    let width = rows[1].width.max(1) as usize;
    let height: usize = lines
        .iter()
        .map(|line| {
            let text: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            wrapped(&text, width)
        })
        .sum();
    let last = height.saturating_sub(rows[1].height as usize);
    details.scroll = details.scroll.min(last.min(u16::MAX as usize) as u16);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((details.scroll, 0)),
        rows[1],
    );
}
