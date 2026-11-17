//! plum markup <-> Markdown conversion.
//!
//! plum text format (per paragraph, separated by `\n`):
//!   `\id=<GUID>` — paragraph id prefix
//!   `\b ... \b0` — bold run
//!   `\i ... \i0` — italic run
//!   `\l `        — list item marker (start of paragraph)
//!   `\l0 `       — list continuation (indented) line
//!   `​`      — zero-width space, stripped on import
//!
//! Lossy mapping (plum is the source of truth, Markdown is the projection):
//!   `# heading` -> plain paragraph without `#`
//!   fenced code block -> line-by-line plain paragraphs
//!   tables/links -> kept as raw Markdown text

mod parse;
mod emit;

pub use parse::{parse_plum, Paragraph, ParagraphKind, Run};
pub use emit::{ir_to_markdown, markdown_to_ir, ir_to_plum};

/// plum markup -> Markdown
pub fn plum_to_markdown(plum: &str) -> String {
    let paras = parse_plum(plum);
    ir_to_markdown(&paras)
}

/// Markdown -> plum markup
pub fn markdown_to_plum(md: &str) -> String {
    let paras = markdown_to_ir(md);
    ir_to_plum(&paras)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\\id=2112efc5-a247-453c-af92-3eab84d5563f\\b\\l navidrome_hf_token\\b0 \n\\id=3ef64f64-a1ce-4748-bf7c-7e110bbee10c hf_FAKE_TOKEN_FOR_TESTS_ONLY";

    #[test]
    fn plum_to_md_bold_list() {
        let md = plum_to_markdown(SAMPLE);
        assert!(md.contains("- **navidrome_hf_token**"), "got: {md}");
        assert!(md.contains("hf_FAKE_TOKEN_FOR_TESTS_ONLY"));
    }

    #[test]
    fn md_to_plum_list_bold() {
        let plum = markdown_to_plum("- **hello** world\nsecond para");
        assert!(plum.contains("\\l "), "list marker missing: {plum}");
        // plum opens style before the list marker: "\b\l hello\b0 world"
        assert!(plum.contains("\\b\\l hello\\b0 world"), "bold missing: {plum}");
        assert!(plum.contains("\\id="), "id missing: {plum}");
        assert_eq!(plum.matches('\n').count(), 1);
    }

    #[test]
    fn roundtrip_plum_shape() {
        // md -> plum -> md should be stable
        let md1 = "- first\n- second\nplain text here";
        let plum = markdown_to_plum(md1);
        let md2 = plum_to_markdown(&plum);
        assert_eq!(md1, md2);
    }

    #[test]
    fn heading_degrades_to_paragraph() {
        let plum = markdown_to_plum("# Title\n\nbody");
        let md = plum_to_markdown(&plum);
        assert!(md.contains("Title"));
        assert!(!md.contains("# Title"), "heading should degrade: {md}");
    }

    #[test]
    fn italic_and_zwsp() {
        let plum = "\\id=abc\\i note\\i0 \u{200b}tail";
        let md = plum_to_markdown(plum);
        assert!(md.contains("*note*"), "italic missing: {md}");
        assert!(!md.contains('\u{200b}'), "zero-width not stripped: {md}");
    }

    #[test]
    fn list_continuation() {
        let plum = "\\id=a\\l item\n\\id=b\\l0 continued";
        let md = plum_to_markdown(plum);
        assert!(md.contains("- item"), "got: {md}");
        assert!(md.contains("  continued"), "indent missing: {md}");
    }
}
