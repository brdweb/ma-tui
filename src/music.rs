//! Music navigation, kept separate from explicit server actions.
use crate::{
    api::ApiClient,
    ui::{Action, App, Focus},
};
use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const PAGE_SIZE: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Home,
    Library {
        kind: Kind,
        offset: usize,
        favorite: bool,
        search: Option<String>,
        order: Order,
    },
    Album {
        id: String,
        provider: String,
    },
    Playlist {
        id: String,
        provider: String,
    },
    Artist {
        id: String,
        provider: String,
    },
    ArtistTracks {
        id: String,
        provider: String,
    },
    /// A podcast's episodes. Audiobooks have no equivalent: MA 2.10.2 models
    /// them as one playable item with a resume point, not a chapter list.
    Podcast {
        id: String,
        provider: String,
    },
    /// Server-maintained shelves keep listening activity current without UI state.
    InProgress,
    RecentlyAdded,
    RecentlyPlayed,
    /// Every episode not yet finished, across every subscribed show. This one
    /// the server does not keep: it has to be assembled here.
    UnplayedEpisodes,
    Providers {
        path: Option<String>,
    },
}

/// Server-supported library orders, so the browser never sends an invented key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Name,
    RecentlyAdded,
    LastPlayed,
    MostPlayed,
    Year,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Playlists,
    Albums,
    Artists,
    Tracks,
    Radio,
    Podcasts,
    Audiobooks,
}
impl Kind {
    /// MA derives the command base from the media type: `music/<type>s/…`.
    fn endpoint(self) -> &'static str {
        match self {
            Self::Playlists => "playlists",
            Self::Albums => "albums",
            Self::Artists => "artists",
            Self::Tracks => "tracks",
            Self::Radio => "radios",
            Self::Podcasts => "podcasts",
            Self::Audiobooks => "audiobooks",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Playlists => "Playlists",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Tracks => "Tracks",
            Self::Radio => "Radio",
            Self::Podcasts => "Podcasts",
            Self::Audiobooks => "Audiobooks",
        }
    }
    fn media_type(self) -> &'static str {
        match self {
            Self::Playlists => "playlist",
            Self::Albums => "album",
            Self::Artists => "artist",
            Self::Tracks => "track",
            Self::Radio => "radio",
            Self::Podcasts => "podcast",
            Self::Audiobooks => "audiobook",
        }
    }
}

impl Order {
    const WITHOUT_YEAR: [Self; 5] = [
        Self::Name,
        Self::RecentlyAdded,
        Self::LastPlayed,
        Self::MostPlayed,
        Self::Random,
    ];
    const WITH_YEAR: [Self; 6] = [
        Self::Name,
        Self::RecentlyAdded,
        Self::LastPlayed,
        Self::MostPlayed,
        Self::Year,
        Self::Random,
    ];

    /// The server's spelling stays beside the UI choices that select it.
    pub fn order_by(self) -> &'static str {
        match self {
            Self::Name => "sort_name",
            Self::RecentlyAdded => "timestamp_added_desc",
            Self::LastPlayed => "last_played_desc",
            Self::MostPlayed => "play_count_desc",
            Self::Year => "year_desc",
            Self::Random => "random",
        }
    }

    /// A compact label leaves room for the active filter and page controls.
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::RecentlyAdded => "recently added",
            Self::LastPlayed => "last played",
            Self::MostPlayed => "most played",
            Self::Year => "year",
            Self::Random => "random",
        }
    }

    fn next(self, kind: Kind) -> Self {
        let orders: &[Self] = if matches!(kind, Kind::Albums | Kind::Tracks) {
            &Self::WITH_YEAR
        } else {
            &Self::WITHOUT_YEAR
        };
        let index = orders.iter().position(|order| *order == self).unwrap_or(0);
        orders[(index + 1) % orders.len()]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub title: String,
    pub detail: String,
    pub uri: String,
    pub kind: String,
    /// Library identity, kept so an item can be named back to the server.
    pub id: String,
    pub provider: String,
    pub favorite: bool,
    /// Whether this row is the server's library copy rather than a provider mapping.
    pub in_library: bool,
    pub playable: bool,
    pub available: bool,
    /// Listening progress, for the media types that report it.
    pub fully_played: bool,
    pub resume_ms: Option<u64>,
    pub open: Option<Target>,
}
impl Media {
    pub fn folder(title: &str, target: Target) -> Self {
        Self {
            title: title.into(),
            detail: "Enter to browse".into(),
            uri: String::new(),
            kind: "folder".into(),
            id: String::new(),
            provider: String::new(),
            favorite: false,
            in_library: false,
            playable: false,
            available: true,
            fully_played: false,
            resume_ms: None,
            open: Some(target),
        }
    }
    pub fn parse(v: &Value, hint: &str) -> Self {
        let value = |key: &str| v[key].as_str().unwrap_or_default().to_owned();
        let id = value("item_id");
        let provider = value("provider");
        let in_library = provider == "library";
        let kind = v["media_type"].as_str().unwrap_or(hint).to_owned();
        let uri = value("uri");
        let open = if kind == "folder" {
            v["path"]
                .as_str()
                .filter(|p| !p.is_empty())
                .map(|p| Target::Providers {
                    path: Some(p.into()),
                })
        } else if !id.is_empty() && !provider.is_empty() {
            let (id, provider) = (id.clone(), provider.clone());
            match kind.as_str() {
                "album" => Some(Target::Album { id, provider }),
                "playlist" => Some(Target::Playlist { id, provider }),
                "artist" => Some(Target::Artist { id, provider }),
                "podcast" => Some(Target::Podcast { id, provider }),
                _ => None,
            }
        } else {
            None
        };
        // Both are null when the provider does not report progress, which is
        // not the same as "not played": nothing is shown then.
        let fully_played = v["fully_played"].as_bool().unwrap_or(false);
        let resume_ms = v["resume_position_ms"].as_u64().filter(|ms| *ms > 0);
        let progress = if fully_played {
            " · played".into()
        } else {
            resume_ms.map_or(String::new(), |ms| format!(" · resume {}", position(ms)))
        };
        // What the item belongs to, if the server said: the show for an
        // episode, the artists and album for a track. Falling back to the media
        // type reads better than a provider instance id, which means nothing to
        // anyone reading it.
        let belongs = crate::api::byline(v);
        let detail = if belongs.is_empty() {
            clean(&format!("{}{progress}", readable(&kind)))
        } else {
            clean(&format!("{belongs}{progress}"))
        };
        Self {
            title: clean(&value("name")),
            detail,
            uri: uri.clone(),
            kind: kind.clone(),
            id,
            provider,
            favorite: v["favorite"] == true,
            in_library,
            available: v["available"] != false,
            playable: !uri.is_empty()
                && v["is_playable"].as_bool().unwrap_or(matches!(
                    kind.as_str(),
                    "track"
                        | "album"
                        | "artist"
                        | "playlist"
                        | "radio"
                        | "audiobook"
                        | "podcast"
                        | "podcast_episode"
                        | "audio_source"
                )),
            fully_played,
            resume_ms,
            open,
        }
    }

    /// A multi-selection names only playable tracks, never collections or folders.
    fn selectable_track(&self) -> bool {
        self.kind == "track" && self.available && self.playable && !self.uri.is_empty()
    }

    /// The identity Music Assistant needs to name this item back to itself.
    /// These are `ItemMapping`'s only required fields.
    pub fn item(&self) -> Option<Value> {
        (!self.id.is_empty() && !self.provider.is_empty() && !self.kind.is_empty()).then(|| {
            json!({
                "item_id": self.id,
                "provider": self.provider,
                "name": self.title,
                "media_type": self.kind,
            })
        })
    }

    /// Favourites and library saves share the persistable Music Assistant kinds.
    fn library_editable(&self) -> bool {
        !self.uri.is_empty()
            && matches!(
                self.kind.as_str(),
                "track"
                    | "album"
                    | "artist"
                    | "playlist"
                    | "radio"
                    | "audiobook"
                    | "podcast"
                    | "podcast_episode"
            )
    }

    /// Toggle this item's favourite state using the identity MA expects.
    pub fn favorite_action(&self) -> Option<Action> {
        self.library_editable().then(|| Action::Favorite {
            uri: self.uri.clone(),
            media_type: self.kind.clone(),
            library_id: self.in_library.then(|| self.id.clone()),
            favorite: !self.favorite,
        })
    }

    /// Save a provider item before offering library-only edits for it.
    pub fn library_action(&self) -> Option<Action> {
        (!self.in_library && self.library_editable()).then(|| Action::AddToLibrary {
            uri: self.uri.clone(),
        })
    }

    /// Whether this kind of item keeps a listening position worth editing.
    fn tracks_progress(&self) -> bool {
        matches!(self.kind.as_str(), "podcast_episode" | "audiobook")
    }
}

/// A media type as a person would say it: "podcast episode", not
/// "podcast_episode".
fn readable(kind: &str) -> String {
    kind.replace('_', " ")
}

/// A resume point, which for an audiobook is routinely hours in.
fn position(ms: u64) -> String {
    let seconds = ms / 1000;
    match (seconds / 3600, (seconds % 3600) / 60, seconds % 60) {
        (0, minutes, seconds) => format!("{minutes}:{seconds:02}"),
        (hours, minutes, seconds) => format!("{hours}:{minutes:02}:{seconds:02}"),
    }
}
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(512).collect()
}

/// Fictional catalog for offline previews; no server or audio access.
pub fn demo_listing(target: &Target) -> Vec<Media> {
    let kind = match target {
        Target::Home => return Page::default().items,
        Target::Library { kind, .. } => kind.media_type(),
        Target::Artist { .. } => "album",
        Target::Providers { .. } => "playlist",
        Target::Album { .. }
        | Target::Playlist { .. }
        | Target::ArtistTracks { .. }
        | Target::Podcast { .. }
        | Target::InProgress
        | Target::RecentlyAdded
        | Target::RecentlyPlayed
        | Target::UnplayedEpisodes => "track",
    };
    vec![Media::parse(
        &json!({"name":format!("Sample {kind} — offline preview"),"item_id":"sample","provider":"demo","uri":format!("demo://{kind}/sample"),"media_type":kind,"artists":[{"name":"Fictional artist"}]}),
        kind,
    )]
}

#[derive(Clone)]
pub struct Page {
    pub target: Target,
    pub title: String,
    pub items: Vec<Media>,
    pub cursor: usize,
    /// URI-keyed selection follows this page into history, not the cursor.
    pub selected: HashSet<String>,
    pub next: Option<Target>,
}
impl Default for Page {
    fn default() -> Self {
        // What you were in the middle of comes before the whole library.
        let mut items = vec![
            Media::folder("Continue listening", Target::InProgress),
            Media::folder("Unplayed podcasts", Target::UnplayedEpisodes),
            Media::folder("Recently added", Target::RecentlyAdded),
            Media::folder("Recently played", Target::RecentlyPlayed),
        ];
        for kind in [
            Kind::Playlists,
            Kind::Albums,
            Kind::Artists,
            Kind::Tracks,
            Kind::Radio,
            Kind::Podcasts,
            Kind::Audiobooks,
        ] {
            items.push(Media::folder(
                kind.label(),
                Target::Library {
                    kind,
                    offset: 0,
                    favorite: false,
                    search: None,
                    order: Order::Name,
                },
            ));
        }
        items.push(Media::folder(
            "Favorite tracks",
            Target::Library {
                kind: Kind::Tracks,
                offset: 0,
                favorite: true,
                search: None,
                order: Order::Name,
            },
        ));
        items.push(Media::folder(
            "Favorite albums",
            Target::Library {
                kind: Kind::Albums,
                offset: 0,
                favorite: true,
                search: None,
                order: Order::Name,
            },
        ));
        items.push(Media::folder(
            "Favorite artists",
            Target::Library {
                kind: Kind::Artists,
                offset: 0,
                favorite: true,
                search: None,
                order: Order::Name,
            },
        ));
        items.push(Media::folder(
            "Favorite playlists",
            Target::Library {
                kind: Kind::Playlists,
                offset: 0,
                favorite: true,
                search: None,
                order: Order::Name,
            },
        ));
        items.push(Media::folder(
            "Favorite radio",
            Target::Library {
                kind: Kind::Radio,
                offset: 0,
                favorite: true,
                search: None,
                order: Order::Name,
            },
        ));
        items.push(Media::folder(
            "Browse music providers",
            Target::Providers { path: None },
        ));
        Self {
            target: Target::Home,
            title: "Music library".into(),
            items,
            cursor: 0,
            next: None,
            selected: HashSet::new(),
        }
    }
}

#[derive(Default)]
pub struct Browser {
    pub page: Page,
    pub history: Vec<Page>,
    pub generation: u64,
    pub loading: bool,
    pub error: String,
    /// The input stays here so a browser filter cannot leak into global search.
    pub filtering: bool,
    pub filter_input: String,
    /// When progress was last re-read, so a playing audiobook's steady stream
    /// of playlog updates cannot turn into a steady stream of requests.
    refreshed: Option<std::time::Instant>,
}
impl Browser {
    /// Whether what is on screen would show a change in listening progress.
    fn shows_progress(&self) -> bool {
        matches!(
            self.page.target,
            Target::InProgress
                | Target::RecentlyPlayed
                | Target::UnplayedEpisodes
                | Target::Podcast { .. }
                | Target::Library {
                    kind: Kind::Podcasts | Kind::Audiobooks,
                    ..
                }
        )
    }

    /// Re-read the listing when progress changed elsewhere, at most this often.
    pub fn progress_changed(&mut self, now: std::time::Instant) -> Option<Action> {
        const THROTTLE: std::time::Duration = std::time::Duration::from_secs(3);
        if self.loading
            || !self.shows_progress()
            || self
                .refreshed
                .is_some_and(|last| now.saturating_duration_since(last) < THROTTLE)
        {
            return None;
        }
        self.refreshed = Some(now);
        Some(self.reload())
    }
    pub fn navigate(&mut self, target: Target, title: String) -> Action {
        if self.history.len() == 32 {
            self.history.remove(0);
        }
        self.history.push(self.page.clone());
        self.page = Page {
            target,
            title,
            items: vec![],
            cursor: 0,
            selected: HashSet::new(),
            next: None,
        };
        self.reload()
    }
    /// Replace the visible page when its listing controls change, not its path.
    pub fn replace(&mut self, target: Target) -> Action {
        let title = std::mem::take(&mut self.page.title);
        self.page = Page {
            target,
            title,
            items: vec![],
            cursor: 0,
            selected: HashSet::new(),
            next: None,
        };
        self.reload()
    }
    pub fn reload(&mut self) -> Action {
        self.generation += 1;
        self.error.clear();
        if self.page.target == Target::Home {
            self.page = Page::default();
            self.loading = false;
            return Action::None;
        }
        self.loading = true;
        Action::Browse {
            generation: self.generation,
            target: self.page.target.clone(),
        }
    }
    pub fn back(&mut self) {
        self.generation += 1;
        self.loading = false;
        self.error.clear();
        self.page = self.history.pop().unwrap_or_default();
    }
    pub fn apply(
        &mut self,
        generation: u64,
        result: std::result::Result<(Vec<Media>, Option<Target>), String>,
    ) {
        if generation != self.generation {
            return;
        }
        self.loading = false;
        match result {
            Ok((items, next)) => {
                self.page.selected.retain(|uri| {
                    items
                        .iter()
                        .any(|item| item.selectable_track() && item.uri == *uri)
                });
                self.page.items = items;
                self.page.next = next;
                self.page.cursor = self
                    .page
                    .cursor
                    .min(self.page.items.len().saturating_sub(1));
            }
            Err(error) => {
                self.error = error;
                self.page.items.clear();
                self.page.next = None;
            }
        }
    }
}

/// Shows asked for their episodes at once. Assembling this list costs one
/// request per subscription, so they overlap — but not without bound, because
/// the server answering them is the one also serving the audio.
const CONCURRENT_SHOWS: usize = 6;
/// Most episodes the unplayed list will gather, so a large subscription list
/// cannot turn into an unbounded read.
const MAX_UNPLAYED: usize = 300;

impl ApiClient {
    /// Every unfinished episode across every show, newest first within each.
    ///
    /// MA 2.10.2 has no server-side filter for this: `library_items` takes
    /// `played_only`, which selects the opposite, and there is no unplayed
    /// equivalent. So the shows are listed and then each is asked for its
    /// episodes and filtered here. That is one request per subscription, which
    /// is why it is bounded and overlapped rather than issued in a loop.
    async fn unplayed_episodes(&self) -> Result<(Vec<Media>, Option<Target>)> {
        use futures_util::StreamExt;
        let shows = self
            .command(
                "music/podcasts/library_items",
                json!({"limit":PAGE_SIZE,"offset":0,"order_by":"sort_name"}),
            )
            .await?;
        let shows: Vec<(String, String)> = shows
            .as_array()
            .ok_or_else(|| anyhow!("Invalid podcast listing"))?
            .iter()
            .filter_map(|show| {
                let id = show["item_id"].as_str().filter(|id| !id.is_empty())?;
                let provider = show["provider"].as_str().filter(|p| !p.is_empty())?;
                Some((id.to_owned(), provider.to_owned()))
            })
            .collect();

        let listings = futures_util::stream::iter(shows)
            .map(|(id, provider)| async move {
                let listing = self
                    .command(
                        "music/podcasts/podcast_episodes",
                        json!({"item_id":id,"provider_instance_id_or_domain":provider}),
                    )
                    .await?;
                let mut episodes: Vec<Media> = listing
                    .as_array()
                    .ok_or_else(|| anyhow!("Invalid podcast episode listing"))?
                    .iter()
                    .map(|episode| Media::parse(episode, "podcast_episode"))
                    .filter(|episode| !episode.fully_played)
                    .collect();
                // Within a show the newest episode is the one to reach for,
                // and position is the only ordering 2.10.2 gives us.
                episodes.reverse();
                Ok::<_, anyhow::Error>(episodes)
            })
            .buffered(CONCURRENT_SHOWS)
            .collect::<Vec<_>>()
            .await;

        // The browser has one success/error state, so do not present a partial
        // shelf as a complete list. Check every read before the item limit can
        // hide a failure in a later show. The usual browser retry reloads all.
        if let Some(error) = listings.iter().find_map(|listing| listing.as_ref().err()) {
            let failed = listings.iter().filter(|listing| listing.is_err()).count();
            return Err(anyhow!(
                "Unplayed podcast list incomplete: could not load episodes for {failed} of {} shows ({error})",
                listings.len()
            ));
        }
        let mut items = Vec::new();
        for listing in listings {
            items.extend(listing?);
            if items.len() >= MAX_UNPLAYED {
                items.truncate(MAX_UNPLAYED);
                break;
            }
        }
        Ok((items, None))
    }

    pub async fn browse(&self, target: &Target) -> Result<(Vec<Media>, Option<Target>)> {
        let (command, args, hint) = match target {
            Target::Home => return Ok((Page::default().items, None)),
            // Assembled from many reads rather than served by one.
            Target::UnplayedEpisodes => return self.unplayed_episodes().await,
            Target::Library {
                kind,
                offset,
                favorite,
                search,
                order,
            } => {
                let mut args = json!({
                    "limit":PAGE_SIZE,
                    "offset":offset,
                    "order_by":order.order_by(),
                    "favorite":if *favorite {Some(true)} else {None}
                });
                if let Some(search) = search {
                    args["search"] = Value::String(search.clone());
                }
                (
                    format!("music/{}/library_items", kind.endpoint()),
                    args,
                    kind.media_type(),
                )
            }
            Target::Album { id, provider } => (
                "music/albums/album_tracks".into(),
                json!({"item_id":id,"provider_instance_id_or_domain":provider}),
                "track",
            ),
            Target::Playlist { id, provider } => (
                "music/playlists/playlist_tracks".into(),
                json!({"item_id":id,"provider_instance_id_or_domain":provider}),
                "track",
            ),
            Target::Artist { id, provider } => (
                "music/artists/artist_albums".into(),
                json!({"item_id":id,"provider_instance_id_or_domain":provider}),
                "album",
            ),
            Target::ArtistTracks { id, provider } => (
                "music/artists/artist_tracks".into(),
                json!({"item_id":id,"provider_instance_id_or_domain":provider}),
                "track",
            ),
            Target::Podcast { id, provider } => (
                "music/podcasts/podcast_episodes".into(),
                json!({"item_id":id,"provider_instance_id_or_domain":provider}),
                "podcast_episode",
            ),
            // Shelves are server-defined lists, not paged libraries.
            Target::InProgress => (
                "music/in_progress_items".into(),
                json!({ "limit": PAGE_SIZE }),
                "",
            ),
            Target::RecentlyAdded => (
                "music/recently_added_tracks".into(),
                json!({ "limit": PAGE_SIZE }),
                "track",
            ),
            Target::RecentlyPlayed => (
                "music/recently_played_items".into(),
                json!({ "limit": 50 }),
                "",
            ),
            Target::Providers { path } => ("music/browse".into(), json!({"path":path}), ""),
        };
        let value = self.command(&command, args).await?;
        let rows = value
            .as_array()
            .ok_or_else(|| anyhow!("Invalid music listing"))?;
        let mut items: Vec<_> = rows.iter().map(|v| Media::parse(v, hint)).collect();
        if let Target::Artist { id, provider } = target {
            items.insert(
                0,
                Media::folder(
                    "Top tracks",
                    Target::ArtistTracks {
                        id: id.clone(),
                        provider: provider.clone(),
                    },
                ),
            );
        }
        let next = match target {
            Target::Library {
                kind,
                offset,
                favorite,
                search,
                order,
            } if rows.len() == PAGE_SIZE => Some(Target::Library {
                kind: *kind,
                offset: offset + PAGE_SIZE,
                favorite: *favorite,
                search: search.clone(),
                order: *order,
            }),
            _ => None,
        };
        Ok((items, next))
    }
}

pub fn choose(app: &mut App, media: &Media) -> Action {
    let player = app
        .players
        .iter()
        .find(|p| Some(&p.id) == app.selected_id.as_ref() && p.available && app.connected);
    let mut entries: Vec<crate::controls::Entry> = Vec::new();
    if player.is_some() && media.available && media.playable {
        entries.extend(
            [
                ("Play now (replace queue)", Action::Play(media.uri.clone())),
                ("Play next", Action::PlayNext(media.uri.clone())),
                ("Add to queue", Action::Enqueue(media.uri.clone())),
            ]
            .into_iter()
            .map(|(label, action)| crate::controls::Entry {
                section: "Play this item",
                label: label.into(),
                action,
            }),
        );
        if matches!(
            media.kind.as_str(),
            "track" | "album" | "artist" | "playlist"
        ) {
            entries.push(crate::controls::Entry {
                section: "Play this item",
                label: "Start radio".into(),
                action: Action::StartRadio(media.uri.clone()),
            });
        }
    }
    // Progress is a library fact, not a playback one, so it needs no speaker.
    if media.tracks_progress() {
        if let Some(item) = media.item() {
            entries.extend(
                [("Mark as played", true), ("Mark as not played", false)]
                    .into_iter()
                    .map(|(label, played)| crate::controls::Entry {
                        section: "Listening progress",
                        label: label.into(),
                        action: Action::MarkPlayed {
                            item: item.clone(),
                            played,
                        },
                    }),
            );
        }
    }
    // Library edits are server state, independent of a selected speaker.
    if let Some(action) = media.favorite_action() {
        let label = if media.favorite {
            "Remove from favourites"
        } else {
            "Add to favourites"
        };
        entries.push(crate::controls::Entry {
            section: "Library",
            label: label.into(),
            action,
        });
    }
    if let Some(action) = media.library_action() {
        entries.push(crate::controls::Entry {
            section: "Library",
            label: "Add to library".into(),
            action,
        });
    }
    if !media.uri.is_empty()
        && matches!(
            media.kind.as_str(),
            "track" | "radio" | "podcast_episode" | "audiobook"
        )
    {
        entries.push(crate::controls::Entry {
            section: "Library",
            label: "Add to playlist…".into(),
            action: Action::LoadPlaylists {
                uri: media.uri.clone(),
            },
        });
    }
    if entries.is_empty() {
        app.status = if player.is_none() {
            "Select a speaker in Players first, then choose music".into()
        } else {
            "This item is not available for playback".into()
        };
        return Action::None;
    }
    app.menu = Some(crate::controls::Menu {
        player: player.map(|p| p.id.clone()),
        title: match player {
            Some(player) => format!("{} · on {}", media.title, player.name),
            None => media.title.clone(),
        },
        entries,
        cursor: 0,
        prompt: None,
        error: String::new(),
        filter: String::new(),
        filtering: false,
    });
    Action::None
}

fn queue_player_available(app: &App) -> bool {
    app.connected
        && app
            .players
            .iter()
            .any(|p| p.available && Some(&p.id) == app.selected_id.as_ref())
}

/// Queue choices capture the speaker at menu creation; controls::key checks
/// that same speaker again when the choice is submitted.
pub(crate) fn queue_choices(app: &mut App, title: &str, replace: Action, add: Action) -> Action {
    let Some(player) = app
        .players
        .iter()
        .find(|p| Some(&p.id) == app.selected_id.as_ref() && p.available && app.connected)
    else {
        app.status = "Select an available speaker first".into();
        return Action::None;
    };
    app.menu = Some(crate::controls::Menu {
        player: Some(player.id.clone()),
        title: format!("{title} · on {}", player.name),
        entries: vec![
            crate::controls::Entry {
                section: "Queue",
                label: "Replace queue".into(),
                action: replace,
            },
            crate::controls::Entry {
                section: "Queue",
                label: "Add to queue".into(),
                action: add,
            },
        ],
        cursor: 0,
        prompt: None,
        error: String::new(),
        filter: String::new(),
        filtering: false,
    });
    Action::None
}

pub fn key(app: &mut App, key: KeyEvent) -> Option<Action> {
    use crossterm::event::KeyModifiers;

    if app.music.filtering {
        match key.code {
            KeyCode::Esc => {
                app.music.filtering = false;
                app.music.filter_input.clear();
            }
            KeyCode::Enter => {
                let search = app.music.filter_input.trim().to_owned();
                app.music.filtering = false;
                app.music.filter_input.clear();
                let target = match app.music.page.target.clone() {
                    Target::Library {
                        kind,
                        favorite,
                        order,
                        ..
                    } => Target::Library {
                        kind,
                        offset: 0,
                        favorite,
                        search: (!search.is_empty()).then_some(search),
                        order,
                    },
                    _ => return Some(Action::None),
                };
                return Some(app.music.replace(target));
            }
            KeyCode::Backspace => {
                app.music.filter_input.pop();
            }
            KeyCode::Char(c)
                if !c.is_control()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && app.music.filter_input.len() + c.len_utf8() <= 128 =>
            {
                app.music.filter_input.push(c);
            }
            _ => {}
        }
        return Some(Action::None);
    }

    match key.code {
        KeyCode::Backspace => {
            app.music.back();
            Some(Action::None)
        }
        KeyCode::Char('f') | KeyCode::Char('F')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            let filter_input = match &app.music.page.target {
                Target::Library { search, .. } => search.clone().unwrap_or_default(),
                _ => return None,
            };
            app.music.filter_input = filter_input;
            app.music.filtering = true;
            Some(Action::None)
        }
        KeyCode::Char('f') if key.modifiers.is_empty() => {
            let action = app
                .music
                .page
                .items
                .get(app.music.page.cursor)
                .and_then(Media::favorite_action);
            Some(action.unwrap_or_else(|| {
                app.status = "This item cannot be a favourite".into();
                Action::None
            }))
        }
        KeyCode::Char('r') => Some(app.music.reload()),
        KeyCode::Char('[') => {
            if app.music.loading {
                return Some(Action::None);
            }
            let previous = match app.music.page.target.clone() {
                Target::Library {
                    kind,
                    offset,
                    favorite,
                    search,
                    order,
                } if offset >= PAGE_SIZE => Target::Library {
                    kind,
                    offset: offset - PAGE_SIZE,
                    favorite,
                    search,
                    order,
                },
                Target::Library { .. } => {
                    app.status = "Already on the first page".into();
                    return Some(Action::None);
                }
                _ => return None,
            };
            if app
                .music
                .history
                .last()
                .is_some_and(|page| page.target == previous)
            {
                app.music.back();
                Some(Action::None)
            } else {
                Some(app.music.replace(previous))
            }
        }
        KeyCode::Char(']') => {
            if app.music.loading {
                return Some(Action::None);
            }
            Some(if let Some(target) = app.music.page.next.clone() {
                app.music.navigate(target, app.music.page.title.clone())
            } else {
                Action::None
            })
        }
        KeyCode::Char('o') => {
            let target = match app.music.page.target.clone() {
                Target::Library {
                    kind,
                    favorite,
                    search,
                    order,
                    ..
                } => {
                    let order = order.next(kind);
                    app.status = format!("sort: {}", order.label());
                    Target::Library {
                        kind,
                        offset: 0,
                        favorite,
                        search,
                        order,
                    }
                }
                _ => return None,
            };
            Some(app.music.replace(target))
        }
        KeyCode::Char('x') if key.modifiers.is_empty() => {
            if !app.music.loading {
                if let Some(media) = app.music.page.items.get(app.music.page.cursor) {
                    if media.selectable_track() {
                        let selected = &mut app.music.page.selected;
                        if !selected.remove(&media.uri) {
                            selected.insert(media.uri.clone());
                        }
                    } else {
                        app.status = "Only available tracks can be selected".into();
                    }
                }
            }
            Some(Action::None)
        }
        KeyCode::Char('A') => {
            if app.music.page.selected.is_empty() {
                app.status = "No tracks selected".into();
                return Some(Action::None);
            }
            if app.music.loading {
                return Some(Action::None);
            }
            let mut uris = Vec::with_capacity(app.music.page.selected.len());
            for media in &app.music.page.items {
                if media.selectable_track()
                    && app.music.page.selected.contains(&media.uri)
                    && !uris.contains(&media.uri)
                {
                    uris.push(media.uri.clone());
                }
            }
            if uris.is_empty() {
                app.status = "No tracks selected".into();
                Some(Action::None)
            } else {
                let title = format!("{} selected tracks", uris.len());
                Some(queue_choices(
                    app,
                    &title,
                    Action::PlayMany(uris.clone()),
                    Action::EnqueueMany(uris),
                ))
            }
        }
        KeyCode::Enter | KeyCode::Char('P') | KeyCode::Char('a') | KeyCode::Char('N') => {
            if app.music.loading {
                return Some(Action::None);
            }
            let Some(media) = app.music.page.items.get(app.music.page.cursor).cloned() else {
                return Some(Action::None);
            };
            if key.code == KeyCode::Enter {
                if let Some(target) = media.open.clone() {
                    return Some(app.music.navigate(target, media.title));
                }
            }
            if matches!(key.code, KeyCode::Enter | KeyCode::Char('P')) {
                return Some(choose(app, &media));
            }
            if !queue_player_available(app) {
                app.status = "Select an available speaker first".into();
                return Some(Action::None);
            }
            if key.code == KeyCode::Char('a') {
                if media.kind == "folder" {
                    if let Some(target) = media.open {
                        return Some(queue_choices(
                            app,
                            &media.title,
                            Action::PlayFolder(target.clone()),
                            Action::EnqueueFolder(target),
                        ));
                    }
                } else if matches!(media.kind.as_str(), "album" | "playlist")
                    && media.available
                    && media.playable
                {
                    return Some(queue_choices(
                        app,
                        &media.title,
                        Action::Play(media.uri.clone()),
                        Action::Enqueue(media.uri),
                    ));
                }
            }
            Some(if media.available && media.playable {
                if key.code == KeyCode::Char('a') {
                    Action::Enqueue(media.uri)
                } else {
                    Action::PlayNext(media.uri)
                }
            } else {
                app.status = "This item is not available for playback".into();
                Action::None
            })
        }
        _ => None,
    }
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let browser = &app.music;
    let palette = app.palette;
    let library = match &browser.page.target {
        Target::Library {
            offset,
            search,
            order,
            ..
        } => {
            let filter = search.as_ref().map_or_else(String::new, |search| {
                format!(" · filter \"{}\"", clean(search))
            });
            format!(
                " · page {}{}{} · sort: {}{}",
                *offset / PAGE_SIZE + 1,
                if *offset >= PAGE_SIZE {
                    " · [ prev"
                } else {
                    ""
                },
                if browser.page.next.is_some() {
                    " · ] next"
                } else {
                    ""
                },
                order.label(),
                filter,
            )
        }
        _ => String::new(),
    };
    let selected = if browser.page.selected.is_empty() {
        String::new()
    } else {
        format!(" · {} selected", browser.page.selected.len())
    };
    let area = crate::ui::heading(
        frame,
        area,
        palette,
        &format!("MUSIC · {}{}{}", browser.page.title, selected, library),
        app.focus == Focus::Music,
    );
    if browser.filtering {
        frame.render_widget(
            Paragraph::new(format!(
                "Filter: {}▏  [Enter apply · Esc cancel]",
                browser.filter_input
            )),
            area,
        );
        return;
    }
    if browser.loading || !browser.error.is_empty() || browser.page.items.is_empty() {
        let message = if browser.loading {
            "Loading music…"
        } else if !browser.error.is_empty() {
            &browser.error
        } else {
            "No items here. Try another category, browse providers, or / search."
        };
        frame.render_widget(Paragraph::new(format!("{message}\n\nEnter opens collections · P chooses playback\nBackspace goes back · r retries")).wrap(ratatui::widgets::Wrap {trim:false}),area);
        return;
    }
    let items = browser.page.items.iter().map(|m| {
        let marker = if m.selectable_track() {
            if browser.page.selected.contains(&m.uri) {
                "[x]"
            } else {
                "[ ]"
            }
        } else {
            "   "
        };
        ListItem::new(vec![
            Line::from(format!(
                "{} {} {}{}{}",
                marker,
                if m.open.is_some() { "›" } else { "♪" },
                m.title,
                if m.favorite { " ♥" } else { "" },
                if m.available { "" } else { " [unavailable]" }
            )),
            Line::styled(
                format!("  {}", m.detail),
                Style::default().fg(palette.secondary),
            ),
        ])
    });
    let mut state = ListState::default().with_selected(Some(browser.page.cursor));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol("▸ ")
            .highlight_style(Style::default().fg(palette.accent).bg(palette.selection)),
        area,
        &mut state,
    );
}
