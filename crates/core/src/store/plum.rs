//! Read-only access to Windows Sticky Notes' `plum.sqlite`.
//!
//! Earlier versions of this app used that file as their own storage. It is
//! now only an import source: opened read-only and never written, so nothing
//! this app does can disturb the native app or its cloud sync.

use super::Result;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::time::Duration;

const PACKAGE_PREFIX: &str = "Microsoft.MicrosoftStickyNotes_";
const DEFAULT_PACKAGE: &str = "Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe";

/// Locate plum.sqlite under `%LOCALAPPDATA%\Packages`. `None` when Windows
/// Sticky Notes is not installed or has never been started.
pub fn find_plum_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    find_plum_in(Path::new(&local))
}

/// Same lookup below an explicit `%LOCALAPPDATA%` folder.
pub fn find_plum_in(local_app_data: &Path) -> Option<PathBuf> {
    let packages = local_app_data.join("Packages");
    let direct = packages
        .join(DEFAULT_PACKAGE)
        .join("LocalState")
        .join("plum.sqlite");
    if direct.exists() {
        return Some(direct);
    }
    // Other package suffixes (sideloaded or regional builds).
    for entry in std::fs::read_dir(&packages).ok()?.flatten() {
        if entry.file_name().to_string_lossy().starts_with(PACKAGE_PREFIX) {
            let candidate = entry.path().join("LocalState").join("plum.sqlite");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// One live row of the native app's `Note` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlumNote {
    pub id: String,
    /// plum markup, see [`crate::markup`].
    pub text: String,
    pub theme: Option<String>,
    /// .NET ticks (UTC).
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

/// Every note that is not in the native app's trash, newest first.
pub fn read_live_notes(path: &Path) -> Result<Vec<PlumNote>> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // The native app may be mid-write; wait for it instead of failing.
    conn.busy_timeout(Duration::from_millis(5000))?;
    let mut stmt = conn.prepare(
        "SELECT Id, Text, Theme, CreatedAt, UpdatedAt FROM Note \
         WHERE DeletedAt IS NULL ORDER BY UpdatedAt DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(PlumNote {
            id: row.get(0)?,
            text: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            theme: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Strip paragraph GUIDs so a rewrite that only re-mints ids (the native app
/// does this on sync) is still recognised as the same content.
pub fn normalize(plum: &str) -> String {
    let mut out = String::with_capacity(plum.len());
    for (i, line) in plum.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match line.strip_prefix("\\id=") {
            // `\id=` + 36-char GUID; keep the remainder verbatim.
            Some(rest) if rest.len() >= 36 && rest.is_char_boundary(36) => {
                out.push_str(&rest[36..])
            }
            _ => out.push_str(line),
        }
    }
    out
}

/// A fake plum database with the exact `Note` table shape of the real one, so
/// nothing in the test suite has to touch the user's notes.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use rusqlite::params;

    pub fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stickynotes-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub struct FakePlum {
        pub path: PathBuf,
    }

    impl FakePlum {
        pub fn create(dir: &Path) -> Self {
            let path = dir.join("plum.sqlite");
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE Note (
                     Text varchar, WindowPosition varchar, IsOpen integer,
                     IsAlwaysOnTop integer, CreationNoteIdAnchor varchar, Theme varchar,
                     IsFutureNote integer, RemoteId varchar, ChangeKey varchar,
                     LastServerVersion varchar, RemoteSchemaVersion integer,
                     IsRemoteDataInvalid integer, PendingInsightsScan integer,
                     Type varchar, Id varchar primary key not null, ParentId varchar,
                     CreatedAt bigint, DeletedAt bigint, UpdatedAt bigint);",
            )
            .unwrap();
            Self { path }
        }

        pub fn put(&self, id: &str, text: &str, theme: &str, created: i64, updated: i64) {
            let conn = Connection::open(&self.path).unwrap();
            conn.execute(
                "INSERT INTO Note (Id, Text, Theme, CreatedAt, UpdatedAt) VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(Id) DO UPDATE SET Text = excluded.Text, Theme = excluded.Theme, \
                 UpdatedAt = excluded.UpdatedAt",
                params![id, text, theme, created, updated],
            )
            .unwrap();
        }

        pub fn delete(&self, id: &str, at: i64) {
            let conn = Connection::open(&self.path).unwrap();
            conn.execute(
                "UPDATE Note SET DeletedAt = ?1, UpdatedAt = ?1 WHERE Id = ?2",
                params![at, id],
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn reads_live_rows_newest_first_and_skips_the_trash() {
        let dir = temp_dir("plum-read");
        let plum = FakePlum::create(&dir);
        plum.put("id-old", "\\id=a old", "Yellow", 10, 20);
        plum.put("id-new", "\\id=b new", "Blue", 10, 30);
        plum.put("id-dead", "\\id=c dead", "Pink", 10, 40);
        plum.delete("id-dead", 50);

        let notes = read_live_notes(&plum.path).unwrap();
        let ids: Vec<&str> = notes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["id-new", "id-old"]);
        assert_eq!(notes[0].theme.as_deref(), Some("Blue"));
        assert_eq!(notes[0].updated_at, Some(30));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn null_text_reads_as_an_empty_note() {
        let dir = temp_dir("plum-null");
        let plum = FakePlum::create(&dir);
        Connection::open(&plum.path)
            .unwrap()
            .execute("INSERT INTO Note (Id, CreatedAt, UpdatedAt) VALUES ('id-1', 1, 2)", [])
            .unwrap();
        let notes = read_live_notes(&plum.path).unwrap();
        assert_eq!(notes[0].text, "");
        assert_eq!(notes[0].theme, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reading_never_modifies_the_native_database() {
        let dir = temp_dir("plum-readonly");
        let plum = FakePlum::create(&dir);
        plum.put("id-1", "\\id=a text", "Yellow", 1, 2);
        let before = std::fs::read(&plum.path).unwrap();
        read_live_notes(&plum.path).unwrap();
        assert_eq!(std::fs::read(&plum.path).unwrap(), before);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finds_the_database_below_local_app_data() {
        let dir = temp_dir("plum-find");
        assert_eq!(find_plum_in(&dir), None);

        let state = dir
            .join("Packages")
            .join("Microsoft.MicrosoftStickyNotes_otherpublisher")
            .join("LocalState");
        std::fs::create_dir_all(&state).unwrap();
        assert_eq!(find_plum_in(&dir), None, "package without a database");
        std::fs::write(state.join("plum.sqlite"), b"").unwrap();
        assert_eq!(find_plum_in(&dir), Some(state.join("plum.sqlite")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn normalize_drops_paragraph_guids_only() {
        let a = "\\id=11111111-1111-1111-1111-111111111111 标题\n\\id=22222222-2222-2222-2222-222222222222\\b 正文\\b0 ";
        let b = "\\id=33333333-3333-3333-3333-333333333333 标题\n\\id=44444444-4444-4444-4444-444444444444\\b 正文\\b0 ";
        assert_eq!(normalize(a), normalize(b));
        assert_eq!(normalize(a), " 标题\n\\b 正文\\b0 ");
        assert_eq!(normalize("plain"), "plain");
        assert_eq!(normalize("\\id=short x"), "\\id=short x");
        // Not a GUID and not sliceable at byte 36: left alone instead of panicking.
        let odd = format!("\\id=a{}", "中".repeat(13));
        assert!(!odd["\\id=".len()..].is_char_boundary(36));
        assert_eq!(normalize(&odd), odd);
    }
}
