//! Smoke test against the real plum.sqlite: list notes, convert to markdown,
//! convert back, and diff the round trip.

use stickynotes_core::{markup, store::PlumStore};

fn main() -> anyhow::Result<()> {
    let store = PlumStore::open()?;
    println!("plum: {}", store.path().display());

    let notes = store.list_notes()?;
    println!("{} active notes", notes.len());
    for n in notes.iter().take(3) {
        println!("  [{}] {} ({})", &n.id[..8], n.title, n.theme.as_deref().unwrap_or("-"));
    }

    // Round-trip check on every note
    let mut lossy = 0usize;
    for n in &notes {
        let Some(plum_text) = store.get_note_text(&n.id)? else { continue };
        let md = markup::plum_to_markdown(&plum_text);
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
