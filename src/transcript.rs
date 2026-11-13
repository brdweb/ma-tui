//! Podcast episode transcripts (Music Assistant 2.11.0b3 and later).
//!
//! `music/podcasts/podcast_episode_transcript` answers `[text, cues]`: readable
//! text, timed cues `{start, end, text, speaker}`, both, or `[null, null]` when
//! there is none. The text is preferred; without it the cues are joined into
//! lines. Everything is remote, so it goes through `text::plain`, and a long
//! transcript is cut to a bound that says so.

use serde_json::Value;

/// Most transcript text kept, in characters. An hour of speech is roughly
/// 50,000, so this holds a long episode and stops a runaway one.
pub const LIMIT: usize = 120_000;

/// What the pane shows for a transcript.
#[derive(Clone, Debug, PartialEq)]
pub enum Transcript {
    Loading,
    Text {
        body: String,
        truncated: bool,
    },
    /// The server answered, but has no transcript for the episode.
    Missing,
    Failed(String),
}

impl Transcript {
    /// The server's `[text, cues]` answer.
    pub fn parse(value: &Value) -> Self {
        let text = value[0].as_str().filter(|text| !text.trim().is_empty());
        // Cues are cleaned one by one; cleaning the joined lines again could
        // read a decoded `<` as markup and flatten them.
        let (body, truncated) = match text {
            Some(text) => crate::text::plain_cut(text, LIMIT),
            None => cap(cues(&value[1])),
        };
        if body.trim().is_empty() {
            return Self::Missing;
        }
        Self::Text { body, truncated }
    }
}

/// At most `LIMIT` characters, ending in `…` when cut, as `text::plain` does,
/// and whether it was cut.
fn cap(text: String) -> (String, bool) {
    if text.chars().count() <= LIMIT {
        return (text, false);
    }
    let mut cut: String = text.chars().take(LIMIT - 1).collect();
    cut.push('…');
    (cut, true)
}

/// Cues as one line each, with the speaker when the source names one.
fn cues(value: &Value) -> String {
    let mut lines = Vec::new();
    let mut length = 0;
    for cue in value.as_array().into_iter().flatten() {
        // Each cue is cleaned on its own, so markup in one cannot swallow the
        // line breaks between them.
        let text =
            crate::text::plain(cue["text"].as_str().unwrap_or_default(), 2_000).replace('\n', " ");
        if text.is_empty() {
            continue;
        }
        let speaker =
            crate::text::plain(cue["speaker"].as_str().unwrap_or_default(), 80).replace('\n', " ");
        let line = if speaker.is_empty() {
            text
        } else {
            format!("{speaker}: {text}")
        };
        length += line.chars().count() + 1;
        lines.push(line);
        // Enough to fill the limit; the rest would be cut anyway.
        if length > LIMIT {
            break;
        }
    }
    lines.join("\n")
}

/// Most cues kept for following playback. A cue every few seconds over a
/// long episode stays well inside this.
pub const MAX_CUES: usize = 20_000;

/// One timed line of a transcript, cleaned for the terminal.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    /// Seconds from the start of the episode.
    pub start: f64,
    pub end: Option<f64>,
    pub speaker: String,
    pub text: String,
}

/// The timed cues of a `[text, cues]` answer, in start order. Cues with no
/// usable start time or no text are dropped, and the whole set is capped by
/// count and by [`LIMIT`] characters, like the text.
pub fn timed_cues(value: &Value) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut length = 0;
    for cue in value[1].as_array().into_iter().flatten() {
        let Some(start) = cue["start"].as_f64().filter(|s| s.is_finite() && *s >= 0.0) else {
            continue;
        };
        let text =
            crate::text::plain(cue["text"].as_str().unwrap_or_default(), 2_000).replace('\n', " ");
        if text.is_empty() {
            continue;
        }
        let speaker =
            crate::text::plain(cue["speaker"].as_str().unwrap_or_default(), 80).replace('\n', " ");
        let end = cue["end"]
            .as_f64()
            .filter(|end| end.is_finite() && *end > start);
        length += speaker.chars().count() + text.chars().count() + 3;
        if length > LIMIT || cues.len() == MAX_CUES {
            break;
        }
        cues.push(Cue {
            start,
            end,
            speaker,
            text,
        });
    }
    // Stable, so cues sharing a start keep the source's order.
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

/// The cue being spoken at `position` seconds: the last one started, unless
/// it has a known end that has passed.
pub fn current(cues: &[Cue], position: f64) -> Option<usize> {
    let index = cues
        .partition_point(|cue| cue.start <= position)
        .checked_sub(1)?;
    match cues[index].end {
        Some(end) if position >= end => None,
        _ => Some(index),
    }
}
