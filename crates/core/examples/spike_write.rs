//! Write-path spike (plan step 4): update one note's text through PlumStore,
//! verify the write landed in plum.sqlite, then restore the original.

use stickynotes_core::{markup, store::PlumStore, sync};

fn main() -> anyhow::Result<()> {
    // Safety: refuse to run while the built-in app is open (WAL concurrent write).
    if sync::is_sticky_notes_running() {
        anyhow::bail!("内置便签正在运行，请先关闭再测试写入");
    }
    let store = PlumStore::open()?;
    println!("plum: {}", store.path().display());

    let notes = store.list_notes()?;
    let target = notes
        .iter()
        .find(|n| n.title.contains("mistral"))
        .expect("no mistral note");
    println!("target: [{}] {}", &target.id[..8], target.title);

    // Read original
    let original = store
        .get_note_text(&target.id)?
        .expect("note vanished");

    // Convert to markdown, append a marker paragraph, convert back, write.
    let md = markup::plum_to_markdown(&original);
    let md2 = format!("{md}\n\n**spike-write-test** 请忽略");
    let plum2 = markup::markdown_to_plum(&md2);
    store.update_text(&target.id, &plum2)?;
    println!("written. verifying...");

    let reread = store.get_note_text(&target.id)?.expect("vanished after write");
    assert!(reread.contains("spike-write-test"), "marker not found after write");
    println!("marker present in plum: OK");

    // Restore original text.
    store.update_text(&target.id, &original)?;
    let restored = store.get_note_text(&target.id)?.expect("vanished");
    assert_eq!(restored, original, "restore mismatch");
    println!("restored: OK (byte-identical)");

    // Now the insert spike (plan step 5): clone-insert a new note, then soft-delete it.
    let new_id = store.insert_note("\\id=00000000-0000-0000-0000-000000000000 spike-insert-test\n\\id=00000000-0000-0000-0000-0000000000001 second line", "Yellow")?;
    println!("inserted note: {new_id}");
    let text = store.get_note_text(&new_id)?.expect("inserted note not found");
    assert!(text.contains("spike-insert-test"));
    println!("insert visible in plum: OK");

    store.soft_delete(&new_id)?;
    let conn = rusqlite::Connection::open_with_flags(
        store.path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let deleted: Option<i64> = conn.query_row(
        "SELECT DeletedAt FROM Note WHERE Id = ?1",
        rusqlite::params![new_id],
        |r| r.get(0),
    )?;
    assert!(deleted.is_some(), "soft delete not recorded");
    println!("soft delete recorded: OK");

    println!("\nAll write-path spikes passed. Backups in %LOCALAPPDATA%\\stickynotes\\backups");
    Ok(())
}
