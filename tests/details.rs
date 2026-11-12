use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ma_tui::{
    controller::Update,
    details::{self, Details},
    music::Media,
    presentation::apply,
    ui::{Action, App, Focus, PlayerView},
};
use serde_json::json;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn episode() -> serde_json::Value {
    json!({
        "name":"Episode 14","uri":"podcast_feed://podcast_episode/14","media_type":"podcast_episode",
        "duration":3725,"podcast":{"name":"Fixture Show"},
        "metadata":{
            "release_date":"2026-10-30T08:00:00+00:00",
            "performers":["Host","Guest\u{1b}[2J"],
            "links":[{"type":"website","url":"https://example.com/14"}],
            "explicit":true,
            "chapters":[{"position":1,"name":"A","start":0},{"position":2,"name":"B","start":60}],
            "description":"<p>Notes &amp; <i>links</i></p><p>Second</p>"
        }
    })
}

#[test]
fn facts_name_only_what_the_server_gave() {
    assert_eq!(
        details::facts(&episode()),
        [
            ("Show", "Fixture Show".to_owned()),
            ("Published", "2026-10-30".into()),
            ("Duration", "1:02:05".into()),
            ("People", "Host, Guest [2J".into()),
            ("Chapters", "2".into()),
            ("Explicit", "yes".into()),
            ("Links", "https://example.com/14".into()),
        ]
    );
    let book = json!({"name":"Book","duration":59,"publisher":"Press",
                      "authors":["Ann"],"narrators":[{"name":"Ned"}]});
    assert_eq!(
        details::facts(&book),
        [
            ("Authors", "Ann".to_owned()),
            ("Narrators", "Ned".into()),
            ("Duration", "0:59".into()),
            ("Publisher", "Press".into()),
        ]
    );
    assert!(details::facts(&json!({"metadata":{"release_date":"soon"}})).is_empty());
}

#[test]
fn filling_sanitises_the_description_and_keeps_the_row_title_when_unnamed() {
    let mut pane = Details::loading("u", "Row title");
    pane.fill(&episode());
    assert_eq!(pane.title, "Episode 14");
    assert_eq!(pane.description, "Notes & links\n\nSecond");
    assert!(!pane.loading);

    let mut pane = Details::loading("u", "Row title");
    pane.fill(&json!({"metadata":{"description":"x".repeat(details::DESCRIPTION_LIMIT * 2)}}));
    assert_eq!(pane.title, "Row title");
    assert_eq!(pane.description.chars().count(), details::DESCRIPTION_LIMIT);
}

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

#[test]
fn i_opens_the_pane_for_the_highlighted_item_only_where_offered() {
    let mut app = connected();
    app.focus = Focus::Search;
    app.results = vec![ma_tui::ui::TrackView {
        media: Some(Media::parse(
            &json!({"name":"Show","uri":"library://podcast/1","media_type":"podcast"}),
            "",
        )),
        ..Default::default()
    }];
    assert_eq!(
        app.key(key(KeyCode::Char('i'))),
        Action::LoadDetails {
            uri: "library://podcast/1".into(),
            title: "Show".into()
        }
    );
    assert!(app.details.as_ref().is_some_and(|d| d.loading));
    // The pane takes the keys while it is open.
    assert_eq!(app.key(key(KeyCode::Char(' '))), Action::None);
    app.key(key(KeyCode::Esc));
    assert!(app.details.is_none());

    app.results[0].media = Some(Media::parse(
        &json!({"name":"Song","uri":"library://track/1","media_type":"track"}),
        "",
    ));
    assert_eq!(app.key(key(KeyCode::Char('i'))), Action::None);
    assert!(app.details.is_none());
    assert_eq!(
        app.status,
        "Details are shown for podcasts, episodes and audiobooks"
    );
}

#[test]
fn the_item_menu_offers_details_without_a_speaker() {
    let book = Media::parse(
        &json!({"name":"Book","uri":"library://audiobook/7","media_type":"audiobook"}),
        "",
    );
    let mut app = App {
        connected: true,
        ..Default::default()
    };
    ma_tui::music::choose(&mut app, &book);
    let menu = app.menu.as_mut().unwrap();
    let index = menu
        .visible()
        .into_iter()
        .position(|i| menu.entries[i].label == "Details")
        .unwrap();
    menu.cursor = index;
    assert_eq!(
        app.key(key(KeyCode::Enter)),
        Action::LoadDetails {
            uri: "library://audiobook/7".into(),
            title: "Book".into()
        }
    );
    assert!(app.menu.is_none());
    assert_eq!(app.details.as_ref().unwrap().title, "Book");
}

#[test]
fn a_late_answer_fills_only_the_pane_it_was_asked_for() {
    let mut app = connected();
    app.details = Some(Details::loading("library://podcast/2", "Second"));
    apply(
        &mut app,
        Update::Details {
            uri: "library://podcast/1".into(),
            result: Ok(episode()),
        },
    );
    assert!(app.details.as_ref().unwrap().loading);
    apply(
        &mut app,
        Update::Details {
            uri: "library://podcast/2".into(),
            result: Err("Music Assistant HTTP 404".into()),
        },
    );
    let pane = app.details.as_ref().unwrap();
    assert!(!pane.loading);
    assert_eq!(pane.error, "Music Assistant HTTP 404");
}

#[test]
fn the_pane_draws_and_scrolling_stops_at_the_end() {
    let mut app = connected();
    let mut pane = Details::loading("u", "Title");
    pane.fill(&json!({"name":"Long notes","metadata":{
        "description": (1..=200).map(|n| format!("Paragraph {n}")).collect::<Vec<_>>().join("\n\n")
    }}));
    pane.scroll = u16::MAX;
    app.details = Some(pane);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
    terminal
        .draw(|frame| ma_tui::ui::draw(frame, &mut app))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("DETAILS"));
    assert!(screen.contains("Paragraph 200"), "the end is reachable");
    assert!(screen.contains("Esc close details"));
    let scroll = app.details.as_ref().unwrap().scroll;
    assert!(
        scroll > 0 && scroll < 500,
        "clamped to the content: {scroll}"
    );
}
