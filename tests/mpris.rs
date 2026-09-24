use ma_tui::{
    controls::Command,
    mpris::{self, LoopStatus, MprisCommand, NowPlaying, PlaybackStatus, NO_TRACK},
    ui::{Action, App, PlayerView},
};
use serde_json::json;

#[test]
fn selected_playing_player_maps_to_mpris_snapshot() {
    let app = App {
        connected: true,
        selected_id: Some("living-room".into()),
        players: vec![PlayerView {
            details: json!({"volume_level": 37}),
            id: "living-room".into(),
            name: "Living room".into(),
            state: "playing".into(),
            available: true,
            ..Default::default()
        }],
        queue_id: "living-room".into(),
        queue_details: json!({
            "shuffle_enabled": true,
            "repeat_mode": "one",
            "current_item": {
                "queue_item_id": "item-1",
                "media_item": {
                    "uri": "library://track/1",
                    "artists": [{"name": "A"}, {"name": "B"}],
                    "album": {"name": "X"},
                    "metadata": {"images": [{"proxy_id": "cover123"}]}
                }
            }
        }),
        title: "The Track".into(),
        artist: "Display artist · X".into(),
        elapsed: 12.5,
        duration: 240.0,
        ..App::default()
    };

    let now = mpris::snapshot(&app, "https://ma.example:8095");

    assert_eq!(now.player_name, "Living room");
    assert_eq!(now.track_id, "/io/github/brdweb/MaTui/track/item_2D1");
    assert_eq!(now.playback_status, PlaybackStatus::Playing);
    assert_eq!(now.title, "The Track");
    assert_eq!(now.artist, vec!["A".to_owned(), "B".to_owned()]);
    assert_eq!(now.album, "X");
    assert_eq!(now.length, Some(240_000_000));
    assert_eq!(now.position, 12_500_000);
    assert_eq!(now.volume, Some(0.37));
    assert_eq!(now.shuffle, Some(true));
    assert_eq!(now.loop_status, Some(LoopStatus::Track));
    assert_eq!(
        now.art_url.as_deref(),
        Some("https://ma.example:8095/imageproxy/cover123?size=256&fmt=jpg")
    );
}

#[test]
fn no_selected_player_has_no_track() {
    let app = App {
        queue_details: json!({
            "current_item": {"queue_item_id": "stale-queue-item"}
        }),
        ..App::default()
    };
    let now = mpris::snapshot(&app, "https://ma.example:8095");

    assert_eq!(now.playback_status, PlaybackStatus::Stopped);
    assert_eq!(now.track_id, NO_TRACK);
}

#[test]
fn selected_player_without_a_queue_item_is_stopped() {
    let app = App {
        connected: true,
        selected_id: Some("living-room".into()),
        players: vec![PlayerView {
            id: "living-room".into(),
            state: "playing".into(),
            available: true,
            ..Default::default()
        }],
        title: "Stale track".into(),
        duration: 240.0,
        ..App::default()
    };
    let now = mpris::snapshot(&app, "https://ma.example:8095");

    assert_eq!(now.playback_status, PlaybackStatus::Stopped);
    assert_eq!(now.track_id, NO_TRACK);
    assert!(now.title.is_empty());
    assert_eq!(now.length, None);
}

fn seekable() -> NowPlaying {
    NowPlaying {
        track_id: "/io/github/brdweb/MaTui/track/current".into(),
        length: Some(100_000_000),
        position: 50_000_000,
        can_control: true,
        can_seek: true,
        can_go_next: true,
        can_go_previous: true,
        ..NowPlaying::default()
    }
}

#[test]
fn stale_set_position_is_ignored() {
    assert_eq!(
        mpris::action(
            MprisCommand::SetPosition {
                track_id: "/io/github/brdweb/MaTui/track/stale".into(),
                position: 10_000_000,
            },
            &seekable(),
            "queue",
        ),
        None
    );
}

#[test]
fn seek_is_clamped_to_the_current_length() {
    let now = seekable();
    assert_eq!(
        mpris::action(MprisCommand::Seek(-100_000_000), &now, "queue"),
        Some(Action::Seek(0.0))
    );
    assert_eq!(
        mpris::action(MprisCommand::Seek(100_000_000), &now, "queue"),
        Some(Action::Seek(100.0))
    );
}

#[test]
fn volume_maps_to_the_server_percentage() {
    let now = NowPlaying {
        can_control: true,
        ..NowPlaying::default()
    };

    assert_eq!(
        mpris::action(MprisCommand::SetVolume(0.37), &now, "queue"),
        Some(Action::Command(Command::Player {
            name: "volume_set",
            args: json!({"volume_level": 37}),
        }))
    );
}

#[test]
fn track_loop_maps_to_repeat_one() {
    let now = NowPlaying {
        can_control: true,
        loop_status: Some(LoopStatus::None),
        ..NowPlaying::default()
    };

    assert_eq!(
        mpris::action(
            MprisCommand::SetLoopStatus(LoopStatus::Track),
            &now,
            "queue",
        ),
        Some(Action::Command(Command::Queue {
            id: "queue".into(),
            name: "repeat",
            args: json!({"repeat_mode": "one"}),
        }))
    );
}

#[test]
fn empty_queue_has_no_shuffle_mode() {
    let now = mpris::snapshot(&App::default(), "https://ma.example:8095");

    assert_eq!(now.shuffle, None);
    assert_eq!(
        mpris::action(MprisCommand::SetShuffle(true), &now, ""),
        None
    );
}
