//! Remote prose made safe and readable in a terminal.
//!
//! Show notes and book descriptions come from feeds and providers, so they are
//! untrusted and often HTML. Nothing here renders HTML: tags are dropped, block
//! boundaries become line breaks, the common entities are decoded, and every
//! control character except the line break is removed afterwards, so a decoded
//! entity cannot smuggle an escape sequence to the terminal either. Invisible
//! formatting characters (bidirectional overrides, zero-width spaces) go too,
//! because they can make a link or name read as something else.
//!
//! The walk is linear in the input: hostile markup such as thousands of
//! unclosed `<a` must not stall the interface that renders it.

/// Elements whose contents are never prose.
const SKIPPED: [&str; 4] = ["script", "style", "head", "title"];

/// Elements that start or end a line of their own.
const BLOCKS: [&str; 24] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "ol",
    "p",
    "pre",
    "section",
    "ul",
];

/// Plain text from an HTML or plain-text description, at most `limit`
/// characters, ending in `…` when it was cut.
pub fn plain(source: &str, limit: usize) -> String {
    plain_cut(source, limit).0
}

/// [`plain`], and whether anything was cut off.
pub fn plain_cut(source: &str, limit: usize) -> (String, bool) {
    // Markup is rarely more than a few times the text it wraps, so there is
    // no need to walk a huge body that will be cut anyway.
    let mut end = source.len().min(limit.saturating_mul(8));
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let capped = end < source.len();
    let source = &source[..end];
    // Plain text keeps its own line breaks; in HTML they are only spacing.
    let html = looks_like_html(source);
    let mut out = String::with_capacity(source.len().min(limit * 4));
    let mut rest = source;
    // Once a tag scan has run off the end, quotes are no longer trusted, and
    // once no `>` is left at all, no later `<` can open a tag. Both keep the
    // walk linear.
    let mut quotes = true;
    let mut closable = true;
    while let Some(start) = rest.find(['<', '&']) {
        push_text(&mut out, &rest[..start], html);
        rest = &rest[start..];
        if rest.starts_with('&') {
            let (decoded, used) = entity(rest);
            push_text(&mut out, &decoded, html);
            rest = &rest[used..];
            continue;
        }
        let scanned = if closable {
            tag(rest, quotes)
        } else {
            Scan::Unclosed
        };
        let scanned = match scanned {
            Scan::Unclosed if quotes => {
                quotes = false;
                tag(rest, false)
            }
            scanned => scanned,
        };
        if matches!(scanned, Scan::Unclosed) {
            closable = false;
        }
        let found = match scanned {
            Scan::Tag(name, closing, used) => Some((name, closing, used)),
            _ => None,
        };
        match found {
            Some((name, closing, used)) => {
                rest = &rest[used..];
                if !closing && SKIPPED.contains(&name.as_str()) {
                    let end = find_closing(rest, &name).unwrap_or(rest.len());
                    rest = &rest[end..];
                } else if name == "li" && !closing {
                    out.push_str("\n• ");
                } else if name == "p" || (name.starts_with('h') && name.len() == 2) {
                    out.push_str("\n\n");
                } else if BLOCKS.contains(&name.as_str()) || name == "tr" {
                    out.push('\n');
                }
            }
            None if closable && rest.starts_with("<!--") => {
                rest = rest.find("-->").map_or("", |end| &rest[end + 3..]);
            }
            // A doctype or other declaration.
            None if closable && rest.starts_with("<!") => {
                rest = rest.find('>').map_or("", |end| &rest[end + 1..]);
            }
            // A `<` that opens no tag is just a character.
            None => {
                out.push('<');
                rest = &rest[1..];
            }
        }
    }
    push_text(&mut out, rest, html);
    tidy(&out, limit, capped)
}

fn looks_like_html(source: &str) -> bool {
    source.match_indices('<').any(|(index, _)| {
        source[index + 1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!')
    })
}

fn push_text(out: &mut String, text: &str, html: bool) {
    for c in text.chars() {
        match c {
            '\n' if !html => out.push('\n'),
            c if c.is_whitespace() => out.push(' '),
            c => out.push(c),
        }
    }
}

enum Scan {
    /// The lowercase name, whether it closes, and how many bytes it spans.
    Tag(String, bool, usize),
    /// Not a tag name at all.
    NotTag,
    /// A tag name with no closing `>` after it.
    Unclosed,
}

/// The tag at the start of `text`. Attributes are skipped, including a `>`
/// inside a quoted value when `quotes` is set.
fn tag(text: &str, quotes: bool) -> Scan {
    let Some(body) = text.strip_prefix('<') else {
        return Scan::NotTag;
    };
    let (closing, body) = match body.strip_prefix('/') {
        Some(body) => (true, body),
        None => (false, body),
    };
    let name: String = body
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Scan::NotTag;
    }
    let mut quote = None;
    for (index, c) in text.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') if quotes => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Scan::Tag(name, closing, index + 1),
            _ => {}
        }
    }
    Scan::Unclosed
}

/// Where `needle` (ASCII) starts in `haystack`, ignoring ASCII case, without
/// copying the haystack.
fn find_ignoring_case(haystack: &str, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    let first = *needle.first()?;
    haystack
        .as_bytes()
        .windows(needle.len())
        .enumerate()
        .find(|(_, window)| window[0] == first && window.eq_ignore_ascii_case(needle))
        .map(|(index, _)| index)
}

/// Where the tag closing `name` starts: `</name` followed by something that
/// cannot continue a tag name, so `</scripture>` does not end a script.
fn find_closing(haystack: &str, name: &str) -> Option<usize> {
    let close = format!("</{name}");
    let mut from = 0;
    while let Some(found) = find_ignoring_case(&haystack[from..], &close) {
        let start = from + found;
        let after = start + close.len();
        if !haystack
            .as_bytes()
            .get(after)
            .is_some_and(|b| b.is_ascii_alphanumeric())
        {
            return Some(start);
        }
        // `</` and the name are ASCII, so this stays on a character boundary.
        from = start + 2;
    }
    None
}

/// Characters that draw nothing but can reorder or hide text: bidirectional
/// controls, zero-width spaces and marks, the soft hyphen and the byte-order
/// mark. The zero-width joiners stay, since emoji sequences need them.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'
            | '\u{200E}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// The entity at the start of `text`, decoded, and how many bytes it spans.
/// Anything unrecognised is kept as the `&` it started with.
fn entity(text: &str) -> (String, usize) {
    let end = text
        .char_indices()
        .take(12)
        .find(|(_, c)| *c == ';')
        .map(|(index, _)| index);
    let Some(end) = end else {
        return ("&".into(), 1);
    };
    let name = &text[1..end];
    let decoded = match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "ndash" => Some('–'),
        "mdash" => Some('—'),
        "hellip" => Some('…'),
        "lsquo" => Some('‘'),
        "rsquo" => Some('’'),
        "ldquo" => Some('“'),
        "rdquo" => Some('”'),
        "copy" => Some('©'),
        _ => name
            .strip_prefix("#x")
            .or_else(|| name.strip_prefix("#X"))
            .map(|hex| u32::from_str_radix(hex, 16))
            .or_else(|| name.strip_prefix('#').map(str::parse))
            .and_then(Result::ok)
            .and_then(char::from_u32),
    };
    match decoded {
        Some(c) => (c.to_string(), end + 1),
        None => ("&".into(), 1),
    }
}

/// Remove control and invisible characters, collapse spacing, keep at most
/// one blank line between paragraphs, and cap the length. `capped` says the
/// source was already cut short, so the result ends in `…` either way.
fn tidy(text: &str, limit: usize, capped: bool) -> (String, bool) {
    let mut lines: Vec<String> = Vec::new();
    let mut blank = 0;
    for line in text.split('\n') {
        let line = line
            .chars()
            .filter(|c| !invisible(*c))
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !lines.is_empty() && blank > 0 {
            lines.push(String::new());
        }
        blank = 0;
        lines.push(line);
    }
    let text = lines.join("\n");
    let count = text.chars().count();
    if count <= limit && !capped {
        return (text, false);
    }
    let keep = if count < limit {
        count
    } else {
        limit.saturating_sub(1)
    };
    let mut cut: String = text.chars().take(keep).collect();
    if limit > 0 {
        cut.push('…');
    }
    (cut, true)
}
