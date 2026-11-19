//! Plain-text previews (list title + snippet) derived from a note's Markdown.

/// Strip Markdown formatting syntax from a single line of text so list previews show clean plain text.
pub fn strip_markdown_syntax(input: &str) -> String {
    let mut s = input.trim();

    // Strip leading block markers: headings (#), blockquotes (>), bullet markers (- / * / +)
    loop {
        if let Some(rest) = s.strip_prefix('#') {
            s = rest.trim_start();
        } else if let Some(rest) = s.strip_prefix('>') {
            s = rest.trim_start();
        } else if let Some(rest) = s
            .strip_prefix("- [ ] ")
            .or_else(|| s.strip_prefix("- [x] "))
            .or_else(|| s.strip_prefix("- [X] "))
            .or_else(|| s.strip_prefix("- "))
            .or_else(|| s.strip_prefix("* "))
            .or_else(|| s.strip_prefix("+ "))
        {
            s = rest.trim_start();
        } else {
            break;
        }
    }

    // Replace Markdown links `[label](url)` and images `![alt](url)` with their visible text,
    // and strip inline emphasis/code markers (`**`, `__`, `~~`, `` ` ``).
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        // Image `![alt](url)` -> `alt`
        if chars[i] == '!' && chars.get(i + 1) == Some(&'[') {
            if let Some((label, next_idx)) = parse_md_link(&chars, i + 1) {
                out.push_str(&label);
                i = next_idx;
                continue;
            }
        }
        // Link `[label](url)` -> `label`
        if chars[i] == '[' {
            if let Some((label, next_idx)) = parse_md_link(&chars, i) {
                out.push_str(&label);
                i = next_idx;
                continue;
            }
        }
        // Strip `**`, `__`, `~~`
        if (chars[i] == '*' || chars[i] == '_' || chars[i] == '~')
            && chars.get(i + 1) == Some(&chars[i])
        {
            i += 2;
            continue;
        }
        // Strip inline code backticks
        if chars[i] == '`' {
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    // Collapse consecutive whitespace into a single space
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_md_link(chars: &[char], start_bracket: usize) -> Option<(String, usize)> {
    debug_assert_eq!(chars[start_bracket], '[');
    let mut close_bracket = None;
    let mut j = start_bracket + 1;
    while j < chars.len() {
        if chars[j] == ']' {
            close_bracket = Some(j);
            break;
        }
        j += 1;
    }
    let close_b = close_bracket?;
    if chars.get(close_b + 1) != Some(&'(') {
        return None;
    }
    let mut k = close_b + 2;
    let mut depth = 1usize;
    while k < chars.len() {
        if chars[k] == '(' {
            depth += 1;
        } else if chars[k] == ')' {
            depth -= 1;
            if depth == 0 {
                let label: String = chars[start_bracket + 1..close_b].iter().collect();
                return Some((label, k + 1));
            }
        }
        k += 1;
    }
    None
}

/// Extract a display title and a brief content preview snippet from a note's
/// Markdown. The first non-empty paragraph is the title.
pub fn title_and_snippet(markdown: &str) -> (String, String) {
    // Paragraph splitting, fence handling and inline emphasis all come from
    // the markup parser, so a note previews the same whether it was written
    // here or imported from Windows Sticky Notes.
    let paras = crate::markup::markdown_to_ir(markdown);
    let mut title = None;
    let mut snippet_parts: Vec<String> = Vec::new();

    for p in &paras {
        let raw_text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
        let cleaned = strip_markdown_syntax(&raw_text);
        let t = cleaned.trim();
        if t.is_empty() {
            continue;
        }
        if title.is_none() {
            let char_count = t.chars().count();
            if paras.len() <= 1 && char_count > 26 {
                let chars: Vec<char> = t.chars().collect();
                title = Some(chars[..26.min(chars.len())].iter().collect::<String>());
                snippet_parts.push(chars[26..].iter().collect::<String>());
            } else {
                title = Some(t.chars().take(30).collect::<String>());
            }
        } else {
            snippet_parts.push(t.to_string());
        }
    }

    let title_str = title.unwrap_or_else(|| "无标题便签".into());
    let joined = snippet_parts.join(" ");
    let snippet_str = if joined.is_empty() {
        String::new()
    } else {
        joined.chars().take(80).collect()
    };

    (title_str, snippet_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_the_first_paragraph_and_snippet_the_rest() {
        let (title, snippet) = title_and_snippet("# 购物清单\n\n- **牛奶**\n- [面包](https://example.com)\n> 记得带袋子");
        assert_eq!(title, "购物清单");
        assert_eq!(snippet, "牛奶 面包 记得带袋子");
    }

    #[test]
    fn a_long_single_paragraph_is_split_between_title_and_snippet() {
        let text = "abcdefghijklmnopqrstuvwxyz0123456789";
        let (title, snippet) = title_and_snippet(text);
        assert_eq!(title, "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(snippet, "0123456789");
    }

    #[test]
    fn empty_notes_get_a_placeholder_title() {
        assert_eq!(title_and_snippet(""), ("无标题便签".to_string(), String::new()));
        assert_eq!(title_and_snippet("\n  \n"), ("无标题便签".to_string(), String::new()));
    }

    #[test]
    fn code_fences_are_previewed_without_their_markers() {
        let (title, snippet) = title_and_snippet("标题\n```rust\nlet x = 1;\n```");
        assert_eq!(title, "标题");
        assert_eq!(snippet, "let x = 1;");
    }
}
