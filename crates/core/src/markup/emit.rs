//! IR -> plum / IR -> Markdown; Markdown -> IR.

use super::parse::{Paragraph, ParagraphKind, Run};
use uuid::Uuid;

/// Render IR as plum markup.
///
/// Control words are followed by exactly one delimiter space (plum's own style);
/// run text is emitted verbatim. The parser consumes one space after each
/// control word, so the round trip is lossless for supported features.
pub fn ir_to_plum(paras: &[Paragraph]) -> String {
    let mut lines = Vec::with_capacity(paras.len());
    for p in paras {
        let mut line = String::from("\\id=");
        line.push_str(&p.guid.unwrap_or_else(Uuid::new_v4).to_string());

        // plum opens the first run's style before the list marker ("\b\l text\b0 ")
        let mut cur_bold = false;
        let mut cur_italic = false;
        if let Some(first) = p.runs.first() {
            cur_bold = first.bold;
            cur_italic = first.italic;
            if cur_bold {
                line.push_str("\\b");
            }
            if cur_italic {
                line.push_str("\\i");
            }
        }

        match p.kind {
            ParagraphKind::ListItem => line.push_str("\\l "),
            ParagraphKind::ListCont => line.push_str("\\l0 "),
            ParagraphKind::Plain => line.push(' '),
        }
        // Deep indents beyond plum's single \l0 level are preserved as literal
        // leading spaces (the parser treats them as content, keeping round trips stable).

        for r in &p.runs {
            // If the next run starts with a space, that space doubles as the
            // control-word delimiter (RTF rule), so we must not emit an extra one.
            let space_leads = r.text.starts_with(' ');
            if r.bold != cur_bold {
                line.push_str(if r.bold { "\\b" } else { "\\b0" });
                if !space_leads {
                    line.push(' ');
                }
                cur_bold = r.bold;
            }
            if r.italic != cur_italic {
                line.push_str(if r.italic { "\\i" } else { "\\i0" });
                if !space_leads {
                    line.push(' ');
                }
                cur_italic = r.italic;
            }
            line.push_str(&r.text);
        }
        // Close trailing style markers. The parser consumes one space after a
        // control word, so emit the delimiter only when text follows; plum's own
        // trailing "\b0 " style is matched by appending a single space at line end.
        if cur_bold {
            line.push_str("\\b0");
        }
        if cur_italic {
            line.push_str("\\i0");
        }
        line.push(' ');
        lines.push(line);
    }
    lines.join("\n")
}

/// Render IR as Markdown.
pub fn ir_to_markdown(paras: &[Paragraph]) -> String {
    let mut lines = Vec::with_capacity(paras.len());
    for p in paras {
        let mut line = String::new();
        match p.kind {
            ParagraphKind::ListItem => line.push_str("- "),
            ParagraphKind::ListCont => {
                let n = if p.indent == 0 { 2 } else { p.indent };
                for _ in 0..n {
                    line.push(' ');
                }
            }
            ParagraphKind::Plain => {}
        }
        for r in &p.runs {
            let text = &r.text;
            let leading_spaces = text.len() - text.trim_start().len();
            let trailing_spaces = text.len() - text.trim_end().len();
            let trimmed = text.trim();
            if trimmed.is_empty() {
                line.push_str(text);
                continue;
            }
            line.push_str(&text[..leading_spaces]);
            if r.bold {
                line.push_str("**");
                line.push_str(trimmed);
                line.push_str("**");
            } else if r.italic {
                line.push('*');
                line.push_str(trimmed);
                line.push('*');
            } else {
                line.push_str(trimmed);
            }
            line.push_str(&text[text.len() - trailing_spaces..]);
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// Parse Markdown into IR (lossy: headings degrade to bold paragraphs,
/// code fences lose their fence, blockquotes lose their marker).
pub fn markdown_to_ir(md: &str) -> Vec<Paragraph> {
    let mut out: Vec<Paragraph> = Vec::new();
    let mut in_fence = false;
    for line in md.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Code fences: drop the fence lines, keep content as plain paragraphs
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push(Paragraph {
                guid: None,
                kind: ParagraphKind::Plain,
                indent: 0,
                runs: vec![Run { text: line.to_string(), bold: false, italic: false }],
            });
            continue;
        }

        let (kind, content, heading) = classify(line, trimmed);
        let mut runs = parse_inline(content);
        if heading {
            for r in &mut runs {
                r.bold = true;
            }
        }
        // Any indented line (>=2 spaces) is plum's \l0 world: a "continuation"
        // line, whether under a list item or a plain paragraph. Keep the indent
        // count so deep indents (JSON blocks etc.) survive the round trip.
        let lead = line.len() - line.trim_start().len();
        if kind == ParagraphKind::Plain && lead >= 2 {
            out.push(Paragraph {
                guid: None,
                kind: ParagraphKind::ListCont,
                indent: lead,
                runs,
            });
            continue;
        }
        out.push(Paragraph { guid: None, kind, indent: 0, runs });
    }
    out
}

fn classify<'a>(line: &'a str, trimmed: &'a str) -> (ParagraphKind, &'a str, bool) {
    // Headings: "# " .. "###### " -> bold paragraph (plum has no heading concept)
    if let Some(rest) = strip_heading(trimmed) {
        return (ParagraphKind::Plain, rest, true);
    }
    // Blockquote: strip "> "
    if let Some(rest) = trimmed.strip_prefix("> ") {
        return (ParagraphKind::Plain, rest, false);
    }
    // Task list: keep raw text (lossy)
    if let Some(rest) = trimmed.strip_prefix("- [ ] ").or_else(|| trimmed.strip_prefix("- [x] ")) {
        return (ParagraphKind::ListItem, rest, false);
    }
    // Indented nested list item
    if line.starts_with("  ") && trimmed.starts_with("- ") {
        return (ParagraphKind::ListCont, trimmed.strip_prefix("- ").unwrap(), false);
    }
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return (ParagraphKind::ListItem, rest, false);
    }
    if let Some(rest) = trimmed.strip_prefix("* ") {
        return (ParagraphKind::ListItem, rest, false);
    }
    (ParagraphKind::Plain, trimmed, false)
}

fn strip_heading(s: &str) -> Option<&str> {
    let hashes = s.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && s.as_bytes().get(hashes) == Some(&b' ') {
        Some(&s[hashes + 1..])
    } else {
        None
    }
}

/// Parse inline Markdown (**bold**, *italic*) into runs.
fn parse_inline(s: &str) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            if let Some(close) = find_marker(&chars, i + 2, "**") {
                push(&mut runs, &mut buf, false, false);
                runs.push(Run {
                    text: chars[i + 2..close].iter().collect(),
                    bold: true,
                    italic: false,
                });
                i = close + 2;
                continue;
            }
        }
        if chars[i] == '*' {
            if let Some(close) = find_marker(&chars, i + 1, "*") {
                push(&mut runs, &mut buf, false, false);
                runs.push(Run {
                    text: chars[i + 1..close].iter().collect(),
                    bold: false,
                    italic: true,
                });
                i = close + 1;
                continue;
            }
        }
        buf.push(chars[i]);
        i += 1;
    }
    push(&mut runs, &mut buf, false, false);
    runs
}

fn find_marker(chars: &[char], from: usize, marker: &str) -> Option<usize> {
    let m: Vec<char> = marker.chars().collect();
    let mut i = from;
    while i + m.len() <= chars.len() {
        if chars[i..i + m.len()] == m[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn push(runs: &mut Vec<Run>, buf: &mut String, bold: bool, italic: bool) {
    if !buf.is_empty() {
        runs.push(Run { text: std::mem::take(buf), bold, italic });
    }
}
