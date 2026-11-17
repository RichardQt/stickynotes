//! plum markup -> intermediate IR.
//!
//! plum text is RTF-like: control words (`\b`, `\b0`, `\i`, `\i0`, `\l`, `\l0`)
//! may be followed by a single space that is a consumed delimiter.
//! Each paragraph starts with `\id=<36-char GUID>`.

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParagraphKind {
    Plain,
    ListItem,
    ListCont,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    /// Original plum paragraph GUID (preserved on write-back when possible).
    pub guid: Option<Uuid>,
    pub kind: ParagraphKind,
    /// Leading-space count for ListCont (plum only distinguishes one level of
    /// `\l0`, but Markdown sources may use more; we keep the count for fidelity).
    pub indent: usize,
    pub runs: Vec<Run>,
}

/// Parse plum markup into paragraphs.
pub fn parse_plum(plum: &str) -> Vec<Paragraph> {
    let mut out = Vec::new();
    for line in plum.split('\n') {
        if let Some(p) = parse_line(line) {
            out.push(p);
        }
    }
    out
}

fn parse_line(line: &str) -> Option<Paragraph> {
    let mut rest = line;
    let mut guid = None;
    let mut kind = ParagraphKind::Plain;
    let mut indent = 0usize;
    let mut bold = false;
    let mut italic = false;

    // Strip leading \id=<GUID>
    if let Some(after) = rest.strip_prefix("\\id=") {
        let (id, remainder) = split_guid(after);
        guid = Uuid::parse_str(id).ok();
        rest = remainder;
        rest = consume_space(rest);
    }

    // Leading control words: list markers and/or bold/italic openers
    // (e.g. "\l text", "\l0 text", "\b\l text\b0").
    loop {
        if rest.starts_with("\\l0") {
            kind = ParagraphKind::ListCont;
            indent = 2;
            rest = consume_space(&rest[3..]);
        } else if rest.starts_with("\\l") {
            if kind == ParagraphKind::Plain {
                kind = ParagraphKind::ListItem;
            }
            rest = consume_space(&rest[2..]);
        } else if rest.starts_with("\\b0") {
            bold = false;
            rest = consume_space(&rest[3..]);
        } else if rest.starts_with("\\b") {
            bold = true;
            rest = consume_space(&rest[2..]);
        } else if rest.starts_with("\\i0") {
            italic = false;
            rest = consume_space(&rest[3..]);
        } else if rest.starts_with("\\i") {
            italic = true;
            rest = consume_space(&rest[2..]);
        } else {
            break;
        }
    }

    // Inline runs: \b \b0 \i \i0 toggles (stack-style state machine).
    let mut runs: Vec<Run> = Vec::new();
    let mut buf = String::new();

    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    let mut consumed_delimiter = false;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'b' if chars.get(i + 2) == Some(&'0') => {
                    push_run(&mut runs, &mut buf, bold, italic);
                    bold = false;
                    i += 3;
                }
                'b' => {
                    push_run(&mut runs, &mut buf, bold, italic);
                    bold = true;
                    i += 2;
                }
                'i' if chars.get(i + 2) == Some(&'0') => {
                    push_run(&mut runs, &mut buf, bold, italic);
                    italic = false;
                    i += 3;
                }
                'i' => {
                    push_run(&mut runs, &mut buf, bold, italic);
                    italic = true;
                    i += 2;
                }
                'l' if chars.get(i + 2) == Some(&'0') => {
                    kind = ParagraphKind::ListCont;
                    if indent == 0 {
                        indent = 2;
                    }
                    i += 3;
                }
                'l' => {
                    if kind == ParagraphKind::Plain {
                        kind = ParagraphKind::ListItem;
                    }
                    i += 2;
                }
                _ => {
                    buf.push('\\');
                    i += 1;
                }
            }
            // Consume one delimiter space after a control word (plum's own style).
            if chars.get(i) == Some(&' ') {
                i += 1;
                consumed_delimiter = true;
            }
        } else {
            buf.push(chars[i]);
            i += 1;
            consumed_delimiter = false;
        }
    }
    push_run(&mut runs, &mut buf, bold, italic);
    let _ = consumed_delimiter;

    // Strip zero-width spaces
    for r in &mut runs {
        r.text = r.text.replace('\u{200b}', "");
    }

    // Trim trailing delimiter spaces from the last run (the emitter appends a
    // delimiter space at line end; plum consumes it, so it is not content).
    if let Some(last) = runs.last_mut() {
        while last.text.ends_with(' ') {
            last.text.pop();
        }
        if last.text.is_empty() {
            runs.pop();
        }
    }

    // Drop fully-empty paragraphs
    if runs.iter().all(|r| r.text.is_empty()) {
        return None;
    }

    Some(Paragraph { guid, kind, indent, runs })
}

/// GUIDs are 36 chars ("8-4-4-4-12" hex); fall back to reading until
/// whitespace/backslash if the prefix is not a valid GUID.
fn split_guid(s: &str) -> (&str, &str) {
    let bytes = s.as_bytes();
    if bytes.len() >= 36 && s.is_char_boundary(36) {
        let (candidate, rest) = s.split_at(36);
        if Uuid::parse_str(candidate).is_ok() {
            return (candidate, rest);
        }
    }
    match s.find(|c: char| c == '\\' || c.is_whitespace()) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

fn push_run(runs: &mut Vec<Run>, buf: &mut String, bold: bool, italic: bool) {
    if !buf.is_empty() {
        runs.push(Run { text: std::mem::take(buf), bold, italic });
    }
}

fn consume_space(s: &str) -> &str {
    s.strip_prefix(' ').unwrap_or(s)
}
