use ma_tui::ui;

#[test]
fn selects_available_player_and_routes_controls_only_when_connected() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    let mut app = ui::App::default();
    assert_eq!(app.key(key(KeyCode::Char(' '))), ui::Action::None);
    app.connected = true;
    app.players = vec![
        ui::PlayerView {
            id: "one".into(),
            available: false,
            ..Default::default()
        },
        ui::PlayerView {
            id: "two".into(),
            available: true,
            ..Default::default()
        },
    ];
    assert_eq!(app.key(key(KeyCode::Enter)), ui::Action::None);
    app.key(key(KeyCode::Down));
    assert_eq!(
        app.key(key(KeyCode::Enter)),
        ui::Action::Select("two".into())
    );
    assert_eq!(app.selected_id.as_deref(), Some("two"));
    let volume = |name| {
        ui::Action::Command(ma_tui::controls::Command::Player {
            name,
            args: serde_json::json!({}),
        })
    };
    for (code, action) in [
        // p is play/pause, as in other players; tracks move to < and >.
        (KeyCode::Char(' '), ui::Action::Toggle),
        (KeyCode::Char('p'), ui::Action::Toggle),
        (KeyCode::Char('n'), ui::Action::Next),
        (KeyCode::Char('>'), ui::Action::Next),
        (KeyCode::Char('<'), ui::Action::Previous),
        (KeyCode::Char(','), ui::Action::Previous),
        // Volume steps are server commands, not a read-modify-write.
        (KeyCode::Char('+'), volume("volume_up")),
        (KeyCode::Char('-'), volume("volume_down")),
    ] {
        assert_eq!(app.key(key(code)), action, "{code:?}");
    }
    // Seeking resolves the target from what is on screen.
    assert_eq!(app.key(key(KeyCode::Left)), ui::Action::None);
    assert!(app.status.contains("no seekable duration"));
    app.duration = 100.0;
    app.elapsed = 50.0;
    assert_eq!(app.key(key(KeyCode::Left)), ui::Action::Seek(40.0));
    assert_eq!(app.key(key(KeyCode::Left)), ui::Action::Seek(30.0));
    assert_eq!(app.key(key(KeyCode::Right)), ui::Action::Seek(40.0));
    assert_eq!(app.elapsed, 40.0, "the position on screen follows the seek");
    assert!(
        app.elapsed_at.is_some(),
        "a seek re-anchors the position so it keeps running from there"
    );
    app.elapsed = 0.0;
    assert_eq!(app.key(key(KeyCode::Left)), ui::Action::Seek(0.0));
    app.focus = ui::Focus::Search;
    app.results = vec![ui::TrackView {
        uri: "library://track/1".into(),
        ..Default::default()
    }];
    assert_eq!(
        app.key(key(KeyCode::Char('a'))),
        ui::Action::Enqueue("library://track/1".into())
    );
    assert_eq!(
        app.key(key(KeyCode::Enter)),
        ui::Action::Play("library://track/1".into())
    );
    // Esc leaves search for the browser rather than jumping to the queue.
    app.key(key(KeyCode::Esc));
    assert!(app.focus == ui::Focus::Music);
    app.key(key(KeyCode::F(4)));
    assert!(app.focus == ui::Focus::Queue);
    assert_eq!(app.key(key(KeyCode::Char('r'))), ui::Action::Refresh);
}

#[test]
fn favorite_keys_work_without_a_speaker_or_selected_player() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    let media = ma_tui::music::Media::parse(
        &serde_json::json!({
            "name":"Library track",
            "item_id":"library-track",
            "provider":"library",
            "media_type":"track",
            "uri":"library://track/library-track",
        }),
        "",
    );
    let mut app = ui::App {
        focus: ui::Focus::Music,
        ..Default::default()
    };
    app.music.page.items = vec![media.clone()];
    assert_eq!(
        app.key(key(KeyCode::Char('f'))),
        ui::Action::Favorite {
            uri: "library://track/library-track".into(),
            media_type: "track".into(),
            library_id: Some("library-track".into()),
            favorite: true,
        }
    );

    app.focus = ui::Focus::Search;
    app.results = vec![ui::TrackView {
        media: Some(media),
        ..Default::default()
    }];
    assert_eq!(
        app.key(key(KeyCode::Char('f'))),
        ui::Action::Favorite {
            uri: "library://track/library-track".into(),
            media_type: "track".into(),
            library_id: Some("library-track".into()),
            favorite: true,
        }
    );
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL)),
        ui::Action::None
    );
    assert!(app.menu.is_none(), "Ctrl-F never opens a favourite menu");

    app.connected = true;
    app.focus = ui::Focus::Queue;
    app.queue_details = serde_json::json!({
        "current_item": {
            "media_item": {
                "name":"Current track",
                "item_id":"current-track",
                "provider":"library",
                "media_type":"track",
                "uri":"library://track/current-track",
                "favorite":true,
            }
        }
    });
    assert_eq!(
        app.key(key(KeyCode::Char('F'))),
        ui::Action::Favorite {
            uri: "library://track/current-track".into(),
            media_type: "track".into(),
            library_id: Some("current-track".into()),
            favorite: false,
        }
    );
}

#[test]
fn search_typing_never_triggers_transport_or_quit() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    let mut app = ui::App::default();
    assert_eq!(app.key(key(KeyCode::Char('/'))), ui::Action::None);
    for c in "quiet night".chars() {
        assert_eq!(app.key(key(KeyCode::Char(c))), ui::Action::None);
    }
    assert_eq!(
        app.key(key(KeyCode::Enter)),
        ui::Action::Search("quiet night".into())
    );
    assert_eq!(app.key(key(KeyCode::Char('q'))), ui::Action::Quit);
}

fn search_key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

fn search_result(title: &str, kind: &str, uri: &str) -> ui::TrackView {
    let media = ma_tui::music::Media::parse(
        &serde_json::json!({
            "name": title,
            "item_id": title,
            "provider": "library",
            "media_type": kind,
            "uri": uri,
        }),
        "",
    );
    ui::TrackView {
        media: Some(media),
        uri: format!("display://{title}"),
        title: title.into(),
        ..Default::default()
    }
}

fn ready_search() -> ui::App {
    ui::App {
        connected: true,
        focus: ui::Focus::Search,
        content: ui::Focus::Search,
        selected_id: Some("speaker".into()),
        players: vec![ui::PlayerView {
            id: "speaker".into(),
            available: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn assert_search_queue_menu(
    app: &mut ui::App,
    key: crossterm::event::KeyCode,
    replace: ui::Action,
    add: ui::Action,
) {
    assert_eq!(app.key(search_key(key)), ui::Action::None);
    let menu = app.menu.as_ref().expect("queue choices open");
    assert_eq!(menu.player, app.selected_id);
    assert_eq!(menu.entries.len(), 2);
    assert!(menu.entries.iter().all(|entry| entry.action.needs_player()));
    assert_eq!(menu.entries[0].label, "Replace queue");
    assert_eq!(menu.entries[0].action, replace);
    assert_eq!(menu.entries[1].label, "Add to queue");
    assert_eq!(menu.entries[1].action, add);
    assert_eq!(
        app.key(search_key(crossterm::event::KeyCode::Enter)),
        replace
    );
    assert!(app.menu.is_none());
    assert_eq!(app.key(search_key(key)), ui::Action::None);
    app.key(search_key(crossterm::event::KeyCode::Down));
    assert_eq!(app.key(search_key(crossterm::event::KeyCode::Enter)), add);
    assert!(app.menu.is_none());
}

#[test]
fn search_selection_uses_media_uris_and_offers_queue_choices_in_display_order() {
    use crossterm::event::KeyCode;
    let mut app = ready_search();
    app.results = vec![
        search_result("First", "track", "provider://track/first"),
        search_result("Second", "track", "provider://track/second"),
        search_result("First again", "track", "provider://track/first"),
        search_result("Third", "track", "provider://track/third"),
    ];
    app.search_cursor = 1;
    assert_eq!(app.key(search_key(KeyCode::Char('x'))), ui::Action::None);
    app.key(search_key(KeyCode::Home));
    assert_eq!(app.key(search_key(KeyCode::Char('x'))), ui::Action::None);
    app.key(search_key(KeyCode::End));
    assert_eq!(app.key(search_key(KeyCode::Char('x'))), ui::Action::None);
    assert_eq!(
        app.search_selection,
        [
            "provider://track/first",
            "provider://track/second",
            "provider://track/third"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
    let uris = vec![
        "provider://track/first".into(),
        "provider://track/second".into(),
        "provider://track/third".into(),
    ];
    assert_search_queue_menu(
        &mut app,
        KeyCode::Char('A'),
        ui::Action::PlayMany(uris.clone()),
        ui::Action::EnqueueMany(uris),
    );
    app.key(search_key(KeyCode::Home));
    app.key(search_key(KeyCode::Down));
    assert_eq!(app.key(search_key(KeyCode::Char('x'))), ui::Action::None);
    let uris = vec![
        "provider://track/first".into(),
        "provider://track/third".into(),
    ];
    assert_search_queue_menu(
        &mut app,
        KeyCode::Char('A'),
        ui::Action::PlayMany(uris.clone()),
        ui::Action::EnqueueMany(uris),
    );
}

#[test]
fn search_only_selects_available_playable_tracks_and_requires_a_speaker_to_append() {
    use crossterm::event::KeyCode;
    let folder = ma_tui::music::Media::folder(
        "Folder",
        ma_tui::music::Target::Providers {
            path: Some("provider/folder".into()),
        },
    );
    let mut unavailable = search_result("Unavailable", "track", "provider://track/unavailable");
    unavailable.media.as_mut().unwrap().available = false;
    let mut unplayable = search_result("Unplayable", "track", "provider://track/unplayable");
    unplayable.media.as_mut().unwrap().playable = false;
    let mut app = ready_search();
    app.results = vec![
        search_result("Album", "album", "provider://album/one"),
        search_result("Playlist", "playlist", "provider://playlist/one"),
        ui::TrackView {
            media: Some(folder),
            title: "Folder".into(),
            ..Default::default()
        },
        unavailable,
        unplayable,
        search_result("No media URI", "track", ""),
        ui::TrackView {
            uri: "display://untyped".into(),
            title: "No media".into(),
            ..Default::default()
        },
        search_result("Playable", "track", "provider://track/playable"),
    ];
    for cursor in 0..7 {
        app.search_cursor = cursor;
        assert_eq!(app.key(search_key(KeyCode::Char('x'))), ui::Action::None);
        assert!(
            app.search_selection.is_empty(),
            "row {cursor} is not selectable"
        );
    }
    assert_eq!(app.key(search_key(KeyCode::Char('A'))), ui::Action::None);
    assert!(app.status.contains("Select tracks with x"));
    app.search_cursor = 7;
    app.selected_id = None;
    app.key(search_key(KeyCode::Char('x')));
    assert_eq!(app.search_selection.len(), 1, "selection needs no speaker");
    assert_eq!(app.key(search_key(KeyCode::Char('A'))), ui::Action::None);
    assert!(app.menu.is_none());
    assert!(app.status.contains("Select an available speaker"));
    app.selected_id = Some("speaker".into());
    app.connected = false;
    assert_eq!(app.key(search_key(KeyCode::Char('A'))), ui::Action::None);
    assert!(app.menu.is_none());
    app.connected = true;
    let uris = vec!["provider://track/playable".into()];
    assert_search_queue_menu(
        &mut app,
        KeyCode::Char('A'),
        ui::Action::PlayMany(uris.clone()),
        ui::Action::EnqueueMany(uris),
    );
}

#[test]
fn a_submitted_search_clears_selection_but_cancelling_keeps_it() {
    use crossterm::event::KeyCode;
    let mut app = ready_search();
    app.results = vec![search_result("Old", "track", "provider://track/old")];
    app.key(search_key(KeyCode::Char('x')));
    assert_eq!(app.search_selection.len(), 1);
    app.key(search_key(KeyCode::Char('/')));
    app.key(search_key(KeyCode::Esc));
    assert_eq!(app.search_selection.len(), 1);
    app.key(search_key(KeyCode::Esc));
    assert!(app.focus == ui::Focus::Music);
    app.key(search_key(KeyCode::Tab));
    app.key(search_key(KeyCode::Tab));
    assert!(app.focus == ui::Focus::Search);
    assert_eq!(
        app.search_selection.len(),
        1,
        "pane switches keep the marks"
    );
    app.key(search_key(KeyCode::Char('/')));
    for c in "new search".chars() {
        app.key(search_key(KeyCode::Char(c)));
    }
    assert_eq!(
        app.key(search_key(KeyCode::Enter)),
        ui::Action::Search("new search".into())
    );
    assert!(app.search_selection.is_empty());
    assert_eq!(app.key(search_key(KeyCode::Char('A'))), ui::Action::None);
}

#[test]
fn search_folder_and_collection_queue_choices_preserve_target_and_uri() {
    use crossterm::event::KeyCode;
    let target = ma_tui::music::Target::Providers {
        path: Some("provider/folder".into()),
    };
    let mut app = ready_search();
    app.results = vec![
        ui::TrackView {
            media: Some(ma_tui::music::Media::folder("Folder", target.clone())),
            title: "Folder".into(),
            ..Default::default()
        },
        search_result("Album", "album", "provider://album/one"),
        search_result("Playlist", "playlist", "provider://playlist/one"),
    ];
    assert_search_queue_menu(
        &mut app,
        KeyCode::Char('a'),
        ui::Action::PlayFolder(target.clone()),
        ui::Action::EnqueueFolder(target),
    );
    assert!(
        app.focus == ui::Focus::Search,
        "folder choices do not navigate"
    );
    app.key(search_key(KeyCode::Down));
    assert_search_queue_menu(
        &mut app,
        KeyCode::Char('a'),
        ui::Action::Play("provider://album/one".into()),
        ui::Action::Enqueue("provider://album/one".into()),
    );
    app.key(search_key(KeyCode::Down));
    assert_search_queue_menu(
        &mut app,
        KeyCode::Char('a'),
        ui::Action::Play("provider://playlist/one".into()),
        ui::Action::Enqueue("provider://playlist/one".into()),
    );
    app.selected_id = None;
    app.search_cursor = 0;
    assert_eq!(app.key(search_key(KeyCode::Char('a'))), ui::Action::None);
    assert!(app.status.contains("Select an available speaker"));
    assert!(app.menu.is_none());
}

#[test]
fn search_single_track_a_appends_immediately_and_needs_a_speaker() {
    use crossterm::event::KeyCode;
    let mut app = ready_search();
    app.results = vec![search_result("Song", "track", "provider://track/song")];
    assert_eq!(
        app.key(search_key(KeyCode::Char('a'))),
        ui::Action::Enqueue("provider://track/song".into())
    );
    assert!(app.menu.is_none());

    app.selected_id = None;
    assert_eq!(app.key(search_key(KeyCode::Char('a'))), ui::Action::None);
    assert!(app.menu.is_none());
}

#[test]
fn search_collection_menu_rechecks_the_selected_speaker() {
    use crossterm::event::KeyCode;
    let mut app = ready_search();
    app.results = vec![search_result("Album", "album", "provider://album/one")];
    assert_eq!(app.key(search_key(KeyCode::Char('a'))), ui::Action::None);
    assert_eq!(
        app.menu.as_ref().unwrap().player.as_deref(),
        Some("speaker")
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Enter choose option"));
    assert!(
        !text.contains("Space/p pause/resume"),
        "transport keys are inactive inside a menu"
    );
    app.players[0].available = false;
    assert_eq!(app.key(search_key(KeyCode::Enter)), ui::Action::None);
    assert!(app.menu.is_none());
    assert!(app.status.contains("Player disconnected"));
}

#[test]
fn selected_search_rows_and_shortcuts_are_visible_after_moving_the_cursor() {
    use crossterm::event::KeyCode;
    let mut app = ready_search();
    app.results = vec![
        search_result("Marked track", "track", "provider://track/marked"),
        search_result("Other track", "track", "provider://track/other"),
    ];
    app.key(search_key(KeyCode::Char('x')));
    app.key(search_key(KeyCode::Down));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let rows = terminal
        .backend()
        .buffer()
        .content
        .chunks(110)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>();
    assert!(
        rows.iter()
            .find(|row| row.contains("Marked track"))
            .unwrap()
            .contains('✓'),
        "the selected track has an on-screen marker even off cursor"
    );
    assert!(
        !rows
            .iter()
            .find(|row| row.contains("Other track"))
            .unwrap()
            .contains('✓'),
        "the cursor is not the selection marker"
    );
    for label in [
        "Enter open/play menu",
        "P playback menu",
        "x select track",
        "A selected replace/add menu",
        "a add item or choose queue",
        "Space/p pause/resume",
        "</> previous/next",
        "? controls",
    ] {
        assert!(
            rows.iter().any(|row| row.contains(label)),
            "{label} must be visible in full"
        );
    }
    app.focus = ui::Focus::Music;
    app.content = ui::Focus::Music;
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("A selected replace/add menu"));
    assert!(text.contains("a add item or choose queue"));
}

#[test]
fn renders_disconnected_and_small_terminal_without_panicking() {
    for (width, height) in [(110, 32), (50, 16), (30, 8), (1, 1)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        let mut app = ui::App::default();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        if width >= 50 {
            assert!(text.contains("MA-TUI"));
            assert!(text.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))));
            assert!(text.contains("Disconnected"));
            assert!(text.contains("No player selected"));
        }
    }
}

#[test]
fn music_filter_and_library_title_show_navigation_state() {
    use ma_tui::music::{Kind, Order, Target, PAGE_SIZE};

    let mut app = ui::App {
        focus: ui::Focus::Music,
        content: ui::Focus::Music,
        ..Default::default()
    };
    app.music.page.title = "Tracks".into();
    app.music.page.target = Target::Library {
        kind: Kind::Tracks,
        offset: PAGE_SIZE,
        favorite: false,
        search: Some("ambient".into()),
        order: Order::RecentlyAdded,
    };
    app.music.page.next = Some(Target::Library {
        kind: Kind::Tracks,
        offset: PAGE_SIZE * 2,
        favorite: false,
        search: Some("ambient".into()),
        order: Order::RecentlyAdded,
    });
    app.music.filtering = true;
    app.music.filter_input = "new filter".into();

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(220, 32)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("sort: recently added"));
    assert!(text.contains("filter \"ambient\""));
    assert!(text.contains("[ prev"));
    assert!(text.contains("] next"));
    assert!(text.contains("Filter: new filter"));

    app.music.filtering = false;
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    for label in [
        "Enter open/play menu",
        "P playback menu",
        "r reload",
        "[/] previous/next page",
        "o sort",
        "Ctrl-F filter",
        "</> previous/next",
        "? controls",
    ] {
        assert!(text.contains(label), "{label} must be visible in full");
    }
}

#[test]
fn the_player_spectrum_explains_a_silent_endpoint() {
    let mut app = ui::App {
        spectrum: Some(ma_tui::visualizer::Analyzer::new()),
        connected: true,
        selected_id: Some("kitchen".into()),
        local_endpoint: Some("ma-tui-endpoint".into()),
        players: vec![ui::PlayerView {
            id: "kitchen".into(),
            name: "Kitchen".into(),
            available: true,
            state: "playing".into(),
            ..Default::default()
        }],
        title: "Something".into(),
        ..Default::default()
    };
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        text.contains("no local audio · playing on Kitchen"),
        "a remote speaker must be named as the reason"
    );
    assert!(!text.contains('▀'), "no bars without local samples");
}

/// Shuffle and repeat are one key each, and only where the server reports them.
#[test]
fn shuffle_and_repeat_keys_act_on_the_displayed_queue() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ma_tui::controls::Command;
    use serde_json::json;
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    let mut app = ui::App {
        connected: true,
        selected_id: Some("one".into()),
        players: vec![ui::PlayerView {
            id: "one".into(),
            available: true,
            ..Default::default()
        }],
        focus: ui::Focus::Queue,
        ..Default::default()
    };
    // No queue yet: the key explains instead of guessing an identity.
    assert_eq!(app.key(key(KeyCode::Char('z'))), ui::Action::None);
    assert!(app.status.contains("No active queue"));
    // Stop is a player command and needs no queue identity.
    assert_eq!(
        app.key(key(KeyCode::Char('s'))),
        ui::Action::Command(Command::Player {
            name: "stop",
            args: json!({})
        })
    );

    app.queue_id = "leader".into();
    app.queue_details = json!({"shuffle_enabled": true, "repeat_mode": "all"});
    assert_eq!(
        app.key(key(KeyCode::Char('z'))),
        ui::Action::Command(Command::Queue {
            id: "leader".into(),
            name: "shuffle",
            args: json!({"shuffle_enabled": false})
        })
    );
    // Repeat cycles off → all → one → off.
    for (mode, next) in [("off", "all"), ("all", "one"), ("one", "off")] {
        app.queue_details = json!({ "repeat_mode": mode });
        assert_eq!(
            app.key(key(KeyCode::Char('l'))),
            ui::Action::Command(Command::Queue {
                id: "leader".into(),
                name: "repeat",
                args: json!({ "repeat_mode": next })
            })
        );
    }
    app.queue_details = json!({"is_dynamic": true});
    assert_eq!(app.key(key(KeyCode::Char('z'))), ui::Action::None);
    assert!(app.status.contains("dynamic queue"));
}

#[test]
fn queue_clear_requires_the_focused_active_queue_and_available_speaker() {
    use crossterm::event::KeyCode;
    use ma_tui::controls::Command;
    use serde_json::json;

    let mut app = ready_search();
    app.focus = ui::Focus::Queue;
    assert_eq!(app.key(search_key(KeyCode::Char('c'))), ui::Action::None);
    assert!(app.status.contains("No active queue"));
    app.queue_id = "  ".into();
    assert_eq!(app.key(search_key(KeyCode::Char('c'))), ui::Action::None);

    app.queue_id = "speaker-queue".into();
    let clear = ui::Action::Command(Command::Queue {
        id: "speaker-queue".into(),
        name: "clear",
        args: json!({}),
    });
    assert_eq!(app.key(search_key(KeyCode::Char('c'))), clear);
    app.focus = ui::Focus::Music;
    assert_eq!(app.key(search_key(KeyCode::Char('c'))), ui::Action::None);
    app.focus = ui::Focus::Queue;
    app.selected_id = None;
    assert_eq!(app.key(search_key(KeyCode::Char('c'))), ui::Action::None);
    app.selected_id = Some("speaker".into());
    app.connected = false;
    assert_eq!(app.key(search_key(KeyCode::Char('c'))), ui::Action::None);

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(50, 16)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("c clear queue"));
    assert!(text.contains("? controls"));
}

/// The queue stays on screen while browsing, and the header carries state.
#[test]
fn the_queue_and_browser_are_visible_together_with_playback_state() {
    let mut app = ui::App {
        connected: true,
        selected_id: Some("one".into()),
        queue_id: "leader".into(),
        queue_details: serde_json::json!({"shuffle_enabled": true, "repeat_mode": "one"}),
        players: vec![ui::PlayerView {
            id: "one".into(),
            name: "Kitchen".into(),
            available: true,
            state: "playing".into(),
            volume: Some(42),
            details: serde_json::json!({"volume_muted": true}),
        }],
        queue: vec![ui::TrackView {
            id: "q1".into(),
            title: "Queued track".into(),
            ..Default::default()
        }],
        title: "Now playing this".into(),
        ..Default::default()
    };
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    for expected in [
        // Pane headings are labels now, not boxes.
        "PLAYERS",
        "QUEUE · 1 ITEM",
        "Queued track",
        "MUSIC",
        "Now playing this",
        // The player names where it is playing and how the queue is ordered.
        "NOW PLAYING · KITCHEN",
        "SHUFFLE ON",
        "REPEAT ONE",
        // Nothing here is clickable, so the transport row says the state
        // rather than drawing buttons that cannot be pressed.
        "PLAYING",
        "MUTED",
        // The queue is a table with a state column.
        "#   TITLE",
        "STATE",
    ] {
        assert!(
            text.contains(expected),
            "{expected} missing from the screen"
        );
    }
    // Rules and a column divider separate the panes; nothing is boxed in.
    assert!(
        text.contains('│') && text.contains('─'),
        "the sections are separated by rules"
    );
    for corner in ['┌', '┐', '└', '┘'] {
        assert!(
            !text.contains(corner),
            "{corner}: no pane is drawn as a box any more"
        );
    }
}

/// The current queue item is named as playing; the others show their length.
#[test]
fn the_queue_table_marks_what_is_playing() {
    let mut app = ui::App {
        connected: true,
        selected_id: Some("one".into()),
        queue_id: "leader".into(),
        queue_details: serde_json::json!({"current_item": {"queue_item_id": "q2"}}),
        players: vec![ui::PlayerView {
            id: "one".into(),
            available: true,
            state: "playing".into(),
            ..Default::default()
        }],
        queue: vec![
            ui::TrackView {
                id: "q1".into(),
                title: "First".into(),
                duration: 90.0,
                ..Default::default()
            },
            ui::TrackView {
                id: "q2".into(),
                title: "Second".into(),
                duration: 120.0,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("PLAYING"), "the current item says so");
    assert!(
        text.contains("1:30"),
        "an item that is not playing shows its length instead"
    );
    assert!(
        text.contains("01") && text.contains("02"),
        "rows are numbered"
    );
}

/// A title too wide for its column scrolls rather than being cut off, and one
/// that fits never moves.
#[test]
fn a_long_title_scrolls_and_a_short_one_does_not() {
    let short = ui::marquee("Short", 20, 0);
    assert_eq!(short, ("Short".into(), false));
    assert_eq!(ui::marquee("Short", 20, 999), ("Short".into(), false));

    let long = "A considerably longer track title than the column can hold";
    let (first, scrolling) = ui::marquee(long, 20, 0);
    assert_eq!(first.chars().count(), 20);
    assert!(scrolling, "a title that does not fit is in motion");
    assert!(long.starts_with(&first), "it starts held at the beginning");

    // It holds, then travels, then holds at the other end.
    let (moved, _) = ui::marquee(long, 20, 40);
    assert_ne!(moved, first, "it moves once the opening hold expires");
    let (end, _) = ui::marquee(long, 20, 10_000);
    assert_eq!(end.chars().count(), 20);
    assert!(
        long.ends_with(&end) || end != first,
        "it never runs past the end of the text"
    );
}

#[test]
fn position_runs_between_snapshots_and_every_snapshot_replaces_it() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let mut app = ui::App {
        connected: true,
        selected_id: Some("one".into()),
        players: vec![ui::PlayerView {
            id: "one".into(),
            available: true,
            state: "playing".into(),
            ..Default::default()
        }],
        elapsed: 10.0,
        elapsed_at: Some(start),
        duration: 100.0,
        ..Default::default()
    };
    app.advance(start + Duration::from_secs(3));
    assert_eq!(app.elapsed, 13.0, "a playing position runs with the clock");

    // Pausing freezes the position without banking the paused time.
    app.players[0].state = "paused".into();
    app.advance(start + Duration::from_secs(9));
    assert_eq!(app.elapsed, 13.0, "a paused position holds");
    app.players[0].state = "playing".into();
    app.advance(start + Duration::from_secs(10));
    assert_eq!(
        app.elapsed, 14.0,
        "resuming does not replay the paused time"
    );

    // The estimate never runs past the end of the item.
    app.advance(start + Duration::from_secs(600));
    assert_eq!(app.elapsed, 100.0, "the position stops at the duration");

    // A snapshot is authoritative: drift is replaced, never added to.
    ma_tui::presentation::apply(
        &mut app,
        ma_tui::controller::Update::Queue(
            "one".into(),
            Ok(ma_tui::api::Queue {
                elapsed: 42.0,
                ..Default::default()
            }),
        ),
    );
    assert_eq!(app.elapsed, 42.0, "the server's position wins");

    // Without an anchor there is nothing to carry forward.
    let mut idle = ui::App {
        elapsed: 5.0,
        duration: 100.0,
        ..Default::default()
    };
    idle.advance(Instant::now());
    assert_eq!(idle.elapsed, 5.0);
}

/// The spectrum is part of the player rather than a mode, but only where this
/// run has local audio to analyse and the terminal can spare the rows.
#[test]
fn the_player_carries_the_spectrum_when_there_is_room_for_it() {
    let render = |spectrum: bool, width: u16, height: u16| {
        let mut app = ui::App {
            spectrum: spectrum.then(ma_tui::visualizer::Analyzer::new),
            connected: true,
            selected_id: Some("kitchen".into()),
            players: vec![ui::PlayerView {
                id: "kitchen".into(),
                name: "Kitchen".into(),
                available: true,
                state: "playing".into(),
                ..Default::default()
            }],
            title: "Something".into(),
            ..Default::default()
        };
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };

    // With local audio and room, the strip is there and says why it is empty
    // rather than animating something it does not have.
    let tall = render(true, 110, 30);
    assert!(
        tall.contains("no local audio"),
        "the strip is drawn, and is honest about having no signal"
    );
    assert!(tall.contains("NOW PLAYING"), "the player is still intact");

    // Too short to spare the rows: the lists matter more than the strip.
    assert!(
        !render(true, 110, 22).contains("no local audio"),
        "a short terminal keeps its lists instead"
    );
    // No local audio this run: there is nothing it could ever show.
    assert!(
        !render(false, 110, 30).contains("no local audio"),
        "without local audio there is no strip at all"
    );
}

/// The transport row is a readout, not a control surface: it names the state
/// rather than drawing buttons that a keyboard cannot press.
#[test]
fn the_transport_row_reads_out_state_and_draws_no_buttons() {
    let mut app = ui::App {
        connected: true,
        selected_id: Some("one".into()),
        players: vec![ui::PlayerView {
            id: "one".into(),
            available: true,
            state: "paused".into(),
            volume: Some(80),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("PAUSED"), "the state is said outright");
    assert!(text.contains("VOL 80"));
    for button in ['⏮', '⏹', '⏭'] {
        assert!(
            !text.contains(button),
            "{button} is not pressable, so it is not drawn"
        );
    }
}

/// The focused pane is findable at a glance: a cell grid has one type size, so
/// the heading is filled rather than enlarged.
#[test]
fn the_focused_pane_heading_is_filled_and_the_others_are_not() {
    let palette = ma_tui::theme::Palette::default();
    let heading_style = |focus: ui::Focus, label: &str| {
        let mut app = ui::App {
            connected: true,
            focus,
            content: ui::Focus::Music,
            ..Default::default()
        };
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        // Scan by cell, not by byte: a cell's symbol can be several bytes, so
        // a string index into the rendered text is not a cell index.
        let cells: Vec<&str> = buffer.content.iter().map(|cell| cell.symbol()).collect();
        let wanted: Vec<String> = label.chars().map(|c| c.to_string()).collect();
        let at = (0..cells.len())
            .find(|start| {
                wanted
                    .iter()
                    .enumerate()
                    .all(|(offset, want)| cells.get(start + offset) == Some(&want.as_str()))
            })
            .expect("heading is on screen");
        let cell = &buffer.content[at];
        (cell.bg, cell.modifier)
    };

    let (focused_bg, focused_modifier) = heading_style(ui::Focus::Players, "PLAYERS");
    assert_eq!(
        focused_bg, palette.accent,
        "the focused heading is filled with the accent"
    );
    assert!(
        focused_modifier.contains(ratatui::style::Modifier::BOLD),
        "and is bold"
    );

    let (resting_bg, resting_modifier) = heading_style(ui::Focus::Music, "PLAYERS");
    assert_ne!(
        resting_bg, palette.accent,
        "a pane that is not focused is not filled"
    );
    assert!(
        resting_modifier.contains(ratatui::style::Modifier::BOLD),
        "every heading stays bold, so they read as headings"
    );
}
