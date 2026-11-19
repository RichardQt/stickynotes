//! Import notes from Windows Sticky Notes into the app's own database.
//!
//! This is a one-way, read-only merge by note id — the native database is
//! never written and nothing is ever deleted here:
//!
//! - a note this database has never seen is added, keeping its id, colour
//!   and timestamps (so WebDAV keeps treating it as the same note);
//! - a note whose native text changed since the last import, and is newer
//!   than our copy, is updated;
//! - everything else is left alone, including notes edited or deleted here.

use super::legacy::ShadowMarkdown;
use super::plum::{self, PlumNote};
use super::{NoteStore, Result};
use crate::{markup, ticks};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// What one import did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport {
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
}

impl ImportReport {
    /// Did the import change any note?
    pub fn changed(&self) -> bool {
        self.added + self.updated > 0
    }

    pub fn summary(&self) -> String {
        format!(
            "新增 {} · 更新 {} · 未变 {}",
            self.added, self.updated, self.unchanged
        )
    }
}

/// Import every live note of the Windows Sticky Notes database at `plum_path`.
pub fn import_from_windows(
    store: &NoteStore,
    plum_path: &Path,
    shadow: &ShadowMarkdown,
) -> Result<ImportReport> {
    let notes = plum::read_live_notes(plum_path)?;
    import_notes(store, &notes, shadow)
}

/// Merge already-read native notes into `store`, all or nothing.
pub fn import_notes(
    store: &NoteStore,
    notes: &[PlumNote],
    shadow: &ShadowMarkdown,
) -> Result<ImportReport> {
    let tx = store.conn.unchecked_transaction()?;
    let report = import_notes_in(&tx, notes, shadow)?;
    tx.commit()?;
    Ok(report)
}

/// The merge itself, inside the caller's transaction.
pub(super) fn import_notes_in(
    conn: &Connection,
    notes: &[PlumNote],
    shadow: &ShadowMarkdown,
) -> rusqlite::Result<ImportReport> {
    let mut report = ImportReport::default();
    for native in notes {
        let normalized = plum::normalize(&native.text);
        let ours: Option<(String, i64, Option<i64>)> = conn
            .query_row(
                "SELECT markdown, updated_at, deleted_at FROM note WHERE id = ?1",
                params![native.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        match ours {
            None => {
                let updated_at = native
                    .updated_at
                    .or(native.created_at)
                    .unwrap_or_else(ticks::now_ticks);
                conn.execute(
                    "INSERT INTO note (id, markdown, theme, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        native.id,
                        markdown_of(native, &normalized, shadow),
                        native.theme.as_deref().unwrap_or("Yellow"),
                        native.created_at.unwrap_or(updated_at),
                        updated_at,
                    ],
                )?;
                report.added += 1;
            }
            Some((markdown, updated_at, deleted_at)) => {
                // What the native row looked like the last time we imported
                // it. Without such a record (the note reached us over WebDAV)
                // the row counts as unchanged while it still says what our
                // Markdown says.
                let seen: Option<String> = conn
                    .query_row(
                        "SELECT plum FROM plum_import WHERE note_id = ?1",
                        params![native.id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let native_changed = match seen {
                    Some(seen) => seen != normalized,
                    None => plum::normalize(&markup::markdown_to_plum(&markdown)) != normalized,
                };
                // A deletion here counts as our latest change to the note.
                let ours_at = updated_at.max(deleted_at.unwrap_or(i64::MIN));
                let native_at = native.updated_at.unwrap_or(i64::MIN);
                if native_changed && native_at > ours_at {
                    conn.execute(
                        "UPDATE note SET markdown = ?1, theme = COALESCE(?2, theme), \
                         updated_at = ?3, deleted_at = NULL WHERE id = ?4",
                        params![
                            markdown_of(native, &normalized, shadow),
                            native.theme,
                            native_at,
                            native.id,
                        ],
                    )?;
                    report.updated += 1;
                } else {
                    report.unchanged += 1;
                }
            }
        }

        // Remember what the native row says now, so the next import can tell
        // whether it changed.
        conn.execute(
            "INSERT INTO plum_import (note_id, plum) VALUES (?1, ?2) \
             ON CONFLICT(note_id) DO UPDATE SET plum = excluded.plum",
            params![native.id, normalized],
        )?;
    }
    Ok(report)
}

/// Markdown for a native note: the exact Markdown an earlier version of this
/// app saved for it when the row is still the one that version wrote,
/// otherwise the (plain) projection of the plum text.
fn markdown_of(native: &PlumNote, normalized: &str, shadow: &ShadowMarkdown) -> String {
    match shadow.markdown_for(&native.id, normalized) {
        Some(markdown) => markdown.to_string(),
        None => markup::plum_to_markdown(&native.text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::RemoteUpdate;

    const A: &str = "11111111-1111-1111-1111-111111111111";
    const B: &str = "22222222-2222-2222-2222-222222222222";

    fn native(id: &str, text: &str, updated_at: i64) -> PlumNote {
        PlumNote {
            id: id.to_string(),
            text: text.to_string(),
            theme: Some("Blue".to_string()),
            created_at: Some(100),
            updated_at: Some(updated_at),
        }
    }

    /// plum text as the native app stores it, with fresh paragraph GUIDs.
    fn plum_text(lines: &[&str]) -> String {
        lines
            .iter()
            .map(|l| format!("\\id={} {l}", uuid::Uuid::new_v4()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn markdown(store: &NoteStore, id: &str) -> Option<String> {
        store.note_markdown(id).unwrap()
    }

    #[test]
    fn first_import_adds_notes_with_their_id_colour_and_timestamps() {
        let store = NoteStore::open_in_memory().unwrap();
        let notes = [
            native(A, &plum_text(&["标题", "\\b 加粗\\b0 "]), 300),
            native(B, &plum_text(&["第二条"]), 200),
        ];
        let report = import_notes(&store, &notes, &ShadowMarkdown::default()).unwrap();
        assert_eq!(report, ImportReport { added: 2, updated: 0, unchanged: 0 });
        assert!(report.changed());

        let live = store.live_notes().unwrap();
        assert_eq!(live.len(), 2);
        assert_eq!(live[0].id, A);
        assert_eq!(live[0].markdown, "标题\n**加粗**");
        assert_eq!(live[0].theme, "Blue");
        assert_eq!((live[0].created_at, live[0].updated_at), (100, 300));
        assert_eq!(live[1].updated_at, 200);
    }

    #[test]
    fn missing_native_fields_get_sane_defaults() {
        let store = NoteStore::open_in_memory().unwrap();
        let bare = PlumNote {
            id: A.to_string(),
            text: String::new(),
            theme: None,
            created_at: None,
            updated_at: None,
        };
        import_notes(&store, &[bare], &ShadowMarkdown::default()).unwrap();
        let note = &store.live_notes().unwrap()[0];
        assert_eq!((note.markdown.as_str(), note.theme.as_str()), ("", "Yellow"));
        assert!(note.updated_at > 0);
        assert_eq!(note.created_at, note.updated_at);
    }

    #[test]
    fn the_saved_markdown_is_used_while_the_native_row_is_still_ours() {
        let store = NoteStore::open_in_memory().unwrap();
        let md = "# 标题\n\n```rust\nfn main() {}\n```";
        let written = markup::markdown_to_plum(md);
        let mut shadow = ShadowMarkdown::default();
        shadow.insert(A, &plum::normalize(&written), md);
        // The other note was edited in the native app after we wrote it.
        shadow.insert(B, &plum::normalize(&plum_text(&["旧内容"])), "# 旧内容");

        // The native app re-mints paragraph ids on sync; still the same text.
        let reminted = markup::markdown_to_plum(md);
        assert_ne!(written, reminted);
        let notes = [native(A, &reminted, 300), native(B, &plum_text(&["新内容"]), 200)];
        import_notes(&store, &notes, &shadow).unwrap();

        assert_eq!(markdown(&store, A).as_deref(), Some(md));
        assert_eq!(markdown(&store, B).as_deref(), Some("新内容"));
    }

    #[test]
    fn importing_again_changes_nothing_and_keeps_local_edits() {
        let store = NoteStore::open_in_memory().unwrap();
        let notes = [native(A, &plum_text(&["原文"]), 300)];
        import_notes(&store, &notes, &ShadowMarkdown::default()).unwrap();
        store.save_markdown(A, "# 本地改过\n带格式").unwrap();

        // Same content, new paragraph ids and even a newer native timestamp.
        let again = [native(A, &plum_text(&["原文"]), i64::MAX)];
        let report = import_notes(&store, &again, &ShadowMarkdown::default()).unwrap();
        assert_eq!(report, ImportReport { added: 0, updated: 0, unchanged: 1 });
        assert!(!report.changed());
        assert_eq!(markdown(&store, A).as_deref(), Some("# 本地改过\n带格式"));
    }

    #[test]
    fn a_newer_native_edit_updates_the_note() {
        let store = NoteStore::open_in_memory().unwrap();
        import_notes(&store, &[native(A, &plum_text(&["原文"]), 300)], &ShadowMarkdown::default())
            .unwrap();

        let mut edited = native(A, &plum_text(&["在原生便签里改了"]), 400);
        edited.theme = Some("Pink".to_string());
        let report = import_notes(&store, &[edited], &ShadowMarkdown::default()).unwrap();
        assert_eq!(report, ImportReport { added: 0, updated: 1, unchanged: 0 });
        let note = &store.live_notes().unwrap()[0];
        assert_eq!(note.markdown, "在原生便签里改了");
        assert_eq!((note.theme.as_str(), note.created_at, note.updated_at), ("Pink", 100, 400));
    }

    #[test]
    fn a_native_edit_older_than_our_copy_loses_and_is_not_offered_again() {
        let store = NoteStore::open_in_memory().unwrap();
        import_notes(&store, &[native(A, &plum_text(&["原文"]), 300)], &ShadowMarkdown::default())
            .unwrap();
        let ours_at = store.save_markdown(A, "本地更新").unwrap();

        let stale = [native(A, &plum_text(&["较早的原生修改"]), ours_at - 1)];
        let report = import_notes(&store, &stale, &ShadowMarkdown::default()).unwrap();
        assert_eq!(report.unchanged, 1);
        assert_eq!(markdown(&store, A).as_deref(), Some("本地更新"));
        // Seen and rejected once; a later import treats that row as known.
        let report = import_notes(&store, &stale, &ShadowMarkdown::default()).unwrap();
        assert_eq!(report.unchanged, 1);
    }

    #[test]
    fn a_note_deleted_here_stays_deleted_until_it_changes_in_the_native_app() {
        let store = NoteStore::open_in_memory().unwrap();
        let original = [native(A, &plum_text(&["原文"]), 300)];
        import_notes(&store, &original, &ShadowMarkdown::default()).unwrap();
        store.soft_delete(A).unwrap();

        let report = import_notes(&store, &original, &ShadowMarkdown::default()).unwrap();
        assert_eq!(report, ImportReport { added: 0, updated: 0, unchanged: 1 });
        assert_eq!(markdown(&store, A), None);

        // Edited in the native app after we deleted it: the newer intent wins.
        let revived = [native(A, &plum_text(&["删除之后又改了"]), i64::MAX)];
        let report = import_notes(&store, &revived, &ShadowMarkdown::default()).unwrap();
        assert_eq!(report.updated, 1);
        assert_eq!(markdown(&store, A).as_deref(), Some("删除之后又改了"));
    }

    #[test]
    fn a_note_that_arrived_over_webdav_keeps_its_markdown_when_the_native_row_matches() {
        let store = NoteStore::open_in_memory().unwrap();
        let md = "# 标题\n- [ ] 待办";
        store
            .apply_remote(&RemoteUpdate {
                id: A,
                markdown: md,
                theme: "Green",
                updated_at: 300,
                tags: &[],
                pinned: false,
            })
            .unwrap();

        // The same note sits in the native database (plain text only) with a
        // later timestamp; it is not a newer edit.
        let same = [native(A, &markup::markdown_to_plum(md), 900)];
        let report = import_notes(&store, &same, &ShadowMarkdown::default()).unwrap();
        assert_eq!(report.unchanged, 1);
        assert_eq!(markdown(&store, A).as_deref(), Some(md));
    }

    #[test]
    fn a_failed_import_leaves_the_database_as_it_was() {
        let store = NoteStore::open_in_memory().unwrap();
        // Recording the import marker fails after the note row was written;
        // the whole import must roll back.
        store.conn.execute_batch("DROP TABLE plum_import").unwrap();
        let notes = [native(A, &plum_text(&["一"]), 300), native(B, &plum_text(&["二"]), 300)];
        assert!(import_notes(&store, &notes, &ShadowMarkdown::default()).is_err());
        assert_eq!(store.note_count().unwrap(), 0);
    }
}
