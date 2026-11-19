//! The app's own note database.
//!
//! One SQLite file (`stickynotes.db`) holds everything the app owns: each
//! note's canonical Markdown, its colour, tags and pins. Markdown is stored
//! verbatim, so headings, fences, tables and task boxes survive unchanged.
//!
//! Windows Sticky Notes is only ever *read*, as an import source — see
//! [`plum`] and [`import`]. [`legacy`] carries over what earlier versions of
//! this app kept beside the native database.

pub mod backup;
pub mod import;
pub mod legacy;
pub mod model;
pub mod plum;
mod preview;

pub use preview::{strip_markdown_syntax, title_and_snippet};

use crate::ticks;
use model::{Note, NoteSummary, RemoteUpdate, Tag};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Marks the file as ours (`PRAGMA application_id`), so pointing the app at
/// some other program's SQLite file is refused instead of written into.
const APPLICATION_ID: i64 = 0x5374_4E74; // "StNt"

/// Current schema version (`PRAGMA user_version`).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA_V1: &str = "
CREATE TABLE note (
    id         TEXT PRIMARY KEY NOT NULL,
    markdown   TEXT NOT NULL DEFAULT '',
    theme      TEXT NOT NULL DEFAULT 'Yellow',
    pinned_at  INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE TABLE tag (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE note_tag (
    note_id TEXT NOT NULL REFERENCES note(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    PRIMARY KEY (note_id, tag_id)
) WITHOUT ROWID;
CREATE INDEX note_tag_by_tag ON note_tag(tag_id);
CREATE TABLE plum_import (
    note_id TEXT PRIMARY KEY NOT NULL,
    plum    TEXT NOT NULL
);
CREATE TABLE meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) WITHOUT ROWID;
";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库文件不存在")]
    Missing(PathBuf),
    #[error("这个文件不是便签数据库")]
    NotANoteDatabase(PathBuf),
    #[error("数据库由更新版本的应用创建（结构版本 {0}），当前版本无法打开")]
    NewerSchema(i64),
    #[error("便签 {0} 不存在或已删除")]
    NoteNotFound(String),
    #[error("数据库错误：{0}")]
    Db(#[from] rusqlite::Error),
    #[error("文件读写失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

impl StoreError {
    /// Another connection held the lock for longer than the busy timeout;
    /// the same write is worth retrying shortly.
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            StoreError::Db(rusqlite::Error::SqliteFailure(e, _))
                if matches!(
                    e.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
        )
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct NoteStore {
    conn: Connection,
    /// Empty for an in-memory database.
    path: PathBuf,
}

impl NoteStore {
    /// Open the database at `path`, creating the file (and its folder) when
    /// it does not exist yet.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        Self::from_connection(Connection::open(path)?, path.to_path_buf())
    }

    /// Open a database that is expected to be there. A missing file is an
    /// error rather than a reason to start an empty database in its place.
    pub fn open_existing(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Err(StoreError::Missing(path.to_path_buf()));
        }
        Self::from_connection(Connection::open(path)?, path.to_path_buf())
    }

    /// A private, empty database that lives only as long as this value.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, PathBuf::new())
    }

    fn from_connection(conn: Connection, path: PathBuf) -> Result<Self> {
        conn.busy_timeout(Duration::from_millis(5000))?;
        init_schema(&conn, &path)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(Self { conn, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_in_memory(&self) -> bool {
        self.path.as_os_str().is_empty()
    }

    // --- Notes ---

    /// Live (non-deleted) notes, newest first.
    pub fn list_notes(&self) -> Result<Vec<NoteSummary>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, markdown, theme, created_at, updated_at FROM note \
             WHERE deleted_at IS NULL ORDER BY updated_at DESC, id",
        )?;
        let rows = stmt.query_map([], |row| {
            let markdown: String = row.get(1)?;
            let (title, snippet) = title_and_snippet(&markdown);
            Ok(NoteSummary {
                id: row.get(0)?,
                title,
                snippet,
                theme: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Live notes with their full Markdown, newest first.
    pub fn live_notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, markdown, theme, created_at, updated_at FROM note \
             WHERE deleted_at IS NULL ORDER BY updated_at DESC, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Note {
                id: row.get(0)?,
                markdown: row.get(1)?,
                theme: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Number of live notes.
    pub fn note_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM note WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// One live note's Markdown.
    pub fn note_markdown(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT markdown FROM note WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Create a note and return its id.
    pub fn create_note(&self, markdown: &str, theme: &str) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = ticks::now_ticks();
        self.conn.execute(
            "INSERT INTO note (id, markdown, theme, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, markdown, theme, now],
        )?;
        Ok(id)
    }

    /// Replace a live note's Markdown and return the `updated_at` written.
    ///
    /// The timestamp always moves forward, even if the system clock stepped
    /// back: sync resolves conflicts by `updated_at`, so an edit that looked
    /// older than the version it was typed over would lose to it.
    pub fn save_markdown(&self, id: &str, markdown: &str) -> Result<i64> {
        self.conn
            .query_row(
                "UPDATE note SET markdown = ?1, updated_at = MAX(?2, updated_at + 1) \
                 WHERE id = ?3 AND deleted_at IS NULL RETURNING updated_at",
                params![markdown, ticks::now_ticks(), id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NoteNotFound(id.to_string()))
    }

    /// Set a note's paper colour.
    pub fn set_theme(&self, id: &str, theme: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE note SET theme = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            params![theme, id],
        )?;
        Ok(())
    }

    /// Delete a note. The row stays behind as a tombstone (so an import from
    /// Windows Sticky Notes does not bring the note back), without its tags
    /// and pin.
    pub fn soft_delete(&self, id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE note SET deleted_at = ?1, pinned_at = NULL \
             WHERE id = ?2 AND deleted_at IS NULL",
            params![ticks::now_ticks(), id],
        )?;
        tx.execute("DELETE FROM note_tag WHERE note_id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    /// Store a note version downloaded from another device, together with its
    /// tags and pin. Creates the note when this device has never seen its id
    /// and brings back one that was deleted here.
    ///
    /// Returns `false`, changing nothing, when the local note is at least as
    /// new: it was edited after the sync pass compared the two, and the next
    /// pass will upload it instead.
    pub fn apply_remote(&self, remote: &RemoteUpdate<'_>) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let local: Option<(i64, Option<i64>)> = tx
            .query_row(
                "SELECT updated_at, deleted_at FROM note WHERE id = ?1",
                params![remote.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match local {
            Some((updated_at, None)) if updated_at >= remote.updated_at => return Ok(false),
            Some(_) => {
                tx.execute(
                    "UPDATE note SET markdown = ?1, theme = ?2, updated_at = ?3, deleted_at = NULL \
                     WHERE id = ?4",
                    params![remote.markdown, remote.theme, remote.updated_at, remote.id],
                )?;
            }
            None => {
                tx.execute(
                    "INSERT INTO note (id, markdown, theme, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![remote.id, remote.markdown, remote.theme, remote.updated_at],
                )?;
            }
        }
        set_note_tags(&tx, remote.id, remote.tags)?;
        set_pinned(&tx, remote.id, remote.pinned)?;
        tx.commit()?;
        Ok(true)
    }

    // --- Tags ---

    pub fn list_tags(&self) -> Result<Vec<Tag>> {
        let mut stmt = self.conn.prepare_cached("SELECT id, name FROM tag ORDER BY name")?;
        let rows = stmt.query_map([], |row| Ok(Tag { id: row.get(0)?, name: row.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The id of the tag called `name`, creating it when it does not exist.
    pub fn create_tag(&self, name: &str) -> Result<i64> {
        Ok(ensure_tag(&self.conn, name)?)
    }

    /// Delete a tag and detach it from every note.
    pub fn delete_tag(&self, id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM note_tag WHERE tag_id = ?1", params![id])?;
        tx.execute("DELETE FROM tag WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn tag_note(&self, note_id: &str, tag_id: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO note_tag (note_id, tag_id) VALUES (?1, ?2)",
            params![note_id, tag_id],
        )?;
        Ok(())
    }

    pub fn untag_note(&self, note_id: &str, tag_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM note_tag WHERE note_id = ?1 AND tag_id = ?2",
            params![note_id, tag_id],
        )?;
        Ok(())
    }

    /// Every (note id, tag id) link.
    pub fn note_tags(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare_cached("SELECT note_id, tag_id FROM note_tag")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // --- Pins ---

    /// Pinned note ids, most recently pinned first.
    pub fn pinned_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id FROM note WHERE pinned_at IS NOT NULL AND deleted_at IS NULL \
             ORDER BY pinned_at DESC",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_pinned(&self, note_id: &str, pinned: bool) -> Result<()> {
        Ok(set_pinned(&self.conn, note_id, pinned)?)
    }

    // --- Bookkeeping ---

    /// A value from the key/value table that records one-time steps.
    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(read_meta(&self.conn, key)?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        Ok(write_meta(&self.conn, key, value)?)
    }

    // --- Copies ---

    /// Write a consistent copy of the whole database to `dest` and verify it.
    /// `dest` must not exist: a copy never overwrites anything.
    pub fn copy_to(&self, dest: &Path) -> Result<()> {
        if dest.exists() {
            return Err(StoreError::Other(format!(
                "目标位置已存在同名文件：{}",
                dest.display()
            )));
        }
        let Some(target) = dest.to_str() else {
            return Err(StoreError::Other("路径包含无法识别的字符".into()));
        };
        if let Some(dir) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let copied = self
            .conn
            .execute("VACUUM INTO ?1", params![target])
            .map_err(StoreError::from)
            .and_then(|_| self.verify_copy(dest));
        if copied.is_err() {
            // `dest` did not exist before this call, so whatever is there now
            // is our own incomplete copy.
            let _ = std::fs::remove_file(dest);
        }
        copied
    }

    fn verify_copy(&self, dest: &Path) -> Result<()> {
        let copy = Connection::open_with_flags(
            dest,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let check: String = copy.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(StoreError::Other(format!("副本校验失败：{check}")));
        }
        if row_counts(&copy)? != row_counts(&self.conn)? {
            return Err(StoreError::Other("副本校验失败：内容与原数据库不一致".into()));
        }
        Ok(())
    }

    /// Copy this database to `new_path` and open the copy. This store is left
    /// exactly as it was; removing its file afterwards (see
    /// [`remove_database_files`]) completes a move.
    pub fn copy_and_open(&self, new_path: &Path) -> Result<NoteStore> {
        self.copy_to(new_path)?;
        NoteStore::open(new_path).inspect_err(|_| {
            let _ = std::fs::remove_file(new_path);
        })
    }
}

/// Delete a database file together with SQLite's side files.
pub fn remove_database_files(path: &Path) -> std::io::Result<()> {
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut side = path.as_os_str().to_owned();
        side.push(suffix);
        match std::fs::remove_file(PathBuf::from(side)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Create the schema in a new file, or check that an existing file is one of
/// ours and not from a newer version.
fn init_schema(conn: &Connection, path: &Path) -> Result<()> {
    let tables: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
        [],
        |r| r.get(0),
    )?;
    if tables == 0 {
        conn.execute_batch(&format!(
            "BEGIN;\n{SCHEMA_V1}\nPRAGMA application_id = {APPLICATION_ID};\n\
             PRAGMA user_version = {SCHEMA_VERSION};\nCOMMIT;"
        ))?;
        return Ok(());
    }
    let application_id: i64 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
    if application_id != APPLICATION_ID {
        return Err(StoreError::NotANoteDatabase(path.to_path_buf()));
    }
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(StoreError::NewerSchema(version));
    }
    // Upgrades from older schema versions go here, one step per version.
    Ok(())
}

fn row_counts(conn: &Connection) -> rusqlite::Result<(i64, i64, i64)> {
    conn.query_row(
        "SELECT (SELECT COUNT(*) FROM note), (SELECT COUNT(*) FROM tag), \
         (SELECT COUNT(*) FROM note_tag)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

fn ensure_tag(conn: &Connection, name: &str) -> rusqlite::Result<i64> {
    conn.execute("INSERT OR IGNORE INTO tag (name) VALUES (?1)", params![name])?;
    conn.query_row("SELECT id FROM tag WHERE name = ?1", params![name], |r| r.get(0))
}

/// Replace a note's tag set with the tags called `names`.
fn set_note_tags(conn: &Connection, note_id: &str, names: &[String]) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM note_tag WHERE note_id = ?1", params![note_id])?;
    for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        let tag_id = ensure_tag(conn, name)?;
        conn.execute(
            "INSERT OR IGNORE INTO note_tag (note_id, tag_id) VALUES (?1, ?2)",
            params![note_id, tag_id],
        )?;
    }
    Ok(())
}

fn set_pinned(conn: &Connection, note_id: &str, pinned: bool) -> rusqlite::Result<()> {
    let pinned_at = pinned.then(ticks::now_ticks);
    conn.execute(
        "UPDATE note SET pinned_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
        params![pinned_at, note_id],
    )?;
    Ok(())
}

fn read_meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0))
        .optional()
}

fn write_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::plum::testing::temp_dir;
    use super::*;

    fn store() -> NoteStore {
        NoteStore::open_in_memory().unwrap()
    }

    fn remote<'a>(id: &'a str, markdown: &'a str, updated_at: i64, tags: &'a [String]) -> RemoteUpdate<'a> {
        RemoteUpdate { id, markdown, theme: "Green", updated_at, tags, pinned: true }
    }

    #[test]
    fn a_new_file_gets_the_schema_and_keeps_its_notes_across_reopens() {
        let dir = temp_dir("store-open");
        let path = dir.join("nested").join("stickynotes.db");
        let id = {
            let store = NoteStore::open(&path).unwrap();
            assert_eq!(store.path(), path);
            assert!(!store.is_in_memory());
            store.create_note("# 标题\n正文", "Blue").unwrap()
        };
        let store = NoteStore::open(&path).unwrap();
        assert_eq!(store.note_markdown(&id).unwrap().as_deref(), Some("# 标题\n正文"));
        let notes = store.list_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "标题");
        assert_eq!(notes[0].snippet, "正文");
        assert_eq!(notes[0].theme, "Blue");
        assert_eq!(notes[0].created_at, notes[0].updated_at);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_expected_database_that_is_missing_is_an_error_not_a_new_file() {
        let dir = temp_dir("store-missing");
        let path = dir.join("gone").join("stickynotes.db");
        assert!(matches!(
            NoteStore::open_existing(&path),
            Err(StoreError::Missing(p)) if p == path
        ));
        assert!(!path.exists() && !path.parent().unwrap().exists());

        // Once it exists, it opens like any other.
        let id = NoteStore::open(&path).unwrap().create_note("x", "Yellow").unwrap();
        let store = NoteStore::open_existing(&path).unwrap();
        assert_eq!(store.note_markdown(&id).unwrap().as_deref(), Some("x"));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_foreign_sqlite_file_is_refused_and_left_untouched() {
        let dir = temp_dir("store-foreign");
        let path = dir.join("stickynotes.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE other(x); INSERT INTO other VALUES (1);")
            .unwrap();
        let before = std::fs::read(&path).unwrap();

        let err = NoteStore::open(&path).err().expect("must be refused");
        assert!(matches!(err, StoreError::NotANoteDatabase(_)), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), before);

        // Not SQLite at all.
        let junk = dir.join("junk.db");
        std::fs::write(&junk, b"definitely not a database, just some text").unwrap();
        assert!(matches!(NoteStore::open(&junk), Err(StoreError::Db(_))));
        assert_eq!(std::fs::read(&junk).unwrap().len(), 41);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_database_from_a_newer_version_is_refused() {
        let dir = temp_dir("store-newer");
        let path = dir.join("stickynotes.db");
        drop(NoteStore::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(matches!(
            NoteStore::open(&path),
            Err(StoreError::NewerSchema(v)) if v == SCHEMA_VERSION + 1
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_keeps_markdown_verbatim_and_moves_the_timestamp_forward() {
        let store = store();
        let id = store.create_note("", "Yellow").unwrap();
        let md = "# 标题\n\n```rust\nfn main() {}\n```\n\n| a | b |\n| - | - |\n- [ ] 待办";
        let first = store.save_markdown(&id, md).unwrap();
        assert_eq!(store.note_markdown(&id).unwrap().as_deref(), Some(md));
        assert_eq!(store.list_notes().unwrap()[0].updated_at, first);

        // A clock that jumped far back must not produce an older timestamp.
        store
            .conn
            .execute("UPDATE note SET updated_at = ?1", params![i64::MAX - 10])
            .unwrap();
        assert_eq!(store.save_markdown(&id, "later").unwrap(), i64::MAX - 9);
    }

    #[test]
    fn saving_a_missing_or_deleted_note_fails_instead_of_resurrecting_it() {
        let store = store();
        assert!(matches!(
            store.save_markdown("nope", "x"),
            Err(StoreError::NoteNotFound(_))
        ));
        let id = store.create_note("kept", "Yellow").unwrap();
        store.soft_delete(&id).unwrap();
        assert!(matches!(
            store.save_markdown(&id, "zombie"),
            Err(StoreError::NoteNotFound(_))
        ));
        let text: String = store
            .conn
            .query_row("SELECT markdown FROM note WHERE id = ?1", params![id], |r| r.get(0))
            .unwrap();
        assert_eq!(text, "kept", "the tombstone keeps its last content");
    }

    #[test]
    fn notes_list_newest_first() {
        let store = store();
        let a = store.create_note("a", "Yellow").unwrap();
        let b = store.create_note("b", "Yellow").unwrap();
        for (id, at) in [(&a, 100), (&b, 200)] {
            store
                .conn
                .execute("UPDATE note SET updated_at = ?1 WHERE id = ?2", params![at, id])
                .unwrap();
        }
        let ids: Vec<String> = store.list_notes().unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(ids, [b.clone(), a.clone()]);

        // Editing the older note moves it to the top.
        store.save_markdown(&a, "a2").unwrap();
        let ids: Vec<String> = store.list_notes().unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(ids, [a.clone(), b.clone()]);
        let full: Vec<String> = store.live_notes().unwrap().into_iter().map(|n| n.markdown).collect();
        assert_eq!(full, ["a2", "b"]);
        assert_eq!(store.note_count().unwrap(), 2);
    }

    #[test]
    fn deleting_a_note_hides_it_and_drops_its_tags_and_pin() {
        let store = store();
        let id = store.create_note("note", "Yellow").unwrap();
        let keep = store.create_note("other", "Yellow").unwrap();
        let tag = store.create_tag("work").unwrap();
        store.tag_note(&id, tag).unwrap();
        store.tag_note(&keep, tag).unwrap();
        store.set_pinned(&id, true).unwrap();
        store.set_theme(&id, "Pink").unwrap();
        assert_eq!(store.pinned_ids().unwrap(), [id.as_str()]);

        store.soft_delete(&id).unwrap();
        assert_eq!(store.note_markdown(&id).unwrap(), None);
        assert_eq!(store.note_count().unwrap(), 1);
        assert_eq!(store.note_tags().unwrap(), [(keep.clone(), tag)]);
        assert!(store.pinned_ids().unwrap().is_empty());
        // Deleting again, or deleting an unknown id, is harmless.
        store.soft_delete(&id).unwrap();
        store.soft_delete("unknown").unwrap();
        // The tag itself stays for the other notes.
        assert_eq!(store.list_tags().unwrap().len(), 1);
    }

    #[test]
    fn tags_are_created_once_and_removed_with_their_links() {
        let store = store();
        let note = store.create_note("n", "Yellow").unwrap();
        let work = store.create_tag("work").unwrap();
        assert_eq!(store.create_tag("work").unwrap(), work, "idempotent");
        let home = store.create_tag("home").unwrap();
        store.tag_note(&note, work).unwrap();
        store.tag_note(&note, work).unwrap();
        store.tag_note(&note, home).unwrap();
        let names: Vec<String> = store.list_tags().unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["home", "work"]);

        store.untag_note(&note, home).unwrap();
        assert_eq!(store.note_tags().unwrap(), [(note.clone(), work)]);
        store.delete_tag(work).unwrap();
        assert!(store.note_tags().unwrap().is_empty());
        assert_eq!(store.list_tags().unwrap().len(), 1);
        // A link needs a real note.
        assert!(store.tag_note("no-such-note", home).is_err());
    }

    #[test]
    fn a_remote_note_is_created_with_its_timestamp_tags_and_pin() {
        let store = store();
        let tags = vec!["工作".to_string(), " dav ".to_string(), String::new()];
        assert!(store.apply_remote(&remote("r-1", "# 远端\n正文", 500, &tags)).unwrap());

        let note = &store.live_notes().unwrap()[0];
        assert_eq!((note.id.as_str(), note.markdown.as_str()), ("r-1", "# 远端\n正文"));
        assert_eq!((note.theme.as_str(), note.created_at, note.updated_at), ("Green", 500, 500));
        let names: Vec<String> = store.list_tags().unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["dav", "工作"]);
        assert_eq!(store.note_tags().unwrap().len(), 2);
        assert_eq!(store.pinned_ids().unwrap(), ["r-1"]);
    }

    #[test]
    fn a_remote_note_replaces_an_older_local_one_but_not_a_newer_one() {
        let store = store();
        let old_tags = vec!["old".to_string()];
        store.apply_remote(&remote("r-1", "v1", 500, &old_tags)).unwrap();

        // Newer remote: content, tags and pin are replaced.
        let update = RemoteUpdate {
            id: "r-1",
            markdown: "v2",
            theme: "Blue",
            updated_at: 600,
            tags: &[],
            pinned: false,
        };
        assert!(store.apply_remote(&update).unwrap());
        let note = &store.live_notes().unwrap()[0];
        assert_eq!((note.markdown.as_str(), note.theme.as_str()), ("v2", "Blue"));
        assert_eq!((note.created_at, note.updated_at), (500, 600));
        assert!(store.note_tags().unwrap().is_empty());
        assert!(store.pinned_ids().unwrap().is_empty());

        // Same or older remote: the local note (edited meanwhile) wins.
        let tags = vec!["x".to_string()];
        assert!(!store.apply_remote(&remote("r-1", "stale", 600, &tags)).unwrap());
        assert!(!store.apply_remote(&remote("r-1", "stale", 599, &tags)).unwrap());
        assert_eq!(store.note_markdown("r-1").unwrap().as_deref(), Some("v2"));
        assert!(store.note_tags().unwrap().is_empty());
    }

    #[test]
    fn a_remote_note_brings_back_one_deleted_here() {
        let store = store();
        store.apply_remote(&remote("r-1", "v1", 500, &[])).unwrap();
        store.soft_delete("r-1").unwrap();
        // Even an older remote version: the file exists on the server and
        // nothing asked for it to be removed.
        assert!(store.apply_remote(&remote("r-1", "back", 400, &[])).unwrap());
        assert_eq!(store.note_markdown("r-1").unwrap().as_deref(), Some("back"));
        assert_eq!(store.note_count().unwrap(), 1);
    }

    #[test]
    fn meta_values_can_be_set_and_replaced() {
        let store = store();
        assert_eq!(store.meta("k").unwrap(), None);
        store.set_meta("k", "1").unwrap();
        store.set_meta("k", "2").unwrap();
        assert_eq!(store.meta("k").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn a_copy_is_complete_openable_and_never_overwrites() {
        let dir = temp_dir("store-copy");
        let store = NoteStore::open(&dir.join("a").join("stickynotes.db")).unwrap();
        let id = store.create_note("# 便签\n内容", "Purple").unwrap();
        let tag = store.create_tag("t").unwrap();
        store.tag_note(&id, tag).unwrap();
        store.set_pinned(&id, true).unwrap();
        store.set_meta("k", "v").unwrap();

        let dest = dir.join("b").join("stickynotes.db");
        let moved = store.copy_and_open(&dest).unwrap();
        assert_eq!(moved.path(), dest);
        assert_eq!(moved.live_notes().unwrap(), store.live_notes().unwrap());
        assert_eq!(moved.note_tags().unwrap(), [(id.clone(), tag)]);
        assert_eq!(moved.pinned_ids().unwrap(), [id.as_str()]);
        assert_eq!(moved.meta("k").unwrap().as_deref(), Some("v"));

        // The copy is independent of the original.
        moved.save_markdown(&id, "changed").unwrap();
        assert_eq!(store.note_markdown(&id).unwrap().as_deref(), Some("# 便签\n内容"));

        // An existing file is never replaced.
        let err = store.copy_to(&dest).unwrap_err();
        assert!(matches!(err, StoreError::Other(_)), "{err}");
        assert_eq!(moved.note_markdown(&id).unwrap().as_deref(), Some("changed"));

        // Removing the original's files completes a move.
        let old_path = store.path().to_path_buf();
        drop(store);
        remove_database_files(&old_path).unwrap();
        assert!(!old_path.exists());
        remove_database_files(&old_path).unwrap();
        drop(moved);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn busy_errors_are_recognised_for_retry() {
        let busy = StoreError::Db(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        ));
        assert!(busy.is_busy());
        assert!(!StoreError::NoteNotFound("x".into()).is_busy());
    }
}
