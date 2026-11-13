use ma_tui::text::{plain, plain_cut};
use std::time::{Duration, Instant};

#[test]
fn tags_are_dropped_and_blocks_become_lines() {
    assert_eq!(
        plain(
            "<p>First <b>bold <i>nested</i></b> line.</p><p>Second<br/>third</p>",
            1000
        ),
        "First bold nested line.\n\nSecond\nthird"
    );
    assert_eq!(
        plain(
            "<ul><li>One</li><li>Two <a href=\"x>y\">link</a></li></ul>",
            1000
        ),
        "• One\n• Two link"
    );
    assert_eq!(
        plain("<h2>Notes</h2><div>Body</div>", 1000),
        "Notes\n\nBody"
    );
}

#[test]
fn script_style_comments_and_declarations_are_not_prose() {
    assert_eq!(
        plain(
            "<!DOCTYPE html><style>p{color:red}</style><!-- hidden -->Shown<SCRIPT>alert(1)</script> text",
            1000
        ),
        "Shown text"
    );
}

#[test]
fn common_entities_are_decoded() {
    assert_eq!(
        plain(
            "Tom &amp; Jerry &lt;3 &quot;hi&quot; &#39;x&#39; &#x2014; &mdash; &hellip;&nbsp;end",
            1000
        ),
        "Tom & Jerry <3 \"hi\" 'x' — — … end"
    );
    // Unknown or broken entities stay as written.
    assert_eq!(
        plain("AT&T &bogus; &#xZZ; & done", 1000),
        "AT&T &bogus; &#xZZ; & done"
    );
    assert_eq!(plain("a < b and c > d", 1000), "a < b and c > d");
}

#[test]
fn nothing_reaches_the_terminal_as_an_escape_sequence() {
    for hostile in [
        "Title\u{1b}[2J\u{1b}]0;owned\u{7}",
        "Title&#27;[2J&#x1b;]0;owned&#7;",
        "Title\u{9b}31m\u{0}\r",
    ] {
        let text = plain(hostile, 1000);
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{hostile:?} became {text:?}"
        );
        assert!(text.starts_with("Title"));
    }
}

#[test]
fn plain_text_keeps_its_own_line_breaks() {
    assert_eq!(
        plain("Line one\nLine two\n\n\n\nAfter a gap\t tabbed", 1000),
        "Line one\nLine two\n\nAfter a gap tabbed"
    );
    // In HTML a source line break is only spacing.
    assert_eq!(plain("<p>Wrapped\nsource</p>", 1000), "Wrapped source");
}

#[test]
fn an_oversized_body_is_capped_with_an_ellipsis() {
    let huge = "<p>".to_owned() + &"word ".repeat(1_000_000) + "</p>";
    let text = plain(&huge, 100);
    assert_eq!(text.chars().count(), 100);
    assert!(text.ends_with('…'));
    assert_eq!(plain("short", 100), "short");
    // Cutting never splits a character.
    let text = plain(&"é".repeat(500), 10);
    assert_eq!(text, "é".repeat(9) + "…");
}

#[test]
fn hostile_markup_is_walked_in_linear_time() {
    // Each of these once made every `<` rescan the rest of the input, so a
    // feed could freeze the interface for minutes.
    let limit = 120_000;
    for hostile in [
        "<a".repeat(limit * 4),
        "<a \"".repeat(limit * 2),
        "<script>x".repeat(limit),
    ] {
        let started = Instant::now();
        let _ = plain(&hostile, limit);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}… took {:?}",
            &hostile[..12],
            started.elapsed()
        );
    }
    // An unmatched quote does not hide every later tag.
    assert_eq!(plain("<a title=\"x>one<b>two</b>", 1000), "onetwo");
}

#[test]
fn invisible_formatting_characters_are_removed() {
    assert_eq!(
        plain(
            "a\u{202E}b\u{200B}c\u{2066}d&#x202e;e\u{FEFF}f\u{00AD}g\u{200F}h",
            1000
        ),
        "abcdefgh"
    );
    // Emoji sequences keep their joiner.
    assert_eq!(plain("👩\u{200D}💻", 1000), "👩\u{200D}💻");
}

#[test]
fn a_body_cut_at_the_byte_cap_still_says_so() {
    // Markup-heavy input stops at the byte cap long before `limit` characters.
    let heavy = "<span class=\"x\">w</span>".repeat(1000);
    let (text, cut) = plain_cut(&heavy, 300);
    assert!(cut);
    assert!(text.ends_with('…'), "{text:?}");
    assert!(text.chars().count() <= 300);
    assert_eq!(plain_cut("short", 100), ("short".to_owned(), false));
}

#[test]
fn a_longer_tag_name_does_not_end_a_skipped_element() {
    assert_eq!(
        plain(
            "<script>const example = '</scripture>'; alert('hidden');</script><p>Visible</p>",
            1000
        ),
        "Visible"
    );
    assert_eq!(plain("<style>a{}</styles>b{}</STYLE>Shown", 1000), "Shown");
}
