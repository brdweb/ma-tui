use ma_tui::{
    chapters::Chapter,
    controller::Update,
    music::Media,
    presentation::apply,
    ui::{Action, App, PlayerView},
};
use serde_json::json;

fn connected() -> App {
    App {
        connected: true,
        selected_id: Some("p".into()),
        players: vec![PlayerView {
            id: "p".into(),
            available: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn labels(app: &App) -> Vec<(String, Action)> {
    app.menu
        .as_ref()
        .map(|menu| {
            menu.entries
                .iter()
                .map(|entry| (entry.label.clone(), entry.action.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn only_an_audiobook_offers_chapters_in_its_item_menu() {
    let book = Media::parse(
        &json!({"name":"Book","item_id":"7","provider":"library",
                "media_type":"audiobook","uri":"library://audiobook/7"}),
        "",
    );
    let mut app = connected();
    ma_tui::music::choose(&mut app, &book);
    assert!(labels(&app).contains(&(
        "Chapters…".into(),
        Action::LoadChapters {
            uri: "library://audiobook/7".into()
        }
    )));

    // Music Assistant 2.10.9 ignores a chapter start for an episode.
    let episode = Media::parse(
        &json!({"name":"Episode","item_id":"e","provider":"library",
                "media_type":"podcast_episode","uri":"library://podcast_episode/e"}),
        "",
    );
    let mut app = connected();
    ma_tui::music::choose(&mut app, &episode);
    assert!(!labels(&app).iter().any(|(label, _)| label == "Chapters…"));
}

#[test]
fn loaded_chapters_open_a_picker_of_numbered_chapters() {
    let mut app = connected();
    app.chapter_picker = Some("library://audiobook/7".into());
    let chapter = |position, name: &str, start| Chapter {
        position,
        name: name.into(),
        start,
        end: None,
    };
    apply(
        &mut app,
        Update::ChapterList {
            uri: "library://audiobook/7".into(),
            result: Ok(vec![
                chapter(Some(1), "Opening", 0.0),
                chapter(None, "Unnumbered", 30.0),
                chapter(Some(3), "", 95.0),
            ]),
        },
    );
    assert_eq!(app.menu.as_ref().unwrap().title, "Chapters");
    let play = |chapter| Action::PlayChapter {
        uri: "library://audiobook/7".into(),
        chapter,
    };
    assert_eq!(
        labels(&app),
        [
            ("0:00 · Opening".to_owned(), play(1)),
            ("1:35 · Chapter 3".to_owned(), play(3)),
        ]
    );
}

#[test]
fn a_book_without_chapters_or_a_failed_read_says_so() {
    let mut app = connected();
    app.chapter_picker = Some("u".into());
    apply(
        &mut app,
        Update::ChapterList {
            uri: "u".into(),
            result: Ok(vec![]),
        },
    );
    assert!(app.menu.is_none());
    assert_eq!(app.status, "This audiobook has no chapters to start from");

    app.chapter_picker = Some("u".into());
    apply(
        &mut app,
        Update::ChapterList {
            uri: "u".into(),
            result: Err("Music Assistant HTTP 500".into()),
        },
    );
    assert!(app.menu.is_none());
    assert_eq!(
        app.status,
        "Could not load chapters: Music Assistant HTTP 500"
    );
}

#[test]
fn a_late_picker_never_replaces_another_menu_or_answers_another_book() {
    let chapters = || {
        Ok(vec![Chapter {
            position: Some(1),
            name: "One".into(),
            start: 0.0,
            end: None,
        }])
    };
    // Asked for one book, then the answer for another arrives.
    let mut app = connected();
    app.chapter_picker = Some("library://audiobook/1".into());
    apply(
        &mut app,
        Update::ChapterList {
            uri: "library://audiobook/2".into(),
            result: chapters(),
        },
    );
    assert!(app.menu.is_none());

    // The user opened the controls menu while the read was out.
    let mut app = connected();
    app.chapter_picker = Some("library://audiobook/1".into());
    app.key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('?'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let title = app.menu.as_ref().map(|menu| menu.title.clone());
    assert!(title.is_some());
    apply(
        &mut app,
        Update::ChapterList {
            uri: "library://audiobook/1".into(),
            result: chapters(),
        },
    );
    assert_eq!(app.menu.as_ref().map(|menu| menu.title.clone()), title);

    // Nothing was asked for at all.
    let mut app = connected();
    apply(
        &mut app,
        Update::ChapterList {
            uri: "library://audiobook/1".into(),
            result: chapters(),
        },
    );
    assert!(app.menu.is_none());
}
