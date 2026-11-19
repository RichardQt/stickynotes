//! Smoke test against the real Windows Sticky Notes database (read-only):
//! list its notes, convert them to Markdown and back, and report any drift.

use stickynotes_core::{
    markup,
    store::{plum, title_and_snippet},
};

fn main() -> anyhow::Result<()> {
    let Some(path) = plum::find_plum_path() else {
        anyhow::bail!("未找到 Windows 便签的数据库（plum.sqlite）");
    };
    println!("plum: {}", path.display());

    let notes = plum::read_live_notes(&path)?;
    println!("{} active notes", notes.len());
    for n in notes.iter().take(3) {
        let (title, _) = title_and_snippet(&markup::plum_to_markdown(&n.text));
        println!("  [{}] {} ({})", &n.id[..8], title, n.theme.as_deref().unwrap_or("-"));
    }

    // Round-trip check on every note
    let mut lossy = 0usize;
    for n in &notes {
        let md = markup::plum_to_markdown(&n.text);
        let back = markup::markdown_to_plum(&md);
        let md2 = markup::plum_to_markdown(&back);
        if md != md2 {
            lossy += 1;
            println!("--- round-trip drift on {} ---\nmd1: {md:?}\nmd2: {md2:?}", &n.id[..8]);
        }
    }
    println!("round-trip drift on {lossy}/{} notes", notes.len());
    Ok(())
}
