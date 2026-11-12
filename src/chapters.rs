//! Chapters of the playing audiobook or podcast episode.
//!
//! Music Assistant keeps them on the media item as `metadata.chapters`, a list
//! of `{position, name, start, end}` with times in seconds
//! (`music_assistant_models` `MediaItemChapter`). A queue item may or may not
//! carry them: on 2.10.x an item started from a listing is a slim summary, and
//! the podcast providers fill chapters only on the single-episode path, which
//! `music/item_by_uri` reaches. So the queue item is read first and the full
//! item fetched only when it has none.
//!
//! Everything here is remote, untrusted data. A missing or unusable list means
//! "no chapters", never an error.

use serde_json::Value;

/// Longest chapter name kept, in characters.
pub const NAME_LIMIT: usize = 200;
/// Most chapters kept for one item. Real books stay far below this.
pub const COUNT_LIMIT: usize = 2000;

#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    /// The server's chapter number, which `play_media` takes as `start_item`.
    pub position: Option<u64>,
    pub name: String,
    /// Seconds from the start of the item.
    pub start: f64,
    pub end: Option<f64>,
}

/// The chapters of one item, named by its URI so a list fetched for an item
/// that has since stopped playing is never shown for the one after it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chapters {
    pub uri: String,
    pub list: Vec<Chapter>,
}

impl Chapters {
    /// The chapters of what the queue says is playing, or none when they were
    /// loaded for something else.
    pub fn current(&self, queue: &Value) -> &[Chapter] {
        let uri = playing_uri(queue);
        if uri.is_empty() || uri != self.uri {
            return &[];
        }
        &self.list
    }
}

/// Usable chapters from a media item, sorted by start time.
pub fn parse(item: &Value) -> Vec<Chapter> {
    let Some(entries) = item["metadata"]["chapters"].as_array() else {
        return Vec::new();
    };
    let mut chapters: Vec<Chapter> = entries
        .iter()
        .filter_map(|entry| {
            let start = entry["start"]
                .as_f64()
                .filter(|s| s.is_finite() && *s >= 0.0)?;
            Some(Chapter {
                position: entry["position"].as_u64(),
                name: entry["name"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(NAME_LIMIT)
                    .collect::<String>()
                    .trim()
                    .to_owned(),
                start,
                end: entry["end"]
                    .as_f64()
                    .filter(|end| end.is_finite() && *end > start),
            })
        })
        .take(COUNT_LIMIT)
        .collect();
    chapters.sort_by(|a, b| a.start.total_cmp(&b.start));
    chapters
}

/// The URI of the item a queue snapshot says is playing.
pub fn playing_uri(queue: &Value) -> &str {
    queue["current_item"]["media_item"]["uri"]
        .as_str()
        .unwrap_or_default()
}

/// Where the playing item's chapters come from.
#[derive(Debug, PartialEq)]
pub enum Source {
    /// The queue item carries them, or the item is a kind that has none.
    Known(Vec<Chapter>),
    /// Only the full item can say: fetch it with `music/item_by_uri`.
    Fetch,
}

/// What a queue snapshot says about the playing item's chapters, and which
/// item that is.
pub fn source(queue: &Value) -> (String, Source) {
    let item = &queue["current_item"]["media_item"];
    let uri = playing_uri(queue).to_owned();
    let chapters = parse(item);
    let chaptered = matches!(
        item["media_type"].as_str(),
        Some("audiobook" | "podcast_episode")
    );
    if chapters.is_empty() && chaptered && !uri.is_empty() {
        return (uri, Source::Fetch);
    }
    (uri, Source::Known(chapters))
}
