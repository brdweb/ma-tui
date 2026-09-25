use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ma_tui::{
    music::{Browser, Kind, Media, Order, Target, PAGE_SIZE},
    ui::{Action, App, Focus, PlayerView, TrackView},
};
use serde_json::json;

fn press(app: &mut App, key: KeyCode) -> Action {
    app.key(KeyEvent::new(key, KeyModifiers::NONE))
}
fn track() -> Media {
    Media::parse(
        &json!({"name":"Fixture song","uri":"library://track/1","media_type":"track"}),
        "",
    )
}
fn connected() -> App {
    App {
        connected: true,
        selected_id: Some("speaker".into()),
        players: vec![PlayerView {
            id: "speaker".into(),
            name: "Fixture speaker".into(),
            available: true,
            ..Default::default()
        }],
        focus: Focus::Music,
        ..Default::default()
    }
}

fn named_track(id: &str) -> Media {
    Media::parse(
        &json!({
            "name": format!("Fixture {id}"),
            "uri": format!("library://track/{id}"),
            "media_type": "track"
        }),
        "",
    )
}

fn assert_queue_menu(app: &mut App, key: KeyCode, replace: Action, add: Action) {
    assert_eq!(press(app, key), Action::None);
    let menu = app.menu.as_ref().expect("queue choices open");
    assert_eq!(menu.player, app.selected_id);
    assert_eq!(menu.entries.len(), 2);
    assert!(menu.entries.iter().all(|entry| entry.action.needs_player()));
    assert_eq!(menu.entries[0].label, "Replace queue");
    assert_eq!(menu.entries[0].action, replace);
    assert_eq!(menu.entries[1].label, "Add to queue");
    assert_eq!(menu.entries[1].action, add);
    assert_eq!(press(app, KeyCode::Enter), replace);
    assert!(app.menu.is_none());
    assert_eq!(press(app, key), Action::None);
    press(app, KeyCode::Down);
    assert_eq!(press(app, KeyCode::Enter), add);
    assert!(app.menu.is_none());
}

#[test]
fn navigation_is_read_only_and_back_rejects_late_responses() {
    let mut app = App::default();
    press(&mut app, KeyCode::Char('b'));
    // What you were in the middle of leads the home listing.
    let action = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        action,
        Action::Browse {
            target: Target::InProgress,
            ..
        }
    ));
    // The shelves lead, in the order they are most reached for.
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Down);
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Action::Browse {
            target: Target::UnplayedEpisodes,
            ..
        }
    ));
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Down);
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Action::Browse {
            target: Target::RecentlyAdded,
            ..
        }
    ));
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Down);
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Action::Browse {
            target: Target::RecentlyPlayed,
            ..
        }
    ));
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Down);
    let action = press(&mut app, KeyCode::Enter);
    assert!(
        matches!(
            action,
            Action::Browse {
                target: Target::Library {
                    kind: Kind::Playlists,
                    ..
                },
                ..
            }
        ),
        "the libraries follow the shelves"
    );
    let generation = app.music.generation;
    press(&mut app, KeyCode::Backspace);
    app.music.apply(generation, Ok((vec![track()], None)));
    assert_eq!(app.music.page.target, Target::Home);
    assert!(!app.music.loading);
    assert!(app.menu.is_none());
}

#[test]
fn collections_open_and_playback_menu_identifies_speaker_and_queue_behavior() {
    let mut app = connected();
    let album = Media::parse(
        &json!({"name":"Fixture album","item_id":"album1","provider":"library","media_type":"album","uri":"library://album/album1"}),
        "",
    );
    app.music.page.items = vec![album];
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Action::Browse {
            target: Target::Album { .. },
            ..
        }
    ));
    app.music.back();
    assert_eq!(press(&mut app, KeyCode::Char('P')), Action::None);
    assert!(app.menu.as_ref().unwrap().title.contains("Fixture speaker"));
    assert!(app.menu.as_ref().unwrap().entries[0]
        .label
        .contains("replace queue"));
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        Action::Play("library://album/album1".into())
    );
    app.music.page.items = vec![track()];
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        Action::PlayNext("library://track/1".into())
    );
    assert_eq!(
        press(&mut app, KeyCode::Char('a')),
        Action::Enqueue("library://track/1".into())
    );
    assert_eq!(
        press(&mut app, KeyCode::Char('N')),
        Action::PlayNext("library://track/1".into())
    );
}

#[test]
fn unavailable_media_and_player_changes_cannot_submit_playback() {
    let mut app = connected();
    let mut media = track();
    media.available = false;
    app.music.page.items = vec![media];
    assert_eq!(press(&mut app, KeyCode::Enter), Action::None);
    let menu = app
        .menu
        .take()
        .expect("unavailable media can still be edited in the library");
    assert!(
        !menu.entries.iter().any(|entry| matches!(
            entry.action,
            Action::Play(_) | Action::PlayNext(_) | Action::Enqueue(_)
        )),
        "an unavailable item offers no playback command"
    );
    app.music.page.items = vec![track()];
    press(&mut app, KeyCode::Enter);
    app.connected = false;
    assert_eq!(press(&mut app, KeyCode::Enter), Action::None);
    assert_eq!(press(&mut app, KeyCode::Char('a')), Action::None);
}

#[test]
fn search_collections_open_in_browser_and_tracks_offer_actions() {
    let mut app = connected();
    app.focus = Focus::Search;
    app.results = vec![TrackView {
        media: Some(Media::parse(
            &json!({"name":"Playlist","media_type":"playlist","item_id":"list","provider":"library","uri":"library://playlist/list"}),
            "",
        )),
        ..Default::default()
    }];
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Action::Browse {
            target: Target::Playlist { .. },
            ..
        }
    ));
    assert!(app.focus == Focus::Music);
    app.focus = Focus::Search;
    app.results[0].media = Some(track());
    assert_eq!(press(&mut app, KeyCode::Enter), Action::None);
    assert!(app.menu.is_some());
}

#[test]
fn folders_are_not_playable_and_terminal_controls_are_removed() {
    let folder = Media::parse(
        &json!({"name":"Folder\n\u{1b}","media_type":"folder","path":"provider://browse/abc","uri":"provider://folder/abc"}),
        "",
    );
    assert_eq!(folder.title, "Folder");
    assert!(!folder.playable);
    assert_eq!(folder.favorite_action(), None);
    assert_eq!(
        folder.open,
        Some(Target::Providers {
            path: Some("provider://browse/abc".into())
        })
    );
}

#[test]
fn selecting_tracks_toggles_by_uri_and_offers_queue_choices_in_visible_order() {
    let mut app = connected();
    app.music.page.items = vec![
        named_track("first"),
        named_track("second"),
        named_track("third"),
        named_track("second"),
    ];

    app.music.page.cursor = 2;
    assert_eq!(press(&mut app, KeyCode::Char('x')), Action::None);
    press(&mut app, KeyCode::Home);
    assert_eq!(press(&mut app, KeyCode::Char('x')), Action::None);
    press(&mut app, KeyCode::Down);
    assert_eq!(press(&mut app, KeyCode::Char('x')), Action::None);
    let uris = vec![
        "library://track/first".into(),
        "library://track/second".into(),
        "library://track/third".into(),
    ];
    assert_queue_menu(
        &mut app,
        KeyCode::Char('A'),
        Action::PlayMany(uris.clone()),
        Action::EnqueueMany(uris),
    );

    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Char('x'));
    let uris = vec![
        "library://track/first".into(),
        "library://track/third".into(),
    ];
    assert_queue_menu(
        &mut app,
        KeyCode::Char('A'),
        Action::PlayMany(uris.clone()),
        Action::EnqueueMany(uris),
    );
    assert_eq!(app.music.page.selected.len(), 2);
}

#[test]
fn selected_track_menu_rechecks_the_selected_speaker_before_submission() {
    let mut app = connected();
    app.music.page.items = vec![named_track("one")];
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(press(&mut app, KeyCode::Char('A')), Action::None);
    assert_eq!(
        app.menu.as_ref().unwrap().player.as_deref(),
        Some("speaker")
    );

    app.selected_id = Some("different".into());
    assert_eq!(press(&mut app, KeyCode::Enter), Action::None);
    assert!(app.menu.is_none());
    assert!(app.status.contains("Player disconnected"));
}

#[test]
fn selection_survives_reload_only_for_still_playable_visible_tracks() {
    let mut app = connected();
    app.music
        .navigate(Target::RecentlyAdded, "Recently added".into());
    app.music.apply(
        app.music.generation,
        Ok((
            vec![
                named_track("gone"),
                named_track("kept"),
                named_track("unavailable"),
            ],
            None,
        )),
    );
    for index in 0..3 {
        app.music.page.cursor = index;
        press(&mut app, KeyCode::Char('x'));
    }
    assert_eq!(app.music.page.selected.len(), 3);

    app.music.reload();
    assert_eq!(press(&mut app, KeyCode::Char('A')), Action::None);
    app.music
        .apply(app.music.generation, Err("Fixture listing failed".into()));
    assert_eq!(
        app.music.page.selected.len(),
        3,
        "failed loads do not prune"
    );
    app.music.reload();
    let mut unavailable = named_track("unavailable");
    unavailable.available = false;
    app.music.apply(
        app.music.generation,
        Ok((vec![unavailable, named_track("kept")], None)),
    );
    assert_eq!(app.music.page.selected.len(), 1);
    assert!(app.music.page.selected.contains("library://track/kept"));
    let uris = vec!["library://track/kept".into()];
    assert_queue_menu(
        &mut app,
        KeyCode::Char('A'),
        Action::PlayMany(uris.clone()),
        Action::EnqueueMany(uris),
    );
}

#[test]
fn browser_history_restores_selection_but_new_and_replaced_pages_start_empty() {
    let mut app = connected();
    app.music
        .navigate(Target::RecentlyAdded, "Recently added".into());
    app.music.apply(
        app.music.generation,
        Ok((vec![named_track("first"), named_track("second")], None)),
    );
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char('x'));

    app.music
        .navigate(Target::RecentlyPlayed, "Recently played".into());
    assert!(
        app.music.page.selected.is_empty(),
        "navigation starts empty"
    );
    app.music
        .apply(app.music.generation, Ok((vec![named_track("other")], None)));
    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.music.page.cursor, 1);
    let uris = vec!["library://track/second".into()];
    assert_queue_menu(
        &mut app,
        KeyCode::Char('A'),
        Action::PlayMany(uris.clone()),
        Action::EnqueueMany(uris),
    );
    assert_eq!(app.music.page.selected.len(), 1);

    app.music.replace(Target::RecentlyAdded);
    assert!(
        app.music.page.selected.is_empty(),
        "replacement starts empty"
    );
    assert_eq!(press(&mut app, KeyCode::Char('A')), Action::None);
}

#[test]
fn multi_selection_ignores_collections_folders_and_unavailable_tracks() {
    let mut app = connected();
    let folder = Media::parse(
        &json!({"name":"Folder","media_type":"folder","path":"provider://browse/folder","uri":"provider://folder/1","is_playable":true}),
        "",
    );
    let mut unavailable = named_track("unavailable");
    unavailable.available = false;
    let mut unplayable = named_track("unplayable");
    unplayable.playable = false;
    app.music.page.items = vec![
        folder,
        Media::parse(
            &json!({"name":"Album","media_type":"album","uri":"library://album/1"}),
            "",
        ),
        Media::parse(
            &json!({"name":"Playlist","media_type":"playlist","uri":"library://playlist/1"}),
            "",
        ),
        unavailable,
        unplayable,
        named_track("good"),
    ];
    for index in 0..5 {
        app.music.page.cursor = index;
        assert_eq!(press(&mut app, KeyCode::Char('x')), Action::None);
        assert!(app.music.page.selected.is_empty());
    }
    app.music.page.cursor = 5;
    press(&mut app, KeyCode::Char('x'));
    let uris = vec!["library://track/good".into()];
    assert_queue_menu(
        &mut app,
        KeyCode::Char('A'),
        Action::PlayMany(uris.clone()),
        Action::EnqueueMany(uris),
    );
}

#[test]
fn queueing_without_a_selection_reports_it_without_queueing() {
    let mut app = connected();
    app.music.page.items = vec![named_track("one")];
    assert_eq!(press(&mut app, KeyCode::Char('A')), Action::None);
    assert!(
        app.status.contains("selected") && app.status.len() < 80,
        "the reason must be concise and about selection, not speaker playback"
    );
    assert!(app.music.page.selected.is_empty());

    press(&mut app, KeyCode::Char('x'));
    app.connected = false;
    assert_eq!(press(&mut app, KeyCode::Char('A')), Action::None);
    assert!(app.menu.is_none());
    assert!(
        app.music.page.selected.contains("library://track/one"),
        "losing the speaker must not discard the local selection"
    );
}

#[test]
fn folder_and_collection_queue_choices_preserve_browse_target_and_uri() {
    let mut app = connected();
    let path = "provider://browse/folder";
    let folder = Media::parse(
        &json!({"name":"Folder","media_type":"folder","path":path,"uri":"provider://folder/1","available":false}),
        "",
    );
    assert!(!folder.playable);
    app.music.page.items = vec![folder];
    let generation = app.music.generation;
    let target = Target::Providers {
        path: Some(path.into()),
    };
    assert_queue_menu(
        &mut app,
        KeyCode::Char('a'),
        Action::PlayFolder(target.clone()),
        Action::EnqueueFolder(target),
    );
    assert_eq!(
        app.music.generation, generation,
        "UI does not load children"
    );
    app.selected_id = None;
    assert_eq!(press(&mut app, KeyCode::Char('a')), Action::None);
    assert!(app.menu.is_none());
    app.selected_id = Some("speaker".into());
    assert_eq!(press(&mut app, KeyCode::Char('N')), Action::None);

    for (kind, uri) in [
        ("album", "library://album/album1"),
        ("playlist", "library://playlist/list1"),
    ] {
        app.music.page.items = vec![Media::parse(
            &json!({"name":"Collection","media_type":kind,"uri":uri,"item_id":"1","provider":"library"}),
            "",
        )];
        assert_queue_menu(
            &mut app,
            KeyCode::Char('a'),
            Action::Play(uri.into()),
            Action::Enqueue(uri.into()),
        );
    }
    app.connected = false;
    assert_eq!(press(&mut app, KeyCode::Char('a')), Action::None);
    assert!(app.menu.is_none());
}

#[test]
fn music_rows_show_selection_independently_of_cursor_highlight() {
    let mut app = connected();
    app.music.page.title = "Tracks".into();
    app.music.page.items = vec![named_track("one"), named_track("two")];
    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Down);

    let width = 60;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 8))
        .expect("create test terminal");
    terminal
        .draw(|frame| ma_tui::music::draw(frame, &app, frame.area()))
        .expect("render browser");
    let buffer = terminal.backend().buffer();
    let lines: Vec<String> = buffer
        .content
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    assert!(lines.iter().any(|line| line.contains("1 selected")));
    assert!(
        lines.iter().any(|line| line.contains("[x] ♪ Fixture one")),
        "selected row keeps its marker when the cursor leaves"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("▸ [ ] ♪ Fixture two")),
        "cursor highlighting is a separate visual state"
    );
}

#[test]
fn favorites_keep_library_identity_and_open_without_a_speaker() {
    let library = Media::parse(
        &json!({
            "name":"Library track",
            "item_id":"library-track",
            "provider":"library",
            "media_type":"track",
            "uri":"library://track/library-track",
            "favorite":true,
        }),
        "",
    );
    assert!(library.favorite);
    assert!(library.in_library);
    assert_eq!(
        library.favorite_action(),
        Some(Action::Favorite {
            uri: "library://track/library-track".into(),
            media_type: "track".into(),
            library_id: Some("library-track".into()),
            favorite: false,
        })
    );
    assert_eq!(library.library_action(), None);

    let provider = Media::parse(
        &json!({
            "name":"Provider track",
            "item_id":"provider-track",
            "provider":"spotify",
            "media_type":"track",
            "uri":"spotify://track/provider-track",
        }),
        "",
    );
    assert!(!provider.favorite);
    assert!(!provider.in_library);
    assert_eq!(
        provider.favorite_action(),
        Some(Action::Favorite {
            uri: "spotify://track/provider-track".into(),
            media_type: "track".into(),
            library_id: None,
            favorite: true,
        })
    );
    assert_eq!(
        provider.library_action(),
        Some(Action::AddToLibrary {
            uri: "spotify://track/provider-track".into(),
        })
    );

    let folder = Media::folder("Folder", Target::Home);
    assert!(!folder.favorite);
    assert!(!folder.in_library);

    let mut app = App::default();
    assert_eq!(ma_tui::music::choose(&mut app, &provider), Action::None);
    let menu = app
        .menu
        .as_ref()
        .expect("library edits open without a speaker");
    assert!(menu.player.is_none());
    let labels: Vec<&str> = menu
        .entries
        .iter()
        .map(|entry| entry.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["Add to favourites", "Add to library", "Add to playlist…"]
    );
}

#[test]
fn choose_offers_radio_and_playlist_actions_for_supported_media() {
    let album = Media::parse(
        &json!({"name":"Fixture album","item_id":"album1","provider":"library",
                "media_type":"album","uri":"library://album/album1"}),
        "",
    );
    let episode = Media::parse(
        &json!({"name":"Fixture episode","item_id":"episode1","provider":"library",
                "media_type":"podcast_episode","uri":"library://podcast_episode/episode1"}),
        "",
    );

    let mut app = connected();
    ma_tui::music::choose(&mut app, &track());
    assert_eq!(
        app.menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.label == "Start radio")
            .map(|entry| entry.action.clone()),
        Some(Action::StartRadio("library://track/1".into()))
    );

    let mut app = connected();
    ma_tui::music::choose(&mut app, &album);
    assert_eq!(
        app.menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.label == "Start radio")
            .map(|entry| entry.action.clone()),
        Some(Action::StartRadio("library://album/album1".into()))
    );

    let mut app = connected();
    ma_tui::music::choose(&mut app, &episode);
    assert!(
        !app.menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.label == "Start radio"),
        "podcast episodes are not radio seeds"
    );

    let mut app = App::default();
    ma_tui::music::choose(&mut app, &track());
    assert_eq!(
        app.menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.label == "Add to playlist…")
            .map(|entry| entry.action.clone()),
        Some(Action::LoadPlaylists {
            uri: "library://track/1".into(),
        })
    );

    for media in [
        album,
        Media::parse(
            &json!({"name":"Fixture artist","item_id":"artist1","provider":"library",
                    "media_type":"artist","uri":"library://artist/artist1"}),
            "",
        ),
    ] {
        let mut app = App::default();
        ma_tui::music::choose(&mut app, &media);
        assert!(
            !app.menu
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .any(|entry| entry.label == "Add to playlist…"),
            "{} cannot be added to playlists",
            media.kind
        );
    }
}

#[test]
fn failed_listing_can_retry_and_pagination_back_restores_position() {
    let mut browser = Browser::default();
    let target = Target::Library {
        kind: Kind::Tracks,
        offset: 0,
        favorite: false,
        search: None,
        order: Order::Name,
    };
    browser.navigate(target.clone(), "Tracks".into());
    browser.apply(browser.generation, Err("Unavailable".into()));
    assert!(!browser.loading);
    assert_eq!(browser.error, "Unavailable");
    browser.reload();
    let next = Target::Library {
        kind: Kind::Tracks,
        offset: 100,
        favorite: false,
        search: None,
        order: Order::Name,
    };
    browser.apply(
        browser.generation,
        Ok((vec![track(), track()], Some(next.clone()))),
    );
    browser.page.cursor = 1;
    browser.navigate(next, "Tracks".into());
    browser.back();
    assert_eq!(browser.page.target, target);
    assert_eq!(browser.page.cursor, 1);
    assert!(browser.error.is_empty());
}

#[test]
fn page_back_uses_history_or_reloads_in_place() {
    let mut app = App {
        focus: Focus::Music,
        ..Default::default()
    };
    let first = Target::Library {
        kind: Kind::Tracks,
        offset: 0,
        favorite: true,
        search: Some("ambient".into()),
        order: Order::RecentlyAdded,
    };
    let second = Target::Library {
        kind: Kind::Tracks,
        offset: PAGE_SIZE,
        favorite: true,
        search: Some("ambient".into()),
        order: Order::RecentlyAdded,
    };
    app.music.navigate(first.clone(), "Tracks".into());
    app.music.apply(
        app.music.generation,
        Ok((vec![track()], Some(second.clone()))),
    );
    assert!(matches!(
        press(&mut app, KeyCode::Char(']')),
        Action::Browse { target, .. } if target == second
    ));
    app.music
        .apply(app.music.generation, Ok((vec![track()], None)));

    assert_eq!(press(&mut app, KeyCode::Char('[')), Action::None);
    assert_eq!(app.music.page.target, first);
    assert_eq!(
        app.music.history.len(),
        1,
        "the cached first page was restored"
    );
    assert!(!app.music.loading);

    app.music.history.clear();
    app.music.page.target = second.clone();
    app.music.page.title = "Tracks".into();
    app.music.page.items = vec![track()];
    app.music.page.next = None;
    assert!(matches!(
        press(&mut app, KeyCode::Char('[')),
        Action::Browse { target, .. } if target == first
    ));
    assert_eq!(app.music.page.target, first);
    assert!(
        app.music.history.is_empty(),
        "reloading did not create history"
    );
    assert!(app.music.loading);

    app.music
        .apply(app.music.generation, Ok((vec![track()], None)));
    assert_eq!(press(&mut app, KeyCode::Char('[')), Action::None);
    assert_eq!(app.status, "Already on the first page");
}

#[test]
fn sort_cycles_valid_orders_in_place() {
    for (order, key) in [
        (Order::Name, "sort_name"),
        (Order::RecentlyAdded, "timestamp_added_desc"),
        (Order::LastPlayed, "last_played_desc"),
        (Order::MostPlayed, "play_count_desc"),
        (Order::Year, "year_desc"),
        (Order::Random, "random"),
    ] {
        assert_eq!(order.order_by(), key);
    }
    let mut app = App {
        focus: Focus::Music,
        ..Default::default()
    };
    app.music.page.target = Target::Library {
        kind: Kind::Artists,
        offset: PAGE_SIZE,
        favorite: false,
        search: Some("ambient".into()),
        order: Order::Name,
    };
    app.music.page.title = "Artists".into();
    let mut orders = Vec::new();
    for _ in 0..5 {
        let action = press(&mut app, KeyCode::Char('o'));
        let Action::Browse {
            target:
                Target::Library {
                    offset,
                    search,
                    order,
                    ..
                },
            ..
        } = action
        else {
            panic!("sort reloads the library");
        };
        assert_eq!(offset, 0);
        assert_eq!(search.as_deref(), Some("ambient"));
        orders.push(order);
    }
    assert_eq!(
        orders,
        vec![
            Order::RecentlyAdded,
            Order::LastPlayed,
            Order::MostPlayed,
            Order::Random,
            Order::Name,
        ]
    );
    assert_eq!(app.status, "sort: name");
    assert!(
        app.music.history.is_empty(),
        "sorting does not create history"
    );

    app.music.page.target = Target::Library {
        kind: Kind::Tracks,
        offset: 0,
        favorite: false,
        search: None,
        order: Order::MostPlayed,
    };
    assert!(matches!(
        press(&mut app, KeyCode::Char('o')),
        Action::Browse {
            target: Target::Library {
                order: Order::Year,
                ..
            },
            ..
        }
    ));
    assert_eq!(app.status, "sort: year");
}

#[test]
fn filter_input_replaces_library_page_and_cancels_without_change() {
    let mut app = App {
        focus: Focus::Music,
        ..Default::default()
    };
    app.music.page.target = Target::Library {
        kind: Kind::Albums,
        offset: PAGE_SIZE,
        favorite: false,
        search: None,
        order: Order::MostPlayed,
    };
    app.music.page.title = "Albums".into();
    app.music.page.items = vec![track(), track()];
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(app.music.page.selected.len(), 1);
    let ctrl_f = || KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(app.key(ctrl_f()), Action::None);
    assert!(app.music.filtering);
    assert_eq!(app.music.page.selected.len(), 1, "editing keeps selection");
    for c in "new".chars() {
        assert_eq!(press(&mut app, KeyCode::Char(c)), Action::None);
    }
    app.paste(" filter");
    assert_eq!(app.music.filter_input, "new filter");
    let generation = app.music.generation;
    let cursor = app.music.page.cursor;
    assert_eq!(press(&mut app, KeyCode::PageDown), Action::None);
    assert_eq!(
        app.music.generation, generation,
        "filtering starts no reload"
    );
    assert_eq!(
        app.music.page.cursor, cursor,
        "filtering swallows pane navigation"
    );

    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Action::Browse {
            target: Target::Library {
                offset: 0,
                search: Some(search),
                order: Order::MostPlayed,
                ..
            },
            ..
        } if search == "new filter"
    ));
    let applied = app.music.page.target.clone();
    assert!(!app.music.filtering);
    assert!(app.music.filter_input.is_empty());
    assert!(
        app.music.page.selected.is_empty(),
        "applying a filter replaces the page"
    );
    assert!(
        app.music.history.is_empty(),
        "filtering does not create history"
    );

    assert_eq!(app.key(ctrl_f()), Action::None);
    assert_eq!(app.music.filter_input, "new filter");
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(press(&mut app, KeyCode::Esc), Action::None);
    assert!(!app.music.filtering);
    assert_eq!(app.music.page.target, applied);
}

#[test]
fn recently_played_shelf_parses_item_mappings() {
    let browser = Browser::default();
    let shelf = browser
        .page
        .items
        .iter()
        .find(|item| item.title == "Recently played")
        .expect("home has a recently played shelf");
    assert_eq!(shelf.open, Some(Target::RecentlyPlayed));
    assert_eq!(
        ma_tui::music::demo_listing(&Target::RecentlyPlayed)[0].kind,
        "track"
    );

    let item = Media::parse(
        &json!({
            "item_id":"track-1",
            "provider":"library",
            "name":"Played fixture",
            "media_type":"track",
            "uri":"library://track/track-1",
        }),
        "",
    );
    assert_eq!(item.id, "track-1");
    assert_eq!(item.provider, "library");
    assert_eq!(item.kind, "track");
    assert_eq!(item.title, "Played fixture");
    assert!(item.playable);
}

/// Podcasts open into episodes; audiobooks deliberately do not, because MA
/// 2.10.2 has no chapter model to open into.
#[test]
fn podcasts_open_into_episodes_and_audiobooks_stay_a_single_item() {
    let podcast = Media::parse(
        &json!({"name":"A show","item_id":"p1","provider":"audiobookshelf",
                "media_type":"podcast","uri":"library://podcast/p1"}),
        "",
    );
    assert_eq!(
        podcast.open,
        Some(Target::Podcast {
            id: "p1".into(),
            provider: "audiobookshelf".into()
        })
    );
    assert!(podcast.playable, "a podcast can still be played whole");

    let audiobook = Media::parse(
        &json!({"name":"A book","item_id":"b1","provider":"audiobookshelf",
                "media_type":"audiobook","uri":"library://audiobook/b1"}),
        "",
    );
    assert_eq!(audiobook.open, None, "an audiobook has no chapter listing");
    assert!(audiobook.playable);
    assert_eq!(audiobook.id, "b1");
    assert_eq!(audiobook.provider, "audiobookshelf");
}

/// Progress is shown when the provider reports it, and nothing is invented
/// when it does not: unknown is not the same as unplayed.
#[test]
fn listening_progress_is_shown_only_when_the_server_reports_it() {
    let episode = |extra: serde_json::Value| {
        let mut item = json!({"name":"Episode 1","item_id":"e1","provider":"abs",
                              "media_type":"podcast_episode","uri":"library://podcast_episode/e1"});
        for (key, value) in extra.as_object().unwrap() {
            item[key] = value.clone();
        }
        Media::parse(&item, "podcast_episode")
    };

    let unknown = episode(json!({}));
    assert!(!unknown.fully_played);
    assert_eq!(unknown.resume_ms, None);
    assert!(
        !unknown.detail.contains("resume") && !unknown.detail.contains("played"),
        "an unreported progress state claims nothing: {}",
        unknown.detail
    );

    let finished = episode(json!({"fully_played": true}));
    assert!(finished.fully_played);
    assert!(finished.detail.contains("played"));

    let partway = episode(json!({"resume_position_ms": 724_000}));
    assert_eq!(partway.resume_ms, Some(724_000));
    assert!(
        partway.detail.contains("resume 12:04"),
        "minutes and seconds: {}",
        partway.detail
    );

    // An audiobook resume point runs to hours, so it is not shown as minutes.
    let deep = episode(json!({"resume_position_ms": 9_305_000}));
    assert!(
        deep.detail.contains("resume 2:35:05"),
        "hours are kept: {}",
        deep.detail
    );

    // A finished item says so rather than also offering a resume point.
    let both = episode(json!({"fully_played": true, "resume_position_ms": 5_000}));
    assert!(both.detail.contains("played") && !both.detail.contains("resume"));
}

/// Marking progress is a library edit, so it is offered without a speaker and
/// names the item the way Music Assistant expects.
#[test]
fn progress_can_be_marked_without_a_speaker_selected() {
    let episode = Media::parse(
        &json!({"name":"Episode 1","item_id":"e1","provider":"abs",
                "media_type":"podcast_episode","uri":"library://podcast_episode/e1"}),
        "",
    );
    // No speaker: playback entries are impossible, the progress ones are not.
    let mut app = App::default();
    assert_eq!(ma_tui::music::choose(&mut app, &episode), Action::None);
    let menu = app.menu.as_ref().expect("a progress menu still opens");
    assert!(menu.player.is_none());
    let labels: Vec<&str> = menu.entries.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "Mark as played",
            "Mark as not played",
            "Add to favourites",
            "Add to library",
            "Add to playlist…",
        ]
    );
    assert_eq!(
        menu.entries[0].action,
        Action::MarkPlayed {
            item: json!({
                "item_id": "e1",
                "provider": "abs",
                "name": "Episode 1",
                "media_type": "podcast_episode",
            }),
            played: true,
        },
        "the four fields ItemMapping requires, and no guesses beyond them"
    );

    // With a speaker, playback comes first and progress is still offered.
    let mut app = connected();
    ma_tui::music::choose(&mut app, &episode);
    let menu = app.menu.as_ref().unwrap();
    assert!(menu.entries[0].label.contains("Play now"));
    assert!(menu.entries.iter().any(|e| e.label == "Mark as played"));

    // A plain track keeps no listening position, so it is not offered one.
    let mut app = connected();
    ma_tui::music::choose(&mut app, &track());
    assert!(!app
        .menu
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .any(|e| e.label.starts_with("Mark")));
}

/// Progress events arrive while an audiobook plays, so re-reading the listing
/// is throttled and only happens where it would show.
#[test]
fn a_progress_event_refreshes_at_most_one_listing_at_a_time() {
    use std::time::{Duration, Instant};
    let now = Instant::now();

    // A track listing shows no progress, so nothing is re-read.
    let mut browser = Browser::default();
    browser.navigate(
        Target::Library {
            kind: Kind::Tracks,
            offset: 0,
            favorite: false,
            search: None,
            order: Order::Name,
        },
        "Tracks".into(),
    );
    browser.apply(browser.generation, Ok((vec![track()], None)));
    assert_eq!(browser.progress_changed(now), None);

    let mut browser = Browser::default();
    browser.navigate(Target::RecentlyPlayed, "Recently played".into());
    browser.apply(browser.generation, Ok((vec![track()], None)));
    assert!(
        browser.progress_changed(now).is_some(),
        "recently played follows listening activity"
    );

    // A podcast listing does.
    let mut browser = Browser::default();
    browser.navigate(Target::InProgress, "Continue listening".into());
    browser.apply(browser.generation, Ok((vec![track()], None)));
    assert!(
        browser.progress_changed(now).is_some(),
        "a shelf built from progress re-reads"
    );
    browser.apply(browser.generation, Ok((vec![track()], None)));
    assert_eq!(
        browser.progress_changed(now + Duration::from_millis(500)),
        None,
        "a burst of events does not become a burst of requests"
    );
    browser.apply(browser.generation, Ok((vec![track()], None)));
    assert!(
        browser
            .progress_changed(now + Duration::from_secs(4))
            .is_some(),
        "but it does catch up once the throttle expires"
    );
}

#[test]
fn unplayed_episodes_refresh_on_progress_changes_and_keep_the_existing_throttle() {
    use std::time::{Duration, Instant};
    let now = Instant::now();
    let mut browser = Browser::default();
    browser.navigate(Target::UnplayedEpisodes, "Unplayed podcasts".into());
    browser.apply(browser.generation, Ok((vec![track()], None)));

    assert_eq!(
        browser.progress_changed(now),
        Some(Action::Browse {
            generation: browser.generation,
            target: Target::UnplayedEpisodes,
        })
    );
    assert_eq!(
        browser.progress_changed(now + Duration::from_secs(4)),
        None,
        "an outstanding shelf refresh cannot start a second batch of requests"
    );
    browser.apply(browser.generation, Ok((vec![], None)));
    assert!(browser.page.items.is_empty(), "a played episode is removed");
    assert_eq!(
        browser.progress_changed(now + Duration::from_millis(500)),
        None,
        "progress bursts remain throttled"
    );
    assert!(
        browser
            .progress_changed(now + Duration::from_secs(4))
            .is_some(),
        "a later change refreshes even when the shelf is now empty"
    );
}

/// A row says what the item belongs to, which is a different question per
/// media type — and never a provider instance id, which means nothing.
#[test]
fn a_row_names_the_show_the_album_or_the_author_but_never_a_provider_id() {
    let episode = Media::parse(
        &json!({"name":"Episode 12","item_id":"e1","provider":"audiobookshelf--zdGFJfeu",
                "media_type":"podcast_episode","uri":"library://podcast_episode/e1",
                "podcast":{"name":"The Cavan Sullivan Show"}}),
        "",
    );
    assert_eq!(episode.detail, "The Cavan Sullivan Show");

    let track = Media::parse(
        &json!({"name":"A Song","item_id":"t1","provider":"library","media_type":"track",
                "uri":"library://track/t1","artists":[{"name":"An Artist"}],
                "album":{"name":"An Album"}}),
        "",
    );
    assert_eq!(track.detail, "An Artist · An Album");

    // Audiobook authors may be plain strings rather than objects.
    let book = Media::parse(
        &json!({"name":"A Book","item_id":"b1","provider":"abs","media_type":"audiobook",
                "uri":"library://audiobook/b1","authors":["An Author"],
                "resume_position_ms":9_305_000}),
        "",
    );
    assert_eq!(book.detail, "An Author · resume 2:35:05");

    // With nothing to say, the media type is said readably rather than a
    // provider id being shown in its place.
    let bare = Media::parse(
        &json!({"name":"Something","item_id":"x","provider":"audiobookshelf--zdGFJfeu",
                "media_type":"podcast_episode","uri":"library://podcast_episode/x"}),
        "",
    );
    assert_eq!(bare.detail, "podcast episode");
    assert!(!bare.detail.contains("zdGFJfeu"));
}
