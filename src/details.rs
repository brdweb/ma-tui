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
    /// The episode's provider id and provider, when it says it has a
    /// transcript (`metadata.has_transcript`, 2.11.0b3 and later).
    pub transcript_source: Option<(String, String)>,
    pub transcript: Option<crate::transcript::Transcript>,
    /// Whether the transcript is shown in place of the details.
    pub showing_transcript: bool,
    /// The transcript's timed cues, empty when it has none.
    pub cues: Vec<crate::transcript::Cue>,
    /// Whether cues were left out or shortened to stay within the limits.
    pub cues_truncated: bool,
    /// Whether the transcript scrolls itself to the cue being played. Off once
    /// the reader scrolls; End turns it back on.
    pub following: bool,
}

impl Details {
    /// The pane as it opens, before the full item arrives.
    pub fn loading(uri: &str, title: &str) -> Self {
        Self {
            uri: uri.into(),
            title: title.into(),
            loading: true,
            following: true,
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
        self.transcript_source = (item["metadata"]["has_transcript"] == true)
            .then(|| {
                let id = item["item_id"].as_str().filter(|id| !id.is_empty())?;
                let provider = item["provider"].as_str().filter(|p| !p.is_empty())?;
                Some((id.to_owned(), provider.to_owned()))
            })
            .flatten();
        if self.transcript_source.is_some() {
            self.facts.push(("Transcript", "press t to read".into()));
        }
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
        (
            "Episode",
            crate::music::episode_number(item).unwrap_or_default(),
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
    // Lists are capped as a whole too, not only entry by entry.
    candidates
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(label, value)| (label, capped(value)))
        .collect()
}

/// `value` cut to [`FACT_LIMIT`] characters, ending in `…` when cut.
fn capped(value: String) -> String {
    if value.chars().count() <= FACT_LIMIT {
        return value;
    }
    let mut cut: String = value.chars().take(FACT_LIMIT - 1).collect();
    cut.push('…');
    cut
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
        // Reading by hand stops the transcript following playback.
        KeyCode::Down | KeyCode::Char('j') => {
            details.following = false;
            details.scroll = details.scroll.saturating_add(1)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            details.following = false;
            details.scroll = details.scroll.saturating_sub(1)
        }
        KeyCode::PageDown | KeyCode::Char(' ') => {
            details.following = false;
            details.scroll = details.scroll.saturating_add(10)
        }
        KeyCode::PageUp => {
            details.following = false;
            details.scroll = details.scroll.saturating_sub(10)
        }
        KeyCode::Home => {
            details.following = false;
            details.scroll = 0
        }
        KeyCode::End => details.following = true,
        KeyCode::Char('t') => {
            let Some((item_id, provider)) = details.transcript_source.clone() else {
                app.status = "No transcript for this item".into();
                return Action::None;
            };
            details.showing_transcript = !details.showing_transcript;
            details.scroll = 0;
            // Read once; switching back and forth costs nothing after that.
            let failed = matches!(
                details.transcript,
                Some(crate::transcript::Transcript::Failed(_))
            );
            if details.showing_transcript && (details.transcript.is_none() || failed) {
                details.transcript = Some(crate::transcript::Transcript::Loading);
                return Action::LoadTranscript {
                    uri: details.uri.clone(),
                    item_id,
                    provider,
                };
            }
        }
        _ => {}
    }
    Action::None
}

/// The cues as lines, the one spoken at `position` marked. Returns the index
/// in `lines` of the spoken cue's line.
fn cue_lines(
    lines: &mut Vec<Line<'static>>,
    cues: &[crate::transcript::Cue],
    position: f64,
    palette: crate::theme::Palette,
) -> Option<usize> {
    let now = crate::transcript::current(cues, position);
    let mut spoken = None;
    for (index, cue) in cues.iter().enumerate() {
        let playing = now == Some(index);
        let mut spans = vec![Span::raw(if playing { "▶ " } else { "  " })];
        if !cue.speaker.is_empty() {
            spans.push(Span::styled(
                format!("{}: ", cue.speaker),
                Style::default().fg(palette.accent),
            ));
        }
        spans.push(Span::raw(cue.text.clone()));
        let mut line = Line::from(spans);
        if playing {
            line = line.style(Style::default().add_modifier(Modifier::BOLD));
            spoken = Some(lines.len());
        }
        lines.push(line);
    }
    spoken
}

/// Rows a line takes when wrapped at word boundaries, as the paragraph does.
/// Widths are in columns, and a word too long for a row is broken between
/// characters, never through one: a wide character that does not fit moves
/// whole to the next row.
pub fn wrapped(text: &str, width: usize) -> usize {
    let columns = |text: &str| ratatui::text::Span::raw(text).width();
    let width = width.max(1);
    // Leading spaces, such as a cue's indent, take columns on the first row.
    let rest = text.trim_start_matches(' ');
    let (mut rows, mut used) = (1, text.len() - rest.len());
    for (index, word) in rest.split(' ').enumerate() {
        let length = columns(word);
        let gap = usize::from(index > 0);
        if used > 0 && used + gap + length > width {
            rows += 1;
            used = 0;
        } else if used > 0 {
            used += gap;
        }
        if used + length <= width {
            used += length;
            continue;
        }
        for c in word.chars() {
            let mut buffer = [0; 4];
            let wide = columns(c.encode_utf8(&mut buffer));
            if used > 0 && used + wide > width {
                rows += 1;
                used = 0;
            }
            used += wide;
        }
    }
    rows
}

/// The pane, in the browser column.
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let palette = app.palette;
    // Where playback is in this episode, when it is the one playing.
    let position = app
        .details
        .as_ref()
        .filter(|details| crate::chapters::playing_uri(&app.queue_details) == details.uri)
        .map(|_| app.elapsed);
    let Some(details) = &mut app.details else {
        return;
    };
    // Timed cues of the playing episode are shown in place of the text, so
    // the line being spoken can be marked and kept in view.
    let live = details.showing_transcript
        && !details.cues.is_empty()
        && matches!(
            details.transcript,
            Some(crate::transcript::Transcript::Text { .. })
        );
    let position = position.filter(|_| live);
    let label = match (details.showing_transcript, position.is_some()) {
        (true, true) => "DETAILS · TRANSCRIPT · LIVE",
        (true, false) => "DETAILS · TRANSCRIPT",
        _ => "DETAILS",
    };
    let area = crate::ui::heading(frame, area, palette, label, true);
    // While following, a fixed row under the hints names who is speaking.
    let speaking = Constraint::Length(u16::from(position.is_some()));
    let parts = Layout::vertical([Constraint::Length(1), speaking, Constraint::Min(1)]).split(area);
    let rows = [parts[0], parts[2]];
    let dim = Style::default().fg(palette.secondary);
    if let Some(position) = position {
        let speaker = crate::transcript::current(&details.cues, position)
            .map(|index| details.cues[index].speaker.clone())
            .filter(|speaker| !speaker.is_empty());
        frame.render_widget(
            Paragraph::new(match speaker {
                Some(speaker) => Line::from(vec![
                    Span::styled("Now speaking: ", dim),
                    Span::styled(speaker, Style::default().fg(palette.accent)),
                ]),
                None => Line::styled("Following playback", dim),
            }),
            parts[1],
        );
    }
    frame.render_widget(
        Paragraph::new(if position.is_some() && details.following {
            "following playback · ↑↓ read freely · t details · Esc returns"
        } else if position.is_some() {
            "↑↓ scroll · End follow playback · t details · Esc returns"
        } else if details.transcript_source.is_some() {
            "↑↓ scroll · PgUp/PgDn page · t transcript/details · Esc returns"
        } else {
            "↑↓ scroll · PgUp/PgDn page · Esc returns"
        })
        .style(dim),
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
    // The line of the cue being spoken, when following playback.
    let mut spoken = None;
    if details.showing_transcript {
        use crate::transcript::Transcript;
        lines.truncate(2);
        match &details.transcript {
            Some(Transcript::Text { body, truncated }) => {
                // While following, the cues are shown, so their own cut counts.
                let notice = match position {
                    Some(_) if details.cues_truncated => Some(format!(
                        "Transcript shortened to its first {} characters or {} cues",
                        crate::transcript::LIMIT,
                        crate::transcript::MAX_CUES
                    )),
                    None if *truncated => Some(format!(
                        "Transcript shortened to its first {} characters",
                        crate::transcript::LIMIT
                    )),
                    _ => None,
                };
                if let Some(notice) = notice {
                    lines.push(Line::styled(notice, dim));
                    lines.push(Line::default());
                }
                match position {
                    Some(position) => {
                        spoken = cue_lines(&mut lines, &details.cues, position, palette);
                    }
                    None => lines.extend(body.lines().map(|text| Line::raw(text.to_owned()))),
                }
            }
            Some(Transcript::Missing) => {
                lines.push(Line::styled("This episode has no transcript", dim))
            }
            Some(Transcript::Failed(error)) => lines.push(Line::raw(format!(
                "Could not load the transcript: {error} (press t twice to retry)"
            ))),
            Some(Transcript::Loading) | None => {
                lines.push(Line::styled("Loading transcript…", dim))
            }
        }
    } else if details.loading {
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
    let heights: Vec<usize> = lines
        .iter()
        .map(|line| {
            let text: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            wrapped(&text, width)
        })
        .collect();
    let height: usize = heights.iter().sum();
    // Keep the spoken line a third of the way down while following.
    if let Some(line) = spoken.filter(|_| details.following) {
        let above: usize = heights[..line].iter().sum();
        let lead = rows[1].height as usize / 3;
        details.scroll = above.saturating_sub(lead).min(u16::MAX as usize) as u16;
    }
    let last = height.saturating_sub(rows[1].height as usize);
    details.scroll = details.scroll.min(last.min(u16::MAX as usize) as u16);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((details.scroll, 0)),
        rows[1],
    );
}
