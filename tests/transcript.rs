use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ma_tui::{
    controller::Update,
    details::Details,
    presentation::apply,
    transcript::{Transcript, LIMIT},
    ui::{Action, App},
};
use serde_json::json;

#[test]
fn text_is_preferred_and_cleaned() {
    assert_eq!(
        Transcript::parse(
            &json!(["<p>Hello &amp; welcome</p><p>Second\u{1b}[2J</p>", [
                {"start":0,"text":"ignored"}
            ]])
        ),
        Transcript::Text {
            body: "Hello & welcome\n\nSecond [2J".into(),
            truncated: false
        }
    );
}

#[test]
fn cues_become_lines_with_their_speakers() {
    assert_eq!(
        Transcript::parse(&json!([null, [
            {"start":0.0,"end":4.2,"text":"Welcome back.","speaker":"Host"},
            {"start":4.2,"text":"<v Guest>Thanks &lt;3</v>","speaker":null},
            {"start":9.0,"text":"   "},
        ]])),
        Transcript::Text {
            body: "Host: Welcome back.\nThanks <3".into(),
            truncated: false
        }
    );
}

#[test]
fn nothing_means_no_transcript() {
    for value in [json!([null, null]), json!(["", []]), json!(null), json!({})] {
        assert_eq!(Transcript::parse(&value), Transcript::Missing, "{value}");
    }
}

#[test]
fn an_oversized_transcript_is_cut_and_says_so() {
    let Transcript::Text { body, truncated } = Transcript::parse(&json!(["word ".repeat(LIMIT)]))
    else {
        panic!("text expected");
    };
    assert!(truncated);
    assert_eq!(body.chars().count(), LIMIT);

    let cues: Vec<_> = (0..LIMIT / 10)
        .map(|n| json!({"start":n,"text":format!("cue number {n}")}))
        .collect();
    let Transcript::Text { body, truncated } = Transcript::parse(&json!([null, cues])) else {
        panic!("text expected");
    };
    assert!(truncated);
    assert_eq!(body.chars().count(), LIMIT);
}

fn key(app: &mut App, code: KeyCode) -> Action {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn episode_pane(has_transcript: bool) -> App {
    let mut pane = Details::loading("feed://podcast_episode/e1", "Episode");
    pane.fill(
        &json!({"name":"Episode","item_id":"e1","provider":"feed--abc",
                      "metadata":{"has_transcript":has_transcript,"description":"Notes"}}),
    );
    App {
        details: Some(pane),
        ..Default::default()
    }
}

#[test]
fn t_reads_the_transcript_once_and_toggles_back() {
    let mut app = episode_pane(true);
    assert!(app
        .details
        .as_ref()
        .unwrap()
        .facts
        .contains(&("Transcript", "press t to read".into())));
    assert_eq!(
        key(&mut app, KeyCode::Char('t')),
        Action::LoadTranscript {
            uri: "feed://podcast_episode/e1".into(),
            item_id: "e1".into(),
            provider: "feed--abc".into()
        }
    );
    // An answer for another pane is ignored; this one's is kept.
    for (uri, text) in [
        ("feed://podcast_episode/other", "Wrong"),
        ("feed://podcast_episode/e1", "Right"),
    ] {
        apply(
            &mut app,
            Update::Transcript {
                uri: uri.into(),
                result: Ok(json!([text, null])),
            },
        );
    }
    assert_eq!(
        app.details.as_ref().unwrap().transcript,
        Some(Transcript::Text {
            body: "Right".into(),
            truncated: false
        })
    );
    // Back to the details, then to the transcript again without a new read.
    assert_eq!(key(&mut app, KeyCode::Char('t')), Action::None);
    assert!(!app.details.as_ref().unwrap().showing_transcript);
    assert_eq!(key(&mut app, KeyCode::Char('t')), Action::None);
    assert!(app.details.as_ref().unwrap().showing_transcript);
}

#[test]
fn a_failed_read_is_shown_and_can_be_retried() {
    let mut app = episode_pane(true);
    key(&mut app, KeyCode::Char('t'));
    apply(
        &mut app,
        Update::Transcript {
            uri: "feed://podcast_episode/e1".into(),
            result: Err("Music Assistant HTTP 500".into()),
        },
    );
    key(&mut app, KeyCode::Char('t'));
    assert!(matches!(
        key(&mut app, KeyCode::Char('t')),
        Action::LoadTranscript { .. }
    ));
}

#[test]
fn without_has_transcript_t_offers_nothing() {
    let mut app = episode_pane(false);
    assert_eq!(key(&mut app, KeyCode::Char('t')), Action::None);
    assert_eq!(app.status, "No transcript for this item");
    assert!(!app
        .details
        .as_ref()
        .unwrap()
        .facts
        .iter()
        .any(|(label, _)| *label == "Transcript"));
}

#[test]
fn the_transcript_draws_in_the_pane() {
    let mut app = episode_pane(true);
    key(&mut app, KeyCode::Char('t'));
    apply(
        &mut app,
        Update::Transcript {
            uri: "feed://podcast_episode/e1".into(),
            result: Ok(json!([null, [{"start":0,"text":"Welcome back.","speaker":"Host"}]])),
        },
    );
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
    assert!(screen.contains("DETAILS · TRANSCRIPT"));
    assert!(screen.contains("Host: Welcome back."));
    assert!(
        !screen.contains("Notes"),
        "the transcript replaces the details"
    );
    if std::env::var_os("MA_TUI_PRINT_SCREEN").is_some() {
        for row in terminal.backend().buffer().content.chunks(110) {
            println!("{}", row.iter().map(|c| c.symbol()).collect::<String>());
        }
    }
}
