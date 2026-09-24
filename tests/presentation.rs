use ma_tui::{
    api::{Player, Queue},
    controller::Update,
    controls::{InputKind, PromptTarget},
    music::{Kind, Media, Order, Target},
    presentation::apply,
    ui::{Action, App, PlayerView, TrackView},
};

#[test]
fn stale_queue_cannot_overwrite_new_player_and_highlight_survives_reorder() {
    let mut app = App {
        selected_id: Some("new".into()),
        title: "New player".into(),
        players: vec![PlayerView {
            id: "highlighted".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    apply(
        &mut app,
        Update::Queue(
            "old".into(),
            Ok(Queue {
                current_title: "Old track".into(),
                ..Default::default()
            }),
        ),
    );
    assert_eq!(app.title, "New player");
    apply(
        &mut app,
        Update::Players(vec![
            Player {
                id: "other".into(),
                ..Default::default()
            },
            Player {
                id: "highlighted".into(),
                ..Default::default()
            },
        ]),
    );
    assert_eq!(app.player_cursor, 1);
    apply(&mut app, Update::Offline("Connection failed".into()));
    assert!(!app.connected);
    assert!(app.status.contains("stale"));
}

#[test]
fn metadata_is_not_allowed_to_emit_terminal_control_characters() {
    let mut app = App::default();
    apply(
        &mut app,
        Update::Players(vec![Player {
            id: "id".into(),
            name: "bad\x1b]52;payload\x07\nname".into(),
            ..Default::default()
        }]),
    );
    assert!(!app.players[0].name.chars().any(char::is_control));
    assert_eq!(app.players[0].id, "id");
}

#[test]
fn local_selection_resolves_universal_wrapper_and_preserves_user_selection() {
    use ma_tui::presentation::select_local;
    let mut app = App {
        connected: true,
        players: vec![
            PlayerView {
                id: "remote".into(),
                available: true,
                name: "MA-TUI".into(),
                ..Default::default()
            },
            PlayerView {
                id: "wrapper".into(),
                available: true,
                details: serde_json::json!({"output_protocols":[{"output_protocol_id":"local-sendspin"}]}),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    assert_eq!(
        select_local(&mut app, "local-sendspin"),
        Some("wrapper".into())
    );
    assert_eq!(app.player_cursor, 1);
    app.selected_id = Some("remote".into());
    assert_eq!(select_local(&mut app, "local-sendspin"), None);
    assert_eq!(app.selected_id.as_deref(), Some("remote"));
    app.selected_id = None;
    app.players[1].available = false;
    assert_eq!(select_local(&mut app, "local-sendspin"), None);
    app.players[1].id = "local-sendspin".into();
    app.players[1].available = true;
    app.players[1].details = serde_json::Value::Null;
    assert_eq!(
        select_local(&mut app, "local-sendspin"),
        Some("local-sendspin".into())
    );
}

/// A position from the event stream belongs to one queue, not to whatever
/// happens to be on screen.
#[test]
fn a_position_event_applies_only_to_the_displayed_queue() {
    let mut app = App {
        queue_id: "q1".into(),
        elapsed: 5.0,
        duration: 200.0,
        ..Default::default()
    };
    apply(&mut app, Update::Elapsed("other".into(), 99.0));
    assert_eq!(app.elapsed, 5.0, "another queue's position is not ours");
    assert!(
        app.elapsed_at.is_none(),
        "and it does not re-anchor the clock"
    );

    apply(&mut app, Update::Elapsed("q1".into(), 42.0));
    assert_eq!(app.elapsed, 42.0);
    assert!(
        app.elapsed_at.is_some(),
        "the position carries on from what the server reported"
    );
}

#[test]
fn media_events_update_mapped_browser_search_and_current_rows() {
    let mut app = App::default();
    app.music.page.items = vec![Media::parse(
        &serde_json::json!({
            "name":"Provider track",
            "item_id":"provider-track",
            "provider":"spotify--account",
            "media_type":"track",
            "uri":"spotify://track/provider-track",
        }),
        "",
    )];
    app.results = vec![TrackView {
        media: Some(Media::parse(
            &serde_json::json!({
                "name":"Domain-mapped track",
                "item_id":"provider-track",
                "provider":"spotify",
                "media_type":"track",
                "uri":"spotify://track/domain-track",
            }),
            "",
        )),
        ..Default::default()
    }];
    app.queue_details = serde_json::json!({
        "current_item": {
            "media_item": {
                "name":"Current provider track",
                "item_id":"provider-track",
                "provider":"spotify",
                "media_type":"track",
                "uri":"spotify://track/current-track",
            }
        }
    });
    let item = serde_json::json!({
        "name":"Library track",
        "item_id":"library-track",
        "provider":"library",
        "media_type":"track",
        "uri":"library://track/library-track",
        "favorite":true,
        "provider_mappings":[
            {
                "item_id":"provider-track",
                "provider_instance":"spotify--account",
                "provider_domain":"spotify",
            }
        ],
    });
    assert_eq!(
        apply(
            &mut app,
            Update::MediaItem {
                uri: "library://track/library-track".into(),
                item: Some(item),
            },
        ),
        None
    );
    for media in [
        &app.music.page.items[0],
        app.results[0].media.as_ref().expect("search media"),
    ] {
        assert!(media.favorite);
        assert!(media.in_library);
    }
    assert_eq!(
        app.queue_details["current_item"]["media_item"]["favorite"], true,
        "the F shortcut reads the event-updated queue item"
    );

    apply(
        &mut app,
        Update::MediaItem {
            uri: "spotify://track/provider-track".into(),
            item: None,
        },
    );
    assert!(
        !app.music.page.items[0].in_library,
        "a deletion clears the direct row's library membership"
    );
}

#[test]
fn a_favorite_event_reloads_the_visible_favorites_listing() {
    let target = Target::Library {
        kind: Kind::Tracks,
        offset: 0,
        favorite: true,
        search: None,
        order: Order::Name,
    };
    let mut app = App::default();
    app.music.page.target = target.clone();
    app.music.page.items = vec![Media::parse(
        &serde_json::json!({
            "name":"Library track",
            "item_id":"library-track",
            "provider":"library",
            "media_type":"track",
            "uri":"library://track/library-track",
            "favorite":false,
        }),
        "",
    )];
    assert_eq!(
        apply(
            &mut app,
            Update::MediaItem {
                uri: "library://track/library-track".into(),
                item: Some(serde_json::json!({
                    "name":"Library track",
                    "item_id":"library-track",
                    "provider":"library",
                    "media_type":"track",
                    "uri":"library://track/library-track",
                    "favorite":true,
                    "provider_mappings":[],
                })),
            },
        ),
        Some(Action::Browse {
            generation: 1,
            target,
        })
    );
}

#[test]
fn playlist_updates_open_picker_or_report_the_failure() {
    let playlist = |id, title| {
        Media::parse(
            &serde_json::json!({
                "name":title,
                "item_id":id,
                "provider":"library",
                "media_type":"playlist",
                "uri":format!("library://playlist/{id}"),
            }),
            "playlist",
        )
    };
    let mut app = App {
        selected_id: Some("speaker".into()),
        ..Default::default()
    };
    assert_eq!(
        apply(
            &mut app,
            Update::Playlists {
                uri: "library://track/1".into(),
                result: Ok(vec![playlist("one", "Road trip"), playlist("two", "Work")]),
            },
        ),
        None
    );
    let menu = app.menu.as_ref().expect("playlist picker");
    assert_eq!(menu.title, "Add to playlist");
    assert_eq!(menu.player.as_deref(), Some("speaker"));
    assert_eq!(
        menu.entries
            .iter()
            .map(|entry| (entry.section, entry.label.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("Playlists", "Road trip"),
            ("Playlists", "Work"),
            ("New", "New playlist…")
        ]
    );
    assert!(matches!(
        &menu.entries[0].action,
        Action::AddToPlaylist { playlist_id, uri }
            if playlist_id == "one" && uri == "library://track/1"
    ));
    assert!(matches!(
        &menu.entries[1].action,
        Action::AddToPlaylist { playlist_id, uri }
            if playlist_id == "two" && uri == "library://track/1"
    ));
    assert!(matches!(
        &menu.entries[2].action,
        Action::Prompt(prompt)
            if prompt.label == "New playlist name"
                && prompt.kind == InputKind::Text
                && prompt.value.is_empty()
                && matches!(
                    &prompt.target,
                    PromptTarget::CreatePlaylist { uri: Some(uri) }
                        if uri == "library://track/1"
                )
    ));
    assert_eq!(
        apply(
            &mut app,
            Update::Playlists {
                uri: "library://track/1".into(),
                result: Ok(vec![]),
            },
        ),
        None
    );
    let empty = app.menu.as_ref().expect("new playlist entry");
    assert_eq!(empty.entries.len(), 1);
    assert_eq!(empty.entries[0].section, "New");
    assert_eq!(empty.entries[0].label, "New playlist…");

    let mut failed = App::default();
    apply(
        &mut failed,
        Update::Playlists {
            uri: "library://track/1".into(),
            result: Err("permission denied".into()),
        },
    );
    assert!(failed.menu.is_none());
    assert_eq!(failed.status, "Could not load playlists: permission denied");
}
