//! Shadow copy of each note's canonical Markdown.
//!
//! plum markup cannot represent headings, fenced code, tables, quotes, rules
//! or task boxes — it only carries bold/italic runs and list items. Writing the
//! editor's Markdown through plum and reading it back silently degrades the
//! formatting (`# 标题` comes back as `**标题**`). So we keep the exact Markdown
//! beside the plum text it produced and prefer it while the plum row is
//! unchanged.
//!
//! An external edit (the native Sticky Notes app or a cloud pull) changes the
//! plum row; the shadow copy then no longer matches and that note falls back to
//! the plum-derived Markdown, losing any formatting plum cannot hold.

use super::{PlumError, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Strip paragraph GUIDs so a rewrite that only re-mints ids (the native app
/// does this on sync) is still recognised as the same content.
fn normalize_plum(plum: &str) -> String {
    let mut out = String::with_capacity(plum.len());
    for (i, line) in plum.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match line.strip_prefix("\\id=") {
            // `\id=` + 36-char GUID; keep the remainder verbatim.
            Some(rest) if rest.len() >= 36 => out.push_str(&rest[36..]),
            _ => out.push_str(line),
        }
    }
    out
}

pub struct MarkdownStore {
    path: PathBuf,
}

impl MarkdownStore {
    pub fn open() -> Result<Self> {
        let local = std::env::var("LOCALAPPDATA")
            .map_err(|_| PlumError::Other("LOCALAPPDATA not set".into()))?;
        Self::open_at(&Path::new(&local).join("stickynotes"))
    }

    /// Open (creating if needed) the store in `dir`. Used by tests too.
    pub fn open_at(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let store = Self { path: dir.join("markdown.sqlite") };
        store.ensure_schema()?;
        Ok(store)
    }

    fn conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(Duration::from_millis(2000))?;
        Ok(conn)
    }

    fn ensure_schema(&self) -> Result<()> {
        self.conn()?.execute_batch(
            "CREATE TABLE IF NOT EXISTS NoteMarkdown (
                 note_id  TEXT PRIMARY KEY,
                 plum     TEXT NOT NULL,
                 markdown TEXT NOT NULL,
                 saved_at INTEGER NOT NULL
             );",
        )?;
        Ok(())
    }

    /// The stored Markdown, but only if it was written from this plum content.
    pub fn load(&self, note_id: &str, plum: &str) -> Option<String> {
        let conn = self.conn().ok()?;
        conn.query_row(
            "SELECT markdown FROM NoteMarkdown WHERE note_id = ?1 AND plum = ?2",
            params![note_id, normalize_plum(plum)],
            |r| r.get(0),
        )
        .ok()
    }

    pub fn save(&self, note_id: &str, plum: &str, markdown: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO NoteMarkdown (note_id, plum, markdown, saved_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(note_id) DO UPDATE SET
                 plum = excluded.plum,
                 markdown = excluded.markdown,
                 saved_at = excluded.saved_at",
            params![note_id, normalize_plum(plum), markdown, crate::ticks::now_ticks()],
        )?;
        Ok(())
    }

    /// Drop the shadow copy for a note (used when it is deleted for good).
    pub fn forget(&self, note_id: &str) -> Result<()> {
        self.conn()?.execute(
            "DELETE FROM NoteMarkdown WHERE note_id = ?1",
            params![note_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLUM: &str = "\\id=11111111-1111-1111-1111-111111111111 标题\n\\id=22222222-2222-2222-2222-222222222222 正文";
    const PLUM_REMINTED: &str = "\\id=33333333-3333-3333-3333-333333333333 标题\n\\id=44444444-4444-4444-4444-444444444444 正文";
    const MD: &str = "# 标题\n正文";

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("stickynotes-shadow-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn roundtrip_returns_markdown_while_plum_matches() {
        let dir = temp_dir();
        let store = MarkdownStore::open_at(&dir).unwrap();
        store.save("note-1", PLUM, MD).unwrap();
        assert_eq!(store.load("note-1", PLUM).as_deref(), Some(MD));
        // Re-minted paragraph ids are the same content.
        assert_eq!(store.load("note-1", PLUM_REMINTED).as_deref(), Some(MD));
        // A real external edit invalidates the shadow copy.
        assert_eq!(store.load("note-1", "\\id=x 别的内容"), None);
        assert_eq!(store.load("other", PLUM), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_replaces_existing_copy() {
        let dir = temp_dir();
        let store = MarkdownStore::open_at(&dir).unwrap();
        store.save("n", "\\id=x a", "**a**").unwrap();
        store.save("n", "\\id=x b", "## b").unwrap();
        assert_eq!(store.load("n", "\\id=x a"), None);
        assert_eq!(store.load("n", "\\id=x b").as_deref(), Some("## b"));
        store.forget("n").unwrap();
        assert_eq!(store.load("n", "\\id=x b"), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn normalize_keeps_non_guid_lines() {
        assert_eq!(normalize_plum("plain"), "plain");
        assert_eq!(normalize_plum("\\id=short x"), "\\id=short x");
    }
}
