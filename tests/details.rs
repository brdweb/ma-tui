use ma_tui::details::{self, Details};
use serde_json::json;

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
