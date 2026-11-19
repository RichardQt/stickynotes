//! Rolling snapshots of the note database.
//!
//! A snapshot is a complete, verified copy (`notes-<UTC stamp>.db`) kept in
//! the app-data folder — deliberately not next to the database, so a custom
//! data folder on another drive still has its backups here.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{NoteStore, Result};

/// Snapshots kept; the oldest are removed first.
pub const KEEP: usize = 30;
/// No new snapshot while the newest one is younger than this.
pub const MIN_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);

const PREFIX: &str = "notes-";
const SUFFIX: &str = ".db";

/// `%LOCALAPPDATA%\stickynotes\backups`.
pub fn default_dir() -> PathBuf {
    crate::config::config_dir().join("backups")
}

fn is_snapshot(name: &str) -> bool {
    name.starts_with(PREFIX) && name.ends_with(SUFFIX)
}

/// Names of the snapshots in `dir`, oldest first (the stamp sorts by time).
fn snapshot_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| is_snapshot(name))
        .collect();
    names.sort();
    names
}

/// Snapshot `store` into `dir` unless a recent snapshot exists or there is
/// nothing to protect yet. Returns the file written, if any.
pub fn backup_if_due(store: &NoteStore, dir: &Path) -> Result<Option<PathBuf>> {
    if store.is_in_memory() || store.note_count()? == 0 {
        return Ok(None);
    }
    let newest = snapshot_names(dir)
        .pop()
        .and_then(|name| fs::metadata(dir.join(name)).ok())
        .and_then(|meta| meta.modified().ok());
    if let Some(written) = newest {
        // A timestamp from the future (clock change) counts as "long ago".
        if written.elapsed().is_ok_and(|age| age < MIN_INTERVAL) {
            return Ok(None);
        }
    }
    snapshot(store, dir).map(Some)
}

/// Write a snapshot now and drop the ones beyond [`KEEP`].
pub fn snapshot(store: &NoteStore, dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = now_stamp();
    let mut dest = dir.join(format!("{PREFIX}{stamp}{SUFFIX}"));
    // Two snapshots within the same second get distinct names.
    let mut n = 1;
    while dest.exists() {
        n += 1;
        dest = dir.join(format!("{PREFIX}{stamp}-{n}{SUFFIX}"));
    }
    store.copy_to(&dest)?;
    prune(dir)?;
    Ok(dest)
}

/// Remove the oldest snapshots beyond [`KEEP`]. Only this module's own files
/// are touched; anything else in the folder stays.
fn prune(dir: &Path) -> Result<()> {
    let names = snapshot_names(dir);
    let excess = names.len().saturating_sub(KEEP);
    for name in &names[..excess] {
        fs::remove_file(dir.join(name))?;
    }
    Ok(())
}

fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, mo, d, h, mi, s) = civil_from_unix(secs as i64);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// Minimal UTC civil-date conversion (no chrono dependency).
fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::plum::testing::temp_dir;

    fn store_with_a_note(dir: &Path) -> NoteStore {
        let store = NoteStore::open(&dir.join("stickynotes.db")).unwrap();
        store.create_note("# 便签\n内容", "Yellow").unwrap();
        store
    }

    #[test]
    fn a_snapshot_is_a_complete_database() {
        let dir = temp_dir("backup-snapshot");
        let store = store_with_a_note(&dir);
        let backups = dir.join("backups");

        let file = snapshot(&store, &backups).unwrap();
        assert!(is_snapshot(&file.file_name().unwrap().to_string_lossy()));
        let copy = NoteStore::open(&file).unwrap();
        assert_eq!(copy.live_notes().unwrap(), store.live_notes().unwrap());

        // Same second: a second snapshot does not overwrite the first.
        let second = snapshot(&store, &backups).unwrap();
        assert_ne!(file, second);
        assert_eq!(snapshot_names(&backups).len(), 2);
        drop((store, copy));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_backup_is_due_only_without_a_recent_snapshot() {
        let dir = temp_dir("backup-due");
        let store = store_with_a_note(&dir);
        let backups = dir.join("backups");

        assert!(backup_if_due(&store, &backups).unwrap().is_some(), "first one");
        assert!(backup_if_due(&store, &backups).unwrap().is_none(), "too recent");

        // Age the snapshot past the interval.
        let name = snapshot_names(&backups).pop().unwrap();
        let old = std::time::SystemTime::now() - MIN_INTERVAL - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(backups.join(name))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(backup_if_due(&store, &backups).unwrap().is_some(), "due again");
        drop(store);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_is_backed_up_before_the_first_note() {
        let dir = temp_dir("backup-empty");
        let backups = dir.join("backups");
        let empty = NoteStore::open(&dir.join("stickynotes.db")).unwrap();
        assert!(backup_if_due(&empty, &backups).unwrap().is_none());
        let memory = NoteStore::open_in_memory().unwrap();
        memory.create_note("x", "Yellow").unwrap();
        assert!(backup_if_due(&memory, &backups).unwrap().is_none());
        assert!(!backups.exists());
        drop(empty);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pruning_keeps_the_newest_snapshots_and_nothing_else_is_touched() {
        let dir = temp_dir("backup-prune");
        let store = store_with_a_note(&dir);
        let backups = dir.join("backups");
        fs::create_dir_all(&backups).unwrap();

        // Backups of the old layout (a folder per backup) and a stray file.
        let legacy = backups.join("20261119-093333");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("plum.sqlite"), b"old").unwrap();
        fs::write(backups.join("readme.txt"), b"keep me").unwrap();
        for i in 0..KEEP + 4 {
            fs::write(backups.join(format!("notes-20200101-0000{i:02}.db")), b"x").unwrap();
        }

        let newest = snapshot(&store, &backups).unwrap();
        let names = snapshot_names(&backups);
        assert_eq!(names.len(), KEEP);
        assert_eq!(names.first().unwrap(), "notes-20200101-000005.db");
        assert_eq!(backups.join(names.last().unwrap()), newest);
        assert!(legacy.join("plum.sqlite").exists());
        assert!(backups.join("readme.txt").exists());
        drop(store);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stamps_are_utc_calendar_times() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
        // 2026-11-19 09:33:33 UTC
        assert_eq!(civil_from_unix(1_795_080_813), (2026, 11, 19, 9, 33, 33));
    }
}
