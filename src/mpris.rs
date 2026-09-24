//! Desktop media-key integration. It mirrors the selected MA player; it never owns playback.
use crate::{
    controls::Command,
    ui::{Action, App},
};
use anyhow::Result;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use zbus::{
    connection::Builder,
    interface,
    object_server::{InterfaceRef, SignalEmitter},
    zvariant::{ObjectPath, OwnedValue, Value},
};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.ma_tui";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const TRACK_PATH: &str = "/io/github/brdweb/MaTui/track/";
const MICROSECONDS: i64 = 1_000_000;
const POSITION_JUMP: i64 = 2 * MICROSECONDS;
const ARTWORK_SIZE: u16 = 256;

/// MPRIS's required sentinel when no queue item is current.
pub const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    #[default]
    Stopped,
}

impl PlaybackStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Playing => "Playing",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopStatus {
    #[default]
    None,
    Track,
    Playlist,
}

impl LoopStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Track => "Track",
            Self::Playlist => "Playlist",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "None" => Some(Self::None),
            "Track" => Some(Self::Track),
            "Playlist" => Some(Self::Playlist),
            _ => None,
        }
    }

    fn from_repeat_mode(value: Option<&str>) -> Self {
        match value {
            Some("one") => Self::Track,
            Some("all") => Self::Playlist,
            _ => Self::None,
        }
    }

    fn repeat_mode(self) -> &'static str {
        match self {
            Self::None => "off",
            Self::Track => "one",
            Self::Playlist => "all",
        }
    }
}

/// The complete desktop-facing view of the selected player at one UI tick.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub player_name: String,
    pub track_id: String,
    pub title: String,
    pub artist: Vec<String>,
    pub album: String,
    pub art_url: Option<String>,
    pub length: Option<i64>,
    pub position: i64,
    pub playback_status: PlaybackStatus,
    pub volume: Option<f64>,
    pub shuffle: Option<bool>,
    pub loop_status: Option<LoopStatus>,
    pub can_control: bool,
    pub can_seek: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
}

impl Default for NowPlaying {
    fn default() -> Self {
        Self {
            player_name: String::new(),
            track_id: NO_TRACK.into(),
            title: String::new(),
            artist: vec![],
            album: String::new(),
            art_url: None,
            length: None,
            position: 0,
            playback_status: PlaybackStatus::Stopped,
            volume: None,
            shuffle: None,
            loop_status: None,
            can_control: false,
            can_seek: false,
            can_go_next: false,
            can_go_previous: false,
        }
    }
}

/// Requests received on D-Bus, kept separate from UI actions until the UI can
/// validate the currently displayed queue identity.
#[derive(Clone, Debug, PartialEq)]
pub enum MprisCommand {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Seek(i64),
    SetPosition { track_id: String, position: i64 },
    SetVolume(f64),
    SetShuffle(bool),
    SetLoopStatus(LoopStatus),
}

/// Read the MPRIS snapshot only from state already rendered by the interface.
pub fn snapshot(app: &App, server_base: &str) -> NowPlaying {
    let selected = app
        .selected_id
        .as_ref()
        .and_then(|id| app.players.iter().find(|player| &player.id == id));
    let player = selected.filter(|player| player.available);
    let can_control = app.connected && player.is_some();
    let item = &app.queue_details["current_item"];
    let item_track_id = selected.map(|_| track_path(item));
    let has_track = item_track_id
        .as_deref()
        .is_some_and(|track_id| track_id != NO_TRACK);
    let track_id = item_track_id.unwrap_or_else(|| NO_TRACK.into());
    let length = has_track
        .then(|| duration_microseconds(app.duration))
        .flatten();
    let can_seek = can_control && has_track && length.is_some();
    let queue_modes = !app.queue_id.is_empty() && app.queue_details["is_dynamic"] != true;
    let volume = selected
        .and_then(|player| player.details["volume_level"].as_f64())
        .filter(|volume| volume.is_finite())
        .map(|volume| (volume / 100.0).clamp(0.0, 1.0));

    NowPlaying {
        player_name: selected
            .map(|player| player.name.clone())
            .unwrap_or_default(),
        track_id,
        title: if has_track {
            app.title.clone()
        } else {
            String::new()
        },
        artist: if has_track {
            artists(item, &app.artist)
        } else {
            vec![]
        },
        album: if has_track {
            item["media_item"]["album"]["name"]
                .as_str()
                .unwrap_or_default()
                .into()
        } else {
            String::new()
        },
        art_url: has_track
            .then(|| crate::artwork::proxy_id(item))
            .flatten()
            .and_then(|id| crate::artwork::url(server_base, &id, ARTWORK_SIZE).ok()),
        length,
        position: if has_track {
            seconds_microseconds(app.elapsed)
        } else {
            0
        },
        playback_status: if has_track {
            match player.map(|player| player.state.as_str()) {
                Some("playing") => PlaybackStatus::Playing,
                Some("paused") => PlaybackStatus::Paused,
                _ => PlaybackStatus::Stopped,
            }
        } else {
            PlaybackStatus::Stopped
        },
        volume,
        shuffle: queue_modes.then(|| {
            app.queue_details["shuffle_enabled"]
                .as_bool()
                .unwrap_or(false)
        }),
        loop_status: queue_modes
            .then(|| LoopStatus::from_repeat_mode(app.queue_details["repeat_mode"].as_str())),
        can_control,
        can_seek,
        can_go_next: can_control,
        can_go_previous: can_control,
    }
}

/// Convert a D-Bus request to the action already understood by the controller.
pub fn action(command: MprisCommand, now: &NowPlaying, queue_id: &str) -> Option<Action> {
    match command {
        MprisCommand::PlayPause if now.can_control => Some(Action::Toggle),
        MprisCommand::Play if now.can_control => player_action("play"),
        MprisCommand::Pause if now.can_control => player_action("pause"),
        MprisCommand::Stop if now.can_control => player_action("stop"),
        MprisCommand::Next if now.can_go_next => Some(Action::Next),
        MprisCommand::Previous if now.can_go_previous => Some(Action::Previous),
        MprisCommand::Seek(offset) => seek(now, now.position.saturating_add(offset)),
        MprisCommand::SetPosition { track_id, position } if track_id == now.track_id => {
            seek(now, position)
        }
        MprisCommand::SetVolume(volume) if now.can_control && volume.is_finite() => {
            let volume_level = (volume.clamp(0.0, 1.0) * 100.0).round() as u8;
            Some(Action::Command(Command::Player {
                name: "volume_set",
                args: json!({"volume_level": volume_level}),
            }))
        }
        MprisCommand::SetShuffle(enabled)
            if now.can_control && !queue_id.is_empty() && now.shuffle.is_some() =>
        {
            Some(Action::Command(Command::Queue {
                id: queue_id.into(),
                name: "shuffle",
                args: json!({"shuffle_enabled": enabled}),
            }))
        }
        MprisCommand::SetLoopStatus(status)
            if now.can_control && !queue_id.is_empty() && now.loop_status.is_some() =>
        {
            Some(Action::Command(Command::Queue {
                id: queue_id.into(),
                name: "repeat",
                args: json!({"repeat_mode": status.repeat_mode()}),
            }))
        }
        _ => None,
    }
}

fn player_action(name: &'static str) -> Option<Action> {
    Some(Action::Command(Command::Player {
        name,
        args: json!({}),
    }))
}

fn seek(now: &NowPlaying, position: i64) -> Option<Action> {
    let length = now.length?;
    if !now.can_seek || length < 0 {
        return None;
    }
    Some(Action::Seek(
        position.clamp(0, length) as f64 / MICROSECONDS as f64,
    ))
}

fn track_path(item: &serde_json::Value) -> String {
    let id = [
        item["queue_item_id"].as_str(),
        item["item_id"].as_str(),
        item["media_item"]["uri"].as_str(),
        item["uri"].as_str(),
    ]
    .into_iter()
    .flatten()
    .find(|id| !id.is_empty());
    let Some(id) = id else {
        return NO_TRACK.into();
    };

    let mut path = String::with_capacity(TRACK_PATH.len() + id.len());
    path.push_str(TRACK_PATH);
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() {
            path.push(byte as char);
        } else {
            path.push('_');
            path.push(hex(byte >> 4));
            path.push(hex(byte & 0x0f));
        }
    }
    path
}

fn artists(item: &serde_json::Value, fallback: &str) -> Vec<String> {
    let artists = item["media_item"]["artists"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artist| artist["name"].as_str())
        .filter(|artist| !artist.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if artists.is_empty() && !fallback.is_empty() {
        vec![fallback.to_owned()]
    } else {
        artists
    }
}

fn hex(value: u8) -> char {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    HEX[value as usize] as char
}

fn duration_microseconds(seconds: f64) -> Option<i64> {
    (seconds.is_finite() && seconds > 0.0).then(|| seconds_microseconds(seconds))
}

fn seconds_microseconds(seconds: f64) -> i64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * MICROSECONDS as f64).min(i64::MAX as f64) as i64
}

struct MediaPlayer2;

#[interface(name = "org.mpris.MediaPlayer2")]
impl MediaPlayer2 {
    async fn raise(&self) {}

    async fn quit(&self) {}

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn fullscreen(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn set_fullscreen(&self, _fullscreen: bool) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "MA-TUI has no fullscreen interface".into(),
        ))
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_set_fullscreen(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn identity(&self) -> String {
        "MA-TUI".into()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn desktop_entry(&self) -> String {
        "ma-tui".into()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec![]
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![]
    }
}

struct PositionClock {
    playback_status: PlaybackStatus,
    position: i64,
    length: Option<i64>,
    updated_at: Instant,
}

impl PositionClock {
    fn new(snapshot: &NowPlaying) -> Self {
        Self {
            playback_status: snapshot.playback_status,
            position: snapshot.position,
            length: snapshot.length,
            updated_at: Instant::now(),
        }
    }

    fn update(&mut self, snapshot: &NowPlaying, updated_at: Instant) {
        self.playback_status = snapshot.playback_status;
        self.position = snapshot.position;
        self.length = snapshot.length;
        self.updated_at = updated_at;
    }

    fn position(&self, now: Instant) -> i64 {
        estimated_position_at(
            self.playback_status,
            self.position,
            self.length,
            self.updated_at,
            now,
        )
    }
}

struct Player {
    commands: mpsc::UnboundedSender<MprisCommand>,
    state: watch::Receiver<NowPlaying>,
    position: Arc<Mutex<PositionClock>>,
}

impl Player {
    fn now(&self) -> NowPlaying {
        self.state.borrow().clone()
    }

    fn estimated_position(&self) -> i64 {
        let clock = match self.position.lock() {
            Ok(clock) => clock,
            Err(poisoned) => poisoned.into_inner(),
        };
        clock.position(Instant::now())
    }

    fn send(&self, command: MprisCommand) -> zbus::fdo::Result<()> {
        self.commands
            .send(command)
            .map_err(|_| zbus::fdo::Error::Failed("MPRIS command receiver stopped".into()))
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    async fn next(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Next)
    }

    async fn previous(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Previous)
    }

    async fn pause(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Pause)
    }

    async fn play_pause(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::PlayPause)
    }

    async fn stop(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Stop)
    }

    async fn play(&self) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Play)
    }

    async fn seek(&self, offset: i64) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::Seek(offset))
    }

    async fn set_position(&self, track_id: ObjectPath<'_>, position: i64) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::SetPosition {
            track_id: track_id.as_str().into(),
            position,
        })
    }

    async fn open_uri(&self, _uri: String) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "MA-TUI cannot open arbitrary URIs".into(),
        ))
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.now().playback_status.as_str().into()
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        self.now().loop_status.unwrap_or_default().as_str().into()
    }

    #[zbus(property)]
    async fn set_loop_status(&self, loop_status: String) -> zbus::fdo::Result<()> {
        let loop_status = LoopStatus::parse(&loop_status).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs("LoopStatus must be None, Track, or Playlist".into())
        })?;
        self.send(MprisCommand::SetLoopStatus(loop_status))
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn set_rate(&self, _rate: f64) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "MA-TUI playback rate is fixed".into(),
        ))
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.now().shuffle.unwrap_or(false)
    }

    #[zbus(property)]
    async fn set_shuffle(&self, shuffle: bool) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::SetShuffle(shuffle))
    }

    #[zbus(property)]
    fn metadata(&self) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        metadata(&self.now())
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.now().volume.unwrap_or(0.0)
    }

    #[zbus(property)]
    async fn set_volume(&self, volume: f64) -> zbus::fdo::Result<()> {
        self.send(MprisCommand::SetVolume(volume))
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        self.estimated_position()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.now().can_go_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.now().can_go_previous
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.now().can_control
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.now().can_control
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.now().can_seek
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        self.now().can_control
    }

    #[zbus(signal)]
    async fn seeked(signal_emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}

fn metadata(now: &NowPlaying) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
    let path = ObjectPath::try_from(now.track_id.as_str())
        .map_err(|_| zbus::fdo::Error::Failed("Invalid MPRIS track identifier".into()))?;
    let mut values = HashMap::new();
    values.insert("mpris:trackid".into(), OwnedValue::from(path));
    if now.track_id == NO_TRACK {
        return Ok(values);
    }
    if !now.title.is_empty() {
        values.insert(
            "xesam:title".into(),
            metadata_value(Value::new(now.title.clone()))?,
        );
    }
    if !now.artist.is_empty() {
        values.insert(
            "xesam:artist".into(),
            metadata_value(Value::new(now.artist.clone()))?,
        );
    }
    if !now.album.is_empty() {
        values.insert(
            "xesam:album".into(),
            metadata_value(Value::new(now.album.clone()))?,
        );
    }
    if let Some(length) = now.length {
        values.insert("mpris:length".into(), OwnedValue::from(length));
    }
    if let Some(art_url) = &now.art_url {
        values.insert(
            "mpris:artUrl".into(),
            metadata_value(Value::new(art_url.clone()))?,
        );
    }
    Ok(values)
}

fn metadata_value(value: Value<'static>) -> zbus::fdo::Result<OwnedValue> {
    OwnedValue::try_from(value).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

/// Holds the session-bus name and the state watcher for as long as MA-TUI runs.
pub struct Mpris {
    connection: zbus::Connection,
    task: JoinHandle<()>,
}

impl Mpris {
    pub async fn start(
        commands: mpsc::UnboundedSender<MprisCommand>,
        state: watch::Receiver<NowPlaying>,
        notifications: bool,
    ) -> Result<Self> {
        let position = Arc::new(Mutex::new(PositionClock::new(&state.borrow())));
        let connection = Builder::session()?
            .name(BUS_NAME)?
            .serve_at(OBJECT_PATH, MediaPlayer2)?
            .serve_at(
                OBJECT_PATH,
                Player {
                    commands,
                    state: state.clone(),
                    position: position.clone(),
                },
            )?
            .build()
            .await?;
        let player = connection
            .object_server()
            .interface::<_, Player>(OBJECT_PATH)
            .await?;
        let task = tokio::spawn(watch_state(
            player,
            state,
            position,
            connection.clone(),
            notifications,
        ));
        Ok(Self { connection, task })
    }

    pub async fn shutdown(self) {
        let Self { connection, task } = self;
        task.abort();
        let _ = task.await;
        drop(connection);
    }
}

async fn watch_state(
    player: InterfaceRef<Player>,
    mut state: watch::Receiver<NowPlaying>,
    position: Arc<Mutex<PositionClock>>,
    connection: zbus::Connection,
    notifications: bool,
) {
    let mut previous = state.borrow_and_update().clone();
    let mut previous_at = Instant::now();
    let mut notification_id = 0;
    while state.changed().await.is_ok() {
        let next = state.borrow_and_update().clone();
        let updated_at = Instant::now();
        let expected = estimated_position(&previous, previous_at, updated_at);
        let track_changed = next.track_id != previous.track_id;
        let jumped = next.position.abs_diff(expected) > POSITION_JUMP as u64;
        {
            let mut clock = match position.lock() {
                Ok(clock) => clock,
                Err(poisoned) => poisoned.into_inner(),
            };
            clock.update(&next, updated_at);
        }
        {
            let interface = player.get().await;
            if emit_changes(&interface, player.signal_emitter(), &previous, &next)
                .await
                .is_err()
            {
                return;
            }
        }
        if (track_changed || jumped)
            && Player::seeked(player.signal_emitter(), next.position)
                .await
                .is_err()
        {
            return;
        }
        if notifications
            && track_changed
            && next.track_id != NO_TRACK
            && next.playback_status == PlaybackStatus::Playing
        {
            if let Some(id) = notify(&connection, notification_id, &next).await {
                notification_id = id;
            }
        }
        previous = next;
        previous_at = updated_at;
    }
}

async fn emit_changes(
    player: &Player,
    emitter: &SignalEmitter<'_>,
    previous: &NowPlaying,
    next: &NowPlaying,
) -> zbus::Result<()> {
    if previous.playback_status != next.playback_status {
        player.playback_status_changed(emitter).await?;
    }
    if metadata_changed(previous, next) {
        player.metadata_changed(emitter).await?;
    }
    if previous.volume != next.volume {
        player.volume_changed(emitter).await?;
    }
    if previous.shuffle != next.shuffle {
        player.shuffle_changed(emitter).await?;
    }
    if previous.loop_status != next.loop_status {
        player.loop_status_changed(emitter).await?;
    }
    if previous.can_go_next != next.can_go_next {
        player.can_go_next_changed(emitter).await?;
    }
    if previous.can_go_previous != next.can_go_previous {
        player.can_go_previous_changed(emitter).await?;
    }
    if previous.can_control != next.can_control {
        player.can_play_changed(emitter).await?;
        player.can_pause_changed(emitter).await?;
        player.can_control_changed(emitter).await?;
    }
    if previous.can_seek != next.can_seek {
        player.can_seek_changed(emitter).await?;
    }
    Ok(())
}

fn metadata_changed(previous: &NowPlaying, next: &NowPlaying) -> bool {
    previous.track_id != next.track_id
        || previous.title != next.title
        || previous.artist != next.artist
        || previous.album != next.album
        || previous.art_url != next.art_url
        || previous.length != next.length
}

fn estimated_position(snapshot: &NowPlaying, updated_at: Instant, now: Instant) -> i64 {
    estimated_position_at(
        snapshot.playback_status,
        snapshot.position,
        snapshot.length,
        updated_at,
        now,
    )
}

fn estimated_position_at(
    playback_status: PlaybackStatus,
    position: i64,
    length: Option<i64>,
    updated_at: Instant,
    now: Instant,
) -> i64 {
    let mut position = position.max(0);
    if playback_status == PlaybackStatus::Playing {
        let elapsed = now.saturating_duration_since(updated_at).as_micros();
        let elapsed = elapsed.min(i64::MAX as u128) as i64;
        position = position.saturating_add(elapsed);
    }
    length
        .map(|length| position.min(length.max(0)))
        .unwrap_or(position)
}

async fn notify(connection: &zbus::Connection, replaces_id: u32, now: &NowPlaying) -> Option<u32> {
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    .ok()?;
    let artist = now.artist.join(", ");
    let body = match (artist.is_empty(), now.album.is_empty()) {
        (false, false) => format!("{artist} — {}", now.album),
        (false, true) => artist,
        (true, false) => now.album.clone(),
        (true, true) => String::new(),
    };
    proxy
        .call(
            "Notify",
            &(
                "MA-TUI",
                replaces_id,
                "",
                now.title.as_str(),
                body.as_str(),
                Vec::<String>::new(),
                HashMap::<String, OwnedValue>::new(),
                -1i32,
            ),
        )
        .await
        .ok()
}
