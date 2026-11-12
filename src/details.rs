//! Show notes and facts about a podcast, episode or audiobook.
//!
//! Rows are slim summaries that keep no description, so details are filled in
//! from the full item (`music/item_by_uri`). Every value is remote text and
//! goes through `text::plain` before it is kept.

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
