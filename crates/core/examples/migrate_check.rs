//! Dry run of the first-start migration against this machine's real data.
//!
//! Reads Windows Sticky Notes and the side files of earlier app versions
//! (all read-only), migrates them into a throwaway database, checks the
//! result against the sources and prints counts — never note content.

use std::collections::HashMap;
use stickynotes_core::store::{legacy, plum, NoteStore};
use stickynotes_core::webdav::SyncCache;

fn main() -> anyhow::Result<()> {
    let sources = legacy::LegacySources::detect();
    match &sources.plum {
        Some(path) => println!("windows notes : {}", path.display()),
        None => println!("windows notes : (not found)"),
    }

    let dir = std::env::temp_dir().join(format!("stickynotes-migrate-check-{}", std::process::id()));
    let store = NoteStore::open(&dir.join("stickynotes.db"))?;
    let report = legacy::migrate_once(&store, &sources)?.expect("a new database always migrates");
    println!(
        "migrated      : {} notes, {} tags, {} tag links, {} pins",
        report.notes.added, report.tags.tags, report.tags.links, report.tags.pins
    );

    let ours = store.live_notes()?;
    if let Some(path) = &sources.plum {
        let native = plum::read_live_notes(path)?;
        let by_id: HashMap<&str, _> = ours.iter().map(|n| (n.id.as_str(), n)).collect();
        anyhow::ensure!(native.len() == ours.len(), "note count differs");
        for n in &native {
            let Some(mine) = by_id.get(n.id.as_str()) else {
                anyhow::bail!("note {} did not arrive", &n.id[..8]);
            };
            anyhow::ensure!(Some(mine.updated_at) == n.updated_at, "timestamp of {} changed", &n.id[..8]);
            anyhow::ensure!(Some(mine.created_at) == n.created_at, "creation time of {} changed", &n.id[..8]);
            anyhow::ensure!(
                Some(mine.theme.as_str()) == n.theme.as_deref(),
                "colour of {} changed",
                &n.id[..8]
            );
        }
        println!("verified      : every note kept its id, colour and timestamps");

        let shadow = sources.shadow_markdown();
        let kept = native
            .iter()
            .filter(|n| shadow.markdown_for(&n.id, &plum::normalize(&n.text)).is_some())
            .count();
        println!(
            "formatting    : {kept} of {} saved Markdown copies still match and were used",
            shadow.len()
        );
    }

    // Same ids and timestamps as the last synced versions mean the first
    // WebDAV pass after the migration has nothing to upload or download.
    let cache = SyncCache::load();
    let in_step = ours
        .iter()
        .filter(|n| {
            cache
                .files
                .get(&format!("{}.md", n.id))
                .is_some_and(|f| f.updated_at == n.updated_at)
        })
        .count();
    println!(
        "webdav        : {in_step} of {} notes match the last synced version ({} files known)",
        ours.len(),
        cache.files.len()
    );

    // A second run must be a no-op.
    anyhow::ensure!(legacy::migrate_once(&store, &sources)?.is_none(), "migration ran twice");
    drop(store);
    std::fs::remove_dir_all(&dir)?;
    println!("ok (throwaway database removed)");
    Ok(())
}
