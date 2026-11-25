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
                out.push_str(&strip_markdown_syntax(&label));
                i = next_idx;
                continue;
            }
        }
        // Strip inline HTML formatting tags (`<u>`, `<mark>`, ...) and unwrap `<https://...>` autolinks
        if chars[i] == '<' {
            if let Some((replacement, next_idx)) = parse_inline_angle(&chars, i) {
                out.push_str(&replacement);
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

fn parse_inline_angle(chars: &[char], start: usize) -> Option<(String, usize)> {
    debug_assert_eq!(chars[start], '<');
    let mut close = None;
    for (j, &ch) in chars.iter().enumerate().skip(start + 1) {
        if ch == '<' {
            return None;
        }
        if ch == '>' {
            close = Some(j);
            break;
        }
    }
    let close = close?;
    let inner: String = chars[start + 1..close].iter().collect();
    let trimmed = inner.trim();
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "u" | "/u" | "s" | "/s" | "del" | "/del" | "mark" | "/mark" | "em" | "/em" | "strong" | "/strong"
    ) || lower.starts_with("mark ")
    {
        return Some((String::new(), close + 1));
    }
    if !trimmed.chars().any(char::is_whitespace)
        && (lower.starts_with("https://")
            || lower.starts_with("http://")
            || lower.starts_with("mailto:")
            || lower.starts_with("tel:"))
    {
        return Some((trimmed.to_string(), close + 1));
    }
    None
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

/// One Markdown table row as a line a card can show: `| a | b |` becomes
/// `a · b`. `None` for the `|---|---|` separator, which carries no content.
fn table_row_preview(line: &str) -> Option<String> {
    let trimmed = line.trim();
    debug_assert!(trimmed.starts_with('|'));
    if trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t')) {
        return None;
    }
    let cells: Vec<String> = trimmed
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .map(strip_markdown_syntax)
        .filter(|cell| !cell.is_empty())
        .collect();
    (!cells.is_empty()).then(|| cells.join(" · "))
}

/// Rewrite table rows outside fenced code blocks into plain preview lines.
/// `keep_fences` keeps the ``` markers for the preview parser (which drops
/// them and treats their content as code); search drops them entirely, so a
/// match window never shows a stray language name.
fn preview_lines(markdown: &str, keep_fences: bool) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_fence = false;
    for line in markdown.split('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            if keep_fences {
                out.push_str(line);
            }
            out.push('\n');
            continue;
        }
        if !in_fence && trimmed.starts_with('|') {
            if let Some(row) = table_row_preview(line) {
                out.push_str(&row);
            }
            // A separator row leaves no text behind; dropping the line (even
            // as an empty one) keeps the paragraphs around it apart.
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn markdown_for_preview(markdown: &str) -> String {
    preview_lines(markdown, true)
}

/// A note's Markdown as the one continuous text search matches in: table rows
/// read as their preview form and fence markers are gone (their content
/// stays). Matches can then be shown as clean previews instead of raw pipes
/// and ``` markers.
pub fn search_text(markdown: &str) -> String {
    preview_lines(markdown, false)
}

/// Extract a display title and a brief content preview snippet from a note's
/// Markdown. The first non-empty paragraph is the title.
pub fn title_and_snippet(markdown: &str) -> (String, String) {
    // Paragraph splitting, fence handling and inline emphasis all come from
    // the markup parser, so a note previews the same whether it was written
    // here or imported from Windows Sticky Notes.
    let preview_md = markdown_for_preview(markdown);
    let paras = crate::markup::markdown_to_ir(&preview_md);
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

/// A short one-line window around the first match of `query` (already
/// lowercased) in `lower`, sliced from `source` so the original casing is
/// kept. Search uses it to show why a note matched when the match sits deep
/// in the body and the normal snippet would not mention it.
pub fn snippet_around_match(source: &str, lower: &str, query: &str) -> Option<String> {
    let pos = lower.find(query)?;
    let match_start = lower[..pos].chars().count();
    let match_len = query.chars().count();
    let chars: Vec<char> = source.chars().collect();
    let start = match_start.saturating_sub(24);
    let end = (match_start + match_len + 56).min(chars.len());
    let window: String = chars[start..end].iter().collect();
    let mut text = strip_markdown_syntax(&window);
    if text.is_empty() {
        return None;
    }
    if start > 0 {
        text.insert(0, '…');
    }
    if end < chars.len() {
        text.push('…');
    }
    Some(text)
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

    #[test]
    fn table_rows_read_as_text_in_the_preview() {
        let md = "WebDAV 备忘\n\n| 状态码 | 含义 | 处理 |\n| --- | :---: | --- |\n| 401 | 未授权 | 检查密码 |\n| 404 | 不存在 | 创建目录 |";
        let (title, snippet) = title_and_snippet(md);
        assert_eq!(title, "WebDAV 备忘");
        assert_eq!(
            snippet,
            "状态码 · 含义 · 处理 401 · 未授权 · 检查密码 404 · 不存在 · 创建目录"
        );
        assert!(!snippet.contains('|'));
        assert!(!snippet.contains("---"));
    }

    #[test]
    fn a_pipe_inside_a_fence_is_not_a_table() {
        let (title, snippet) = title_and_snippet("标题\n```sh\ncat a | grep b\n```");
        assert_eq!(title, "标题");
        assert_eq!(snippet, "cat a | grep b");
    }

    #[test]
    fn search_text_flattens_tables_and_drops_fence_markers() {
        let md = "标题\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n```rust\nfn x() {}\n```";
        let text = search_text(md);
        assert!(!text.contains('|'), "got: {text}");
        assert!(!text.contains("```"), "got: {text}");
        assert!(text.contains("a · b"), "got: {text}");
        assert!(text.contains("fn x() {}"), "got: {text}");
    }

    #[test]
    fn a_match_deep_in_the_body_gets_a_window_that_keeps_its_case() {
        let md = "Title\n\nThese are enough leading words before the **Quick** brown fox jumps over the lazy dog and then keeps running for a while.";
        let lower = md.to_lowercase();
        let snippet = snippet_around_match(md, &lower, "quick").unwrap();
        assert!(snippet.contains("Quick"));
        assert!(snippet.starts_with('…'));
        assert!(snippet.chars().count() <= 90);
    }

    #[test]
    fn underline_highlight_and_links_preview_as_clean_text() {
        let (title, snippet) = title_and_snippet(
            "<u>重点</u>\n\n请查看 [<u>官网</u>](https://example.com) 或 <https://rust-lang.org> 和 <mark>高亮</mark>",
        );
        assert_eq!(title, "重点");
        assert_eq!(snippet, "请查看 官网 或 https://rust-lang.org 和 高亮");
    }

    #[test]
    fn a_match_is_reported_at_its_original_position_only() {
        let md = "前言。这里有一个关键词，后面还有很长的内容，继续写下去，直到窗口之外。";
        let lower = md.to_lowercase();
        assert!(snippet_around_match(md, &lower, "关键词").is_some());
        assert!(snippet_around_match(md, &lower, "不存在的词").is_none());
    }
}
