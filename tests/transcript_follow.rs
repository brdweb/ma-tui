//! Following playback through a transcript's timed cues.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ma_tui::{
    controller::Update,
    details::Details,
    presentation::apply,
    transcript::{current, timed_cues, Cue},
    ui::App,
};
use serde_json::{json, Value};

const URI: &str = "feed://podcast_episode/e1";

/// Two hundred cues, ten seconds apart, alternating between two speakers.
fn cues() -> Value {
    let cues: Vec<Value> = (0..200)
        .map(|n| {
            json!({"start": n * 10, "end": n * 10 + 9, "text": format!("Line {n}."),
                   "speaker": if n % 2 == 0 { "Host" } else { "Guest" }})
        })
        .collect();
    json!(["Plain text of the whole episode.", cues])
}

/// The episode's transcript pane, open, with `playing` as the queue's item.
fn pane(playing: &str, elapsed: f64) -> App {
    let mut details = Details::loading(URI, "Episode");
    details.fill(
        &json!({"name":"Episode","item_id":"e1","provider":"feed--abc",
                         "metadata":{"has_transcript":true}}),
    );
    let mut app = App {
        details: Some(details),
        queue_details: json!({"current_item":{"media_item":{"uri":playing}}}),
        elapsed,
        ..Default::default()
    };
    key(&mut app, KeyCode::Char('t'));
    apply(
        &mut app,
        Update::Transcript {
            uri: URI.into(),
            result: Ok(cues()),
        },
    );
    app
}

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn screen(app: &mut App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| ma_tui::ui::draw(frame, app)).unwrap();
    let rows: Vec<String> = terminal
        .backend()
        .buffer()
        .content
        .chunks(110)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    if std::env::var_os("MA_TUI_PRINT_SCREEN").is_some() {
        println!("{}", rows.join("\n"));
    }
    rows.join("\n")
}

#[test]
fn cues_are_cleaned_sorted_and_unusable_ones_dropped() {
    let (cues, cut) = timed_cues(&json!([null, [
        {"start": 20, "text": "Second"},
        {"start": 5, "end": 9, "text": "<b>First</b>", "speaker": "Host\u{1b}[2J"},
        {"start": -1, "text": "Negative start"},
        {"start": 30, "text": "   "},
        {"text": "No start"},
        {"start": 40, "end": 35, "text": "End before start"},
    ]]));
    assert_eq!(
        cues,
        [
            Cue {
                start: 5.0,
                end: Some(9.0),
                speaker: "Host [2J".into(),
                text: "First".into()
            },
            Cue {
                start: 20.0,
                end: None,
                speaker: String::new(),
                text: "Second".into()
            },
            Cue {
                start: 40.0,
                end: None,
                speaker: String::new(),
                text: "End before start".into()
            },
        ]
    );
    assert!(!cut, "dropping unusable cues is not a cut");
    assert!(timed_cues(&json!(["text only", null])).0.is_empty());
    assert!(timed_cues(&json!([null, null])).0.is_empty());
}

#[test]
fn the_current_cue_is_the_last_started_until_its_end() {
    let (cues, _) = timed_cues(&cues());
    assert_eq!(current(&cues, 0.0), Some(0));
    assert_eq!(current(&cues, 15.0), Some(1));
    // Between a cue's end and the next start nobody is speaking.
    assert_eq!(current(&cues, 19.5), None);
    assert_eq!(current(&cues, 1995.0), Some(199));
    assert_eq!(current(&[], 10.0), None);
}

#[test]
fn the_playing_episode_follows_playback_and_names_the_speaker() {
    let mut app = pane(URI, 1505.0);
    let shown = screen(&mut app);
    assert!(shown.contains("DETAILS · TRANSCRIPT · LIVE"));
    assert!(shown.contains("Now speaking: Host"), "{shown}");
    // Cue 150, far below the first screen, has been scrolled into view.
    assert!(shown.contains("▶ Host: Line 150."), "{shown}");
    assert!(!shown.contains("Line 0."));
    // The position moves on: so does the view.
    app.elapsed = 1615.0;
    let shown = screen(&mut app);
    assert!(shown.contains("Now speaking: Guest"));
    assert!(shown.contains("▶ Guest: Line 161."));
}

#[test]
fn scrolling_by_hand_stops_following_and_end_resumes_it() {
    let mut app = pane(URI, 1505.0);
    screen(&mut app);
    for _ in 0..30 {
        key(&mut app, KeyCode::Up);
    }
    app.elapsed = 1805.0;
    let shown = screen(&mut app);
    assert!(!shown.contains("Line 180."), "the reader's place is kept");
    assert!(shown.contains("End follow playback"));
    key(&mut app, KeyCode::End);
    let shown = screen(&mut app);
    assert!(shown.contains("▶ Host: Line 180."), "{shown}");
}

#[test]
fn another_episode_playing_shows_the_plain_transcript() {
    let mut app = pane("feed://podcast_episode/other", 1505.0);
    let shown = screen(&mut app);
    assert!(shown.contains("DETAILS · TRANSCRIPT"));
    assert!(!shown.contains("LIVE"));
    assert!(!shown.contains("Now speaking"));
    assert!(shown.contains("Plain text of the whole episode."));
}

#[test]
fn a_long_cue_is_followed_whole_within_the_transcript_limit() {
    let long = "word ".repeat(600);
    let (cues, cut) = timed_cues(&json!([null, [{"start":0,"text":long}]]));
    assert_eq!(cues[0].text, long.trim_end());
    assert!(!cut);
}

#[test]
fn cues_past_the_limits_are_reported_as_cut() {
    use ma_tui::transcript::{LIMIT, MAX_CUES};
    let many = |count: usize| -> Vec<Value> {
        (0..count)
            .map(|n| json!({"start": n, "text": "x"}))
            .collect()
    };
    // Exactly the most cues kept, with an empty one after: nothing left out.
    let mut exact = many(MAX_CUES);
    exact.push(json!({"start": MAX_CUES, "text": "  "}));
    let (cues, cut) = timed_cues(&json!([null, exact]));
    assert_eq!((cues.len(), cut), (MAX_CUES, false));
    // One more that would show: cut.
    let (cues, cut) = timed_cues(&json!([null, many(MAX_CUES + 1)]));
    assert_eq!((cues.len(), cut), (MAX_CUES, true));
    // A cue longer than what is left of the character budget is shortened.
    let long = "y".repeat(LIMIT + 10);
    let (cues, cut) = timed_cues(&json!([null, [{"start": 0, "text": long}]]));
    assert!(cut);
    assert!(cues[0].text.ends_with('…'));
    // The budget used up, and another cue with text: cut.
    let full = "z".repeat(LIMIT - 3);
    let (cues, cut) = timed_cues(&json!([null, [
        {"start": 0, "text": full},
        {"start": 1, "text": "more"},
    ]]));
    assert_eq!((cues.len(), cut), (1, true));
}

#[test]
fn a_cut_set_of_cues_says_so_while_following() {
    let cues: Vec<Value> = (0..ma_tui::transcript::MAX_CUES + 1)
        .map(|n| json!({"start": n, "text": format!("Line {n}.")}))
        .collect();
    let mut app = pane(URI, 3.0);
    apply(
        &mut app,
        Update::Transcript {
            uri: URI.into(),
            // The text itself is short and whole.
            result: Ok(json!(["Plain text of the whole episode.", cues])),
        },
    );
    let shown = screen(&mut app);
    assert!(shown.contains("LIVE"));
    assert!(shown.contains("Transcript shortened"), "{shown}");

    // The same episode not playing shows the whole text, with no notice.
    let mut app = pane("feed://podcast_episode/other", 3.0);
    apply(
        &mut app,
        Update::Transcript {
            uri: URI.into(),
            result: Ok(json!([
                "Plain text of the whole episode.",
                (0..ma_tui::transcript::MAX_CUES + 1)
                    .map(|n| json!({"start": n, "text": "x"}))
                    .collect::<Vec<Value>>()
            ])),
        },
    );
    assert!(!screen(&mut app).contains("Transcript shortened"));
}
