//! Data written by earlier versions of this app, carried over once.
//!
//! Those versions kept notes inside Windows Sticky Notes' own database and
//! stored what that database could not hold in two files of their own:
//!
//! - `markdown.sqlite` — the exact Markdown of each note. plum only carries
//!   bold/italic runs and list items, so a note's headings, fences and
//!   tables lived here, valid while the native row was still the one the app
//!   had written;
//! - `tags.db` — tags, note↔tag links and pins.
//!
//! [`migrate_once`] moves all of it (plus the notes themselves) into the
//! app's own database. The old files are only read and are left in place.

use super::import::{self, ImportReport};
use super::plum;
use super::{read_meta, write_meta, NoteStore, Result};
use crate::ticks;
use rusqlite::{params, Connection, OpenFlags};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `meta` key holding when [`migrate_once`] completed for a database.
const META_MIGRATED: &str = "legacy_migrated_at";

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    // Read-only also means a missing file is an error, not a new empty file.
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // An older build of the app may still be running and writing these files.
    conn.busy_timeout(Duration::from_millis(5000))?;
    Ok(conn)
}

fn has_table(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
}

/// The full-fidelity Markdown copies from a legacy `markdown.sqlite`.
#[derive(Debug, Default)]
pub struct ShadowMarkdown {
    /// note id -> (normalized plum the Markdown was written as, Markdown).
    by_note: HashMap<String, (String, String)>,
}

impl ShadowMarkdown {
    /// Load every copy. A missing or unreadable file yields no copies: they
    /// only add formatting, the notes themselves come from the native rows.
    pub fn load(path: &Path) -> Self {
        Self::read(path).unwrap_or_default()
    }

    fn read(path: &Path) -> rusqlite::Result<Self> {
        let conn = open_read_only(path)?;
        let mut stmt = conn.prepare("SELECT note_id, plum, markdown FROM NoteMarkdown")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        })?;
        Ok(Self { by_note: rows.collect::<rusqlite::Result<_>>()? })
    }

    pub fn len(&self) -> usize {
        self.by_note.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_note.is_empty()
    }

    /// The stored Markdown, but only if it was written as this plum content
    /// (see [`plum::normalize`]); a note edited in the native app since then
    /// no longer matches.
    pub fn markdown_for(&self, note_id: &str, normalized_plum: &str) -> Option<&str> {
        self.by_note
            .get(note_id)
            .filter(|(plum, _)| plum == normalized_plum)
            .map(|(_, markdown)| markdown.as_str())
    }

    #[cfg(test)]
    pub(crate) fn insert(&mut self, note_id: &str, normalized_plum: &str, markdown: &str) {
        self.by_note.insert(
            note_id.to_string(),
            (normalized_plum.to_string(), markdown.to_string()),
        );
    }
}

/// What was carried over from a legacy `tags.db`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LegacyTagsReport {
    pub tags: usize,
    pub links: usize,
    pub pins: usize,
}

#[derive(Debug, Default)]
struct LegacyTags {
    /// (legacy id, name)
    tags: Vec<(i64, String)>,
    /// (note id, legacy tag id)
    links: Vec<(String, i64)>,
    /// (note id, unix milliseconds)
    pins: Vec<(String, i64)>,
}

impl LegacyTags {
    /// A missing file is simply "no tags". A file that exists but cannot be
    /// read is an error, which the migration reports instead of hiding.
    fn read(path: &Path) -> rusqlite::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let conn = open_read_only(path)?;
        let mut out = Self::default();
        if has_table(&conn, "tag")? {
            let mut stmt = conn.prepare("SELECT id, name FROM tag")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            out.tags = rows.collect::<rusqlite::Result<_>>()?;
        }
        if has_table(&conn, "note_tag")? {
            let mut stmt = conn.prepare("SELECT note_guid, tag_id FROM note_tag")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            out.links = rows.collect::<rusqlite::Result<_>>()?;
        }
        // Added after the first release; older files do not have it.
        if has_table(&conn, "pinned_note")? {
            let mut stmt = conn.prepare("SELECT note_guid, pinned_at FROM pinned_note")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            out.pins = rows.collect::<rusqlite::Result<_>>()?;
        }
        Ok(out)
    }

    /// Write into the new database. Links and pins of notes that are not
    /// live there are dropped: the old app pruned those lazily.
    fn write_into(&self, conn: &Connection) -> rusqlite::Result<LegacyTagsReport> {
        let mut report = LegacyTagsReport::default();
        let is_live = |note_id: &str| -> rusqlite::Result<bool> {
            conn.query_row(
                "SELECT COUNT(*) FROM note WHERE id = ?1 AND deleted_at IS NULL",
                params![note_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
        };

        let mut new_ids: HashMap<i64, i64> = HashMap::new();
        for (legacy_id, name) in &self.tags {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            report.tags += conn.execute("INSERT OR IGNORE INTO tag (name) VALUES (?1)", params![name])?;
            let id: i64 = conn.query_row("SELECT id FROM tag WHERE name = ?1", params![name], |r| r.get(0))?;
            new_ids.insert(*legacy_id, id);
        }
        for (note_id, legacy_tag) in &self.links {
            let Some(tag_id) = new_ids.get(legacy_tag) else { continue };
            if is_live(note_id)? {
                report.links += conn.execute(
                    "INSERT OR IGNORE INTO note_tag (note_id, tag_id) VALUES (?1, ?2)",
                    params![note_id, tag_id],
                )?;
            }
        }
        for (note_id, pinned_ms) in &self.pins {
            // The old table stored unix milliseconds; this database uses ticks.
            let pinned_at = ticks::EPOCH_TICKS + pinned_ms.max(&0) * 10_000;
            report.pins += conn.execute(
                "UPDATE note SET pinned_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
                params![pinned_at, note_id],
            )?;
        }
        Ok(report)
    }
}

/// Copy tags, note↔tag links and pins from a legacy `tags.db` into `store`.
pub fn import_tags(store: &NoteStore, tags_db: &Path) -> Result<LegacyTagsReport> {
    let legacy = LegacyTags::read(tags_db)?;
    let tx = store.conn.unchecked_transaction()?;
    let report = legacy.write_into(&tx)?;
    tx.commit()?;
    Ok(report)
}

/// Where the previous layout kept its data.
#[derive(Debug, Clone)]
pub struct LegacySources {
    /// Windows Sticky Notes' database, if that app is installed.
    pub plum: Option<PathBuf>,
    /// `markdown.sqlite` of earlier versions of this app.
    pub shadow: PathBuf,
    /// `tags.db` of earlier versions of this app.
    pub tags: PathBuf,
}

impl LegacySources {
    /// The locations on this machine.
    pub fn detect() -> Self {
        let dir = crate::config::config_dir();
        Self {
            plum: plum::find_plum_path(),
            shadow: dir.join("markdown.sqlite"),
            tags: dir.join("tags.db"),
        }
    }

    /// The saved Markdown copies, for [`import::import_from_windows`].
    pub fn shadow_markdown(&self) -> ShadowMarkdown {
        ShadowMarkdown::load(&self.shadow)
    }
}

/// What [`migrate_once`] carried over.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    /// Was a Windows Sticky Notes database found?
    pub windows_notes_found: bool,
    pub notes: ImportReport,
    pub tags: LegacyTagsReport,
    /// The legacy `tags.db` exists but could not be read; the notes were
    /// migrated without their tags and pins.
    pub tags_unreadable: bool,
}

/// Move the previous layout into `store`: the notes of Windows Sticky Notes
/// (with the Markdown saved beside them) and the legacy tags and pins.
///
/// Runs once per database — `None` means it already has. The notes are all
/// or nothing: if Windows Sticky Notes cannot be read nothing is written, and
/// the next start tries again. A damaged legacy tags file does not hold the
/// notes back.
pub fn migrate_once(store: &NoteStore, sources: &LegacySources) -> Result<Option<MigrationReport>> {
    if store.meta(META_MIGRATED)?.is_some() {
        return Ok(None);
    }
    // Read everything before opening the write transaction.
    let native = match &sources.plum {
        Some(path) => Some(plum::read_live_notes(path)?),
        None => None,
    };
    let shadow = sources.shadow_markdown();
    let (legacy_tags, tags_unreadable) = match LegacyTags::read(&sources.tags) {
        Ok(tags) => (tags, false),
        Err(_) => (LegacyTags::default(), true),
    };

    let tx = store.conn.unchecked_transaction()?;
    let report = MigrationReport {
        windows_notes_found: native.is_some(),
        notes: import::import_notes_in(&tx, native.as_deref().unwrap_or_default(), &shadow)?,
        tags: legacy_tags.write_into(&tx)?,
        tags_unreadable,
    };
    write_meta(&tx, META_MIGRATED, &ticks::now_ticks().to_string())?;
    tx.commit()?;
    Ok(Some(report))
}

/// Has [`migrate_once`] completed for this database?
pub fn is_migrated(store: &NoteStore) -> Result<bool> {
    Ok(read_meta(&store.conn, META_MIGRATED)?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup;
    use crate::store::plum::testing::{temp_dir, FakePlum};

    const A: &str = "11111111-1111-1111-1111-111111111111";
    const B: &str = "22222222-2222-2222-2222-222222222222";

    fn legacy_tags_db(dir: &Path, with_pins: bool) -> PathBuf {
        let path = dir.join("tags.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE tag(id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, color TEXT);
             CREATE TABLE note_tag(note_guid TEXT NOT NULL, tag_id INTEGER NOT NULL,
                                   PRIMARY KEY(note_guid, tag_id));
             INSERT INTO tag(id, name) VALUES (7, '工作'), (9, 'home');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO note_tag VALUES (?1, 7), (?1, 9), (?2, 9), ('gone-note', 7)",
            params![A, B],
        )
        .unwrap();
        if with_pins {
            conn.execute_batch(
                "CREATE TABLE pinned_note(note_guid TEXT PRIMARY KEY, pinned_at INTEGER NOT NULL);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO pinned_note VALUES (?1, 1763510400000), ('gone-note', 5)",
                params![B],
            )
            .unwrap();
        }
        path
    }

    fn legacy_shadow_db(dir: &Path, rows: &[(&str, &str, &str)]) -> PathBuf {
        let path = dir.join("markdown.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE NoteMarkdown (note_id TEXT PRIMARY KEY, plum TEXT NOT NULL,
                                        markdown TEXT NOT NULL, saved_at INTEGER NOT NULL);",
        )
        .unwrap();
        for (id, plum, markdown) in rows {
            conn.execute(
                "INSERT INTO NoteMarkdown VALUES (?1, ?2, ?3, 0)",
                params![id, plum::normalize(plum), markdown],
            )
            .unwrap();
        }
        path
    }

    fn store_with_notes(ids: &[&str]) -> NoteStore {
        let store = NoteStore::open_in_memory().unwrap();
        for id in ids {
            store
                .conn
                .execute(
                    "INSERT INTO note (id, created_at, updated_at) VALUES (?1, 1, 1)",
                    params![id],
                )
                .unwrap();
        }
        store
    }

    #[test]
    fn shadow_copies_load_and_match_only_their_own_plum() {
        let dir = temp_dir("legacy-shadow");
        let path = legacy_shadow_db(&dir, &[(A, "\\id=11111111-1111-1111-1111-111111111111 标题", "# 标题")]);
        let shadow = ShadowMarkdown::load(&path);
        assert_eq!(shadow.len(), 1);
        assert_eq!(shadow.markdown_for(A, " 标题"), Some("# 标题"));
        assert_eq!(shadow.markdown_for(A, " 别的内容"), None);
        assert_eq!(shadow.markdown_for(B, " 标题"), None);

        // Missing or unrelated files are just "no copies" — and not created.
        let missing = dir.join("nope.sqlite");
        assert!(ShadowMarkdown::load(&missing).is_empty());
        assert!(!missing.exists());
        assert!(ShadowMarkdown::load(&legacy_tags_db(&dir, false)).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tags_links_and_pins_are_carried_over_for_live_notes() {
        let dir = temp_dir("legacy-tags");
        let store = store_with_notes(&[A, B]);
        let tags_db = legacy_tags_db(&dir, true);
        let report = import_tags(&store, &tags_db).unwrap();
        assert_eq!(report, LegacyTagsReport { tags: 2, links: 3, pins: 1 });

        let tags = store.list_tags().unwrap();
        let id_of = |name: &str| tags.iter().find(|t| t.name == name).unwrap().id;
        let mut links = store.note_tags().unwrap();
        links.sort();
        let mut expected = vec![
            (A.to_string(), id_of("工作")),
            (A.to_string(), id_of("home")),
            (B.to_string(), id_of("home")),
        ];
        expected.sort();
        assert_eq!(links, expected);
        assert_eq!(store.pinned_ids().unwrap(), [B]);

        // Importing twice adds nothing new.
        let again = import_tags(&store, &tags_db).unwrap();
        assert_eq!((again.tags, again.links), (0, 0));
        assert_eq!(store.list_tags().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_tags_file_without_the_pin_table_or_missing_entirely_is_fine() {
        let dir = temp_dir("legacy-tags-old");
        let store = store_with_notes(&[A]);
        let report = import_tags(&store, &legacy_tags_db(&dir, false)).unwrap();
        assert_eq!(report, LegacyTagsReport { tags: 2, links: 2, pins: 0 });

        let missing = dir.join("missing.db");
        assert_eq!(import_tags(&store, &missing).unwrap(), LegacyTagsReport::default());
        assert!(!missing.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn migration_moves_notes_markdown_tags_and_pins_exactly_once() {
        let dir = temp_dir("legacy-migrate");
        let md = "# 周计划\n\n- [ ] 写周报\n```sh\ncargo test\n```";
        let written = markup::markdown_to_plum(md);
        let plum = FakePlum::create(&dir);
        plum.put(A, &written, "Green", 100, 300);
        plum.put(B, "\\id=33333333-3333-3333-3333-333333333333 原生便签", "Pink", 100, 200);
        plum.put("trash", "\\id=44444444-4444-4444-4444-444444444444 已删", "Blue", 100, 250);
        plum.delete("trash", 260);
        let native_before = std::fs::read(&plum.path).unwrap();

        let sources = LegacySources {
            plum: Some(plum.path.clone()),
            shadow: legacy_shadow_db(&dir, &[(A, &written, md)]),
            tags: legacy_tags_db(&dir, true),
        };
        let store = NoteStore::open(&dir.join("data").join("stickynotes.db")).unwrap();
        assert!(!is_migrated(&store).unwrap());

        let report = migrate_once(&store, &sources).unwrap().expect("first run migrates");
        assert!(report.windows_notes_found);
        assert_eq!(report.notes, ImportReport { added: 2, updated: 0, unchanged: 0 });
        assert_eq!(report.tags, LegacyTagsReport { tags: 2, links: 3, pins: 1 });

        let live = store.live_notes().unwrap();
        assert_eq!(live.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), [A, B]);
        assert_eq!(live[0].markdown, md, "full Markdown, not the plain projection");
        assert_eq!((live[0].theme.as_str(), live[0].updated_at), ("Green", 300));
        assert_eq!(live[1].markdown, "原生便签");
        assert_eq!(store.pinned_ids().unwrap(), [B]);
        assert_eq!(store.note_tags().unwrap().len(), 3);

        // The old files were only read.
        assert_eq!(std::fs::read(&plum.path).unwrap(), native_before);

        // A second start does nothing, even though the sources still exist.
        assert!(is_migrated(&store).unwrap());
        store.soft_delete(B).unwrap();
        assert_eq!(migrate_once(&store, &sources).unwrap(), None);
        assert_eq!(store.note_count().unwrap(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn migration_without_windows_sticky_notes_still_completes() {
        let dir = temp_dir("legacy-none");
        let sources = LegacySources {
            plum: None,
            shadow: dir.join("markdown.sqlite"),
            tags: dir.join("tags.db"),
        };
        let store = NoteStore::open_in_memory().unwrap();
        let report = migrate_once(&store, &sources).unwrap().unwrap();
        assert_eq!(report, MigrationReport::default());
        assert!(is_migrated(&store).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_source_writes_nothing_and_is_retried() {
        let dir = temp_dir("legacy-broken");
        let broken = dir.join("plum.sqlite");
        std::fs::write(&broken, b"this is not a sqlite database at all, sorry").unwrap();
        let sources = LegacySources {
            plum: Some(broken),
            shadow: dir.join("markdown.sqlite"),
            tags: legacy_tags_db(&dir, true),
        };
        let store = NoteStore::open_in_memory().unwrap();
        assert!(migrate_once(&store, &sources).is_err());
        assert!(!is_migrated(&store).unwrap());
        assert!(store.list_tags().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_tags_file_does_not_hold_the_notes_back() {
        let dir = temp_dir("legacy-bad-tags");
        let plum = FakePlum::create(&dir);
        plum.put(A, "\\id=33333333-3333-3333-3333-333333333333 便签", "Blue", 100, 200);
        let bad_tags = dir.join("tags.db");
        std::fs::write(&bad_tags, b"this is not a sqlite database at all, sorry").unwrap();
        let sources = LegacySources {
            plum: Some(plum.path.clone()),
            shadow: dir.join("markdown.sqlite"),
            tags: bad_tags,
        };
        let store = NoteStore::open_in_memory().unwrap();
        let report = migrate_once(&store, &sources).unwrap().unwrap();
        assert!(report.tags_unreadable);
        assert_eq!(report.notes.added, 1);
        assert_eq!(report.tags, LegacyTagsReport::default());
        assert!(is_migrated(&store).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }
}
