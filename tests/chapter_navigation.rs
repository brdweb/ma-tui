use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ma_tui::{
    chapters::{self, Chapter, Chapters},
    controls::Menu,
    ui::{Action, App, Focus, PlayerView},
};
use serde_json::json;

fn list(starts: &[(&str, f64)]) -> Vec<Chapter> {
    starts
        .iter()
        .enumerate()
        .map(|(index, (name, start))| Chapter {
            position: Some(index as u64 + 1),
            name: (*name).into(),
            start: *start,
            end: None,
        })
        .collect()
}

#[test]
fn the_current_chapter_changes_exactly_at_each_start() {
    let book = list(&[("A", 5.0), ("B", 60.0), ("C", 120.5)]);
    assert_eq!(chapters::index_at(&book, 0.0), None, "before the first");
    assert_eq!(chapters::index_at(&book, 5.0), Some(0));
    assert_eq!(chapters::index_at(&book, 59.99), Some(0));
    assert_eq!(chapters::index_at(&book, 60.0), Some(1));
    assert_eq!(chapters::index_at(&book, 120.4), Some(1));
    assert_eq!(chapters::index_at(&book, 120.5), Some(2));
    assert_eq!(chapters::index_at(&book, 1e9), Some(2), "past the last");
    assert_eq!(chapters::index_at(&book, f64::NAN), None);
    assert_eq!(chapters::index_at(&[], 10.0), None);
    assert_eq!(chapters::label(&book, 70.0).as_deref(), Some("Ch 2/3 · B"));
    assert_eq!(
        chapters::label(&list(&[("", 0.0)]), 1.0).as_deref(),
        Some("Ch 1/1")
    );
}

#[test]
fn previous_restarts_the_chapter_unless_just_into_it() {
    let book = list(&[("A", 0.0), ("B", 60.0), ("C", 120.5)]);
    assert_eq!(chapters::previous(&book, 90.0), Some(60.0), "restart B");
    assert_eq!(chapters::previous(&book, 62.0), Some(0.0), "back to A");
    assert_eq!(chapters::previous(&book, 63.5), Some(60.0));
    assert_eq!(
        chapters::previous(&book, 1.0),
        Some(0.0),
        "the first restarts"
    );
    // A fractional start is sought as the next whole second, never the one
    // before it, which would still be the previous chapter.
    assert_eq!(chapters::previous(&book, 200.0), Some(121.0));
    assert_eq!(
        chapters::previous(&list(&[("A", 30.0)]), 10.0),
        Some(0.0),
        "before the first chapter goes to the start of the item"
    );
    assert_eq!(chapters::previous(&[], 10.0), None);
}

#[test]
fn next_steps_forward_and_stops_at_the_last_chapter() {
    let book = list(&[("A", 0.0), ("B", 60.0), ("C", 120.5)]);
    assert_eq!(chapters::next(&book, 0.0), Some(60.0));
    assert_eq!(chapters::next(&book, 60.0), Some(121.0));
    // Landing on the rounded-up start still counts as being in C.
    assert_eq!(chapters::next(&book, 121.0), None);
    assert_eq!(chapters::next(&list(&[("A", 30.0)]), 10.0), Some(30.0));
}

fn playing_app(chapter_list: Vec<Chapter>) -> App {
    App {
        connected: true,
        selected_id: Some("p".into()),
        players: vec![PlayerView {
            id: "p".into(),
            available: true,
            state: "playing".into(),
            ..Default::default()
        }],
        focus: Focus::Queue,
        queue_id: "q".into(),
        queue_details: json!({"current_item":{"media_item":{"uri":"x://book/1"}}}),
        chapters: Chapters {
            uri: "x://book/1".into(),
            list: chapter_list,
        },
        elapsed: 70.0,
        duration: 300.0,
        ..Default::default()
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn parenthesis_keys_seek_by_chapter_and_show_the_new_position() {
    let mut app = playing_app(list(&[("A", 0.0), ("B", 60.0), ("C", 120.0)]));
    assert_eq!(app.key(key(KeyCode::Char(')'))), Action::Seek(120.0));
    assert_eq!(app.elapsed, 120.0, "repeated presses start from here");
    assert_eq!(app.key(key(KeyCode::Char(')'))), Action::None);
    assert_eq!(app.status, "Already in the last chapter");
    assert_eq!(app.key(key(KeyCode::Char('('))), Action::Seek(60.0));
    assert_eq!(app.key(key(KeyCode::Char('('))), Action::Seek(0.0));

    // Chapters loaded for another item are not this item's.
    app.queue_details = json!({"current_item":{"media_item":{"uri":"x://book/2"}}});
    assert_eq!(app.key(key(KeyCode::Char(')'))), Action::None);
    assert_eq!(app.status, "This item has no chapters");
}

#[test]
fn the_controls_menu_lists_chapters_as_seeks() {
    let mut app = playing_app(list(&[("Intro", 0.0), ("", 60.5)]));
    let menu = Menu::new(&app);
    let chapters: Vec<_> = menu
        .entries
        .iter()
        .filter(|entry| entry.section == "Chapters")
        .map(|entry| (entry.label.as_str(), entry.action.clone()))
        .collect();
    assert_eq!(
        chapters,
        [
            ("1. 0:00 · Intro", Action::Seek(0.0)),
            ("2. 1:00 · Chapter 2 · playing", Action::Seek(61.0)),
        ]
    );
    // Chapters come straight after playback.
    let sections: Vec<_> = menu.entries.iter().map(|entry| entry.section).collect();
    let first_chapter = sections.iter().position(|s| *s == "Chapters").unwrap();
    assert!(sections[..first_chapter].iter().all(|s| *s == "Playback"));

    app.menu = Some(menu);
    app.key(key(KeyCode::Char('/')));
    for c in "intro".chars() {
        app.key(key(KeyCode::Char(c)));
    }
    assert_eq!(app.key(key(KeyCode::Enter)), Action::Seek(0.0));
    assert_eq!(app.elapsed, 0.0);
    assert!(app.menu.is_none());

    let none = playing_app(vec![]);
    assert!(Menu::new(&none)
        .entries
        .iter()
        .all(|entry| entry.section != "Chapters"));
}

#[test]
fn the_player_shows_the_chapter_and_the_hints_name_the_keys() {
    let render = |app: &mut App| {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
        terminal.draw(|frame| ma_tui::ui::draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let mut app = playing_app(list(&[("Opening", 0.0), ("The long middle", 60.0)]));
    let screen = render(&mut app);
    assert!(screen.contains("Ch 2/2 · The long middle"));
    assert!(screen.contains("(/) chapter"));

    // A wide name is cut by columns, so it still ends in an ellipsis instead
    // of being clipped mid-character by the layout.
    let wide = "長".repeat(60);
    let mut app = playing_app(list(&[("Opening", 0.0), (wide.as_str(), 60.0)]));
    let screen = render(&mut app);
    assert!(screen.contains("Ch 2/2 · 長"), "{screen}");
    assert!(screen.replace(' ', "").contains("長…"), "{screen}");

    let mut plain = playing_app(vec![]);
    let screen = render(&mut plain);
    assert!(!screen.contains("Ch "));
    assert!(!screen.contains("(/) chapter"));
}

#[test]
fn a_chapter_seek_is_refused_once_another_item_plays() {
    let mut app = playing_app(list(&[("Intro", 0.0), ("Middle", 60.5)]));
    app.menu = Some(Menu::new(&app));
    app.key(key(KeyCode::Char('/')));
    for c in "middle".chars() {
        app.key(key(KeyCode::Char(c)));
    }
    // The queue moves on to another item while the menu is open.
    app.queue_details = json!({"current_item":{"media_item":{"uri":"x://book/2"}}});
    app.elapsed = 5.0;
    assert_eq!(app.key(key(KeyCode::Enter)), Action::None);
    assert_eq!(app.elapsed, 5.0, "the new item was not sought");
    assert!(app.status.contains("Another item is playing"));
}
