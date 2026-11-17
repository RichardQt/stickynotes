//! Local tags database (app-owned, not cloud-synced).

use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
}

pub struct TagStore {
    conn: Connection,
}

fn db_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    let dir = std::path::Path::new(&base).join("stickynotes");
    std::fs::create_dir_all(&dir).ok();
    dir.join("tags.db")
}

impl TagStore {
    pub fn open() -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path())?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tag(
                id INTEGER PRIMARY KEY,
                name TEXT UNIQUE NOT NULL,
                color TEXT);
             CREATE TABLE IF NOT EXISTS note_tag(
                note_guid TEXT NOT NULL,
                tag_id INTEGER NOT NULL,
                PRIMARY KEY(note_guid, tag_id));
             CREATE INDEX IF NOT EXISTS idx_note_tag_tag ON note_tag(tag_id);
             CREATE TABLE IF NOT EXISTS pinned_note(
                note_guid TEXT PRIMARY KEY,
                pinned_at INTEGER NOT NULL);",
        )?;
        Ok(Self { conn })
    }

    pub fn list_tags(&self) -> rusqlite::Result<Vec<Tag>> {
        let mut stmt = self.conn.prepare("SELECT id, name, color FROM tag ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok(Tag { id: row.get(0)?, name: row.get(1)?, color: row.get(2)? })
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn create_tag(&self, name: &str) -> rusqlite::Result<i64> {
        self.conn.execute("INSERT OR IGNORE INTO tag(name) VALUES (?1)", params![name])?;
        let mut stmt = self.conn.prepare("SELECT id FROM tag WHERE name = ?1")?;
        let id: i64 = stmt.query_row(params![name], |r| r.get(0))?;
        Ok(id)
    }

    pub fn all_note_tags(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare("SELECT note_guid, tag_id FROM note_tag")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.flatten().collect())
    }

    pub fn delete_tag(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute_batch(&format!(
            "BEGIN; DELETE FROM note_tag WHERE tag_id = {id}; DELETE FROM tag WHERE id = {id}; COMMIT;"
        ))?;
        Ok(())
    }

    pub fn tag_note(&self, note_guid: &str, tag_id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO note_tag(note_guid, tag_id) VALUES (?1, ?2)",
            params![note_guid, tag_id],
        )?;
        Ok(())
    }

    pub fn untag_note(&self, note_guid: &str, tag_id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM note_tag WHERE note_guid = ?1 AND tag_id = ?2",
            params![note_guid, tag_id],
        )?;
        Ok(())
    }

    /// Tag ids attached to a note.
    pub fn tags_of_note(&self, note_guid: &str) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self.conn.prepare("SELECT tag_id FROM note_tag WHERE note_guid = ?1")?;
        let rows = stmt.query_map(params![note_guid], |r| r.get(0))?;
        Ok(rows.flatten().collect())
    }

    /// Note guids attached to a tag (for filtering).
    pub fn notes_of_tag(&self, tag_id: i64) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT note_guid FROM note_tag WHERE tag_id = ?1")?;
        let rows = stmt.query_map(params![tag_id], |r| r.get(0))?;
        Ok(rows.flatten().collect())
    }

    /// Lazy cleanup: drop associations whose note no longer exists in plum.
    pub fn prune_orphans(&self, live_guids: &[String]) -> rusqlite::Result<()> {
        // An empty live set means plum could not be read (locked, mid-write,
        // transient IO error), not that every note was deleted. Pruning then
        // would silently wipe all tag links and pins — skip instead.
        if live_guids.is_empty() {
            return Ok(());
        }
        self.conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS live(guid TEXT PRIMARY KEY); DELETE FROM live;",
        )?;
        for g in live_guids {
            self.conn.execute("INSERT OR IGNORE INTO live(guid) VALUES (?1)", params![g])?;
        }
        self.conn.execute(
            "DELETE FROM note_tag WHERE note_guid NOT IN (SELECT guid FROM live)",
            [],
        )?;
        self.conn.execute(
            "DELETE FROM pinned_note WHERE note_guid NOT IN (SELECT guid FROM live)",
            [],
        )?;
        Ok(())
    }

    // --- Pinned notes ---

    /// All pinned note guids, most recently pinned first.
    pub fn pinned_guids(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT note_guid FROM pinned_note ORDER BY pinned_at DESC")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.flatten().collect())
    }

    /// Pin or unpin a note.
    pub fn set_pinned(&self, note_guid: &str, pinned: bool) -> rusqlite::Result<()> {
        if pinned {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            self.conn.execute(
                "INSERT OR REPLACE INTO pinned_note(note_guid, pinned_at) VALUES (?1, ?2)",
                params![note_guid, now],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM pinned_note WHERE note_guid = ?1",
                params![note_guid],
            )?;
        }
        Ok(())
    }

    pub fn tag_name(&self, id: i64) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT name FROM tag WHERE id = ?1", params![id], |r| r.get(0))
            .optional()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> TagStore {
        let dir = std::env::temp_dir().join(format!("sn-tags-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("tags.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tag(id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, color TEXT);
             CREATE TABLE IF NOT EXISTS note_tag(note_guid TEXT NOT NULL, tag_id INTEGER NOT NULL, PRIMARY KEY(note_guid, tag_id));
             CREATE TABLE IF NOT EXISTS pinned_note(note_guid TEXT PRIMARY KEY, pinned_at INTEGER NOT NULL);",
        ).unwrap();
        TagStore { conn }
    }

    #[test]
    fn tag_crud() {
        let s = tmp_store();
        let id = s.create_tag("work").unwrap();
        s.tag_note("abc", id).unwrap();
        s.tag_note("abc", id).unwrap(); // idempotent
        assert_eq!(s.tags_of_note("abc").unwrap(), vec![id]);
        assert_eq!(s.notes_of_tag(id).unwrap(), vec!["abc".to_string()]);
        s.untag_note("abc", id).unwrap();
        assert!(s.tags_of_note("abc").unwrap().is_empty());
        s.delete_tag(id).unwrap();
        assert!(s.list_tags().unwrap().is_empty());
    }
}
